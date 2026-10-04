//! HLS（m3u8）下载：选最高码率，并行拉分片，再按顺序拼接。
//! 加密流这一版不解，直接告诉用户。

use crate::core::http::build_client;
use crate::core::io::{atomic_replace, atomic_write, concat_files_with_control, create_temp_file};
use crate::core::limiter::DownloadLimiter;
use crate::core::progress::{wait_interruptible, Control, JobOutcome, LiveProgress};
use crate::core::urlx::filename_from_url;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use url::Url;

/// HLS 分片很短，过多线程只会增加文件句柄和上下文切换。
const MAX_HLS_WORKERS_PER_TASK: u32 = 16;
const SEGMENT_RETRIES: u32 = 3;
const SEGMENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

pub struct HlsJob<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub connections: u32,
    pub ua: &'a str,
    pub referer: Option<&'a str>,
    pub cookies: Option<&'a str>,
    pub progress: Arc<LiveProgress>,
    pub ctrl: Control,
    pub limiter: DownloadLimiter,
    pub proxy: Option<&'a str>,
}

#[derive(Debug, Clone)]
struct Variant {
    bandwidth: u64,
    uri: String,
}

#[derive(Debug, Clone)]
struct Segment {
    uri: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct HlsResume {
    media_url: String,
    segments: Vec<String>,
}

fn done_marker(path: &Path) -> PathBuf {
    let mut marker = path.as_os_str().to_os_string();
    marker.push(".done");
    PathBuf::from(marker)
}

fn hls_manifest_matches(path: &Path, media_url: &str, segs: &[Segment]) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(old) = serde_json::from_str::<HlsResume>(&text) else {
        return false;
    };
    old.media_url == media_url
        && old.segments == segs.iter().map(|s| s.uri.clone()).collect::<Vec<_>>()
}

fn save_hls_manifest(path: &Path, media_url: &str, segs: &[Segment]) -> std::io::Result<()> {
    let manifest = HlsResume {
        media_url: media_url.to_string(),
        segments: segs.iter().map(|s| s.uri.clone()).collect(),
    };
    let text = serde_json::to_vec(&manifest).map_err(|e| std::io::Error::other(e.to_string()))?;
    atomic_write(path, &text)
}

fn segment_is_complete(path: &Path) -> bool {
    let marker = done_marker(path);
    let Ok(marker_text) = std::fs::read_to_string(marker) else {
        return false;
    };
    let Ok(expected) = marker_text.trim().parse::<u64>() else {
        return false;
    };
    if expected == 0 {
        return false;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.is_file() && meta.len() == expected
}

fn unsupported_hls_features(text: &str) -> Option<&'static str> {
    if text.lines().any(|line| {
        let line = line.trim();
        line.starts_with("#EXT-X-MAP:") || line.starts_with("#EXT-X-BYTERANGE:")
    }) {
        Some("暂不支持 HLS 的 EXT-X-MAP/EXT-X-BYTERANGE")
    } else if text.lines().any(|line| {
        line.trim_start()
            .strip_prefix("#EXT-X-KEY:")
            .is_some_and(|attrs| !attrs.contains("METHOD=NONE"))
    }) {
        Some("暂不支持加密 HLS")
    } else {
        None
    }
}

