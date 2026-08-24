//! 多连接 HTTP/HTTPS Range 下载。
//!
//! 先探体积和是否支持分段，能分段就按连接数切开，每个线程独立 TCP
//! 拉自己那一段，写到预分配文件的对应偏移。线程下完自己的段之后会把
//! 别人还没读的尾巴切开接着拉，避免收尾只剩一两根管子。不接 gzip。
//!
//! 探测用 `Range: bytes=0-1`，不用 `0-0`。部分 nginx（地理空间数据云
//! 的下载节点就是）把 `0-0` 当成非法范围直接 404，浏览器正常 GET 却
//! 能下。`0-1` 在这些机器上返回 206。

use crate::core::io::{finalize_part, open_part, preallocate, write_at};
use crate::core::model::suggested_connections;
use crate::core::progress::{Control, JobOutcome, LiveProgress};
use crate::core::urlx::filename_from_disposition;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const BUF: usize = 256 * 1024;
const RETRIES: u32 = 8;
/// 单次 HTTP Range 窗口。窗口小，空闲连接才能及时把尾巴切开。
const HTTP_WINDOW: u64 = 4 * 1024 * 1024;
/// 小于这个的未读尾巴不再拆，避免切得太碎。
const MIN_STEAL: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartMeta {
    pub start: u64,
    pub end: u64,
    pub done: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResumeMeta {
    pub url: String,
    pub size: u64,
    pub etag: Option<String>,
    pub parts: Vec<PartMeta>,
}

pub fn build_client(ua: &str) -> Result<Client, String> {
    Client::builder()
        .use_rustls_tls()
        .http1_only()
        .redirect(reqwest::redirect::Policy::limited(16))
        .connect_timeout(Duration::from_secs(15))
        .timeout(None)
        .tcp_nodelay(true)
        .pool_max_idle_per_host(0)
        .default_headers(default_headers(ua))
        .build()
        .map_err(|e| e.to_string())
}

fn default_headers(ua: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(ua) {
        h.insert(reqwest::header::USER_AGENT, v);
    }
    h.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("*/*"),
    );
    h.insert(
        reqwest::header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    h
}

pub fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

pub struct HttpJob<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub part: &'a Path,
    pub meta_path: &'a Path,
    pub connections: u32,
    pub ua: &'a str,
    pub referer: Option<&'a str>,
    pub cookies: Option<&'a str>,
    pub progress: Arc<LiveProgress>,
    pub ctrl: Control,
}

pub fn download_http(job: HttpJob<'_>) -> JobOutcome {
    let client = match build_client(job.ua) {
        Ok(c) => c,
        Err(e) => return JobOutcome::Failed(e),
    };

    let info = match probe_remote(&client, job.url, job.referer, job.cookies) {
        Ok(i) => i,
        Err(e) => return JobOutcome::Failed(e),
    };

    let mut dest = job.dest.to_path_buf();
    let mut part = job.part.to_path_buf();
    let mut meta = job.meta_path.to_path_buf();
    if let Some(name) = info.filename.as_ref() {
        crate::core::log::write(format_args!("HTTP 文件名: {name}"));
        let clean = crate::core::urlx::sanitize_filename(name);
        if !clean.is_empty() {
            let (d, p, m) = retarget_dest(dest, part, meta, &clean);
            dest = d;
            part = p;
            meta = m;
        }
    }
    let job = HttpJob {
        url: job.url,
        dest: &dest,
        part: &part,
        meta_path: &meta,
        connections: job.connections,
        ua: job.ua,
        referer: job.referer,
        cookies: job.cookies,
        progress: job.progress.clone(),
        ctrl: job.ctrl.clone(),
    };
    job.progress.set_total(info.size);

    if info.size > 0 && info.ranges {
        multi_range(&client, &job, &info)
    } else {
        single_stream(&client, &job, &info)
    }
}

pub struct RemoteInfo {
    pub final_url: String,
    pub size: u64,
    pub ranges: bool,
    pub filename: Option<String>,
    pub etag: Option<String>,
    pub content_type: String,
}

