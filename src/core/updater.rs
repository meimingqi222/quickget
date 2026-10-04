//! 应用自动更新：版本检测、下载校验、安装交接
//!
//! 分层约束：本模块只做与 OS 无关的解析/比较/文件校验与解压；
//! 「替换正在运行的二进制并重启」走 platform 门面
//! [`crate::platform::apply_update_and_restart`]。
//!
//! 更新源是 GitHub Releases（`meimingqi222/quickget`），没有独立
//! 更新服务器。正式通道只认 `releases/latest` 里非 draft / 非 prerelease
//! 的那一条。

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub const GITHUB_OWNER: &str = "meimingqi222";
pub const GITHUB_REPO: &str = "quickget";
/// 无 token 时 GitHub API 要求 UA，否则 403。
const USER_AGENT: &str = concat!("QuickGet-Updater/", env!("CARGO_PKG_VERSION"));

/// 启动后首次检查延迟（毫秒）。
pub const FIRST_CHECK_DELAY_MS: u64 = 10_000;
/// 周期检查间隔（前一次检查结束后再计时）。
pub const CHECK_INTERVAL_MS: u64 = 4 * 60 * 60 * 1000;

/// 更新下载进度。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DownloadProgress {
    pub percent: f32,
    pub transferred: u64,
    pub total: Option<u64>,
}

/// 更新状态机。UI 与调度都只认这个枚举。
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateStatus {
    Idle {
        current_version: String,
    },
    Checking {
        current_version: String,
    },
    NotAvailable {
        current_version: String,
    },
    Available {
        current_version: String,
        latest_version: String,
        release_url: String,
        notes: String,
        asset_name: String,
        asset_url: String,
        checksum_url: String,
    },
    Downloading {
        current_version: String,
        latest_version: String,
        progress: DownloadProgress,
        asset_name: String,
        asset_url: String,
        checksum_url: String,
        release_url: String,
        notes: String,
    },
    Verifying {
        current_version: String,
        latest_version: String,
    },
    Downloaded {
        current_version: String,
        latest_version: String,
        /// 解压后待安装的路径：Windows 为 exe，macOS 为 `.app`。
        payload: PathBuf,
    },
    Installing {
        current_version: String,
        latest_version: String,
    },
    Error {
        current_version: String,
        operation: UpdateOperation,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOperation {
    Check,
    Download,
    Install,
}

impl UpdateStatus {
    pub fn current_version(&self) -> &str {
        match self {
            UpdateStatus::Idle { current_version }
            | UpdateStatus::Checking { current_version }
            | UpdateStatus::NotAvailable { current_version }
            | UpdateStatus::Available { current_version, .. }
            | UpdateStatus::Downloading { current_version, .. }
            | UpdateStatus::Verifying { current_version, .. }
            | UpdateStatus::Downloaded { current_version, .. }
            | UpdateStatus::Installing { current_version, .. }
            | UpdateStatus::Error { current_version, .. } => current_version,
        }
    }

    pub fn latest_version(&self) -> Option<&str> {
        match self {
            UpdateStatus::Available { latest_version, .. }
            | UpdateStatus::Downloading { latest_version, .. }
            | UpdateStatus::Verifying { latest_version, .. }
            | UpdateStatus::Downloaded { latest_version, .. }
            | UpdateStatus::Installing { latest_version, .. } => Some(latest_version),
            _ => None,
        }
    }

    /// 侧栏或界面是否应提醒用户注意。
    pub fn wants_attention(&self) -> bool {
        matches!(
            self,
            UpdateStatus::Available { .. }
                | UpdateStatus::Downloading { .. }
                | UpdateStatus::Verifying { .. }
                | UpdateStatus::Downloaded { .. }
                | UpdateStatus::Installing { .. }
                | UpdateStatus::Error { .. }
        )
    }
}

/// 轻量 semver：去掉可选 `v` 前缀，比较三段数字，缺省按 0。
pub fn is_version_newer(candidate: &str, current: &str) -> bool {
    let next = version_parts(candidate);
    let base = version_parts(current);
    let (Some(next), Some(base)) = (next, base) else {
        return normalize_version(candidate) != normalize_version(current);
    };
    for i in 0..next.len().max(base.len()) {
        let l = next.get(i).copied().unwrap_or(0);
        let r = base.get(i).copied().unwrap_or(0);
        if l > r {
            return true;
        }
        if l < r {
            return false;
        }
    }
    false
}

pub fn normalize_version(version: &str) -> String {
    version.trim().trim_start_matches(['v', 'V']).to_string()
}

fn version_parts(version: &str) -> Option<[u64; 3]> {
    let norm = normalize_version(version);
    let mut it = norm.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = it
        .next()
        .map(|s| s.split(|c: char| !c.is_ascii_digit()).next().unwrap_or("0"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Some([major, minor, patch])
}

/// GitHub Releases API 的 `releases/latest` 数据结构。
#[derive(Debug, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    pub html_url: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

/// 当前构建应订阅的发行包目标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateTarget {
    WindowsX64,
    MacosUniversal,
    MacosAarch64,
    MacosX86_64,
}

pub fn current_target() -> UpdateTarget {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        UpdateTarget::WindowsX64
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        UpdateTarget::MacosAarch64
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        UpdateTarget::MacosX86_64
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        target_os = "macos"
    )))]
    {
        UpdateTarget::WindowsX64
    }
}

