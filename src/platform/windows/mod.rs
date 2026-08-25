//! Windows 平台适配。

use crate::core::i18n::Language;
use std::path::Path;

pub fn detect_system_language() -> Language {
    use winapi::um::winnls::{GetSystemDefaultUILanguage, GetUserDefaultUILanguage};
    const LANG_CHINESE: u16 = 0x04;
    let langid = unsafe {
        let user = GetUserDefaultUILanguage();
        if user == 0 {
            GetSystemDefaultUILanguage()
        } else {
            user
        }
    };
    if langid & 0x3ff == LANG_CHINESE {
        Language::Zh
    } else {
        Language::En
    }
}

pub fn reveal_in_explorer(path: &Path) {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            open_folder(parent);
        }
        return;
    }
    let _ = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(path)
        .spawn();
}

pub fn open_in_default_app(path: &Path) {
    if !path.exists() {
        return;
    }
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", &path.to_string_lossy()])
        .spawn();
}

pub fn open_folder(path: &Path) {
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

pub fn move_to_trash(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::shellapi::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
        SHFILEOPSTRUCTW,
    };

    let mut from: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE as u32,
        pFrom: from.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc != 0 {
        return Err(format!("SHFileOperationW 返回 0x{rc:08X}"));
    }
    Ok(())
}

/// 从已有控制台启动时接上 stdout，从资源管理器双击则保持无黑框。
pub fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFFFFFF;
    unsafe {
        winapi::um::wincon::AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// 把 Native Messaging 清单路径写进当前用户的 Chromium 注册表。
pub fn register_native_host(json_path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use winapi::shared::minwindef::HKEY;
    use winapi::um::winnt::KEY_SET_VALUE;
    use winapi::um::winreg::{RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CURRENT_USER};

    let path = json_path.to_string_lossy();
    let value: Vec<u16> = std::ffi::OsStr::new(path.as_ref())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let keys = [
        r"Software\Google\Chrome\NativeMessagingHosts\com.quickget.host",
        r"Software\Chromium\NativeMessagingHosts\com.quickget.host",
        r"Software\Microsoft\Edge\NativeMessagingHosts\com.quickget.host",
        r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.quickget.host",
    ];
    for key in keys {
        let wkey: Vec<u16> = std::ffi::OsStr::new(key)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let mut hkey: HKEY = std::ptr::null_mut();
            let ok = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wkey.as_ptr(),
                0,
                std::ptr::null_mut(),
                0,
                KEY_SET_VALUE,
                std::ptr::null_mut(),
                &mut hkey,
                std::ptr::null_mut(),
            );
            if ok == 0 && !hkey.is_null() {
                let _ = RegSetValueExW(
                    hkey,
                    std::ptr::null(),
                    0,
                    winapi::um::winnt::REG_SZ,
                    value.as_ptr() as *const u8,
                    ((value.len()) * 2) as u32,
                );
                RegCloseKey(hkey);
            }
        }
    }
}