pub(crate) fn attach_auth(
    mut req: reqwest::blocking::RequestBuilder,
    referer: Option<&str>,
    cookies: Option<&str>,
) -> reqwest::blocking::RequestBuilder {
    if let Some(r) = referer {
        if !r.is_empty() {
            req = req.header("Referer", r);
        }
    }
    if let Some(c) = cookies {
        let c = c.trim();
        if !c.is_empty() {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(c) {
                req = req.header(reqwest::header::COOKIE, v);
            }
        }
    }
    req
}

/// 探体积与 Range 支持。
///
/// 顺序：`GET Range: bytes=0-1` → `HEAD` → 普通 `GET`（只读头就丢掉）。
/// 不用 `bytes=0-0`：部分 nginx 对它回 404，对 `0-1` 却正常 206。
pub fn probe_remote(
    client: &Client,
    url: &str,
    referer: Option<&str>,
    cookies: Option<&str>,
) -> Result<RemoteInfo, String> {
    let timeout = Duration::from_secs(20);

    let range_req = attach_auth(
        client
            .get(url)
            .header("Range", "bytes=0-1")
            .header("Accept-Encoding", "identity")
            .timeout(timeout),
        referer,
        cookies,
    );
    let range_resp = range_req.send().map_err(|e| e.to_string())?;
    let range_code = range_resp.status().as_u16();
    if range_code == 206 || range_resp.status().is_success() {
        return Ok(info_from_response(&range_resp, true));
    }
    drop(range_resp);

    let head_req = attach_auth(
        client
            .head(url)
            .header("Accept-Encoding", "identity")
            .timeout(timeout),
        referer,
        cookies,
    );
    if let Ok(head_resp) = head_req.send() {
        if head_resp.status().is_success() {
            return Ok(info_from_response(&head_resp, false));
        }
    }

    let get_req = attach_auth(
        client
            .get(url)
            .header("Accept-Encoding", "identity")
            .timeout(timeout),
        referer,
        cookies,
    );
    let get_resp = get_req.send().map_err(|e| e.to_string())?;
    if !get_resp.status().is_success() {
        return Err(format!("HTTP {}", get_resp.status()));
    }
    Ok(info_from_response(&get_resp, false))
}

fn info_from_response(resp: &reqwest::blocking::Response, range_requested: bool) -> RemoteInfo {
    let status = resp.status().as_u16();
    let headers = resp.headers();
    let mut size = 0u64;
    let mut ranges = status == 206;
    if let Some(cr) = header_str(headers, "content-range") {
        if let Some(total) = crate::core::probe::parse_content_range_total(&cr) {
            size = total;
            ranges = true;
        }
    }
    if size == 0 {
        if let Some(cl) = header_str(headers, "content-length") {
            if let Ok(n) = cl.parse::<u64>() {
                // Range 探测成功时 Content-Length 是这一段的长度，不是整文件。
                if status != 206 {
                    size = n;
                }
            }
        }
    }
    if !ranges {
        if let Some(ar) = header_str(headers, "accept-ranges") {
            ranges = ar.to_ascii_lowercase().contains("bytes") && size > 0;
        }
    }
    // 要了 Range 却拿到 200：服务器忽略了分段，不能当多连接用。
    if range_requested && status == 200 {
        ranges = false;
    }
    RemoteInfo {
        final_url: resp.url().to_string(),
        size,
        ranges,
        filename: header_str(headers, "content-disposition")
            .and_then(|h| filename_from_disposition(&h)),
        etag: header_str(headers, "etag"),
        content_type: header_str(headers, "content-type").unwrap_or_default(),
    }
}

fn split_ranges(size: u64, n: u32) -> Vec<(u64, u64)> {
    let n = (n as u64).max(1).min(size.max(1));
    let chunk = size / n;
    let mut out = Vec::with_capacity(n as usize);
    let mut start = 0u64;
    for i in 0..n {
        let extra = if i == n - 1 { size - chunk * n } else { 0 };
        let end = (start + chunk + extra).saturating_sub(1).min(size.saturating_sub(1));
        if start <= end {
            out.push((start, end));
        }
        start = end + 1;
    }
    out
}