/// 本平台的候选资产名列表（优先序）。
pub fn candidate_asset_names(target: UpdateTarget) -> &'static [&'static str] {
    match target {
        UpdateTarget::WindowsX64 => &["quickget-windows-x86_64.zip", "quickget.zip"],
        UpdateTarget::MacosUniversal => &["quickget-macos-universal.zip"],
        UpdateTarget::MacosAarch64 => &[
            "quickget-macos-universal.zip",
            "quickget-macos-aarch64.zip",
        ],
        UpdateTarget::MacosX86_64 => &[
            "quickget-macos-universal.zip",
            "quickget-macos-x86_64.zip",
        ],
    }
}

pub fn select_asset_by_candidates<'a>(
    release: &'a GithubRelease,
    candidates: &[&str],
) -> Option<&'a GithubAsset> {
    if release.draft || release.prerelease {
        return None;
    }
    for name in candidates {
        if let Some(asset) = release.assets.iter().find(|a| a.name == *name) {
            return Some(asset);
        }
    }
    None
}

/// 解析 `<hex>  <filename>` 形式的 sidecar，返回十六进制摘要（小写）。
pub fn parse_checksum_sidecar(text: &str) -> Option<String> {
    let text = text.trim_start_matches('\u{feff}');
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let hex = line.split_whitespace().next()?;
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex.to_ascii_lowercase())
}

/// 计算文件 SHA-256（小写 hex）。
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 解压 zip 到 `dest_dir`，拒绝 zip-slip（任一条目逃出目标根）。
pub fn extract_update_zip(zip_path: &Path, dest_dir: &Path) -> Result<PathBuf, String> {
    if dest_dir.exists() {
        let _ = std::fs::remove_dir_all(dest_dir);
    }
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("create extract dir: {e}"))?;

    let file = std::fs::File::open(zip_path).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {e}"))?;

    let dest_real = dest_dir
        .canonicalize()
        .unwrap_or_else(|_| dest_dir.to_path_buf());

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip entry: {e}"))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(format!("zip entry escapes destination: {:?}", entry.name()));
        };
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
        {
            return Err(format!("zip entry has absolute path: {:?}", entry.name()));
        }
        let out = dest_dir.join(&rel);
        if let Ok(out_real) = out.canonicalize() {
            if !out_real.starts_with(&dest_real) {
                return Err(format!("zip entry escapes destination: {:?}", entry.name()));
            }
        }

        if entry.is_dir() || entry.name().ends_with('/') {
            std::fs::create_dir_all(&out).map_err(|e| format!("mkdir {}: {e}", out.display()))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let mut outfile =
            std::fs::File::create(&out).map_err(|e| format!("create {}: {e}", out.display()))?;
        std::io::copy(&mut entry, &mut outfile)
            .map_err(|e| format!("write {}: {e}", out.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
            }
        }
    }

    find_payload(dest_dir)
}

fn find_payload(root: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let exe = root.join("quickget.exe");
        if exe.is_file() {
            return Ok(exe);
        }
        // 有的 zip 可能多一层目录
        if let Ok(rd) = std::fs::read_dir(root) {
            for ent in rd.flatten() {
                let p = ent.path().join("quickget.exe");
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
        Err("解压目录中未找到 quickget.exe".into())
    }
    #[cfg(target_os = "macos")]
    {
        let app = root.join("QuickGet.app");
        if app.is_dir() {
            return Ok(app);
        }
        if let Ok(rd) = std::fs::read_dir(root) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.is_dir()
                    && p.extension().and_then(|e| e.to_str()) == Some("app")
                    && p.file_name().and_then(|n| n.to_str()) == Some("QuickGet.app")
                {
                    return Ok(p);
                }
            }
        }
        Err("解压目录中未找到 QuickGet.app".into())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = root;
        Err("当前平台不支持自动更新".into())
    }
}

