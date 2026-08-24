//! BitTorrent：磁力链接与 `.torrent` 文件。
//!
//! 引擎本身是阻塞的，BT 底层用独立的 tokio 运行时跑 `librqbit`。
//! 下完就从 session 里拿掉，不长期做种。

use crate::core::model::{BtPeers, TaskFile};
use crate::core::progress::{Control, JobOutcome, LiveMeta, LiveProgress};
use crate::core::urlx::{
    filename_from_magnet, filename_from_source, is_bt_placeholder, sanitize_filename, unique_path,
};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerMode, ListenerOptions, Session,
    SessionOptions,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// 单种子 Peer 上限。librqbit 默认 128，热门种子再抬一点把带宽打满。
const BT_PEER_LIMIT: usize = 200;

fn worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(4, 16)
}

/// 公共 UDP tracker。磁力链接 tracker 少时补上，加快找人。
fn extra_public_trackers() -> HashSet<url::Url> {
    [
        "udp://tracker.opentrackr.org:1337/announce",
        "udp://open.stealth.si:80/announce",
        "udp://tracker.torrent.eu.org:451/announce",
        "udp://exodus.desync.com:6969/announce",
        "udp://tracker.moeking.me:6969/announce",
        "udp://explodie.org:6969/announce",
    ]
    .into_iter()
    .filter_map(|u| url::Url::parse(u).ok())
    .collect()
}

fn session_options(listen: bool) -> SessionOptions {
    SessionOptions {
        client_name_and_version: Some(format!("QuickGet/{}", env!("CARGO_PKG_VERSION"))),
        listen: listen.then_some(ListenerOptions {
            mode: ListenerMode::TcpAndUtp,
            enable_upnp_port_forwarding: true,
            ..Default::default()
        }),
        peer_limit: Some(BT_PEER_LIMIT),
        concurrent_init_limit: Some(8),
        runtime_worker_threads: Some(worker_threads()),
        ..Default::default()
    }
}

fn extra_tracker_list() -> Vec<String> {
    extra_public_trackers()
        .into_iter()
        .map(|u| u.to_string())
        .collect()
}

fn download_add_opts(dest: &Path, private: bool) -> AddTorrentOptions {
    AddTorrentOptions {
        overwrite: true,
        output_folder: Some(dest.to_string_lossy().into_owned()),
        trackers: if private {
            None
        } else {
            Some(extra_tracker_list())
        },
        ..Default::default()
    }
}

async fn drop_session_torrents_at(session: &Arc<Session>, dest: &Path) {
    let ids: Vec<usize> = session.with_torrents(|it| {
        it.filter(|(_, h)| h.output_folder() == dest)
            .map(|(id, _)| id)
            .collect()
    });
    for id in ids {
        let _ = session
            .delete(librqbit::api::TorrentIdOrHash::Id(id), false)
            .await;
    }
}

pub struct BtJob<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub progress: Arc<LiveProgress>,
    pub meta: LiveMeta,
    pub ctrl: Control,
}

fn runtime() -> tokio::runtime::Handle {
    static HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();
    HANDLE
        .get_or_init(|| {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(worker_threads())
                .enable_all()
                .thread_name("quickget-bt")
                .build()
                .expect("无法创建 BT 运行时");
            let handle = rt.handle().clone();
            std::thread::Builder::new()
                .name("quickget-bt-rt".into())
                .spawn(move || {
                    rt.block_on(std::future::pending::<()>());
                })
                .expect("无法启动 BT 运行时线程");
            handle
        })
        .clone()
}

async fn session() -> Result<Arc<Session>, String> {
    static SESSION: OnceLock<tokio::sync::Mutex<Option<Arc<Session>>>> = OnceLock::new();
    let slot = SESSION.get_or_init(|| tokio::sync::Mutex::new(None));
    let mut guard = slot.lock().await;
    if let Some(s) = guard.as_ref() {
        return Ok(s.clone());
    }
    let dir = crate::core::settings::default_download_dir();
    let s = match Session::new_with_opts(dir.clone(), session_options(true)).await {
        Ok(s) => {
            crate::core::log::write(format_args!(
                "BT 会话已启动（TCP+uTP 入站，UPnP，Peer 上限 {BT_PEER_LIMIT}）"
            ));
            s
        }
        Err(e) => {
            crate::core::log::write(format_args!("BT 入站监听失败，改用仅出站：{e}"));
            Session::new_with_opts(dir, session_options(false))
                .await
                .map_err(|e2| format!("无法启动 BT 会话：{e2}"))?
        }
    };
    *guard = Some(s.clone());
    Ok(s)
}