fn sidecar_paths(dest: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let mut p = dest.as_os_str().to_os_string();
    p.push(".part");
    let mut m = dest.as_os_str().to_os_string();
    m.push(".qg.json");
    (std::path::PathBuf::from(p), std::path::PathBuf::from(m))
}

/// 用 Content-Disposition 的真名改落盘路径；已有 .part 则跟着改名，不断档。
fn retarget_dest(
    dest: std::path::PathBuf,
    part: std::path::PathBuf,
    meta: std::path::PathBuf,
    clean: &str,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let current = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if clean.is_empty() || clean == current {
        return (dest, part, meta);
    }
    let Some(dir) = dest.parent() else {
        return (dest, part, meta);
    };
    let new_dest = dir.join(clean);
    let (new_part, new_meta) = sidecar_paths(&new_dest);
    if part.exists() {
        if new_part != part && !new_part.exists() {
            let _ = std::fs::rename(&part, &new_part);
        }
        if meta.exists() && new_meta != meta && !new_meta.exists() {
            let _ = std::fs::rename(&meta, &new_meta);
        }
        let dest_out = if new_part.exists() || part.exists() {
            if new_part.exists() {
                new_dest
            } else {
                dest
            }
        } else {
            new_dest
        };
        let (p, m) = sidecar_paths(&dest_out);
        return (dest_out, p, m);
    }
    let (_, path) = crate::core::urlx::unique_path(dir, clean);
    let (p, m) = sidecar_paths(&path);
    (path, p, m)
}

fn load_resume(
    path: &Path,
    part_path: &Path,
    url: &str,
    size: u64,
    etag: &Option<String>,
    dest_name: &str,
) -> Option<ResumeMeta> {
    if !part_path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let mut meta: ResumeMeta = serde_json::from_str(&text).ok()?;
    if meta.size != size {
        return None;
    }
    if crate::core::urlx::url_identity(&meta.url) != crate::core::urlx::url_identity(url) {
        return None;
    }
    if etag.is_some() && meta.etag.is_some() && etag != &meta.etag {
        return None;
    }
    let expect_zip = crate::core::verify::filename_looks_zip(dest_name);
    for p in &mut meta.parts {
        p.done = crate::core::verify::trusted_done(
            part_path,
            p.start,
            p.done,
            expect_zip && p.start == 0,
        );
    }
    Some(meta)
}

fn save_resume(path: &Path, meta: &ResumeMeta) {
    if let Ok(text) = serde_json::to_string(meta) {
        let _ = std::fs::write(path, text);
    }
}

struct LivePart {
    start: u64,
    end: AtomicU64,
    written: AtomicU64,
    busy: AtomicBool,
}

fn part_len(start: u64, end: u64) -> u64 {
    if end < start {
        0
    } else {
        end - start + 1
    }
}

fn unread(part: &LivePart) -> u64 {
    let end = part.end.load(Ordering::Acquire);
    let pos = part.start.saturating_add(part.written.load(Ordering::Acquire));
    if pos > end {
        0
    } else {
        end - pos + 1
    }
}

fn snapshot_parts(pool: &[Arc<LivePart>]) -> Vec<PartMeta> {
    let mut v: Vec<PartMeta> = pool
        .iter()
        .map(|p| PartMeta {
            start: p.start,
            end: p.end.load(Ordering::Relaxed),
            done: p.written.load(Ordering::Relaxed),
        })
        .filter(|p| p.end >= p.start)
        .collect();
    v.sort_by_key(|p| p.start);
    v
}

fn lock_pool(pool: &Mutex<Vec<Arc<LivePart>>>) -> std::sync::MutexGuard<'_, Vec<Arc<LivePart>>> {
    pool.lock().unwrap_or_else(|p| p.into_inner())
}

