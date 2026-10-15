//! Windows 自替换：spawn PowerShell helper，等父进程退出后换 exe 再拉起。
//!
//! 运行中的 exe 可以 rename、不能覆盖。因此采用 rename-first：
//! 任一时刻磁盘上至少有一份完整二进制。

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// 当前进程是否来自发行安装（不在 cargo target 目录里）。
pub fn is_packaged_install() -> bool {
    match std::env::current_exe() {
        Ok(exe) => !crate::core::updater::looks_like_dev_build(&exe),
        Err(_) => false,
    }
}

/// 更新缓存目录：`<config_dir>/QuickGet/update-cache`。
pub fn update_cache_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("QuickGet").join("update-cache"))
}

/// 用系统默认浏览器打开 http(s) 链接。
pub fn open_url(url: &str) {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return;
    }
    let wide_url: Vec<u16> = std::ffi::OsStr::new(url)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let wide_open: Vec<u16> = std::ffi::OsStr::new("open")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        winapi::um::shellapi::ShellExecuteW(
            std::ptr::null_mut(),
            wide_open.as_ptr(),
            wide_url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            winapi::um::winuser::SW_SHOWNORMAL,
        );
    }
}

/// 启动时清理上次更新留下的 `.old` 备份。
pub fn cleanup_previous_update_leftovers() {
    if let Ok(exe) = std::env::current_exe() {
        gpui_app_update_installer::remove_exe_backup(&exe);
    }
}

/// `payload` 为解压出的新 `quickget.exe`。
///
/// 预检留在本产品；通过后由共享安装器写 helper 并 spawn，调用方随后退出进程。
/// helper rename-first：任一时刻磁盘上至少有一份完整二进制，copy 失败时回滚 `.old`。
/// helper 以新进程组启动并尝试脱离父进程 Job，Job 不允许时退回不脱离的方式。
pub fn apply_update_and_restart(payload: &Path) -> Result<(), String> {
    if !payload.is_file() {
        return Err(format!("更新包缺失载荷: {}", payload.display()));
    }
    let current =
        std::env::current_exe().map_err(|e| format!("获取当前可执行文件路径失败: {e}"))?;
    if current == payload {
        return Err("载荷与当前正在运行的程序相同".into());
    }

    // 预检：目标安装目录必须可写。
    let Some(parent) = current.parent() else {
        return Err("当前程序没有父目录".into());
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
    gpui_app_update_installer::spawn_windows_exe_replacement(&current, payload, &cache)
        .map_err(crate::platform::install_error_message)
}