pub fn download_hls(job: HlsJob<'_>) -> JobOutcome {
    if job.ctrl.interrupted() {
        return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), 0);
    }
    let client = match build_client(job.ua, job.proxy) {
        Ok(c) => c,
        Err(e) => return JobOutcome::Failed(e),
    };
    let Some(_playlist_permit) = crate::core::budget::acquire_network_worker(&job.ctrl) else {
        return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), 0);
    };
    let text = match fetch_text(&client, job.url, job.referer, job.cookies, &job.ctrl) {
        Ok(t) => t,
        Err(e) if job.ctrl.interrupted() => {
            return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), 0);
        }
        Err(e) => return JobOutcome::Failed(e),
    };
    if let Some(error) = unsupported_hls_features(&text) {
        return JobOutcome::Failed(error.into());
    }

    let media_url = if looks_like_master(&text) {
        match pick_best_variant(&text, job.url) {
            Some(u) => u,
            None => return JobOutcome::Failed("主播放列表中没有可用码率".into()),
        }
    } else {
        job.url.to_string()
    };

    let media = if media_url == job.url {
        text
    } else {
        match fetch_text(&client, &media_url, job.referer, job.cookies, &job.ctrl) {
            Ok(t) => t,
            Err(e) if job.ctrl.interrupted() => {
                return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), 0);
            }
            Err(e) => return JobOutcome::Failed(e),
        }
    };
    drop(_playlist_permit);
    if let Some(error) = unsupported_hls_features(&media) {
        return JobOutcome::Failed(error.into());
    }

    let segs = parse_segments(&media, &media_url);
    if segs.is_empty() {
        return JobOutcome::Failed("播放列表中没有分片".into());
    }

    let tmp_dir = job.dest.with_extension("hls-tmp");
    let manifest_path = tmp_dir.join("manifest.json");
    if tmp_dir.exists() && !hls_manifest_matches(&manifest_path, &media_url, &segs) {
        if let Err(e) = std::fs::remove_dir_all(&tmp_dir) {
            return JobOutcome::Failed(format!("无法清理旧 HLS 临时目录：{e}"));
        }
    }
    if std::fs::create_dir_all(&tmp_dir).is_err() {
        return JobOutcome::Failed("无法创建临时目录".into());
    }
    if job.ctrl.interrupted() {
        return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), segs.len() as u64);
    }
    if let Err(e) = save_hls_manifest(&manifest_path, &media_url, &segs) {
        return JobOutcome::Failed(format!("无法保存 HLS 断点信息：{e}"));
    }

    let n = job.connections.clamp(1, MAX_HLS_WORKERS_PER_TASK) as usize;
    let paths: Vec<PathBuf> = (0..segs.len())
        .map(|i| tmp_dir.join(format!("seg-{i:06}")))
        .collect();
    let completed = paths
        .iter()
        .filter(|path| segment_is_complete(path))
        .count() as u64;
    job.progress.set_total(segs.len() as u64);
    job.progress.downloaded.store(completed, Ordering::Relaxed);

    let errors = std::sync::Mutex::new(Vec::<String>::new());
    let next = AtomicUsize::new(0);
    let remaining = segs.len().saturating_sub(completed as usize);
    std::thread::scope(|scope| {
        let next = &next;
        let segs = &segs;
        let paths = &paths;
        for _ in 0..n.min(remaining) {
            // 初始 permit 只覆盖第一个网络请求；后续请求按次获取，避免
            // 某个 HLS worker 在退避或本地落盘时长期占住全局网络预算。
            let Some(permit) = crate::core::budget::acquire_network_worker(&job.ctrl) else {
                break;
            };
            let client = client.clone();
            let progress = job.progress.clone();
            let ctrl = job.ctrl.clone();
            let limiter = job.limiter.clone();
            let referer = job.referer.map(|s| s.to_string());
            let cookies = job.cookies.map(|s| s.to_string());
            let errors = &errors;
            scope.spawn(move || {
                let mut first_permit = Some(permit);
                loop {
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    if idx >= segs.len() || ctrl.interrupted() {
                        return;
                    }
                    let uri = segs[idx].uri.clone();
                    let dest = paths[idx].clone();
                    let marker = done_marker(&dest);
                    if segment_is_complete(&dest) {
                        // 已完成分片已计入初始化进度。
                        continue;
                    }
                    // A stale marker must never survive a new download attempt.
                    let _ = std::fs::remove_file(&marker);
                    let mut last_error = None;
                    for attempt in 0..=SEGMENT_RETRIES {
                        if ctrl.interrupted() {
                            return;
                        }
                        let permit = match first_permit.take() {
                            Some(permit) => permit,
                            None => match crate::core::budget::acquire_network_worker(&ctrl) {
                                Some(permit) => permit,
                                None => return,
                            },
                        };
                        let result = {
                            let _permit = permit;
                            fetch_file(
                                &client,
                                &uri,
                                &dest,
                                referer.as_deref(),
                                cookies.as_deref(),
                                &ctrl,
                                &limiter,
                            )
                        };
                        match result {
                            Ok(size) => {
                                // marker 只在整个分片原子发布后创建；没有 marker 的文件
                                // 会在下次启动时被当作未完成分片重新下载。
                                if let Err(e) = atomic_write(&marker, size.to_string().as_bytes()) {
                                    let _ = std::fs::remove_file(&dest);
                                    last_error = Some(format!("保存分片完成标记失败：{e}"));
                                } else {
                                    progress.add(1);
                                    last_error = None;
                                    break;
                                }
                            }
                            Err(e) => {
                                last_error = Some(e);
                            }
                        }
                        if attempt < SEGMENT_RETRIES
                            && wait_interruptible(
                                &ctrl,
                                std::time::Duration::from_millis(200 * (attempt + 1) as u64),
                            )
                        {
                            return;
                        }
                    }
                    if let Some(error) = last_error {
                        if let Ok(mut g) = errors.lock() {
                            g.push(format!("分片 {idx}: {error}"));
                        }
                    }
                }
            });
        }
    });

    if job.ctrl.interrupted() {
        // 暂停保留已完成分片和 manifest，恢复时无需重复下载；取消则清理。
        if job.ctrl.is_stop() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }
        return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), segs.len() as u64);
    }
    if let Ok(g) = errors.lock() {
        if let Some(e) = g.first() {
            // 失败也保留完整分片，用户重试时只需补齐缺失部分。
            return JobOutcome::Failed(e.clone());
        }
    }

    if job.ctrl.interrupted() {
        if job.ctrl.is_stop() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }
        return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), segs.len() as u64);
    }
    match concat_files_with_control(&paths, job.dest, Some(&job.ctrl)) {
        Ok(size) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            job.progress.set_total(size);
            job.progress.downloaded.store(size, Ordering::Relaxed);
            JobOutcome::Completed {
                size,
                path: job.dest.to_path_buf(),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
            if job.ctrl.is_stop() {
                let _ = std::fs::remove_dir_all(&tmp_dir);
            }
            JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), segs.len() as u64)
        }
        Err(e) => JobOutcome::Failed(e.to_string()),
    }
}