/// 从最大的未读尾巴切开后半段。调用方须持有 pool 锁。
fn steal_from(pool: &mut Vec<Arc<LivePart>>) -> Option<Arc<LivePart>> {
    let mut best = None;
    let mut best_rem = MIN_STEAL.saturating_mul(2);
    for (i, p) in pool.iter().enumerate() {
        let rem = unread(p);
        if rem > best_rem {
            best_rem = rem;
            best = Some(i);
        }
    }
    let i = best?;
    let victim = pool[i].clone();
    let end = victim.end.load(Ordering::Acquire);
    let pos = victim.start.saturating_add(victim.written.load(Ordering::Acquire));
    if pos > end {
        return None;
    }
    let rem = end - pos + 1;
    if rem < MIN_STEAL.saturating_mul(2) {
        return None;
    }
    let steal_start = pos + rem / 2;
    if steal_start <= pos || steal_start > end {
        return None;
    }
    if victim
        .end
        .compare_exchange(end, steal_start - 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    let pos2 = victim
        .start
        .saturating_add(victim.written.load(Ordering::Acquire));
    if pos2 >= steal_start {
        victim.end.store(end, Ordering::Release);
        return None;
    }
    let stolen = Arc::new(LivePart {
        start: steal_start,
        end: AtomicU64::new(end),
        written: AtomicU64::new(0),
        busy: AtomicBool::new(true),
    });
    pool.push(stolen.clone());
    Some(stolen)
}

fn take_work(pool: &Mutex<Vec<Arc<LivePart>>>) -> Option<Arc<LivePart>> {
    let mut g = lock_pool(pool);
    for p in g.iter() {
        if unread(p) == 0 {
            continue;
        }
        if p.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Some(p.clone());
        }
    }
    steal_from(&mut g)
}

fn multi_range(client: &Client, job: &HttpJob<'_>, info: &RemoteInfo) -> JobOutcome {
    let n = suggested_connections(info.size, job.connections);
    let dest_name = job
        .dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let parts: Vec<PartMeta> = if let Some(old) = load_resume(
        job.meta_path,
        job.part,
        job.url,
        info.size,
        &info.etag,
        dest_name,
    ) {
        old.parts
    } else {
        split_ranges(info.size, n)
            .into_iter()
            .map(|(start, end)| PartMeta {
                start,
                end,
                done: 0,
            })
            .collect()
    };

    let already: u64 = parts.iter().map(|p| p.done).sum();
    job.progress
        .downloaded
        .store(already, Ordering::Relaxed);
    job.progress.last_bytes.store(already, Ordering::Relaxed);

    let file = match open_part(job.part) {
        Ok(f) => f,
        Err(e) => return JobOutcome::Failed(e.to_string()),
    };
    preallocate(&file, info.size);
    drop(file);

    if parts
        .iter()
        .all(|p| p.done >= part_len(p.start, p.end))
    {
        return finish_ok(job, info.size);
    }

    let live: Vec<Arc<LivePart>> = parts
        .iter()
        .map(|p| {
            Arc::new(LivePart {
                start: p.start,
                end: AtomicU64::new(p.end),
                written: AtomicU64::new(p.done),
                busy: AtomicBool::new(false),
            })
        })
        .collect();
    let worker_n = n.max(1) as usize;
    let mut initials: Vec<Option<Arc<LivePart>>> = vec![None; worker_n];
    let mut k = 0usize;
    for p in &live {
        if unread(p) == 0 {
            continue;
        }
        if k >= worker_n {
            break;
        }
        p.busy.store(true, Ordering::Release);
        initials[k] = Some(p.clone());
        k += 1;
    }

    let pool = Arc::new(Mutex::new(live));
    std::thread::scope(|scope| {
        for assigned in initials {
            let client = client.clone();
            let url = job.url.to_string();
            let part_path = job.part.to_path_buf();
            let progress = job.progress.clone();
            let ctrl = job.ctrl.clone();
            let referer = job.referer.map(|s| s.to_string());
            let cookies = job.cookies.map(|s| s.to_string());
            let pool = pool.clone();
            let meta_path = job.meta_path.to_path_buf();
            let job_url = job.url.to_string();
            let etag = info.etag.clone();
            let size = info.size;
            scope.spawn(move || {
                range_worker(
                    &client,
                    &url,
                    &part_path,
                    assigned,
                    referer.as_deref(),
                    cookies.as_deref(),
                    &progress,
                    &ctrl,
                    &pool,
                    &meta_path,
                    &job_url,
                    etag.as_deref(),
                    size,
                );
            });
        }
    });

    let snap = snapshot_parts(&lock_pool(&pool));
    save_resume(
        job.meta_path,
        &ResumeMeta {
            url: job.url.to_string(),
            size: info.size,
            etag: info.etag.clone(),
            parts: snap.clone(),
        },
    );

    if job.ctrl.interrupted() {
        return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), info.size);
    }

    let complete = snap.iter().all(|p| p.done >= part_len(p.start, p.end));
    if complete {
        finish_ok(job, info.size)
    } else {
        JobOutcome::Failed("分段未完成，请重试".into())
    }
}

