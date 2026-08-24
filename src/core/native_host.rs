//! Chrome Native Messaging：从 stdin 读一条 JSON，写入收件箱后退出。

use crate::core::capture::{ensure_gui, enqueue, CaptureJob};
use serde_json::{json, Value};
use std::io::{Read, Write};

pub fn is_native_host_invocation(args: &[String]) -> bool {
    args.iter().any(|a| {
        a == "--native-host" || a.starts_with("chrome-extension://")
    })
}

pub fn run() {
    set_stdio_binary();
    while let Some(msg) = read_msg() {
        let reply = handle(msg);
        if !write_msg(&reply) {
            break;
        }
    }
}

fn handle(msg: Value) -> Value {
    let job = match serde_json::from_value::<CaptureJob>(msg) {
        Ok(j) if !j.url.trim().is_empty() => j,
        Ok(_) => return json!({"ok": false, "error": "empty url"}),
        Err(e) => return json!({"ok": false, "error": e.to_string()}),
    };
    match enqueue(&job) {
        Ok(_) => {
            ensure_gui();
            json!({"ok": true})
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
    if stdout.write_all(&(bytes.len() as u32).to_le_bytes()).is_err() {
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
}
