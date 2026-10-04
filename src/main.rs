//! QuickGet 高速多协议下载器

#![windows_subsystem = "windows"]

use gpui::{actions, px, size, App, AppContext, Application, Bounds, WindowBounds, WindowOptions};
use quickget::core::engine::run_task;
use quickget::core::limiter::DownloadLimiter;
use quickget::core::model::Task;
use quickget::core::progress::{Control, JobOutcome, LiveMeta, LiveProgress};
use quickget::core::settings::{default_download_dir, default_ua};
use quickget::core::urlx::{detect_protocol, filename_from_url, unique_path, Protocol};
use quickget::ui::Root;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

actions!(quickget, [Quit]);

fn main() {
    #[cfg(windows)]
    quickget::platform::windows::attach_parent_console();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if quickget::core::native_host::is_native_host_invocation(&args) {
        quickget::core::native_host::run();
        return;
    }
    if args.iter().any(|a| a == "--install-host") {
        quickget::core::capture::install_native_host();
        eprintln!("已注册 Chromium Native Messaging 宿主。");
        return;
    }
    if !args.is_empty() && args.iter().all(|a| a != "--gui") {
        cli_main(&args);
        return;
    }

    if quickget::core::capture::gui_alive() {
        eprintln!("QuickGet 已在运行，请求前置已有窗口。");
        eprintln!("若确认没有任何窗口在跑，删除配置目录里的 gui.pid 后重试。");
        quickget::core::capture::request_raise();
        return;
    }
    let Some(_gui_lock) = quickget::core::capture::try_acquire_gui_lock() else {
        eprintln!("另一个 QuickGet 实例正持有单实例锁。");
        eprintln!("若确认没有窗口在跑，删除配置目录里的 gui.lock 后重试。");
        quickget::core::capture::request_raise();
        return;
    };
    // 抢到锁立刻写 pid，后面的 native host 就不会再各拉起一个 GUI。
    quickget::core::capture::write_pid();

    quickget::core::log::init();
    quickget::core::log::install_panic_hook();

    Application::new().run(move |cx: &mut App| {
        #[cfg(target_os = "macos")]
        quickget::platform::macos::set_dock_icon();

        #[cfg(target_os = "macos")]
        {
            cx.set_menus(vec![gpui::Menu {
                name: "QuickGet".into(),
                items: vec![gpui::MenuItem::action("Quit QuickGet", Quit)],
            }]);
            cx.bind_keys([
                gpui::KeyBinding::new("cmd-q", Quit, None),
                gpui::KeyBinding::new("cmd-w", Quit, None),
            ]);
        }
        #[cfg(not(target_os = "macos"))]
        cx.bind_keys([
            gpui::KeyBinding::new("ctrl-q", Quit, None),
            gpui::KeyBinding::new("alt-f4", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(1040.), px(680.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    true
                });
                cx.new(|cx| Root::new(cx))
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

fn cli_main(args: &[String]) {
    let mut dir = default_download_dir();
    let mut connections = 16u32;
    let mut limit_kib = 0u64;
    let mut proxy = None;
    let mut urls = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" | "-d" => {
                if let Some(p) = args.get(i + 1) {
                    dir = PathBuf::from(p);
                    i += 2;
                    continue;
                }
            }
            "--connections" | "-c" => {
                if let Some(n) = args.get(i + 1).and_then(|s| s.parse().ok()) {
                    connections = n;
                    i += 2;
                    continue;
                }
            }
            "--limit" => {
                if let Some(n) = args.get(i + 1).and_then(|s| s.parse().ok()) {
                    limit_kib = n;
                    i += 2;
                    continue;
                }
            }
            "--proxy" => {
                if let Some(value) = args.get(i + 1) {
                    proxy = Some(value.clone());
                    i += 2;
                    continue;
                }
            }
            "--help" | "-h" => {
                eprintln!(
                    "QuickGet\n  quickget <url>...\n  --dir <path>   保存目录\n  --connections N  连接数（默认 16）\n  --limit KiB/s  全局限速（0 不限）\n  --proxy URL    HTTP/SOCKS5 代理\n  --gui          打开界面"
                );
                return;
            }
            s => urls.push(s.to_string()),
        }
        i += 1;
    }
    if urls.is_empty() {
        eprintln!("请提供 URL。用法见 quickget --help。");
        std::process::exit(2);
    }

    let ua = default_ua();
    let limiter = DownloadLimiter::new(limit_kib.saturating_mul(1024));
    let mut failed = 0;
    for url in urls {
        let proto = detect_protocol(&url);
        if proto == Protocol::Unknown {
            eprintln!("无法识别的协议：{url}");
            failed += 1;
            continue;
        }
        let name = if proto == Protocol::Hls {
            quickget::core::hls::default_hls_filename(&url)
        } else if proto == Protocol::Magnet {
            quickget::core::urlx::filename_from_source(&url)
        } else {
            filename_from_url(&url)
        };
        let (filename, dest) = unique_path(&dir, &name);
        eprintln!("→ {filename}");
        let output_dir = if proto == Protocol::Magnet {
            Some(dest.clone())
        } else {
            None
        };
        let mut task = Task::new(proto, url.clone(), filename, dir.clone(), connections, None, output_dir);
        // 命令行下载：把「排队时刻」放到启动前，不记入耗时。
        task.started_at = None;
        let progress = LiveProgress::new(0, 0);
        let ctrl = Control {
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
        };
        match run_task(
            &task,
            ua,
            progress.clone(),
            ctrl,
            LiveMeta::new(),
            limiter.clone(),
            proxy.as_deref(),
        ) {
            JobOutcome::Completed { size, path } => {
                eprintln!(
                    "  完成 {} ({})",
                    path.display(),
                    quickget::core::model::fmt_size(size)
                );
                let _ = dest;
            }
            JobOutcome::Failed(e) => {
                eprintln!("  失败：{e}");
                failed += 1;
            }
            other => {
                eprintln!("  已中止：{other:?}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
}