fn persist_pool(
    pool: &Mutex<Vec<Arc<LivePart>>>,
    meta_path: &Path,
    url: &str,
    etag: Option<&str>,
    size: u64,
) {
    let snap = snapshot_parts(&lock_pool(pool));
    save_resume(
        meta_path,
        &ResumeMeta {
            url: url.to_string(),
            size,
            etag: etag.map(|s| s.to_string()),
            parts: snap,
        },
    );
}

fn range_worker(
    client: &Client,
    url: &str,
    part_path: &Path,
    mut assigned: Option<Arc<LivePart>>,
    referer: Option<&str>,
    cookies: Option<&str>,
    progress: &LiveProgress,
    ctrl: &Control,
    pool: &Mutex<Vec<Arc<LivePart>>>,
    meta_path: &Path,
    job_url: &str,
    etag: Option<&str>,
    size: u64,
) {
    loop {
        if ctrl.interrupted() {
            return;
        }
        let part = match assigned.take() {
            Some(p) => p,
            None => match take_work(pool) {
                Some(p) => {
                    persist_pool(pool, meta_path, job_url, etag, size);
                    p
                }
                None => return,
            },
        };
        download_part(
            client, url, part_path, &part, referer, cookies, progress, ctrl,
        );
        part.busy.store(false, Ordering::Release);
    }
}

fn download_part(
    client: &Client,
    url: &str,
    part_path: &Path,
    part: &LivePart,
    referer: Option<&str>,
    cookies: Option<&str>,
    progress: &LiveProgress,
    ctrl: &Control,
) {
    let mut attempt = 0u32;
    while unread(part) > 0 {
        if ctrl.interrupted() {
            return;
        }
        let pos = part.start.saturating_add(part.written.load(Ordering::Acquire));
        let end = part.end.load(Ordering::Acquire);
        if pos > end {
            return;
        }
        let window_end = pos.saturating_add(HTTP_WINDOW.saturating_sub(1)).min(end);
        match fetch_range(
            client,
            url,
            part_path,
            pos,
            window_end,
            part,
            referer,
            cookies,
            progress,
            ctrl,
        ) {
            Ok(()) => {
                attempt = 0;
            }
            Err(_) => {
                attempt += 1;
                if attempt > RETRIES {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200 * attempt as u64));
            }
        }
    }
}

