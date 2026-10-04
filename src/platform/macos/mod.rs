//! macOS 平台适配。

pub mod dock;
pub mod trash;

pub use dock::set_dock_icon;
pub use trash::move_to_trash;

use crate::core::i18n::Language;
use std::path::Path;

pub fn detect_system_language() -> Language {
    Language::from_locale_tag(&crate::platform::posix_locale_tag())
}

pub fn reveal_in_explorer(path: &Path) {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            open_folder(parent);
        }
        return;
    }
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
}

pub fn open_in_default_app(path: &Path) {
    if !path.exists() {
        return;
    }
    let _ = std::process::Command::new("open").arg(path).spawn();
}

pub fn open_folder(path: &Path) {
    let _ = std::process::Command::new("open").arg(path).spawn();
}

pub fn is_packaged_install() -> bool {
    match std::env::current_exe() {
        Ok(exe) => !crate::core::updater::looks_like_dev_build(&exe),
        Err(_) => false,
    }
}

pub fn update_cache_dir() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("QuickGet").join("update-cache"))
}

pub fn open_url(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}

pub fn cleanup_previous_update_leftovers() {}

pub fn apply_update_and_restart(_payload: &Path) -> Result<(), String> {
    Err("macOS 自动更新将在后续版本支持".into())
}
