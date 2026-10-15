//! 应用自动更新：版本检测、下载校验、安装交接
//!
//! 分层约束：本模块只做与 OS 无关的解析/比较/文件校验与解压，纯逻辑来自
//! `gpui-app-update-core`，这里保留资产名、reqwest 请求、sidecar 发现策略与文案；
//! 「替换正在运行的二进制并重启」走 platform 门面
//! [`crate::platform::apply_update_and_restart`]。
//!
//! 更新源是 GitHub Releases（`meimingqi222/quickget`），没有独立
//! 更新服务器。正式通道只认 `releases/latest` 里非 draft / 非 prerelease
//! 的那一条。

use std::io::Read;
use std::path::{Path, PathBuf};
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
            | UpdateStatus::Available {
                current_version, ..
            }
            | UpdateStatus::Downloading {
                current_version, ..
            }
            | UpdateStatus::Verifying {
                current_version, ..
            }
            | UpdateStatus::Downloaded {
                current_version, ..
            }
            | UpdateStatus::Installing {
                current_version, ..
            }
            | UpdateStatus::Error {
                current_version, ..
            } => current_version,
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

pub use gpui_app_update_core::{
    is_skipped, is_version_newer, looks_like_dev_build, normalize_version, parse_checksum_sidecar,
    select_asset_by_candidates, sha256_file, GithubAsset, GithubRelease,
};

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
        UpdateTarget::MacosAarch64 => {
            &["quickget-macos-universal.zip", "quickget-macos-aarch64.zip"]
        }
        UpdateTarget::MacosX86_64 => &["quickget-macos-universal.zip", "quickget-macos-x86_64.zip"],
    }
}

/// 校验下载包与 sidecar；摘要不符时删除下载包，避免下次误用损坏文件。
///
/// SHA-256 sidecar 与安装包来自同一发布源，只能发现损坏或截断，不是发布者认证。
pub fn verify_checksum(zip_path: &Path, sidecar: &str) -> Result<(), String> {
    use gpui_app_update_core::ChecksumError;
    gpui_app_update_core::verify_sha256_sidecar(zip_path, sidecar).map_err(|e| match e {
        ChecksumError::MalformedSidecar => "校验文件格式异常".to_string(),
        ChecksumError::Io(e) => format!("计算 SHA-256 失败: {e}"),
        ChecksumError::Mismatch { expected, actual } => {
            let _ = std::fs::remove_file(zip_path);
            format!("SHA-256 校验不匹配（期望 {expected}，实际 {actual}）")
        }
    })
}

/// 在 `cache_dir` 下新建独占 staging 目录并解压，返回待安装载荷路径
/// （Windows 为 `quickget.exe`，macOS 为 `QuickGet.app`）。
///
/// 整个归档先校验（逃逸、绝对路径、重复项、符号链接、数量和体积上限）再写入；
/// 只清理本产品缓存里旧的 staging 与旧版固定的 `extracted` 目录。
pub fn extract_update_zip(zip_path: &Path, cache_dir: &Path) -> Result<PathBuf, String> {
    use gpui_app_update_core::ExtractError;
    let _ = std::fs::remove_dir_all(cache_dir.join("extracted"));
    let staging = gpui_app_update_core::create_staging_dir(cache_dir)
        .map_err(|e| format!("create extract dir: {e}"))?;
    gpui_app_update_core::extract_zip(
        zip_path,
        &staging,
        gpui_app_update_core::ExtractLimits::default(),
    )
    .map_err(|e| e.to_string())?;
    let Some(spec) = payload_spec() else {
        return Err("当前平台不支持自动更新".into());
    };
    gpui_app_update_core::find_payload(&staging, spec).map_err(|e| match e {
        ExtractError::PayloadNotFound { expected } => format!("解压目录中未找到 {expected}"),
        other => other.to_string(),
    })
}

