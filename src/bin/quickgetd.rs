//! QuickGet 的本机下载守护进程。
//!
//! 该进程只接受 loopback 请求，供 baidu-share 等上层服务调用；绝不直接暴露到
//! 局域网。下载任务 API 会在此稳定边界上逐步扩展，避免 Web 服务通过 CLI/stdout
//! 驱动下载内核。

use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use quickget::core::{
    engine::run_task,
    limiter::DownloadLimiter,
    model::{Task, TaskStatus},
    progress::{Control, JobOutcome, LiveMeta, LiveProgress},
    providers::{self, NetdiskRequest},
    settings::default_ua,
    urlx::{detect_protocol, filename_from_url, Protocol},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    sync::{atomic::Ordering, Arc, Mutex},
};

#[derive(Clone)]
struct AppState {
    token: Arc<str>,
    limiter: DownloadLimiter,
    download_root: Arc<PathBuf>,
    downloads: Arc<Mutex<HashMap<String, DownloadEntry>>>,
}

#[derive(Clone)]
struct DownloadEntry {
    task: Task,
    progress: Arc<LiveProgress>,
    control: Control,
    result: Option<DownloadResult>,
}

#[derive(Clone, Serialize)]
struct DownloadResult {
    path: Option<PathBuf>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct CreateDownload {
    url: String,
    destination: PathBuf,
    filename: Option<String>,
    #[serde(default = "default_connections")]
    connections: u32,
}

fn default_connections() -> u32 {
    16
}

#[derive(Serialize)]
struct DownloadView {
    id: String,
    status: &'static str,
    downloaded_bytes: u64,
    total_bytes: u64,
    speed_bps: u64,
    result: Option<DownloadResult>,
}

#[derive(Serialize)]
struct Health {
    service: &'static str,
    status: &'static str,
}

/// 仅供本机上层服务调用。Cookie 永不写日志，也绝不从此 API 返回给浏览器。
#[derive(Deserialize)]
struct ResolveBaiduRequest {
    share_url: String,
    cookies: String,
    #[serde(default)]
    fids: Vec<String>,
    #[serde(default)]
    sekey: Option<String>,
    #[serde(default)]
    js_token: Option<String>,
}

#[derive(Serialize)]
struct ResolvedFile {
    dlink: String,
    filename: String,
    size: u64,
    transfer_path: Option<String>,
}

#[derive(Deserialize)]
struct UpdateSettings {
    #[serde(default)]
    download_speed_limit_bps: Option<u64>,
}

#[derive(Serialize)]
struct DaemonSettings {
    download_speed_limit_bps: u64,
}

#[tokio::main]
async fn main() {
    let token = std::env::var("QUICKGETD_TOKEN")
        .expect("必须设置 QUICKGETD_TOKEN，拒绝启动未认证的下载守护进程");
    if token.len() < 32 {
        panic!("QUICKGETD_TOKEN 至少需要 32 个字符");
    }

    let listen: SocketAddr = std::env::var("QUICKGETD_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:18667".into())
        .parse()
        .expect("QUICKGETD_LISTEN 必须是 IP:端口，例如 127.0.0.1:18667");
    if !listen.ip().is_loopback() {
        panic!("quickgetd 只能绑定 loopback 地址，不能暴露到局域网");
    }

    let download_root = std::env::var_os("QUICKGETD_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Downloads")
        });
    std::fs::create_dir_all(&download_root).expect("无法创建 QUICKGETD_DOWNLOAD_DIR");
    let download_root = download_root
        .canonicalize()
        .expect("无法解析 QUICKGETD_DOWNLOAD_DIR");

    let state = AppState {
        token: Arc::from(token),
        limiter: DownloadLimiter::new(0),
        download_root: Arc::new(download_root),
        downloads: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/health", get(authenticated_health))
        .route("/v1/settings", get(get_settings).put(update_settings))
        .route("/v1/baidu/resolve", post(resolve_baidu))
        .route("/v1/downloads", post(create_download))
        .route("/v1/downloads/{id}", get(get_download))
        .route("/v1/downloads/{id}/cancel", post(cancel_download))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .expect("无法绑定 quickgetd 监听地址");
    eprintln!("quickgetd 正在监听 http://{listen}");
    axum::serve(listener, app)
        .await
        .expect("quickgetd 服务异常");
}

