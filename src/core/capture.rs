//! 浏览器扩展投递的任务：收件箱 + Native Messaging 宿主注册。
//!
//! Chrome 每次 `sendNativeMessage` 都会新拉起一个进程。那个进程不能开 GUI，
//! 只把任务写进收件箱；正在跑的 GUI 轮询收件箱入队。这样不会打断当前下载。

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
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
    let _lock = lock_path(&dir.join(".lock"), true);
    drain_in_unlocked(dir)
}

fn drain_in_unlocked(dir: &Path) -> Vec<CaptureJob> {
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
        let claimed = path.with_extension("claimed");
        if fs::rename(&path, &claimed).is_err() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&claimed) else {
            let _ = fs::remove_file(&claimed);
            continue;
        };
        let _ = fs::remove_file(&claimed);
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

/// 进程活着期间持有的 GUI 单实例锁。掉了锁就表示这个 GUI 退出了。
pub struct GuiLock {
    _file: File,
}

const GUI_SPAWN_STAMP: &str = "gui.spawn";
const GUI_SPAWN_QUIET_MS: u128 = 2500;

/// 若 GUI 没在跑，把本进程的 GUI 拉起来。正在跑则只投递收件箱。
pub fn ensure_gui() {
    if gui_alive() {
        request_raise();
        return;
    }
    let Some(dir) = crate::core::settings::Settings::dir() else {
        return;
    };
    let stamp = dir.join(GUI_SPAWN_STAMP);
    if spawn_stamp_fresh(&stamp, GUI_SPAWN_QUIET_MS) {
        return;
    }
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&stamp, b"1");
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe).arg("--gui").spawn();
}

fn spawn_stamp_fresh(path: &Path, quiet_ms: u128) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    modified
        .elapsed()
        .map(|d| d.as_millis() < quiet_ms)
        .unwrap_or(false)
}

/// 第二个 GUI 进程抢不到锁时写这个文件，已有 GUI 轮询到就前置窗口。
pub fn request_raise() {
    let Some(dir) = inbox_dir() else {
        return;
    };
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join(".raise"), b"1");
}

pub fn take_raise() -> bool {
    let Some(dir) = inbox_dir() else {
        return false;
    };
    fs::remove_file(dir.join(".raise")).is_ok()
}

pub fn try_acquire_gui_lock() -> Option<GuiLock> {
    let dir = crate::core::settings::Settings::dir()?;
    let _ = fs::create_dir_all(&dir);
    lock_path(&dir.join("gui.lock"), false)
}

fn lock_path(path: &Path, blocking: bool) -> Option<GuiLock> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).ok()?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .ok()?;
    if !lock_exclusive(&file, blocking) {
        return None;
    }
    Some(GuiLock { _file: file })
}

fn lock_exclusive(file: &File, blocking: bool) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let mut flags = libc::LOCK_EX;
        if !blocking {
            flags |= libc::LOCK_NB;
        }
        unsafe { libc::flock(file.as_raw_fd(), flags) == 0 }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let mut flags = LOCKFILE_EXCLUSIVE_LOCK;
        if !blocking {
            flags |= LOCKFILE_FAIL_IMMEDIATELY;
        }
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        unsafe {
            LockFileEx(
                file.as_raw_handle() as winapi::shared::ntdef::HANDLE,
                flags,
                0,
                1,
                0,
                &mut overlapped,
            ) != 0
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, blocking);
        true
    }
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
        use winapi::um::handleapi::CloseHandle;
        use winapi::um::processthreadsapi::OpenProcess;
        use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;
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
    fn exclusive_lock_blocks_second() {
        let dir = std::env::temp_dir().join(format!("qg-lock-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gui.lock");
        let a = lock_path(&path, false).expect("first lock");
        assert!(lock_path(&path, false).is_none());
        drop(a);
        let mut unlocked = lock_path(&path, false);
        for _ in 0..20 {
            if unlocked.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            unlocked = lock_path(&path, false);
        }
        assert!(unlocked.is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_drain_takes_each_job_once() {
        let dir = std::env::temp_dir().join(format!("qg-inbox-race-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        for i in 0..8 {
            enqueue_in(
                &dir,
                &CaptureJob {
                    url: format!("https://example.com/{i}.zip"),
                    ..CaptureJob::default()
                },
            )
            .unwrap();
        }
        let a_dir = dir.clone();
        let b_dir = dir.clone();
        let a = std::thread::spawn(move || drain_in(&a_dir));
        let b = std::thread::spawn(move || drain_in(&b_dir));
        let mut all = a.join().unwrap();
        all.extend(b.join().unwrap());
        all.sort_by(|x, y| x.url.cmp(&y.url));
        assert_eq!(all.len(), 8);
        all.dedup();
        assert_eq!(all.len(), 8);
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

    #[test]
    fn missing_spawn_stamp_is_not_fresh() {
        let path = std::env::temp_dir().join(format!("qg-no-stamp-{}", uuid::Uuid::new_v4()));
        assert!(!spawn_stamp_fresh(&path, 2500));
    }

    #[test]
    fn extension_skips_history_replay() {
        let status = std::process::Command::new("node")
            .arg("extension/policy_test.js")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("node");
        assert!(
            status.status.success(),
            "{}{}",
            String::from_utf8_lossy(&status.stderr),
            String::from_utf8_lossy(&status.stdout)
        );
    }

    #[test]
    fn recent_spawn_stamp_blocks_second_launch() {
        let dir = std::env::temp_dir().join(format!("qg-spawn-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gui.spawn");
        fs::write(&path, b"1").unwrap();
        assert!(spawn_stamp_fresh(&path, 2500));
        assert!(!spawn_stamp_fresh(&path, 0));
        let _ = fs::remove_dir_all(&dir);
    }
}