/// 是否运行在 cargo 构建目录里（开发态，不打扰更新）。
pub fn looks_like_dev_build(exe: &Path) -> bool {
    let s = exe.to_string_lossy().to_ascii_lowercase();
    s.contains("target\\debug")
        || s.contains("target/debug")
        || s.contains("target\\release")
        || s.contains("target/release")
}

pub fn is_skipped(latest: &str, skipped: Option<&str>) -> bool {
    skipped.is_some_and(|s| normalize_version(s) == normalize_version(latest))
}

fn build_http_client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .use_rustls_tls()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))
}

/// 阻塞拉取 latest release JSON。
pub fn fetch_latest_release(timeout_ms: u64) -> Result<GithubRelease, String> {
    let url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest");
    let client = build_http_client(Duration::from_millis(timeout_ms))?;
    let resp = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .map_err(|e| format!("GitHub API 请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API 返回 HTTP {}", resp.status()));
    }
    let text = resp.text().map_err(|e| format!("读取 Release 文本失败: {e}"))?;
    serde_json::from_str::<GithubRelease>(&text)
        .map_err(|e| format!("解析 Release JSON 失败: {e}"))
}

/// 下载 URL 到文件；`on_progress` 为 (transferred, total)。
pub fn download_to_file(
    url: &str,
    dest: &Path,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let client = build_http_client(Duration::from_secs(300))?;
    let resp = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .map_err(|e| format!("下载更新包失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载返回 HTTP {}", resp.status()));
    }
    let total = resp.content_length();
    let mut reader = resp;
    let mut file =
        std::fs::File::create(dest).map_err(|e| format!("创建文件失败 {}: {e}", dest.display()))?;
    let mut buf = [0u8; 64 * 1024];
    let mut transferred = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("读取下载数据失败: {e}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n])
            .map_err(|e| format!("写入文件失败: {e}"))?;
        transferred += n as u64;
        on_progress(transferred, total);
    }
    Ok(())
}

pub fn fetch_text(url: &str, timeout_ms: u64) -> Result<String, String> {
    let client = build_http_client(Duration::from_millis(timeout_ms))?;
    let resp = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .map_err(|e| format!("获取数据失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("获取数据返回 HTTP {}", resp.status()));
    }
    resp.text().map_err(|e| format!("读取文本失败: {e}"))
}

/// 完整的「检查 → 有新版本则产出 Available」纯逻辑；网络由调用方注入。
pub fn evaluate_release(
    current_version: &str,
    release: &GithubRelease,
    candidates: &[&str],
) -> Result<UpdateStatus, String> {
    if release.draft || release.prerelease {
        return Ok(UpdateStatus::NotAvailable {
            current_version: current_version.to_string(),
        });
    }
    let latest_version = normalize_version(&release.tag_name);
    if !is_version_newer(&latest_version, current_version) {
        return Ok(UpdateStatus::NotAvailable {
            current_version: current_version.to_string(),
        });
    }
    let Some(asset) = select_asset_by_candidates(release, candidates) else {
        return Err(format!("Release 里缺少本平台安装包（候选：{candidates:?}）"));
    };
    let checksum_name = format!("{}.sha256", asset.name);
    let checksum_asset = release
        .assets
        .iter()
        .find(|a| a.name == checksum_name)
        .ok_or_else(|| format!("Release 里缺少校验文件 {checksum_name}"))?;

    Ok(UpdateStatus::Available {
        current_version: current_version.to_string(),
        latest_version,
        release_url: release.html_url.clone(),
        notes: release.body.clone().unwrap_or_default(),
        asset_name: asset.name.clone(),
        asset_url: asset.browser_download_url.clone(),
        checksum_url: checksum_asset.browser_download_url.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison() {
        assert!(is_version_newer("0.2.0", "0.1.0"));
        assert!(is_version_newer("v0.1.1", "0.1.0"));
        assert!(is_version_newer("1.0.0", "0.9.9"));
        assert!(!is_version_newer("0.1.0", "0.1.0"));
        assert!(!is_version_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn parse_sidecar() {
        let text = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2  quickget.zip\n";
        assert_eq!(
            parse_checksum_sidecar(text),
            Some("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".into())
        );
    }
}
