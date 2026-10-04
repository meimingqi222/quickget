//! 操作系统适配层。

#[cfg(not(windows))]
pub(crate) fn posix_locale_tag() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(any(windows, target_os = "macos")))]
pub fn detect_system_language() -> crate::core::i18n::Language {
    crate::core::i18n::Language::from_locale_tag(&posix_locale_tag())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn reveal_in_explorer(path: &std::path::Path) {
    if let Some(dir) = path.parent() {
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn open_in_default_app(path: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn open_folder(path: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn move_to_trash(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let r = std::process::Command::new("gio")
        .args(["trash", "--"])
        .arg(path)
        .status();
    if matches!(r, Ok(s) if s.success()) {
        return Ok(());
    }
    Err("当前平台没有回收站".into())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn is_packaged_install() -> bool {
    false
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn update_cache_dir() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("QuickGet").join("update-cache"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn open_url(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn cleanup_previous_update_leftovers() {}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn apply_update_and_restart(_payload: &std::path::Path) -> Result<(), String> {
    Err("当前平台不支持自动更新".into())
}
