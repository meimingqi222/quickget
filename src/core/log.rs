//! 文件日志。尽力而为，写不进去就静默放弃。

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const FILE_NAME: &str = "quickget.log";
const OLD_FILE_NAME: &str = "quickget.log.1";
const MAX_BYTES: u64 = 2 * 1024 * 1024;

static SINK: OnceLock<Option<Mutex<File>>> = OnceLock::new();

pub fn path() -> Option<PathBuf> {
    Some(dir()?.join(FILE_NAME))
}

fn dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("QuickGet"))
}

fn open() -> Option<Mutex<File>> {
    let path = path()?;
    let parent = path.parent()?;
    std::fs::create_dir_all(parent).ok()?;
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_BYTES {
        let old = parent.join(OLD_FILE_NAME);
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&path, &old);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    Some(Mutex::new(file))
}

pub fn init() {
    write(format_args!(
        "===== QuickGet v{} 启动 | pid={} =====",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));
}

pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<未命名>").to_string();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<位置未知>".into());
        write(format_args!(
            "!!!!! panic 于 {location}（线程 {name}）: {}",
            panic_message(info)
        ));
        write(format_args!(
            "调用栈:\n{}",
            std::backtrace::Backtrace::force_capture()
        ));
        previous(info);
    }));
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<无法识别的 panic payload>".into()
    }
}

pub fn write(args: std::fmt::Arguments) {
    let Some(sink) = SINK.get_or_init(open) else {
        return;
    };
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let mut guard = match sink.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let _ = writeln!(guard, "[{ts}] {args}");
    let _ = guard.flush();
}