fn add_from_url(url: &str) -> Result<AddTorrent<'static>, String> {
    let path = Path::new(url);
    if path.is_file() {
        return AddTorrent::from_local_filename(url).map_err(|e| e.to_string());
    }
    if let Some(rest) = url.strip_prefix("file://") {
        let decoded = percent_encoding::percent_decode_str(rest)
            .decode_utf8_lossy()
            .into_owned();
        let local = decoded.trim_start_matches('/');
        let local = if cfg!(windows) && local.len() >= 2 && local.as_bytes()[1] == b':' {
            local.to_string()
        } else {
            decoded
        };
        return AddTorrent::from_local_filename(&local).map_err(|e| e.to_string());
    }
    Ok(AddTorrent::from_url(url.to_string()))
}

async fn wait_interrupt(ctrl: &Control) {
    loop {
        if ctrl.interrupted() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn torrent_files(handle: &librqbit::ManagedTorrent) -> Vec<TaskFile> {
    let progress = handle.stats().file_progress;
    handle
        .with_metadata(|m| {
            m.file_infos
                .iter()
                .enumerate()
                .filter_map(|(i, fi)| {
                    if fi.attrs.padding {
                        return None;
                    }
                    Some(TaskFile {
                        path: fi.relative_filename.to_string_lossy().replace('\\', "/"),
                        size: fi.len,
                        downloaded: progress.get(i).copied().unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn torrent_display_name(handle: &librqbit::ManagedTorrent, files: &[TaskFile]) -> Option<String> {
    let from_meta = handle.name().and_then(|n| {
        let s = sanitize_filename(&n);
        if s.is_empty() || is_bt_placeholder(&s) {
            None
        } else {
            Some(s)
        }
    });
    if from_meta.is_some() {
        return from_meta;
    }
    name_from_files(files)
}

fn name_from_files(files: &[TaskFile]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    if files.len() == 1 {
        let last = files[0]
            .path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&files[0].path);
        let s = sanitize_filename(last);
        if !s.is_empty() && !is_bt_placeholder(&s) {
            return Some(s);
        }
        return None;
    }
    let roots: Vec<&str> = files
        .iter()
        .map(|f| f.path.split(['/', '\\']).next().unwrap_or(f.path.as_str()))
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(root) = roots.first() {
        if roots.iter().all(|r| r == root) {
            let s = sanitize_filename(root);
            if !s.is_empty() && !is_bt_placeholder(&s) {
                return Some(s);
            }
        }
    }
    None
}

fn publish_meta(handle: &librqbit::ManagedTorrent, meta: &LiveMeta) {
    let files = torrent_files(handle);
    if let Some(name) = torrent_display_name(handle, &files) {
        let prev = meta.snapshot();
        if prev.filename.as_deref() != Some(name.as_str()) {
            crate::core::log::write(format_args!("BT 标题: {name}"));
        }
        meta.set_filename(name);
    }
    if !files.is_empty() {
        meta.set_files(files);
    }
    meta.set_output_dir(handle.output_folder().to_path_buf());
    if let Some(live) = handle.stats().live {
        let p = live.snapshot.peer_stats;
        meta.set_peers(BtPeers {
            live: p.live,
            live_tcp: p.live_tcp,
            live_utp: p.live_utp,
            connecting: p.connecting,
            seen: p.seen,
        });
    }
}

fn pick_torrent_name(meta_name: Option<&str>, files: &[TaskFile], dest: &Path) -> String {
    if let Some(n) = meta_name {
        let s = sanitize_filename(n);
        if !s.is_empty() && !is_bt_placeholder(&s) {
            return s;
        }
    }
    if let Some(n) = name_from_files(files) {
        return n;
    }
    dest.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "BT".into())
}

/// 元数据到了就把占位目录改成种子真名。目录还不存在时只返回目标路径，不先建成 `BT`。
fn prepare_bt_folder(dest: &Path, name: &str) -> PathBuf {
    let Some(parent) = dest.parent() else {
        return dest.to_path_buf();
    };
    let current = dest
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = name.trim();
    if name.is_empty() || is_bt_placeholder(name) {
        return dest.to_path_buf();
    }
    if current == name {
        return dest.to_path_buf();
    }
    let (_, target) = unique_path(parent, name);
    if &target == dest {
        return dest.to_path_buf();
    }
    if dest.exists() {
        match std::fs::rename(dest, &target) {
            Ok(()) => target,
            Err(e) => {
                crate::core::log::write(format_args!(
                    "BT 文件夹改名失败 {} -> {}：{e}",
                    dest.display(),
                    target.display()
                ));
                dest.to_path_buf()
            }
        }
    } else {
        target
    }
}

/// 下载结束后再兜一次：session 已释放文件句柄，占位名还能改。
fn settle_bt_folder(folder: &Path, name: Option<&str>) -> PathBuf {
    let Some(name) = name.filter(|s| !s.is_empty()) else {
        return folder.to_path_buf();
    };
    prepare_bt_folder(folder, name)
}

async fn download_bt_async(
    url: String,
    mut dest: PathBuf,
    progress: Arc<LiveProgress>,
    meta: LiveMeta,
    ctrl: Control,
) -> JobOutcome {
    let session = match session().await {
        Ok(s) => s,
        Err(e) => return JobOutcome::Failed(e),
    };
    if ctrl.interrupted() {
        return JobOutcome::from_interrupt(&ctrl, progress.downloaded(), progress.total());
    }

    let add = match add_from_url(&url) {
        Ok(a) => a,
        Err(e) => return JobOutcome::Failed(e),
    };

    crate::core::log::write(format_args!("BT 解析 {}", url));
    let listed = tokio::select! {
        r = session.add_torrent(
            add,
            Some(AddTorrentOptions {
                list_only: true,
                ..Default::default()
            }),
        ) => r,
        _ = wait_interrupt(&ctrl) => {
            return JobOutcome::from_interrupt(&ctrl, progress.downloaded(), progress.total());
        }
    };
    let listed = match listed {
        Ok(AddTorrentResponse::ListOnly(l)) => l,
        Ok(_) => return JobOutcome::Failed("BT 元数据不完整".into()),
        Err(e) => return JobOutcome::Failed(e.to_string()),
    };

    let listed_files: Vec<TaskFile> = listed
        .info
        .iter_file_details()
        .filter_map(|fd| {
            if fd.attrs().padding {
                return None;
            }
            Some(TaskFile {
                path: fd.filename.to_pathbuf().to_string_lossy().replace('\\', "/"),
                size: fd.len,
                downloaded: 0,
            })
        })
        .collect();
    let meta_name = listed.info.name();
    let name = pick_torrent_name(meta_name.as_deref(), &listed_files, &dest);
    let private = listed.info.info().private;
    dest = prepare_bt_folder(&dest, &name);
    if let Err(e) = std::fs::create_dir_all(&dest) {
        return JobOutcome::Failed(format!("无法创建下载目录：{e}"));
    }
    meta.set_filename(name);
    if !listed_files.is_empty() {
        meta.set_files(listed_files);
    }
    meta.set_output_dir(dest.clone());

    let torrent_bytes = listed.torrent_bytes;
    let add = if torrent_bytes.is_empty() {
        match add_from_url(&url) {
            Ok(a) => a,
            Err(e) => return JobOutcome::Failed(e),
        }
    } else {
        AddTorrent::from_bytes(torrent_bytes.clone())
    };

    crate::core::log::write(format_args!("BT 加入 {} -> {}", url, dest.display()));
    let added = tokio::select! {
        r = session.add_torrent(add, Some(download_add_opts(&dest, private))) => r,
        _ = wait_interrupt(&ctrl) => {
            drop_session_torrents_at(&session, &dest).await;
            return JobOutcome::from_interrupt(&ctrl, progress.downloaded(), progress.total());
        }
    };
    let handle = match added {
        Ok(AddTorrentResponse::Added(_, h)) => h,
        Ok(AddTorrentResponse::AlreadyManaged(_, h)) => {
            if h.output_folder() != dest.as_path() {
                let old = h.id();
                let _ = session
                    .delete(librqbit::api::TorrentIdOrHash::Id(old), false)
                    .await;
                let add = if torrent_bytes.is_empty() {
                    match add_from_url(&url) {
                        Ok(a) => a,
                        Err(e) => return JobOutcome::Failed(e),
                    }
                } else {
                    AddTorrent::from_bytes(torrent_bytes)
                };
                match session
                    .add_torrent(add, Some(download_add_opts(&dest, private)))
                    .await
                {
                    Ok(AddTorrentResponse::Added(_, h2)) => h2,
                    Ok(AddTorrentResponse::AlreadyManaged(_, h2)) => {
                        let _ = session.unpause(&h2).await;
                        h2
                    }
                    Ok(AddTorrentResponse::ListOnly(_)) => {
                        return JobOutcome::Failed("BT 元数据不完整".into());
                    }
                    Err(e) => return JobOutcome::Failed(e.to_string()),
                }
            } else {
                let _ = session.unpause(&h).await;
                h
            }
        }
        Ok(AddTorrentResponse::ListOnly(_)) => {
            return JobOutcome::Failed("BT 元数据不完整".into());
        }
        Err(e) => return JobOutcome::Failed(e.to_string()),
    };

    let id = handle.id();
    let cleanup = {
        let session = session.clone();
        move || {
            let session = session.clone();
            async move {
                let _ = session
                    .delete(librqbit::api::TorrentIdOrHash::Id(id), false)
                    .await;
            }
        }
    };

    tokio::select! {
        r = handle.wait_until_initialized() => {
            if let Err(e) = r {
                cleanup().await;
                return JobOutcome::Failed(e.to_string());
            }
        }
        _ = wait_interrupt(&ctrl) => {
            cleanup().await;
            return JobOutcome::from_interrupt(&ctrl, progress.downloaded(), progress.total());
        }
    }

    publish_meta(&handle, &meta);
    let stats = handle.stats();
    progress.set_total(stats.total_bytes);
    progress.set_downloaded(stats.progress_bytes);

    loop {
        if ctrl.interrupted() {
            publish_meta(&handle, &meta);
            let downloaded = progress.downloaded();
            let total = progress.total();
            cleanup().await;
            return JobOutcome::from_interrupt(&ctrl, downloaded, total);
        }
        let stats = handle.stats();
        if stats.total_bytes > 0 {
            progress.set_total(stats.total_bytes);
        }
        progress.set_downloaded(stats.progress_bytes);
        publish_meta(&handle, &meta);
        if let Some(err) = stats.error.as_ref() {
            cleanup().await;
            return JobOutcome::Failed(err.clone());
        }
        if stats.finished {
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    let folder = handle.output_folder().to_path_buf();
    let files = torrent_files(&handle);
    let name = torrent_display_name(&handle, &files);
    let size = handle.stats().total_bytes.max(progress.downloaded());
    cleanup().await;
    let path = settle_bt_folder(&folder, name.as_deref());
    JobOutcome::Completed { size, path }
}

pub fn download_bt(job: BtJob<'_>) -> JobOutcome {
    let url = job.url.to_string();
    let dest = job.dest.to_path_buf();
    runtime().block_on(download_bt_async(
        url,
        dest,
        job.progress,
        job.meta,
        job.ctrl,
    ))
}

pub fn default_bt_filename(url: &str) -> String {
    let name = filename_from_source(url);
    if name.is_empty() {
        filename_from_magnet(url)
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_from_magnet_dn() {
        let u = "magnet:?xt=urn:btih:abc&dn=Ubuntu%2024.04";
        assert_eq!(default_bt_filename(u), "Ubuntu 24.04");
        assert_eq!(
            default_bt_filename("magnet:?xt=urn:btih:abcdef0123456789"),
            "BT"
        );
    }

    #[test]
    fn name_from_single_and_multi_files() {
        assert_eq!(
            name_from_files(&[TaskFile {
                path: "movie.mkv".into(),
                size: 1,
                downloaded: 0,
            }])
            .as_deref(),
            Some("movie.mkv")
        );
        assert_eq!(
            name_from_files(&[
                TaskFile {
                    path: "Album/a.flac".into(),
                    size: 1,
                    downloaded: 0,
                },
                TaskFile {
                    path: "Album/b.flac".into(),
                    size: 1,
                    downloaded: 0,
                },
            ])
            .as_deref(),
            Some("Album")
        );
    }

    #[test]
    fn settle_renames_placeholder_folder() {
        let root = std::env::temp_dir().join(format!("qg-bt-{}", uuid::Uuid::new_v4()));
        let src = root.join("BT");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.bin"), b"ok").unwrap();
        let got = settle_bt_folder(&src, Some("Ubuntu 24.04"));
        assert_eq!(got, root.join("Ubuntu 24.04"));
        assert!(got.join("a.bin").is_file());
        assert!(!src.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn settle_keeps_matching_name() {
        let root = std::env::temp_dir().join(format!("qg-bt-{}", uuid::Uuid::new_v4()));
        let src = root.join("Album");
        std::fs::create_dir_all(&src).unwrap();
        let got = settle_bt_folder(&src, Some("Album"));
        assert_eq!(got, src);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn prepare_uses_real_name_without_creating_placeholder() {
        let root = std::env::temp_dir().join(format!("qg-bt-{}", uuid::Uuid::new_v4()));
        let placeholder = root.join("BT");
        let got = prepare_bt_folder(&placeholder, "Ubuntu 24.04");
        assert_eq!(got, root.join("Ubuntu 24.04"));
        assert!(!placeholder.exists());
        assert!(!got.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn prepare_renames_existing_placeholder() {
        let root = std::env::temp_dir().join(format!("qg-bt-{}", uuid::Uuid::new_v4()));
        let placeholder = root.join("BT");
        std::fs::create_dir_all(&placeholder).unwrap();
        std::fs::write(placeholder.join("part.bin"), b"x").unwrap();
        let got = prepare_bt_folder(&placeholder, "Album");
        assert_eq!(got, root.join("Album"));
        assert!(got.join("part.bin").is_file());
        assert!(!placeholder.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pick_name_prefers_meta_over_placeholder_dest() {
        let dest = PathBuf::from("/tmp/BT");
        assert_eq!(
            pick_torrent_name(Some("Ubuntu 24.04"), &[], &dest),
            "Ubuntu 24.04"
        );
        assert_eq!(pick_torrent_name(Some("BT"), &[], &dest), "BT");
        assert_eq!(
            pick_torrent_name(
                None,
                &[TaskFile {
                    path: "movie.mkv".into(),
                    size: 1,
                    downloaded: 0,
                }],
                &dest
            ),
            "movie.mkv"
        );
    }

    #[test]
    fn extra_trackers_are_udp() {
        let set = extra_public_trackers();
        assert!(set.len() >= 4);
        assert!(set.iter().all(|u| u.scheme() == "udp"));
    }

    #[test]
    fn public_trackers_skipped_for_private_torrents() {
        let dest = PathBuf::from("/tmp/x");
        assert!(download_add_opts(&dest, true).trackers.is_none());
        let public = download_add_opts(&dest, false).trackers.expect("public");
        assert!(!public.is_empty());
    }

    #[test]
    fn session_options_open_incoming_when_listen() {
        let on = session_options(true);
        assert_eq!(on.peer_limit, Some(BT_PEER_LIMIT));
        assert_eq!(on.concurrent_init_limit, Some(8));
        let listen = on.listen.expect("应开入站");
        assert!(matches!(listen.mode, ListenerMode::TcpAndUtp));
        assert!(listen.enable_upnp_port_forwarding);
        assert!(on.trackers.is_empty());

        let off = session_options(false);
        assert!(off.listen.is_none());
        assert_eq!(off.peer_limit, Some(BT_PEER_LIMIT));
    }
}