async fn create_download(State(state): State<AppState>, request: Request) -> impl IntoResponse {
    if !authorized(&state, &request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let body = match to_bytes(request.into_body(), 16 * 1024).await {
        Ok(body) => body,
        Err(_) => return (StatusCode::BAD_REQUEST, "请求体无效或过大").into_response(),
    };
    let input: CreateDownload = match serde_json::from_slice(&body) {
        Ok(input) => input,
        Err(_) => return (StatusCode::BAD_REQUEST, "请求 JSON 无效").into_response(),
    };
    if detect_protocol(&input.url) != Protocol::Http || input.connections == 0 {
        return (
            StatusCode::BAD_REQUEST,
            "仅支持 HTTP(S)，connections 必须大于 0",
        )
            .into_response();
    }
    let destination = match resolve_destination(&state.download_root, &input.destination) {
        Ok(path) => path,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let filename = input
        .filename
        .unwrap_or_else(|| filename_from_url(&input.url));
    if !valid_filename(&filename) {
        return (StatusCode::BAD_REQUEST, "filename 必须是单个文件名").into_response();
    }

    // 百度 PCS 直链（*.baidupcs.com/file/...?bkt=...）只认安卓客户端 UA；
    // 其余 URL 用平台的默认 UA。
    let is_baidu_pcs = input.url.contains("baidupcs.com") || input.url.contains("baidupcs");
    let download_ua = if is_baidu_pcs {
        "netdisk;P2SP;3.0.0.8;netdisk;11.12.3;ANG-AN00;android-android;10.0;JSbridge4.4.0;jointBridge;1.1.0;"
    } else {
        default_ua()
    };
    // 百度 PCS 直链对单次 Range > 4MB 回 31326 风控并降速，必须限制每片大小。
    let max_part_size = if is_baidu_pcs {
        Some(4 * 1024 * 1024)
    } else {
        None
    };

    let task = Task::new(
        Protocol::Http,
        input.url,
        filename,
        destination,
        input.connections,
        max_part_size,
        None,
    );
    let id = task.id.clone();
    let progress = LiveProgress::new(0, 0);
    let control = Control::new();
    state
        .downloads
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(
            id.clone(),
            DownloadEntry {
                task: task.clone(),
                progress: progress.clone(),
                control: control.clone(),
                result: None,
            },
        );
    let ticker_state = state.clone();
    let ticker_id = id.clone();
    let ticker_progress = progress.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            ticker_progress.tick_speed(1000);
            let active = ticker_state
                .downloads
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&ticker_id)
                .is_some_and(|entry| entry.task.status.is_active());
            if !active {
                break;
            }
        }
    });
    let worker_state = state.clone();
    let worker_id = id.clone();
    tokio::task::spawn_blocking(move || {
        let outcome = run_task(
            &task,
            download_ua,
            progress,
            control,
            LiveMeta::new(),
            worker_state.limiter.clone(),
            None,
        );
        let mut jobs = worker_state
            .downloads
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = jobs.get_mut(&worker_id) {
            entry.task.finished_at = Some(chrono::Utc::now().timestamp());
            match outcome {
                JobOutcome::Completed { size, path } => {
                    entry.task.status = TaskStatus::Completed;
                    entry.task.size = size;
                    entry.result = Some(DownloadResult {
                        path: Some(path),
                        error: None,
                    });
                }
                JobOutcome::Cancelled => {
                    entry.task.status = TaskStatus::Cancelled;
                }
                JobOutcome::Paused { .. } => {
                    entry.task.status = TaskStatus::Cancelled;
                }
                JobOutcome::Failed(error) => {
                    entry.task.status = TaskStatus::Failed;
                    entry.task.error = Some(error.clone());
                    entry.result = Some(DownloadResult {
                        path: None,
                        error: Some(error),
                    });
                }
            }
        }
    });
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "id": id }))).into_response()
}