fn fetch_text(
    client: &reqwest::blocking::Client,
    url: &str,
    referer: Option<&str>,
    cookies: Option<&str>,
    ctrl: &Control,
) -> Result<String, String> {
    if ctrl.interrupted() {
        return Err("下载已暂停或取消".into());
    }
    let req =
        crate::core::http::attach_auth(client.get(url).timeout(SEGMENT_TIMEOUT), referer, cookies);
    let resp = req.send().map_err(|e| e.to_string())?;
    if ctrl.interrupted() {
        return Err("下载已暂停或取消".into());
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = resp.text().map_err(|e| e.to_string())?;
    if ctrl.interrupted() {
        return Err("下载已暂停或取消".into());
    }
    Ok(text)
}

fn fetch_file(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    referer: Option<&str>,
    cookies: Option<&str>,
    ctrl: &Control,
    limiter: &DownloadLimiter,
) -> Result<u64, String> {
    if ctrl.interrupted() {
        return Err("下载已暂停或取消".into());
    }
    let req =
        crate::core::http::attach_auth(client.get(url).timeout(SEGMENT_TIMEOUT), referer, cookies);
    let mut resp = req.send().map_err(|e| e.to_string())?;
    if ctrl.interrupted() {
        return Err("下载已暂停或取消".into());
    }
    if resp.status().as_u16() != 200 {
        return Err(format!("HTTP {}", resp.status()));
    }
    let expected = resp.content_length();
    let (temp, mut file) = create_temp_file(dest).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let allowed = limiter.acquire(buf.len(), ctrl);
            if allowed == 0 {
                return Err("下载已暂停或取消".into());
            }
            let n = resp.read(&mut buf[..allowed]).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            total += n as u64;
        }
        if ctrl.interrupted() {
            return Err("下载已暂停或取消".into());
        }
        if total == 0 {
            return Err("空的分片响应".into());
        }
        if expected.is_some_and(|expected| expected != total) {
            return Err(format!(
                "分片长度不符：收到 {total}，应为 {}",
                expected.unwrap()
            ));
        }
        file.flush().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        atomic_replace(&temp, dest).map_err(|e| e.to_string())?;
        Ok(total)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn looks_like_master(text: &str) -> bool {
    text.contains("#EXT-X-STREAM-INF")
}

fn pick_best_variant(text: &str, base: &str) -> Option<String> {
    let mut best: Option<Variant> = None;
    let mut pending_bw = 0u64;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending_bw = parse_attr_u64(rest, "BANDWIDTH").unwrap_or(0);
        } else if !line.is_empty() && !line.starts_with('#') && pending_bw > 0 {
            let uri = resolve(base, line);
            if best.as_ref().map(|v| v.bandwidth).unwrap_or(0) < pending_bw {
                best = Some(Variant {
                    bandwidth: pending_bw,
                    uri,
                });
            }
            pending_bw = 0;
        }
    }
    best.map(|v| v.uri)
}

