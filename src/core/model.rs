//! 任务模型与格式化工具。

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

use crate::core::urlx::{url_identity, Protocol};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Queued,
    Probing,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn is_active(self) -> bool {
        matches!(self, TaskStatus::Probing | TaskStatus::Downloading)
    }

    pub fn is_open(self) -> bool {
        matches!(
            self,
            TaskStatus::Queued | TaskStatus::Probing | TaskStatus::Downloading | TaskStatus::Paused
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub save_dir: PathBuf,
    pub protocol: Protocol,
    pub status: TaskStatus,
    pub size: u64,
    pub downloaded: u64,
    pub connections: u32,
    pub error: Option<String>,
    pub created_at: i64,
    /// 本次下载尝试实际开始的时间；排队期间不计入耗时。
    #[serde(default)]
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub referer: Option<String>,
    /// 浏览器扩展带过来的 Cookie 头，登录态下载靠它。
    #[serde(default)]
    pub cookies: Option<String>,
    /// 覆盖设置里的 UA。扩展会带上浏览器真实 UA。
    #[serde(default)]
    pub user_agent: Option<String>,
    /// BT 种子里的文件。HTTP/FTP/HLS 通常为空，界面会用主文件名顶上。
    #[serde(default)]
    pub files: Vec<TaskFile>,
    /// BT 实际落盘目录。标题换成种子真名后 dest_path 仍指向这个文件夹。
    #[serde(default)]
    pub output_dir: Option<PathBuf>,
    /// 单个 Range 请求的最大字节数。None = 按 connections 均分。
    /// 百度 PCS 直链对单次 Range > 4MB 回 31326 风控，需限制每片大小。
    #[serde(default)]
    pub max_part_size: Option<u64>,
    /// 下载完成后需清理的网盘路径（转存临时文件）。
    /// None = 非转存方式，无需清理。
    #[serde(default)]
    pub cleanup_paths: Option<Vec<String>>,
    /// 网盘 Cookie，用于下载完成后清理转存文件。
    #[serde(default)]
    pub cleanup_cookies: Option<String>,
    /// 当前 Peer 快照。只给界面看，不写任务文件。
    #[serde(default, skip)]
    pub peers: BtPeers,
}

/// BT 节点汇总。Tracker / DHT / 入站都算进这里。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BtPeers {
    pub live: u32,
    pub live_tcp: u32,
    pub live_utp: u32,
    pub connecting: u32,
    pub seen: u32,
}