async fn get_download(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: Request,
) -> impl IntoResponse {
    if !authorized(&state, &request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let jobs = state.downloads.lock().unwrap_or_else(|p| p.into_inner());
    match jobs.get(&id) {
        Some(entry) => Json(download_view(entry)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn cancel_download(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: Request,
) -> impl IntoResponse {
    if !authorized(&state, &request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut jobs = state.downloads.lock().unwrap_or_else(|p| p.into_inner());
    match jobs.get_mut(&id) {
        Some(entry) => {
            if entry.task.status.is_active() {
                entry.control.stop.store(true, Ordering::Relaxed);
            }
            Json(download_view(entry)).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn download_view(entry: &DownloadEntry) -> DownloadView {
    let status = match entry.task.status {
        TaskStatus::Queued => "queued",
        TaskStatus::Probing => "probing",
        TaskStatus::Downloading => "downloading",
        TaskStatus::Paused => "paused",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    };
    DownloadView {
        id: entry.task.id.clone(),
        status,
        downloaded_bytes: entry.progress.downloaded(),
        total_bytes: entry.progress.total(),
        speed_bps: entry.progress.speed(),
        result: entry.result.clone(),
    }
}

fn resolve_destination(root: &Path, requested: &Path) -> Result<PathBuf, &'static str> {
    if requested
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("destination 不得包含 ..");
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    // 在产生任何目录前先做词法边界检查；canonicalize 后再检查一次以阻止符号链接逃逸。
    if !candidate.starts_with(root) {
        return Err("destination 必须位于 QUICKGETD_DOWNLOAD_DIR 下");
    }
    std::fs::create_dir_all(&candidate).map_err(|_| "无法创建 destination")?;
    let canonical = candidate
        .canonicalize()
        .map_err(|_| "无法解析 destination")?;
    if !canonical.starts_with(root) {
        return Err("destination 必须位于 QUICKGETD_DOWNLOAD_DIR 下");
    }
    Ok(canonical)
}

fn valid_filename(filename: &str) -> bool {
    !filename.is_empty()
        && Path::new(filename).components().count() == 1
        && matches!(
            Path::new(filename).components().next(),
            Some(Component::Normal(_))
        )
}

async fn healthz() -> Json<Health> {
    Json(Health {
        service: "quickgetd",
        status: "ok",
    })
}

async fn authenticated_health(
    State(state): State<AppState>,
    request: Request,
) -> impl IntoResponse {
    if !authorized(&state, &request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(Health {
        service: "quickgetd",
        status: "ok",
    })
    .into_response()
}

async fn resolve_baidu(State(state): State<AppState>, request: Request) -> impl IntoResponse {
    if !authorized(&state, &request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let body = match to_bytes(request.into_body(), 64 * 1024).await {
        Ok(body) => body,
        Err(_) => return (StatusCode::BAD_REQUEST, "请求体无效或过大").into_response(),
    };
    let input: ResolveBaiduRequest = match serde_json::from_slice(&body) {
        Ok(input) => input,
        Err(_) => return (StatusCode::BAD_REQUEST, "请求 JSON 无效").into_response(),
    };
    if input.share_url.trim().is_empty() || input.cookies.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "分享链接和登录 Cookie 均不能为空").into_response();
    }

    let work = tokio::task::spawn_blocking(move || {
        let provider = providers::find(Some("baidu"), &input.share_url)
            .ok_or_else(|| "不支持的百度分享链接".to_string())?;
        provider
            .resolve(
                &NetdiskRequest {
                    provider: Some("baidu".into()),
                    share_url: input.share_url,
                    fids: input.fids,
                    sekey: input.sekey,
                    js_token: input.js_token,
                },
                &input.cookies,
                default_ua(),
            )
            .map(|files| {
                files
                    .into_iter()
                    .map(|file| ResolvedFile {
                        dlink: file.dlink,
                        filename: file.filename,
                        size: file.size,
                        transfer_path: file.transfer_path,
                    })
                    .collect::<Vec<_>>()
            })
    });
    match work.await {
        Ok(Ok(files)) => Json(files).into_response(),
        Ok(Err(error)) => (StatusCode::BAD_GATEWAY, error).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "解析任务异常退出").into_response(),
    }
}

async fn get_settings(State(state): State<AppState>, request: Request) -> impl IntoResponse {
    if !authorized(&state, &request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(DaemonSettings {
        download_speed_limit_bps: state.limiter.limit(),
    })
    .into_response()
}

async fn update_settings(State(state): State<AppState>, request: Request) -> impl IntoResponse {
    if !authorized(&state, &request) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let body = match to_bytes(request.into_body(), 4 * 1024).await {
        Ok(body) => body,
        Err(_) => return (StatusCode::BAD_REQUEST, "请求体无效或过大").into_response(),
    };
    let input: UpdateSettings = match serde_json::from_slice(&body) {
        Ok(input) => input,
        Err(_) => return (StatusCode::BAD_REQUEST, "请求 JSON 无效").into_response(),
    };
    state
        .limiter
        .set_limit(input.download_speed_limit_bps.unwrap_or(0));
    Json(DaemonSettings {
        download_speed_limit_bps: state.limiter.limit(),
    })
    .into_response()
}

fn authorized(state: &AppState, request: &Request) -> bool {
    let Some(value) = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return false;
    };
    // token 来自本机服务配置。这里不记录它，避免日志泄露。
    constant_time_eq(token.as_bytes(), state.token.as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;

    #[test]
    fn constant_time_comparison_checks_content_and_length() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"secret", b"short"));
    }

    fn test_state() -> AppState {
        AppState {
            token: Arc::from("01234567890123456789012345678901"),
            limiter: DownloadLimiter::new(0),
            download_root: Arc::new(std::env::temp_dir()),
            downloads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[test]
    fn bearer_auth_requires_exact_scheme_and_token() {
        let state = test_state();
        let request = HttpRequest::builder()
            .header(AUTHORIZATION, "Bearer 01234567890123456789012345678901")
            .body(Body::empty())
            .unwrap();
        assert!(authorized(&state, &request));
        let request = HttpRequest::builder()
            .header(AUTHORIZATION, "bearer 01234567890123456789012345678901")
            .body(Body::empty())
            .unwrap();
        assert!(!authorized(&state, &request));
        assert!(!authorized(&state, &HttpRequest::new(Body::empty())));
    }

    #[test]
    fn destination_stays_beneath_canonical_root() {
        let root = std::env::temp_dir().join(format!("quickgetd-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        assert_eq!(
            resolve_destination(&root, Path::new("nested")).unwrap(),
            root.join("nested")
        );
        assert!(resolve_destination(&root, Path::new("../escape")).is_err());
        assert!(
            resolve_destination(&root, &std::env::temp_dir().join("outside-quickgetd")).is_err()
        );
        assert!(valid_filename("file.bin"));
        assert!(!valid_filename("../file.bin"));
        assert!(!valid_filename("sub/file.bin"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
