//! 浏览器扩展投递的任务：收件箱 + Native Messaging 宿主注册。
//!
//! Chrome 每次 `sendNativeMessage` 都会新拉起一个进程。那个进程不能开 GUI，
//! 只把任务写进收件箱；正在跑的 GUI 轮询收件箱入队。这样不会打断当前下载。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const HOST_NAME: &str = "com.quickget.host";
pub const EXTENSION_ID: &str = "agkijomhcpkgagodkkjknfnocfnalmcc";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureJob {
    pub url: String,
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub referer: Option<String>,
    #[serde(default)]
    pub ua: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
}

pub fn inbox_dir() -> Option<PathBuf> {
    crate::core::settings::Settings::dir().map(|d| d.join("inbox"))
}

pub fn enqueue(job: &CaptureJob) -> Result<PathBuf, String> {
    let dir = inbox_dir().ok_or_else(|| "找不到配置目录".to_string())?;
    enqueue_in(&dir, job)
}

pub fn drain() -> Vec<CaptureJob> {
    let Some(dir) = inbox_dir() else {
        return Vec::new();
    };
    drain_in(&dir)
}

pub fn enqueue_in(dir: &Path, job: &CaptureJob) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = format!("{}.json", uuid::Uuid::new_v4());
    let path = dir.join(name);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string(job).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn drain_in(dir: &Path) -> Vec<CaptureJob> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut jobs = Vec::new();
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    for path in files {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let _ = fs::remove_file(&path);
        if let Ok(job) = serde_json::from_str::<CaptureJob>(&text) {
            if !job.url.trim().is_empty() {
                jobs.push(job);
            }
        }
    }
    jobs
}

pub fn native_host_manifest(exe: &Path) -> String {
    let origin = format!("chrome-extension://{EXTENSION_ID}/");
    serde_json::json!({
        "name": HOST_NAME,
        "description": "QuickGet native messaging host",
        "path": exe.display().to_string(),
        "type": "stdio",
        "allowed_origins": [origin],
    })
    .to_string()
}

/// 把 Native Messaging 清单写到各 Chromium 换壳的目录。失败不致命。
pub fn install_native_host() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(exe) = exe.canonicalize().ok().or(Some(exe)) else {
        return;
    };
    let text = native_host_manifest(&exe);
    let filename = format!("{HOST_NAME}.json");

    if let Some(dir) = crate::core::settings::Settings::dir() {
        let _ = fs::create_dir_all(&dir);
        let dest = dir.join(&filename);
        let _ = fs::write(&dest, &text);
        #[cfg(windows)]
        crate::platform::windows::register_native_host(&dest);
    }

    for dir in chromium_host_dirs() {
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join(&filename), &text);
    }
}

fn chromium_host_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    {
        let Some(home) = dirs::home_dir() else {
            return out;
        };
        let app = home.join("Library/Application Support");
        for rel in [
            "Google/Chrome/NativeMessagingHosts",
            "Google/Chrome Beta/NativeMessagingHosts",
            "Google/Chrome Canary/NativeMessagingHosts",
            "Google/Chrome Dev/NativeMessagingHosts",
            "Chromium/NativeMessagingHosts",
            "Microsoft Edge/NativeMessagingHosts",
            "Microsoft Edge Beta/NativeMessagingHosts",
            "Microsoft Edge Canary/NativeMessagingHosts",
            "BraveSoftware/Brave-Browser/NativeMessagingHosts",
            "BraveSoftware/Brave-Browser-Beta/NativeMessagingHosts",
            "BraveSoftware/Brave-Browser-Nightly/NativeMessagingHosts",
            "Vivaldi/NativeMessagingHosts",
            "Arc/User Data/NativeMessagingHosts",
            "com.operasoftware.Opera/NativeMessagingHosts",
            "com.operasoftware.OperaGX/NativeMessagingHosts",
            "CocCoc/NativeMessagingHosts",
            "Yandex/YandexBrowser/NativeMessagingHosts",
        ] {
            out.push(app.join(rel));
        }
    }
    #[cfg(windows)]
    {
        if let Some(local) = dirs::data_local_dir() {
            for rel in [
                r"Google\Chrome\User Data\NativeMessagingHosts",
                r"Chromium\User Data\NativeMessagingHosts",
                r"Microsoft\Edge\User Data\NativeMessagingHosts",
                r"BraveSoftware\Brave-Browser\User Data\NativeMessagingHosts",
                r"Vivaldi\User Data\NativeMessagingHosts",
            ] {
                out.push(local.join(rel));
            }
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        if let Some(config) = dirs::config_dir() {
            for rel in [
                "google-chrome/NativeMessagingHosts",
                "chromium/NativeMessagingHosts",
                "microsoft-edge/NativeMessagingHosts",
                "BraveSoftware/Brave-Browser/NativeMessagingHosts",
            ] {
                out.push(config.join(rel));
            }
        }
    }
    out
}

/// 若 GUI 没在跑，把本进程的 GUI 拉起来。正在跑则只投递收件箱。
pub fn ensure_gui() {
    if gui_alive() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe).arg("--gui").spawn();
}

pub fn write_pid() {
    let Some(dir) = crate::core::settings::Settings::dir() else {
        return;
    };
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("gui.pid"), std::process::id().to_string());
}

pub fn gui_alive() -> bool {
    let Some(dir) = crate::core::settings::Settings::dir() else {
        return false;
    };
    let Ok(text) = fs::read_to_string(dir.join("gui.pid")) else {
        return false;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        return false;
    };
    pid_alive(pid)
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        unsafe {
            if libc::kill(pid as i32, 0) == 0 {
                true
            } else {
                std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            }
        }
    }
    #[cfg(windows)]
    {
        use winapi::um::processthreadsapi::OpenProcess;
        use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;
        use winapi::um::handleapi::CloseHandle;
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                false
            } else {
                CloseHandle(h);
                true
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_round_trips() {
        let dir = std::env::temp_dir().join(format!("qg-inbox-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let job = CaptureJob {
            url: "https://example.com/a.zip".into(),
            cookies: Some("sid=1".into()),
            referer: Some("https://example.com/".into()),
            ua: None,
            filename: Some("a.zip".into()),
        };
        enqueue_in(&dir, &job).unwrap();
        let got = drain_in(&dir);
        assert_eq!(got, vec![job]);
        assert!(drain_in(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_contains_stable_extension_id() {
        let m = native_host_manifest(Path::new("/opt/quickget"));
        assert!(m.contains(EXTENSION_ID));
        assert!(m.contains(HOST_NAME));
        assert!(m.contains("/opt/quickget"));
    }
}