fn payload_spec() -> Option<gpui_app_update_core::PayloadSpec<'static>> {
    #[cfg(windows)]
    {
        Some(gpui_app_update_core::PayloadSpec::WindowsExe(
            "quickget.exe",
        ))
    }
    #[cfg(target_os = "macos")]
    {
        Some(gpui_app_update_core::PayloadSpec::MacApp("QuickGet.app"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
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
    let text = resp
        .text()
        .map_err(|e| format!("读取 Release 文本失败: {e}"))?;
    serde_json::from_str::<GithubRelease>(&text).map_err(|e| format!("解析 Release JSON 失败: {e}"))
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
///
/// 校验文件必须作为 `<资产名>.sha256` 出现在 Release 资产列表里，缺失时检查即失败。
pub fn evaluate_release(
    current_version: &str,
    release: &GithubRelease,
    candidates: &[&str],
) -> Result<UpdateStatus, String> {
    use gpui_app_update_core::ReleaseError;
    let candidate = gpui_app_update_core::evaluate_release(
        current_version,
        release,
        candidates,
        gpui_app_update_core::ChecksumSource::ReleaseAsset,
    )
    .map_err(|e| match e {
        ReleaseError::NoMatchingAsset { .. } => {
            format!("Release 里缺少本平台安装包（候选：{candidates:?}）")
        }
        ReleaseError::MissingChecksumAsset { name } => format!("Release 里缺少校验文件 {name}"),
    })?;
    Ok(match candidate {
        None => UpdateStatus::NotAvailable {
            current_version: current_version.to_string(),
        },
        Some(c) => UpdateStatus::Available {
            current_version: current_version.to_string(),
            latest_version: c.latest_version,
            release_url: c.release_url,
            notes: c.notes,
            asset_name: c.asset_name,
            asset_url: c.asset_url,
            checksum_url: c.checksum_url,
        },
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
        let text =
            "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2  quickget.zip\n";
        assert_eq!(
            parse_checksum_sidecar(text),
            Some("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".into())
        );
    }

    fn release(tag: &str, assets: &[&str]) -> GithubRelease {
        GithubRelease {
            tag_name: tag.into(),
            html_url: "https://example/rel".into(),
            name: None,
            body: Some("notes".into()),
            prerelease: false,
            draft: false,
            assets: assets
                .iter()
                .map(|name| GithubAsset {
                    name: name.to_string(),
                    browser_download_url: format!("https://dl.example/{name}"),
                    size: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn mac_fallback_never_crosses_architecture() {
        let both = release(
            "v9.0.0",
            &[
                "quickget-macos-aarch64.zip",
                "quickget-macos-aarch64.zip.sha256",
                "quickget-macos-x86_64.zip",
                "quickget-macos-x86_64.zip.sha256",
            ],
        );
        for (target, expected) in [
            (UpdateTarget::MacosX86_64, "quickget-macos-x86_64.zip"),
            (UpdateTarget::MacosAarch64, "quickget-macos-aarch64.zip"),
        ] {
            match evaluate_release("0.1.0", &both, candidate_asset_names(target)).unwrap() {
                UpdateStatus::Available { asset_name, .. } => assert_eq!(asset_name, expected),
                other => panic!("expected Available, got {other:?}"),
            }
        }
        let arm_only = release(
            "v9.0.0",
            &[
                "quickget-macos-aarch64.zip",
                "quickget-macos-aarch64.zip.sha256",
            ],
        );
        let err = evaluate_release(
            "0.1.0",
            &arm_only,
            candidate_asset_names(UpdateTarget::MacosX86_64),
        )
        .unwrap_err();
        assert!(err.contains("缺少本平台安装包"), "{err}");
    }

    #[test]
    fn windows_prefers_named_zip_and_falls_back_to_legacy_name() {
        let both = release(
            "v9.0.0",
            &[
                "quickget.zip",
                "quickget.zip.sha256",
                "quickget-windows-x86_64.zip",
                "quickget-windows-x86_64.zip.sha256",
            ],
        );
        let pick = |r: &GithubRelease| match evaluate_release(
            "0.1.0",
            r,
            candidate_asset_names(UpdateTarget::WindowsX64),
        )
        .unwrap()
        {
            UpdateStatus::Available { asset_name, .. } => asset_name,
            other => panic!("expected Available, got {other:?}"),
        };
        assert_eq!(pick(&both), "quickget-windows-x86_64.zip");
        let legacy = release("v9.0.0", &["quickget.zip", "quickget.zip.sha256"]);
        assert_eq!(pick(&legacy), "quickget.zip");
    }

    #[test]
    fn checksum_must_be_a_listed_release_asset() {
        let missing = release("v9.0.0", &["quickget-windows-x86_64.zip"]);
        let err = evaluate_release(
            "0.1.0",
            &missing,
            candidate_asset_names(UpdateTarget::WindowsX64),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Release 里缺少校验文件 quickget-windows-x86_64.zip.sha256"
        );

        let mut listed = release(
            "v9.0.0",
            &[
                "quickget-windows-x86_64.zip",
                "quickget-windows-x86_64.zip.sha256",
            ],
        );
        listed.assets[1].browser_download_url = "https://mirror.example/sum".into();
        match evaluate_release("0.1.0", &listed, &["quickget-windows-x86_64.zip"]).unwrap() {
            UpdateStatus::Available {
                checksum_url,
                latest_version,
                ..
            } => {
                assert_eq!(checksum_url, "https://mirror.example/sum");
                assert_eq!(latest_version, "9.0.0");
            }
            other => panic!("expected Available, got {other:?}"),
        }

        let same = release("v0.1.0", &[]);
        assert!(matches!(
            evaluate_release("0.1.0", &same, &["quickget.zip"]).unwrap(),
            UpdateStatus::NotAvailable { .. }
        ));
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qg-updater-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_zip(path: &Path, files: &[&str]) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for name in files {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(b"binary").unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn extract_rejects_zip_slip() {
        let dir = test_dir("zip-slip");
        let zip_path = dir.join("evil.zip");
        write_zip(&zip_path, &["quickget.exe", "../escape.txt"]);
        let err = extract_update_zip(&zip_path, &dir.join("cache")).unwrap_err();
        assert!(err.contains("escape"), "{err}");
        assert!(!dir.join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 发行包布局与 release.yml 一致：Windows 根目录 exe + README，macOS 为 QuickGet.app。
    #[test]
    fn extract_finds_platform_payload() {
        let dir = test_dir("extract-ok");
        let zip_path = dir.join("pack.zip");
        write_zip(
            &zip_path,
            &[
                "quickget.exe",
                "README.md",
                "QuickGet.app/Contents/Info.plist",
                "QuickGet.app/Contents/MacOS/quickget",
            ],
        );
        let result = extract_update_zip(&zip_path, &dir.join("cache"));
        #[cfg(windows)]
        assert!(result.unwrap().ends_with("quickget.exe"));
        #[cfg(target_os = "macos")]
        assert!(result.unwrap().ends_with("QuickGet.app"));
        #[cfg(not(any(windows, target_os = "macos")))]
        assert_eq!(result.unwrap_err(), "当前平台不支持自动更新");

        write_zip(&zip_path, &["README.md"]);
        let err = extract_update_zip(&zip_path, &dir.join("cache")).unwrap_err();
        #[cfg(windows)]
        assert_eq!(err, "解压目录中未找到 quickget.exe");
        #[cfg(target_os = "macos")]
        assert_eq!(err, "解压目录中未找到 QuickGet.app");
        #[cfg(not(any(windows, target_os = "macos")))]
        assert_eq!(err, "当前平台不支持自动更新");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checksum_verification_messages() {
        let dir = test_dir("checksum");
        let file = dir.join("a.zip");
        std::fs::write(&file, b"abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        verify_checksum(&file, &format!("{abc}  a.zip")).unwrap();
        assert_eq!(
            verify_checksum(&file, "junk").unwrap_err(),
            "校验文件格式异常"
        );
        assert!(file.exists(), "malformed sidecar keeps the download");
        let zero = "0".repeat(64);
        assert_eq!(
            verify_checksum(&file, &zero).unwrap_err(),
            format!("SHA-256 校验不匹配（期望 {zero}，实际 {abc}）")
        );
        assert!(!file.exists(), "mismatched download is removed");
        assert!(verify_checksum(&file, abc)
            .unwrap_err()
            .starts_with("计算 SHA-256 失败"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
