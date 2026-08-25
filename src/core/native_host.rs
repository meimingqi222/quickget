//! Chrome Native Messaging：从 stdin 读一条 JSON，写入收件箱后退出。

use crate::core::capture::{enqueue, ensure_gui, gui_alive, CaptureJob};
use serde_json::{json, Value};
use std::io::{Read, Write};

pub fn is_native_host_invocation(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--native-host" || a.starts_with("chrome-extension://"))
}

pub fn run() {
    set_stdio_binary();
    // 看门狗：浏览器侧协议卡住时（实测 Edge 拉起宿主后从不完成握手），
    // 宿主会永远阻塞在 read_msg 上，每点一次下载就攒一个僵尸进程。
    // 一次性消息正常 1 秒内完成，60 秒兜底退出足够宽裕。
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(60));
        std::process::exit(0);
    });
    while let Some(msg) = read_msg() {
        let reply = handle(msg);
        if !write_msg(&reply) {
            break;
        }
    }
}

pub(crate) fn handle(msg: Value) -> Value {
    if msg.get("ping").and_then(|v| v.as_bool()) == Some(true) {
        ensure_gui();
        return json!({"ok": true, "gui": gui_alive()});
    }
    let job = match serde_json::from_value::<CaptureJob>(msg) {
        Ok(j) if !j.url.trim().is_empty() => j,
        Ok(_) => return json!({"ok": false, "error": "empty url"}),
        Err(e) => return json!({"ok": false, "error": e.to_string()}),
    };
    match enqueue(&job) {
        Ok(_) => {
            ensure_gui();
            json!({"ok": true, "gui": gui_alive()})
        }
        Err(e) => json!({"ok": false, "error": e}),
    }
}

fn read_msg() -> Option<Value> {
    let mut stdin = std::io::stdin().lock();
    let mut len_buf = [0u8; 4];
    stdin.read_exact(&mut len_buf).ok()?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > 4 * 1024 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; len];
    stdin.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn write_msg(v: &Value) -> bool {
    let Ok(bytes) = serde_json::to_vec(v) else {
        return false;
    };
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .is_err()
    {
        return false;
    }
    if stdout.write_all(&bytes).is_err() {
        return false;
    }
    stdout.flush().is_ok()
}

fn set_stdio_binary() {
    #[cfg(windows)]
    {
        extern "C" {
            fn _setmode(fd: i32, mode: i32) -> i32;
        }
        const O_BINARY: i32 = 0x8000;
        unsafe {
            _setmode(0, O_BINARY);
            _setmode(1, O_BINARY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chrome_origin_arg() {
        assert!(is_native_host_invocation(&[
            "chrome-extension://agkijomhcpkgagodkkjknfnocfnalmcc/".into()
        ]));
        assert!(is_native_host_invocation(&["--native-host".into()]));
        assert!(!is_native_host_invocation(&[
            "https://example.com/a.zip".into()
        ]));
    }

    #[test]
    fn empty_url_rejected() {
        let v = handle(json!({"url": ""}));
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("empty"));
    }
}
