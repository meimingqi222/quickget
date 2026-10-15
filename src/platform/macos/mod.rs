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

/// 启动时清理上次更新留下的 `QuickGet.app.old`。
pub fn cleanup_previous_update_leftovers() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if let Some(bundle) = gpui_app_update_installer::enclosing_app_bundle(&exe) {
        gpui_app_update_installer::remove_bundle_backup(&bundle);
    }
}

/// `payload` 为解压出的 `QuickGet.app`。
///
/// 只支持从 `.app` bundle 运行的安装；预检通过后由共享安装器的 shell helper
/// 等本进程退出、rename-first 替换 bundle 并 `open`，copy 失败时回滚 `.old`。
/// helper 以 `setsid` + `trap '' HUP` 脱离本进程会话。不做签名校验，也不提权：
/// 安装目录不可写（如需管理员权限的 /Applications）时直接报错。
pub fn apply_update_and_restart(payload: &Path) -> Result<(), String> {
    if !payload.is_dir() {
        return Err(format!("更新包缺失载荷: {}", payload.display()));
    }
    let exe = std::env::current_exe().map_err(|e| format!("获取当前可执行文件路径失败: {e}"))?;
    let current_bundle = gpui_app_update_installer::enclosing_app_bundle(&exe)
        .ok_or_else(|| "当前程序不在 .app 包内运行，无法自动替换".to_string())?;
    if current_bundle == payload {
        return Err("载荷与当前正在运行的程序相同".into());
    }

    let Some(parent) = current_bundle.parent() else {
        return Err("当前 .app 包没有父目录".into());
    };
    let probe = parent.join(format!(".qg-update-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
        }
        Err(e) => {
            return Err(format!("安装目录无写入权限 ({}): {e}", parent.display()));
        }
    }

    let cache = update_cache_dir().ok_or_else(|| "无法获取用户缓存目录".to_string())?;
    std::fs::create_dir_all(&cache).map_err(|e| format!("创建更新缓存目录失败: {e}"))?;
    gpui_app_update_installer::spawn_macos_bundle_replacement(&current_bundle, payload, &cache)
        .map_err(crate::platform::install_error_message)
}