impl BtPeers {
    pub fn is_idle(self) -> bool {
        self.live == 0 && self.connecting == 0 && self.seen == 0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskFile {
    pub path: String,
    pub size: u64,
    #[serde(default)]
    pub downloaded: u64,
}

impl Task {
    /// 构造一个新下载任务，填入最常用的字段，其余用合理的默认值。
    /// 调用方可再按需覆盖 `referer` / `cookies` / `user_agent` / `output_dir` /
    /// `max_part_size` / `started_at` 等公开字段。
    pub fn new(
        protocol: Protocol,
        url: String,
        filename: String,
        save_dir: PathBuf,
        connections: u32,
        max_part_size: Option<u64>,
        output_dir: Option<PathBuf>,
    ) -> Task {
        let now = chrono::Utc::now().timestamp();
        Task {
            id: uuid::Uuid::new_v4().to_string(),
            url,
            filename,
            save_dir,
            protocol,
            status: TaskStatus::Downloading,
            size: 0,
            downloaded: 0,
            connections,
            error: None,
            created_at: now,
            started_at: Some(now),
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files: Vec::new(),
            output_dir,
            max_part_size,
            cleanup_paths: None,
            cleanup_cookies: None,
            peers: Default::default(),
        }
    }

    pub fn dest_path(&self) -> PathBuf {
        if self.protocol == Protocol::Magnet {
            if let Some(dir) = &self.output_dir {
                if !dir.as_os_str().is_empty() {
                    return dir.clone();
                }
            }
        }
        self.save_dir.join(&self.filename)
    }

    pub fn part_path(&self) -> PathBuf {
        let mut p = self.dest_path().into_os_string();
        p.push(".part");
        PathBuf::from(p)
    }

    pub fn meta_path(&self) -> PathBuf {
        let mut p = self.dest_path().into_os_string();
        p.push(".qg.json");
        PathBuf::from(p)
    }

    /// 删除任务时送进废纸篓的路径：成品、`.part`、`.qg.json`、HLS 临时目录。
    /// BT 只收任务自己的文件夹，不扫下载根目录里的同名文件。
    pub fn leftover_paths(&self) -> Vec<PathBuf> {
        let dest = self.dest_path();
        let mut out = Vec::new();
        self.push_leftover(&mut out, dest.clone());
        if let Some(dir) = &self.output_dir {
            self.push_leftover(&mut out, dir.clone());
        }

        if self.protocol == Protocol::Magnet {
            return out;
        }

        self.push_leftover(&mut out, self.save_dir.join(&self.filename));

        self.push_leftover(&mut out, self.part_path());
        self.push_leftover(&mut out, self.meta_path());
        if !self.filename.to_ascii_lowercase().ends_with(".zip") {
            let zip = self.save_dir.join(format!("{}.zip", self.filename));
            let mut part = zip.clone().into_os_string();
            part.push(".part");
            let mut meta = zip.clone().into_os_string();
            meta.push(".qg.json");
            self.push_leftover(&mut out, zip);
            self.push_leftover(&mut out, PathBuf::from(part));
            self.push_leftover(&mut out, PathBuf::from(meta));
        }
        let mut hls = dest.as_os_str().to_os_string();
        hls.push(".hls-tmp");
        self.push_leftover(&mut out, PathBuf::from(hls));
        if let Some(stem) = dest.file_stem() {
            self.push_leftover(
                &mut out,
                self.save_dir
                    .join(format!("{}.hls-tmp", stem.to_string_lossy())),
            );
        }
        out
    }

    fn push_leftover(&self, out: &mut Vec<PathBuf>, path: PathBuf) {
        if path.as_os_str().is_empty() {
            return;
        }
        if path == self.save_dir {
            return;
        }
        if !path.starts_with(&self.save_dir) {
            return;
        }
        if out.iter().any(|p| p == &path) {
            return;
        }
        out.push(path);
    }

    pub fn fraction(&self) -> f32 {
        if self.size == 0 {
            if self.status == TaskStatus::Completed {
                1.0
            } else {
                0.0
            }
        } else {
            (self.downloaded as f64 / self.size as f64).clamp(0.0, 1.0) as f32
        }
    }

    pub fn elapsed_secs(&self) -> Option<u64> {
        let start = self.started_at.or(if self.created_at > 0 {
            Some(self.created_at)
        } else {
            None
        })?;
        let end = self.finished_at?;
        Some((end.saturating_sub(start) as u64).max(1))
    }

    pub fn average_speed(&self) -> Option<u64> {
        let elapsed = self.elapsed_secs()?;
        let bytes = if self.size > 0 {
            self.size
        } else {
            self.downloaded
        };
        if bytes == 0 {
            return None;
        }
        Some(bytes / elapsed.max(1))
    }

    /// 详情面板用的文件列表。BT 用种子清单；其它协议就是目标文件本身。
    pub fn display_files(&self) -> Vec<TaskFile> {
        if !self.files.is_empty() {
            return self.files.clone();
        }
        vec![TaskFile {
            path: self.filename.clone(),
            size: self.size,
            downloaded: self.downloaded,
        }]
    }

    pub fn file_disk_path(&self, file: &TaskFile) -> PathBuf {
        let Some(rel) = safe_relative(&file.path) else {
            return self.dest_path();
        };
        if self.protocol == Protocol::Magnet {
            let nested = self.dest_path().join(&rel);
            if nested.exists() {
                return nested;
            }
            let flat = self.save_dir.join(&rel);
            if flat.exists() {
                return flat;
            }
            nested
        } else if self.files.len() <= 1 {
            self.dest_path()
        } else {
            self.dest_path().join(rel)
        }
    }
}

/// `adopt_existing_task` 命中已有任务之后的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdoptResult {
    /// 排队 / 探测 / 下载中：只更新凭证，不新建。
    Active(String),
    /// 暂停 / 失败 / 取消，或已完成但文件没了：重新入队。
    Requeued(String),
    /// 已完成且目标还在：不重下。
    Completed(String),
}

impl AdoptResult {
    pub fn name(&self) -> &str {
        match self {
            Self::Active(n) | Self::Requeued(n) | Self::Completed(n) => n,
        }
    }
}

/// 同一资源（去掉 sid 等 query）已在列表里：更新凭证，失败的重新排队。
/// 已完成且文件还在则保持完成；文件没了则重新入队。
pub fn adopt_existing_task(
    tasks: &mut [Task],
    url: &str,
    cookies: Option<String>,
    referer: Option<String>,
    ua: Option<String>,
) -> Option<AdoptResult> {
    let key = url_identity(url);
    let idx = tasks
        .iter()
        .position(|t| t.status != TaskStatus::Completed && url_identity(&t.url) == key)
        .or_else(|| {
            tasks
                .iter()
                .position(|t| t.status == TaskStatus::Completed && url_identity(&t.url) == key)
        })?;
    let t = &mut tasks[idx];
    t.url = url.to_string();
    if cookies.as_ref().is_some_and(|s| !s.is_empty()) {
        t.cookies = cookies;
    }
    if referer.as_ref().is_some_and(|s| !s.is_empty()) {
        t.referer = referer;
    }
    if ua.as_ref().is_some_and(|s| !s.is_empty()) {
        t.user_agent = ua;
    }
    let name = t.filename.clone();
    match t.status {
        TaskStatus::Paused | TaskStatus::Failed | TaskStatus::Cancelled => {
            t.status = TaskStatus::Queued;
            t.error = None;
            t.started_at = None;
            t.finished_at = None;
            Some(AdoptResult::Requeued(name))
        }
        TaskStatus::Completed => {
            if t.dest_path().exists() {
                Some(AdoptResult::Completed(name))
            } else {
                t.status = TaskStatus::Queued;
                t.error = None;
                t.started_at = None;
                t.finished_at = None;
                Some(AdoptResult::Requeued(name))
            }
        }
        _ => Some(AdoptResult::Active(name)),
    }
}

/// 种子相对路径：拒绝绝对路径和 `..`，避免删到保存目录外面。
fn safe_relative(path: &str) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let p = Path::new(path);
    if p.is_absolute() {
        return None;
    }
    if p.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return None;
    }
    Some(p.to_path_buf())
}

