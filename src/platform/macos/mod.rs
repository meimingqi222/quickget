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
