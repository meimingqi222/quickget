//! 界面文案。中英各一份，渲染时按当前语言取。

use crate::core::i18n::Language;
use crate::core::model::TaskStatus;

pub fn tr_app(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "QuickGet",
        Language::En => "QuickGet",
    }
}

pub fn tr_view_all(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "全部",
        Language::En => "All",
    }
}
pub fn tr_view_active(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "进行中",
        Language::En => "Active",
    }
}
pub fn tr_view_done(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "已完成",
        Language::En => "Done",
    }
}
pub fn tr_view_failed(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "失败",
        Language::En => "Failed",
    }
}
pub fn tr_view_settings(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "设置",
        Language::En => "Settings",
    }
}

pub fn tr_url_placeholder(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "粘贴链接，回车开始下载",
        Language::En => "Paste a URL, press Return",
    }
}

pub fn tr_btn_add(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "下载",
        Language::En => "Download",
    }
}
pub fn tr_btn_paste(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "粘贴",
        Language::En => "Paste",
    }
}
pub fn tr_btn_pause(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "暂停",
        Language::En => "Pause",
    }
}
pub fn tr_btn_resume(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "继续",
        Language::En => "Resume",
    }
}
pub fn tr_btn_retry(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "重试",
        Language::En => "Retry",
    }
}
pub fn tr_btn_remove(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "删除",
        Language::En => "Remove",
    }
}
pub fn tr_btn_open(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "打开",
        Language::En => "Open",
    }
}
pub fn tr_btn_reveal(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "定位",
        Language::En => "Reveal",
    }
}
pub fn tr_btn_clear_done(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "清除已完成",
        Language::En => "Clear completed",
    }
}
pub fn tr_btn_browse(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "选择目录",
        Language::En => "Browse",
    }
}

pub fn tr_empty_all(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "暂无任务。粘贴链接开始下载。",
        Language::En => "No tasks. Paste a URL to start.",
    }
}
pub fn tr_empty_active(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "没有正在进行的下载。",
        Language::En => "No active downloads.",
    }
}
pub fn tr_empty_done(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "还没有已完成的任务。",
        Language::En => "No completed downloads.",
    }
}
pub fn tr_empty_failed(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "没有失败的任务。",
        Language::En => "No failed downloads.",
    }
}

pub fn tr_status_ready(lang: Language) -> String {
    match lang {
        Language::Zh => "就绪".into(),
        Language::En => "Ready".into(),
    }
}
pub fn tr_status_added(lang: Language, name: &str) -> String {
    match lang {
        Language::Zh => format!("已加入队列：{name}"),
        Language::En => format!("Queued: {name}"),
    }
}
pub fn tr_status_trashed(lang: Language) -> String {
    match lang {
        Language::Zh => "已移到废纸篓".into(),
        Language::En => "Moved to Trash".into(),
    }
}
pub fn tr_status_already(lang: Language, name: &str) -> String {
    match lang {
        Language::Zh => format!("已在下载：{name}"),
        Language::En => format!("Already downloading: {name}"),
    }
}
pub fn tr_status_bad_url(lang: Language) -> String {
    match lang {
        Language::Zh => "无法识别的链接。".into(),
        Language::En => "Unrecognized URL.".into(),
    }
}
pub fn tr_status_magnet(lang: Language) -> String {
    match lang {
        Language::Zh => "暂不支持磁力链接。".into(),
        Language::En => "Magnet links are not supported yet.".into(),
    }
}
pub fn tr_status_done(lang: Language, name: &str) -> String {
    match lang {
        Language::Zh => format!("下载完成：{name}"),
        Language::En => format!("Completed: {name}"),
    }
}
pub fn tr_status_fail(lang: Language, msg: &str) -> String {
    match lang {
        Language::Zh => format!("下载失败：{msg}"),
        Language::En => format!("Failed: {msg}"),
    }
}
pub fn tr_status_paused(lang: Language, name: &str) -> String {
    match lang {
        Language::Zh => format!("已暂停：{name}"),
        Language::En => format!("Paused: {name}"),
    }
}
pub fn tr_status_speed(lang: Language, speed: &str, n: usize) -> String {
    match lang {
        Language::Zh => format!("下载中 {n} 项 · {speed}"),
        Language::En => format!("{n} downloading · {speed}"),
    }
}
pub fn tr_clipboard_hint(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "剪贴板中有链接，是否下载？",
        Language::En => "Clipboard has a URL. Download it?",
    }
}

pub fn tr_status_label(lang: Language, s: TaskStatus) -> &'static str {
    match (lang, s) {
        (Language::Zh, TaskStatus::Queued) => "排队中",
        (Language::En, TaskStatus::Queued) => "Queued",
        (Language::Zh, TaskStatus::Probing) => "探测中",
        (Language::En, TaskStatus::Probing) => "Probing",
        (Language::Zh, TaskStatus::Downloading) => "下载中",
        (Language::En, TaskStatus::Downloading) => "Downloading",
        (Language::Zh, TaskStatus::Paused) => "已暂停",
        (Language::En, TaskStatus::Paused) => "Paused",
        (Language::Zh, TaskStatus::Completed) => "已完成",
        (Language::En, TaskStatus::Completed) => "Done",
        (Language::Zh, TaskStatus::Failed) => "失败",
        (Language::En, TaskStatus::Failed) => "Failed",
        (Language::Zh, TaskStatus::Cancelled) => "取消",
        (Language::En, TaskStatus::Cancelled) => "Cancelled",
    }
}

pub fn tr_settings_folder(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "保存到",
        Language::En => "Save to",
    }
}
pub fn tr_settings_conn(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "单文件连接数",
        Language::En => "Connections per file",
    }
}
pub fn tr_settings_conc(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "同时下载数",
        Language::En => "Concurrent downloads",
    }
}
pub fn tr_settings_ua(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "User-Agent",
        Language::En => "User-Agent",
    }
}
pub fn tr_settings_clip(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "监听剪贴板",
        Language::En => "Watch clipboard",
    }
}
pub fn tr_settings_lang(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "界面语言",
        Language::En => "Language",
    }
}
pub fn tr_settings_blurb(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "支持 HTTP / HTTPS / FTP / HLS。多连接分段下载，支持断点续传。",
        Language::En => "HTTP, HTTPS, FTP, HLS. Multi-connection segmented download with resume.",
    }
}
pub fn tr_settings_ext(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "Chromium 扩展：右键链接投递到 QuickGet，并带上登录 Cookie。默认不接管浏览器下载。",
        Language::En => "Chromium extension: right-click a link to send it here with cookies. Browser downloads are not hijacked by default.",
    }
}
pub fn tr_on(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "开",
        Language::En => "On",
    }
}
pub fn tr_off(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "关",
        Language::En => "Off",
    }
}