/// 格式化字节大小。macOS 用 1000 进制，Windows 用 1024。
pub fn fmt_size(bytes: u64) -> String {
    const BASE: f64 = if cfg!(windows) { 1024.0 } else { 1000.0 };
    const KB: f64 = BASE;
    const MB: f64 = KB * BASE;
    const GB: f64 = MB * BASE;
    const TB: f64 = GB * BASE;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.2} TB", b / TB)
    } else if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn fmt_speed(bps: u64) -> String {
    format!("{}/s", fmt_size(bps))
}

pub fn fmt_eta(remaining: u64, bps: u64) -> String {
    if bps == 0 {
        return "--".into();
    }
    let secs = remaining / bps;
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

pub fn fmt_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}

/// 根据文件大小建议连接数：小文件少开，大文件吃满上限。
pub fn suggested_connections(size: u64, max: u32) -> u32 {
    let max = max.clamp(1, 64);
    let want = if size == 0 {
        max.min(8)
    } else if size < 1_000_000 {
        1
    } else if size < 8_000_000 {
        4
    } else if size < 50_000_000 {
        8
    } else {
        max
    };
    want.min(max).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("短名", 22), "短名");
        assert_eq!(truncate("一二三四", 2), "一二…");
    }

    #[test]
    fn suggested_connections_scales() {
        assert_eq!(suggested_connections(100, 16), 1);
        assert_eq!(suggested_connections(3_000_000, 16), 4);
        assert_eq!(suggested_connections(20_000_000, 16), 8);
        assert_eq!(suggested_connections(200_000_000, 16), 16);
        assert_eq!(suggested_connections(200_000_000, 4), 4);
    }

    #[test]
    fn leftover_paths_include_zip_sidecars() {
        let t = Task {
            id: "1".into(),
            url: "https://ex.com/a.SAFE".into(),
            filename: "a.SAFE".into(),
            save_dir: PathBuf::from("/tmp"),
            protocol: crate::core::urlx::Protocol::Http,
            status: TaskStatus::Failed,
            size: 1,
            downloaded: 0,
            connections: 1,
            error: None,
            created_at: 0,
            started_at: None,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files: Vec::new(),
            output_dir: None,
            max_part_size: None,
            cleanup_paths: None,
            cleanup_cookies: None,
            peers: BtPeers::default(),
        };
        let paths = t.leftover_paths();
        let s: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(s.iter().any(|p| p.ends_with("a.SAFE")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.part")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.qg.json")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.zip")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.zip.part")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.zip.qg.json")));
    }

    #[test]
    fn display_files_falls_back_to_filename() {
        let t = Task {
            id: "1".into(),
            url: "https://ex.com/a.bin".into(),
            filename: "a.bin".into(),
            save_dir: PathBuf::from("/tmp"),
            protocol: crate::core::urlx::Protocol::Http,
            status: TaskStatus::Downloading,
            size: 10,
            downloaded: 3,
            connections: 1,
            error: None,
            created_at: 0,
            started_at: None,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files: Vec::new(),
            output_dir: None,
            max_part_size: None,
            cleanup_paths: None,
            cleanup_cookies: None,
            peers: BtPeers::default(),
        };
        let files = t.display_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a.bin");
        assert_eq!(files[0].size, 10);
        assert_eq!(files[0].downloaded, 3);
    }

    #[test]
    fn file_disk_path_joins_multi_file() {
        let t = Task {
            id: "1".into(),
            url: "magnet:?xt=urn:btih:abc".into(),
            filename: "Album".into(),
            save_dir: PathBuf::from("/tmp"),
            protocol: crate::core::urlx::Protocol::Magnet,
            status: TaskStatus::Downloading,
            size: 20,
            downloaded: 0,
            connections: 1,
            error: None,
            created_at: 0,
            started_at: None,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files: vec![
                TaskFile {
                    path: "a.flac".into(),
                    size: 10,
                    downloaded: 0,
                },
                TaskFile {
                    path: "b.flac".into(),
                    size: 10,
                    downloaded: 0,
                },
            ],
            output_dir: None,
            max_part_size: None,
            cleanup_paths: None,
            cleanup_cookies: None,
            peers: BtPeers::default(),
        };
        let p = t.file_disk_path(&t.files[1]);
        assert!(p.ends_with("Album/b.flac") || p.ends_with(r"Album\b.flac"));
    }

    fn magnet_task(
        filename: &str,
        save_dir: &str,
        files: Vec<TaskFile>,
        output_dir: Option<PathBuf>,
    ) -> Task {
        Task {
            id: "1".into(),
            url: "magnet:?xt=urn:btih:abc".into(),
            filename: filename.into(),
            save_dir: PathBuf::from(save_dir),
            protocol: crate::core::urlx::Protocol::Magnet,
            status: TaskStatus::Completed,
            size: 20,
            downloaded: 20,
            connections: 1,
            error: None,
            created_at: 0,
            started_at: None,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files,
            output_dir,
            max_part_size: None,
            cleanup_paths: None,
            cleanup_cookies: None,
            peers: BtPeers::default(),
        }
    }

    #[test]
    fn peers_idle_when_all_zero() {
        assert!(BtPeers::default().is_idle());
        assert!(!BtPeers {
            live: 1,
            ..BtPeers::default()
        }
        .is_idle());
    }

    #[test]
    fn dest_path_prefers_bt_output_dir() {
        let t = magnet_task("Ubuntu", "/tmp", vec![], Some(PathBuf::from("/tmp/BT")));
        assert_eq!(t.dest_path(), PathBuf::from("/tmp/BT"));
    }

    #[test]
    fn leftover_paths_include_bt_folder_and_files() {
        let t = magnet_task(
            "Ubuntu",
            "/tmp",
            vec![
                TaskFile {
                    path: "a.iso".into(),
                    size: 10,
                    downloaded: 10,
                },
                TaskFile {
                    path: "docs/readme.txt".into(),
                    size: 10,
                    downloaded: 10,
                },
            ],
            Some(PathBuf::from("/tmp/BT")),
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(s.iter().any(|p| p == "/tmp/BT"));
        assert!(!s.iter().any(|p| p == "/tmp/Ubuntu"));
        assert!(!s.iter().any(|p| p == "/tmp/a.iso"));
        assert!(!s.iter().any(|p| p == "/tmp/docs"));
        assert!(!s.iter().any(|p| p == "/tmp"));
        assert!(!s.iter().any(|p| p.ends_with(".zip")));
    }

    #[test]
    fn leftover_paths_do_not_trash_title_sibling() {
        let t = magnet_task(
            "Ubuntu",
            "/tmp",
            vec![TaskFile {
                path: "disk.iso".into(),
                size: 1,
                downloaded: 1,
            }],
            Some(PathBuf::from("/tmp/BT (1)")),
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(s.iter().any(|p| p == "/tmp/BT (1)"));
        assert!(!s.iter().any(|p| p == "/tmp/Ubuntu"));
        assert!(!s.iter().any(|p| p == "/tmp/disk.iso"));
    }

    #[test]
    fn leftover_paths_include_flattened_bt_files() {
        let t = magnet_task(
            "Album",
            "/tmp",
            vec![
                TaskFile {
                    path: "Album/a.flac".into(),
                    size: 10,
                    downloaded: 10,
                },
                TaskFile {
                    path: "Album/b.flac".into(),
                    size: 10,
                    downloaded: 10,
                },
            ],
            None,
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(s.iter().any(|p| p == "/tmp/Album"));
        assert!(!s.iter().any(|p| p == "/tmp/a.flac"));
        assert!(!s.iter().any(|p| p == "/tmp"));
    }

    #[test]
    fn leftover_ignores_loose_files_in_save_dir() {
        let root = std::env::temp_dir().join(format!("qg-left-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.iso"), b"x").unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let t = magnet_task(
            "Album",
            root.to_str().unwrap(),
            vec![
                TaskFile {
                    path: "a.iso".into(),
                    size: 1,
                    downloaded: 1,
                },
                TaskFile {
                    path: "docs/readme.txt".into(),
                    size: 1,
                    downloaded: 1,
                },
            ],
            None,
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        let a = root.join("a.iso").to_string_lossy().replace('\\', "/");
        let docs = root.join("docs").to_string_lossy().replace('\\', "/");
        let dest = root.join("Album").to_string_lossy().replace('\\', "/");
        assert!(s.iter().any(|p| p == &dest));
        assert!(!s.iter().any(|p| p == &a));
        assert!(!s.iter().any(|p| p == &docs));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn leftover_paths_reject_parent_dir_escape() {
        let t = magnet_task(
            "Album",
            "/tmp",
            vec![TaskFile {
                path: "../secret.txt".into(),
                size: 1,
                downloaded: 1,
            }],
            Some(PathBuf::from("/tmp/Album")),
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(s.iter().all(|p| p.starts_with("/tmp/")));
        assert!(!s.iter().any(|p| p.contains("secret")));
    }

    fn http_task(url: &str, filename: &str, save_dir: &Path, status: TaskStatus) -> Task {
        Task {
            id: "1".into(),
            url: url.into(),
            filename: filename.into(),
            save_dir: save_dir.to_path_buf(),
            protocol: crate::core::urlx::Protocol::Http,
            status,
            size: 1,
            downloaded: 1,
            connections: 1,
            error: None,
            created_at: 0,
            started_at: None,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files: Vec::new(),
            output_dir: None,
            max_part_size: None,
            cleanup_paths: None,
            cleanup_cookies: None,
            peers: BtPeers::default(),
        }
    }

    #[test]
    fn adopt_skips_new_url() {
        let mut tasks = vec![http_task(
            "https://ex.com/a.zip",
            "a.zip",
            Path::new("/tmp"),
            TaskStatus::Downloading,
        )];
        assert!(
            adopt_existing_task(&mut tasks, "https://ex.com/b.zip", None, None, None).is_none()
        );
    }

    #[test]
    fn adopt_active_strips_query_and_merges_cookies() {
        let mut tasks = vec![http_task(
            "https://ex.com/a.zip?sid=old",
            "a.zip",
            Path::new("/tmp"),
            TaskStatus::Downloading,
        )];
        let r = adopt_existing_task(
            &mut tasks,
            "https://ex.com/a.zip?sid=new",
            Some("k=v".into()),
            Some("https://ex.com/".into()),
            Some("UA".into()),
        );
        assert_eq!(r, Some(AdoptResult::Active("a.zip".into())));
        assert_eq!(tasks[0].url, "https://ex.com/a.zip?sid=new");
        assert_eq!(tasks[0].cookies.as_deref(), Some("k=v"));
        assert_eq!(tasks[0].status, TaskStatus::Downloading);
    }

    #[test]
    fn adopt_requeues_paused() {
        let mut tasks = vec![http_task(
            "https://ex.com/a.zip",
            "a.zip",
            Path::new("/tmp"),
            TaskStatus::Paused,
        )];
        let r = adopt_existing_task(&mut tasks, "https://ex.com/a.zip", None, None, None);
        assert_eq!(r, Some(AdoptResult::Requeued("a.zip".into())));
        assert_eq!(tasks[0].status, TaskStatus::Queued);
    }

    #[test]
    fn adopt_completed_file_still_there() {
        let dir = std::env::temp_dir().join(format!("qg-adopt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.zip"), b"x").unwrap();
        let mut tasks = vec![http_task(
            "https://ex.com/a.zip?sid=1",
            "a.zip",
            &dir,
            TaskStatus::Completed,
        )];
        let r = adopt_existing_task(&mut tasks, "https://ex.com/a.zip?sid=2", None, None, None);
        assert_eq!(r, Some(AdoptResult::Completed("a.zip".into())));
        assert_eq!(tasks[0].status, TaskStatus::Completed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adopt_completed_missing_file_requeues() {
        let dir = std::env::temp_dir().join(format!("qg-adopt-miss-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut tasks = vec![http_task(
            "https://ex.com/a.zip",
            "a.zip",
            &dir,
            TaskStatus::Completed,
        )];
        let r = adopt_existing_task(&mut tasks, "https://ex.com/a.zip", None, None, None);
        assert_eq!(r, Some(AdoptResult::Requeued("a.zip".into())));
        assert_eq!(tasks[0].status, TaskStatus::Queued);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adopt_prefers_live_task_over_completed() {
        let dir = std::env::temp_dir().join(format!("qg-adopt-pref-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.zip"), b"x").unwrap();
        let mut tasks = vec![
            http_task("https://ex.com/a.zip", "a.zip", &dir, TaskStatus::Completed),
            http_task(
                "https://ex.com/a.zip",
                "a (1).zip",
                &dir,
                TaskStatus::Downloading,
            ),
        ];
        let r = adopt_existing_task(&mut tasks, "https://ex.com/a.zip", None, None, None);
        assert_eq!(r, Some(AdoptResult::Active("a (1).zip".into())));
        assert_eq!(tasks[1].status, TaskStatus::Downloading);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