fn parse_segments(text: &str, base: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.is_empty() && !line.starts_with('#') {
            out.push(Segment {
                uri: resolve(base, line),
            });
        }
    }
    out
}

fn parse_attr_u64(s: &str, key: &str) -> Option<u64> {
    let pat = format!("{key}=");
    let idx = s.find(&pat)?;
    let rest = &s[idx + pat.len()..];
    let token = rest.split(',').next()?.trim();
    token.parse().ok()
}

fn resolve(base: &str, rel: &str) -> String {
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return rel.to_string();
    }
    match Url::parse(base).and_then(|u| u.join(rel)) {
        Ok(u) => u.to_string(),
        Err(_) => rel.to_string(),
    }
}

pub fn default_hls_filename(url: &str) -> String {
    let mut name = filename_from_url(url);
    if name.ends_with(".m3u8") {
        name = name.trim_end_matches(".m3u8").to_string();
        if name.is_empty() {
            name = "stream".into();
        }
        name.push_str(".ts");
    } else if !name.contains('.') {
        name.push_str(".ts");
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_highest_bandwidth() {
        let text = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=800000
low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2500000
high.m3u8
"#;
        let u = pick_best_variant(text, "https://cdn.example/master.m3u8").unwrap();
        assert_eq!(u, "https://cdn.example/high.m3u8");
    }

    #[test]
    fn parses_media_segments() {
        let text = r#"#EXTM3U
#EXTINF:4.0,
a.ts
#EXTINF:4.0,
b.ts
"#;
        let segs = parse_segments(text, "https://cdn.example/play.m3u8");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].uri, "https://cdn.example/a.ts");
        assert_eq!(segs[1].uri, "https://cdn.example/b.ts");
    }

    #[test]
    fn resolve_absolute_keeps_itself() {
        assert_eq!(
            resolve("https://a/x.m3u8", "https://b/c.ts"),
            "https://b/c.ts"
        );
    }

    #[test]
    fn invalid_or_truncated_done_marker_is_not_complete() {
        let dir = std::env::temp_dir().join(format!("qg-hls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("seg-000000");
        let marker = done_marker(&path);
        std::fs::write(&path, b"segment").unwrap();
        std::fs::write(&marker, b"done").unwrap();
        assert!(!segment_is_complete(&path));
        std::fs::write(&marker, b"3").unwrap();
        assert!(!segment_is_complete(&path));
        std::fs::write(&marker, b"7\n").unwrap();
        assert!(segment_is_complete(&path));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unsupported_features_are_rejected() {
        assert!(unsupported_hls_features("#EXT-X-MAP:URI=\"init.mp4\"").is_some());
        assert!(unsupported_hls_features("#EXT-X-BYTERANGE:100@0").is_some());
        assert!(unsupported_hls_features("#EXT-X-KEY:METHOD=SAMPLE-AES").is_some());
        assert!(unsupported_hls_features("#EXT-X-KEY:METHOD=NONE").is_none());
    }

    #[test]
    fn fetch_file_rejects_short_content_length_without_publishing() {
        use std::io::{Read as _, Write as _};
        use std::net::{Shutdown, TcpListener};
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nabc",
                )
                .unwrap();
            let _ = stream.shutdown(Shutdown::Both);
        });
        let dir = std::env::temp_dir().join(format!("qg-hls-fetch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("segment");
        let client = build_client("quickget-test", None).unwrap();
        let result = fetch_file(
            &client,
            &format!("http://{addr}/segment.ts"),
            &dest,
            None,
            None,
            &Control::new(),
            &DownloadLimiter::new(0),
        );
        assert!(result.is_err());
        assert!(!dest.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