fn fetch_range(
    client: &Client,
    url: &str,
    part_path: &Path,
    from: u64,
    req_end: u64,
    part: &LivePart,
    referer: Option<&str>,
    cookies: Option<&str>,
    progress: &LiveProgress,
    ctrl: &Control,
) -> Result<(), String> {
    if from > req_end {
        return Ok(());
    }
    if ctrl.interrupted() {
        return Ok(());
    }
    let req = attach_auth(
        client
            .get(url)
            .header("Range", format!("bytes={from}-{req_end}"))
            .header("Accept-Encoding", "identity"),
        referer,
        cookies,
    );
    let mut resp = req.send().map_err(|e| e.to_string())?;
    let code = resp.status().as_u16();
    if code == 200 && from > 0 {
        return Err("服务器忽略了 Range".into());
    }
    if code != 206 && code != 200 {
        return Err(format!("HTTP {code}"));
    }
    let mut file = open_part(part_path).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; BUF];
    let mut offset = from;
    let mut got_any = false;
    loop {
        if ctrl.interrupted() {
            return Ok(());
        }
        let cur_end = part.end.load(Ordering::Acquire);
        if offset > cur_end {
            return Ok(());
        }
        let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let mut use_n = n as u64;
        if offset + use_n - 1 > cur_end {
            use_n = cur_end + 1 - offset;
        }
        if use_n == 0 {
            break;
        }
        write_at(&mut file, offset, &buf[..use_n as usize]).map_err(|e| e.to_string())?;
        offset += use_n;
        got_any = true;
        part.written
            .store(offset.saturating_sub(part.start), Ordering::Release);
        progress.add(use_n);
        if use_n < n as u64 {
            break;
        }
        if offset > req_end {
            break;
        }
    }
    if !got_any {
        return Err("空的分段响应".into());
    }
    Ok(())
}

fn single_stream(client: &Client, job: &HttpJob<'_>, info: &RemoteInfo) -> JobOutcome {
    let resume_from = job.part.exists().then(|| {
        std::fs::metadata(job.part).map(|m| m.len()).unwrap_or(0)
    }).unwrap_or(0);

    if resume_from > 0 {
        job.progress.downloaded.store(resume_from, Ordering::Relaxed);
        job.progress.last_bytes.store(resume_from, Ordering::Relaxed);
    }

    let mut req = client.get(job.url).header("Accept-Encoding", "identity");
    if resume_from > 0 && info.ranges {
        req = req.header("Range", format!("bytes={resume_from}-"));
    }
    req = attach_auth(req, job.referer, job.cookies);
    let mut resp = match req.send() {
        Ok(r) => r,
        Err(e) => return JobOutcome::Failed(e.to_string()),
    };
    let code = resp.status().as_u16();
    if code != 200 && code != 206 {
        return JobOutcome::Failed(format!("HTTP {code}"));
    }
    if resume_from > 0 && code == 200 {
        // 服务器忽略了 Range，只能从头来。
        let _ = std::fs::remove_file(job.part);
        job.progress.downloaded.store(0, Ordering::Relaxed);
    }
    let start_at = if code == 206 { resume_from } else { 0 };

    let mut file = match open_part(job.part) {
        Ok(f) => f,
        Err(e) => return JobOutcome::Failed(e.to_string()),
    };
    if info.size > 0 {
        preallocate(&file, info.size);
    }

    let mut buf = vec![0u8; BUF];
    let mut offset = start_at;
    loop {
        if job.ctrl.interrupted() {
            return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), info.size);
        }
        let n = match resp.read(&mut buf) {
            Ok(n) => n,
            Err(e) => return JobOutcome::Failed(e.to_string()),
        };
        if n == 0 {
            break;
        }
        if let Err(e) = write_at(&mut file, offset, &buf[..n]) {
            return JobOutcome::Failed(e.to_string());
        }
        offset += n as u64;
        job.progress.add(n as u64);
    }

    if job.ctrl.interrupted() {
        return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), info.size);
    }
    drop(file);
    finish_ok(job, offset.max(info.size))
}

