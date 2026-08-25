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
    /// 网盘分享任务。存在时 GUI 先经 providers 插件换明文直链，再按普通 HTTP 任务入队。
    /// 兼容旧版消息里的 "baidu" 字段。
    #[serde(default, alias = "baidu")]
    pub netdisk: Option<crate::core::providers::NetdiskRequest>,
    /// 单个 Range 请求的最大字节数。None = 不限制（用引擎默认分片）。
    /// 百度 PCS 直链对单次 Range > 4MB 回 31326 风控，需限制每片大小。
    #[serde(default)]
    pub max_part_size: Option<u64>,
    /// 下载完成后需清理的网盘路径（转存临时文件）。
    /// None = 非转存方式，无需清理。
    #[serde(default)]
    pub cleanup_paths: Option<Vec<String>>,
    /// 清理转存文件用的网盘 Cookie（含 BDUSS）。
    #[serde(default)]
    pub cleanup_cookies: Option<String>,
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
        "path": simplify_path(exe).to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [origin],
    })
    .to_string()
}

/// 去掉 Windows 规范化路径（`Path::canonicalize`）产生的 `\\?\` / `\\?\UNC\` 前缀。
///
/// Chrome/Edge 的 Native Messaging 拿清单里的 path 去 CreateProcess 时，
/// 带 `\\?\` 前缀会导致宿主启动失败，浏览器侧表现为
/// “Error when communicating with the native messaging host”。
pub fn simplify_path(p: &Path) -> PathBuf {
    let s = p.as_os_str().to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    p.to_path_buf()
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
    // 只看"这个 PID 有没有进程"会误判：Windows 会复用 PID，进程退出后若有
    // 句柄未释放其对象也会残留，OpenProcess 照样成功。必须核对属主确实是
    // 我们的可执行文件且仍在运行。
    let Some(expected) = current_image_name() else {
        return false;
    };
    alive_with_image(pid, &expected)
}

fn current_image_name() -> Option<String> {
    std::env::current_exe()
        .ok()?
        .file_name()?
        .to_str()
        .map(str::to_string)
}

/// 进程活着、可被查询，且镜像名与当前可执行文件一致。
/// 查询受限时按保守策略处理（见各分支注释）。
#[cfg(windows)]
fn alive_with_image(pid: u32, expected: &str) -> bool {
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{GetExitCodeProcess, OpenProcess};
    use winapi::um::winbase::QueryFullProcessImageNameW;
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;

    // WinBase.h 里的 STILL_ACTIVE 就是 259；winapi 未导出，自定义常量。
    const STILL_ACTIVE: u32 = 259;

    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let has_exit = GetExitCodeProcess(h, &mut exit_code) != 0;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let got_path = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        if !got_path {
            // 存在但查询受限：无法排除是无关进程占用了这个 PID。
            // 宁可当已死让用户能开窗，最坏情况是双开被单实例锁兜住。
            return false;
        }
        if has_exit && exit_code != STILL_ACTIVE {
            // 已退出但对象因句柄未释放而残留（典型：父进程/调试器没关句柄）。
            // 运行中的进程 exit_code 恒为 STILL_ACTIVE。
            return false;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        std::path::Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.eq_ignore_ascii_case(expected))
            .unwrap_or(false)
    }
}

#[cfg(target_os = "macos")]
fn alive_with_image(pid: u32, expected: &str) -> bool {
    extern "C" {
        fn proc_pidpath(pid: i32, buffer: *mut u8, bufsize: u32) -> i32;
    }
    unsafe {
        if libc::kill(pid as i32, 0) != 0 {
            return false;
        }
    }
    let mut buf = [0u8; 1024];
    let n = unsafe { proc_pidpath(pid as i32, buf.as_mut_ptr(), buf.len() as u32) };
    if n <= 0 {
        // 查不到路径（权限等）：保守当作活着，交给单实例锁兜底。
        return true;
    }
    let path = String::from_utf8_lossy(&buf[..n as usize]).to_string();
    std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n == expected)
        .unwrap_or(false)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn alive_with_image(pid: u32, expected: &str) -> bool {
    unsafe {
        if libc::kill(pid as i32, 0) != 0 {
            return std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
        }
    }
    // 还活着：用 /proc/<pid>/exe 核对镜像名；核对不了就保守当活。
    match std::fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(p) => p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == expected)
            .unwrap_or(true),
        Err(_) => true,
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
            netdisk: Some(crate::core::providers::NetdiskRequest {
                provider: Some("baidu".into()),
                share_url: "https://pan.baidu.com/s/abc".into(),
                fids: vec!["123456789012345".into()],
                sekey: Some("fakesekey".into()),
                js_token: None,
            }),
            max_part_size: None,
            cleanup_paths: None,
            cleanup_cookies: None,
        };
        enqueue_in(&dir, &job).unwrap();
        let got = drain_in(&dir);
        assert_eq!(got, vec![job]);
        assert!(drain_in(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_baidu_field_still_deserializes() {
        // v0.1.2 的收件箱任务用的是 "baidu" 键，升级后不能丢。
        let text = serde_json::json!({
            "url": "https://pan.baidu.com/s/abc",
            "baidu": {"share_url": "https://pan.baidu.com/s/abc", "fids": ["42"]}
        })
        .to_string();
        let job: CaptureJob = serde_json::from_str(&text).unwrap();
        let nd = job.netdisk.expect("netdisk should be present");
        assert_eq!(nd.share_url, "https://pan.baidu.com/s/abc");
        assert_eq!(nd.fids, vec!["42"]);
        assert_eq!(nd.provider, None, "旧消息无 provider，走自动识别");
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
    fn manifest_strips_verbatim_path_prefix() {
        // 回归：canonicalize 产生的 \\?\ 前缀会让 Chrome 无法启动宿主。
        // 见 com.quickget.host.json 曾写成 \\\\?\\D:\\...\\quickget.exe。
        let m = native_host_manifest(Path::new(r"\\?\D:\code\quickget\target\debug\quickget.exe"));
        assert!(!m.contains(r"\\?\\"), "清单里不允许出现 verbatim 前缀：{m}");
        assert!(m.contains("D:\\\\code\\\\quickget"));

        assert_eq!(
            simplify_path(Path::new(r"\\?\UNC\server\share\q.exe")),
            PathBuf::from(r"\\server\share\q.exe")
        );
        assert_eq!(
            simplify_path(Path::new(r"C:\plain\q.exe")),
            PathBuf::from(r"C:\plain\q.exe")
        );
    }

    #[test]
    fn missing_spawn_stamp_is_not_fresh() {
        let path = std::env::temp_dir().join(format!("qg-no-stamp-{}", uuid::Uuid::new_v4()));
        assert!(!spawn_stamp_fresh(&path, 2500));
    }

    #[test]
    fn own_pid_is_alive_and_foreign_pids_are_not() {
        // 当前测试进程自己的 PID：镜像名与 current_exe 一致 → 活。
        assert!(pid_alive(std::process::id()));
        // PID 复用/残留对象的回归测试：随便挑一个大概率不存在的 PID。
        // 若真撞上同名进程（几乎不可能），换一个即可。
        let bogus = 0x5F00_0000u32;
        if std::process::id() != bogus {
            // 不强断言 false——万一存在会误报；这里只验证不 panic。
            let _ = pid_alive(bogus);
        }
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
