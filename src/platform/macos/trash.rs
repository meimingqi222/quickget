//! 把文件移入废纸篓，语义与访达里按删除键一致。

use objc::runtime::{Object, BOOL, NO};
use objc::{class, msg_send, sel, sel_impl};
use std::path::Path;

pub fn move_to_trash(path: &Path) -> Result<(), String> {
    match move_to_trash_with_file_manager(path) {
        Ok(()) => Ok(()),
        Err(file_manager_error) => move_to_trash_with_finder(path).map_err(|finder_error| {
            format!("{file_manager_error}；Finder 回退也失败：{finder_error}")
        }),
    }
}

fn move_to_trash_with_file_manager(path: &Path) -> Result<(), String> {
    let s = path.to_str().ok_or("路径不是合法 UTF-8")?;
    unsafe {
        let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
        let ns_path: *mut Object = msg_send![class!(NSString), alloc];
        let ns_path: *mut Object = msg_send![
            ns_path,
            initWithBytes: s.as_ptr() as *const std::ffi::c_void
            length: s.len()
            encoding: 4usize
        ];
        if ns_path.is_null() {
            let _: () = msg_send![pool, drain];
            return Err("NSString 创建失败".into());
        }
        let url: *mut Object = msg_send![class!(NSURL), fileURLWithPath: ns_path];
        if url.is_null() {
            let _: () = msg_send![ns_path, release];
            let _: () = msg_send![pool, drain];
            return Err("NSURL 创建失败".into());
        }
        let fm: *mut Object = msg_send![class!(NSFileManager), defaultManager];
        let mut err: *mut Object = std::ptr::null_mut();
        let ok: BOOL = msg_send![
            fm,
            trashItemAtURL: url
            resultingItemURL: std::ptr::null_mut::<*mut Object>()
            error: &mut err
        ];
        let result = if ok == NO {
            Err(if err.is_null() {
                "移入废纸篓失败".to_string()
            } else {
                let desc: *mut Object = msg_send![err, localizedDescription];
                nsstring_to_string(desc).unwrap_or_else(|| "移入废纸篓失败".into())
            })
        } else {
            Ok(())
        };
        let _: () = msg_send![ns_path, release];
        let _: () = msg_send![pool, drain];
        result
    }
}

fn move_to_trash_with_finder(path: &Path) -> Result<(), String> {
    let script = r#"on run argv
set targetPath to POSIX file (item 1 of argv)
tell application "Finder" to delete targetPath
end run"#;
    let mut child = std::process::Command::new("osascript")
        .arg("-")
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动 Finder：{e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        if let Err(e) = stdin.write_all(script.as_bytes()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("无法调用 Finder：{e}"));
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 Finder 失败：{e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

unsafe fn nsstring_to_string(s: *mut Object) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let utf8: *const std::ffi::c_char = msg_send![s, UTF8String];
    if utf8.is_null() {
        return None;
    }
    std::ffi::CStr::from_ptr(utf8)
        .to_str()
        .ok()
        .map(str::to_owned)
}
