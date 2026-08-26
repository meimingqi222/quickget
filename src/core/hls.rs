//! HLS（m3u8）下载：选最高码率，并行拉分片，再按顺序拼接。
//! 加密流这一版不解，直接告诉用户。

use crate::core::http::build_client;
use crate::core::io::concat_files;
use crate::core::limiter::DownloadLimiter;
use crate::core::progress::{Control, JobOutcome, LiveProgress};
use crate::core::urlx::filename_from_url;
use std::io::Read;
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

pub fn download_hls(job: HlsJob<'_>) -> JobOutcome {
    let client = match build_client(job.ua, job.proxy) {
        Ok(c) => c,
        Err(e) => return JobOutcome::Failed(e),
    };
    let text = match fetch_text(&client, job.url, job.referer, job.cookies) {
        Ok(t) => t,
        Err(e) => return JobOutcome::Failed(e),
    };
    if text.contains("#EXT-X-KEY") && text.contains("METHOD=AES") {
        return JobOutcome::Failed("暂不支持加密 HLS".into());
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
        match fetch_text(&client, &media_url, job.referer, job.cookies) {
            Ok(t) => t,
            Err(e) => return JobOutcome::Failed(e),
        }
    };
    if media.contains("#EXT-X-KEY") && media.contains("METHOD=AES") {
        return JobOutcome::Failed("暂不支持加密 HLS".into());
    }

    let segs = parse_segments(&media, &media_url);
    if segs.is_empty() {
        return JobOutcome::Failed("播放列表中没有分片".into());
    }

    let tmp_dir = job.dest.with_extension("hls-tmp");
    if std::fs::create_dir_all(&tmp_dir).is_err() {
        return JobOutcome::Failed("无法创建临时目录".into());
    }

    job.progress.set_total(segs.len() as u64);
    job.progress.downloaded.store(0, Ordering::Relaxed);

    let n = job.connections.clamp(1, MAX_HLS_WORKERS_PER_TASK) as usize;
    let paths: Vec<PathBuf> = (0..segs.len())
        .map(|i| tmp_dir.join(format!("seg-{i:06}")))
        .collect();

    let errors = std::sync::Mutex::new(Vec::<String>::new());
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let next = &next;
        let segs = &segs;
        let paths = &paths;
        for _ in 0..n.min(segs.len()) {
            let client = client.clone();
            let progress = job.progress.clone();
            let ctrl = job.ctrl.clone();
            let limiter = job.limiter.clone();
            let referer = job.referer.map(|s| s.to_string());
            let cookies = job.cookies.map(|s| s.to_string());
            let errors = &errors;
            scope.spawn(move || loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                if idx >= segs.len() || ctrl.interrupted() {
                    return;
                }
                let uri = segs[idx].uri.clone();
                let dest = paths[idx].clone();
                    if ctrl.interrupted() {
                        return;
                    }
                    let Some(_permit) = crate::core::budget::acquire_network_worker(&ctrl) else {
                        return;
                    };
                    let mut last_error = None;
                    for attempt in 0..=SEGMENT_RETRIES {
                        if ctrl.interrupted() {
                            return;
                        }
                        match fetch_file(
                            &client,
                            &uri,
                            &dest,
                            referer.as_deref(),
                            cookies.as_deref(),
                            &ctrl,
                            &limiter,
                        ) {
                            Ok(_) => {
                                progress.add(1);
                                return;
                            }
                            Err(e) => {
                                last_error = Some(e);
                                if attempt < SEGMENT_RETRIES {
                                    std::thread::sleep(std::time::Duration::from_millis(
                                        200 * (attempt + 1) as u64,
                                    ));
                                }
                            }
                        }
                    }
                    if let (Some(error), Ok(mut g)) = (last_error, errors.lock()) {
                        g.push(format!("分片 {idx}: {error}"));
                    }
            });
        }
    });

    if job.ctrl.interrupted() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), segs.len() as u64);
    }
    if let Ok(g) = errors.lock() {
        if let Some(e) = g.first() {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return JobOutcome::Failed(e.clone());
        }
    }

    match concat_files(&paths, job.dest) {
        Ok(size) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            job.progress.set_total(size);
            job.progress.downloaded.store(size, Ordering::Relaxed);
            JobOutcome::Completed {
                size,
                path: job.dest.to_path_buf(),
            }
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            JobOutcome::Failed(e.to_string())
        }
    }
}

fn fetch_text(
    client: &reqwest::blocking::Client,
    url: &str,
    referer: Option<&str>,
    cookies: Option<&str>,
) -> Result<String, String> {
    let req = crate::core::http::attach_auth(client.get(url).timeout(SEGMENT_TIMEOUT), referer, cookies);
    let resp = req.send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.text().map_err(|e| e.to_string())
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
    let req = crate::core::http::attach_auth(client.get(url).timeout(SEGMENT_TIMEOUT), referer, cookies);
    let mut resp = req.send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
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
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(|e| e.to_string())?;
        total += n as u64;
    }
    Ok(total)
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
}