fn finish_ok(job: &HttpJob<'_>, size: u64) -> JobOutcome {
    let name = job
        .dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if let Err(e) = crate::core::verify::verify_finished(job.part, size, name) {
        crate::core::log::write(format_args!("成品校验失败：{e}"));
        return JobOutcome::Failed(e);
    }
    if let Err(e) = finalize_part(job.part, job.dest) {
        return JobOutcome::Failed(e.to_string());
    }
    let _ = std::fs::remove_file(job.meta_path);
    job.progress.set_total(size);
    job.progress.downloaded.store(size, Ordering::Relaxed);
    JobOutcome::Completed {
        size,
        path: job.dest.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_covers_whole_file() {
        let parts = split_ranges(1000, 4);
        assert_eq!(parts.first().unwrap().0, 0);
        assert_eq!(parts.last().unwrap().1, 999);
        let mut covered = 0u64;
        let mut next = 0u64;
        for (s, e) in &parts {
            assert_eq!(*s, next);
            covered += e - s + 1;
            next = e + 1;
        }
        assert_eq!(covered, 1000);
    }

    #[test]
    fn split_small_file_one_part() {
        let parts = split_ranges(10, 16);
        assert_eq!(parts.len(), 10);
        assert_eq!(parts[0], (0, 0));
        assert_eq!(parts[9], (9, 9));
    }

    fn live(start: u64, end: u64, written: u64) -> Arc<LivePart> {
        Arc::new(LivePart {
            start,
            end: AtomicU64::new(end),
            written: AtomicU64::new(written),
            busy: AtomicBool::new(true),
        })
    }

    #[test]
    fn steal_splits_unread_tail_in_half() {
        let victim = live(0, 10_000_000 - 1, 0);
        let mut pool = vec![victim.clone()];
        let stolen = steal_from(&mut pool).expect("should steal");
        assert_eq!(pool.len(), 2);
        assert_eq!(stolen.start, 5_000_000);
        assert_eq!(stolen.end.load(Ordering::Relaxed), 9_999_999);
        assert_eq!(victim.end.load(Ordering::Relaxed), 4_999_999);
        assert_eq!(unread(&victim), 5_000_000);
        assert_eq!(unread(stolen.as_ref()), 5_000_000);
    }

    #[test]
    fn steal_skips_small_remainder() {
        let mut pool = vec![live(0, 100, 0)];
        assert!(steal_from(&mut pool).is_none());
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn steal_from_partially_written_part() {
        // 已写 1MB，剩余 9MB，应切未读尾巴。
        let victim = live(0, 10 * 1024 * 1024 - 1, 1024 * 1024);
        let mut pool = vec![victim.clone()];
        let stolen = steal_from(&mut pool).unwrap();
        assert!(stolen.start > 1024 * 1024);
        assert!(stolen.start < 10 * 1024 * 1024);
        assert!(victim.end.load(Ordering::Relaxed) < stolen.start);
        assert_eq!(unread(&victim) + unread(stolen.as_ref()), 9 * 1024 * 1024);
    }

    #[test]
    fn load_resume_zeros_out_holes() {
        let dir = std::env::temp_dir().join(format!("qg-resume-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("a.zip");
        let part = {
            let mut p = dest.clone().into_os_string();
            p.push(".part");
            std::path::PathBuf::from(p)
        };
        let meta_path = {
            let mut p = dest.clone().into_os_string();
            p.push(".qg.json");
            std::path::PathBuf::from(p)
        };
        let mut data = vec![0u8; 200];
        data[100] = 7;
        std::fs::write(&part, &data).unwrap();
        let meta = ResumeMeta {
            url: "https://ex.com/a.zip?sid=1".into(),
            size: 200,
            etag: Some("\"x\"".into()),
            parts: vec![
                PartMeta {
                    start: 0,
                    end: 99,
                    done: 100,
                },
                PartMeta {
                    start: 100,
                    end: 199,
                    done: 100,
                },
            ],
        };
        std::fs::write(&meta_path, serde_json::to_string(&meta).unwrap()).unwrap();
        let got = load_resume(
            &meta_path,
            &part,
            "https://ex.com/a.zip?sid=2",
            200,
            &Some("\"x\"".into()),
            "a.zip",
        )
        .unwrap();
        assert_eq!(got.parts[0].done, 0, "开头空洞不能信");
        assert!(got.parts[1].done > 0, "后半有数据应保留");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn take_work_claims_idle_incomplete() {
        let idle = live(0, 8 * 1024 * 1024 - 1, 0);
        idle.busy.store(false, Ordering::Relaxed);
        let pool = Mutex::new(vec![idle.clone()]);
        let got = take_work(&pool).unwrap();
        assert!(Arc::ptr_eq(&got, &idle));
        assert!(idle.busy.load(Ordering::Relaxed));
    }
}
