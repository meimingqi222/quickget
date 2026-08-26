//! 界面文案。中英各一份，渲染时按当前语言取。

use crate::core::i18n::Language;
use crate::core::model::{BtPeers, TaskStatus};

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
        Language::Zh => "粘贴链接或磁力链接，回车开始下载",
        Language::En => "Paste a URL or magnet link, press Return",
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
pub fn tr_btn_pause_all(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "全部暂停",
        Language::En => "Pause all",
    }
}
pub fn tr_btn_resume_all(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "全部继续",
        Language::En => "Resume all",
    }
}
pub fn tr_btn_clear_done(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "清除已完成",
        Language::En => "Clear completed",
    }
}
pub fn tr_btn_cancel(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "取消",
        Language::En => "Cancel",
    }
}
pub fn tr_confirm_delete_title(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "删除这个任务？",
        Language::En => "Delete this task?",
    }
}
pub fn tr_confirm_delete_body(lang: Language, name: &str) -> String {
    match lang {
        Language::Zh => format!("「{name}」会从列表里去掉，相关文件会移到废纸篓。"),
        Language::En => {
            format!("“{name}” will be removed from the list, and its files moved to Trash.")
        }
    }
}
pub fn tr_confirm_delete_action(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "删除",
        Language::En => "Delete",
    }
}
pub fn tr_confirm_clear_title(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "清除已完成的任务？",
        Language::En => "Clear completed tasks?",
    }
}
pub fn tr_confirm_clear_body(lang: Language, n: usize) -> String {
    match lang {
        Language::Zh => format!("从列表移除 {n} 项，文件仍留在下载目录。"),
        Language::En => {
            format!("Remove {n} completed items from the list. Files stay in the download folder.")
        }
    }
}
pub fn tr_confirm_clear_action(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "清除",
        Language::En => "Clear",
    }
}
pub fn tr_confirm_add_title(lang: Language, n: usize) -> String {
    match lang {
        Language::Zh => format!("添加 {n} 个下载任务？"),
        Language::En => format!("Add {n} downloads?"),
    }
}
pub fn tr_confirm_add_body(lang: Language, n: usize) -> String {
    match lang {
        Language::Zh => format!("短时间内收到 {n} 项。确认后才会开始下载。"),
        Language::En => {
            format!("{n} items arrived together. They will start only after you confirm.")
        }
    }
}
pub fn tr_confirm_add_action(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "添加",
        Language::En => "Add",
    }
}
pub fn tr_confirm_add_more(lang: Language, n: usize) -> String {
    match lang {
        Language::Zh => format!("还有 {n} 个"),
        Language::En => {
            if n == 1 {
                "and 1 more".into()
            } else {
                format!("and {n} more")
            }
        }
    }
}
pub fn tr_btn_browse(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "选择目录",
        Language::En => "Browse",
    }
}

pub fn tr_detail_url(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "链接",
        Language::En => "URL",
    }
}
pub fn tr_detail_save(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "保存到",
        Language::En => "Save to",
    }
}
pub fn tr_detail_started(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "开始时间",
        Language::En => "Started",
    }
}
pub fn tr_detail_finished(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "完成时间",
        Language::En => "Finished",
    }
}
pub fn tr_detail_duration(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "耗时",
        Language::En => "Duration",
    }
}
pub fn tr_detail_average_speed(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "平均速度",
        Language::En => "Average speed",
    }
}
pub fn tr_detail_files(lang: Language, n: usize) -> String {
    match lang {
        Language::Zh => format!("{n} 个文件"),
        Language::En => {
            if n == 1 {
                "1 file".into()
            } else {
                format!("{n} files")
            }
        }
    }
}
pub fn tr_detail_waiting(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "正在获取文件列表…",
        Language::En => "Waiting for torrent metadata…",
    }
}
pub fn tr_detail_peers_label(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "节点",
        Language::En => "Peers",
    }
}
pub fn tr_detail_peers(lang: Language, p: BtPeers) -> String {
    if p.is_idle() {
        return match lang {
            Language::Zh => "还没有节点".into(),
            Language::En => "no peers yet".into(),
        };
    }
    match lang {
        Language::Zh => {
            if p.live == 0 {
                format!("正在连接 {} · 已知 {}", p.connecting, p.seen)
            } else if p.live_tcp > 0 && p.live_utp > 0 {
                format!(
                    "连接 {}（TCP {} / uTP {}）· 已知 {}",
                    p.live, p.live_tcp, p.live_utp, p.seen
                )
            } else if p.live_utp > 0 {
                format!("连接 {}（uTP {}）· 已知 {}", p.live, p.live_utp, p.seen)
            } else {
                format!("连接 {} · 已知 {}", p.live, p.seen)
            }
        }
        Language::En => {
            if p.live == 0 {
                format!("connecting {} · seen {}", p.connecting, p.seen)
            } else if p.live_tcp > 0 && p.live_utp > 0 {
                format!(
                    "{} live (TCP {} / uTP {}) · {} seen",
                    p.live, p.live_tcp, p.live_utp, p.seen
                )
            } else if p.live_utp > 0 {
                format!("{} live (uTP {}) · {} seen", p.live, p.live_utp, p.seen)
            } else {
                format!("{} live · {} seen", p.live, p.seen)
            }
        }
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
pub fn tr_status_already_done(lang: Language, name: &str) -> String {
    match lang {
        Language::Zh => format!("已下载过：{name}"),
        Language::En => format!("Already downloaded: {name}"),
    }
}
pub fn tr_status_bad_url(lang: Language) -> String {
    match lang {
        Language::Zh => "无法识别的链接。".into(),
        Language::En => "Unrecognized URL.".into(),
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
pub fn tr_netdisk_resolving(lang: Language, name: &str) -> String {
    match lang {
        Language::Zh => format!("正在解析{name}直链…"),
        Language::En => format!("Resolving {name} direct links…"),
    }
}
pub fn tr_netdisk_added(
    lang: Language,
    name: &str,
    count: usize,
    account: crate::core::providers::AccountInfo,
) -> String {
    use crate::core::providers::Speed;
    let who: Option<String> = if !account.logged_in {
        None
    } else {
        match account.speed {
            Speed::Full => Some(match lang {
                Language::Zh => format!("{}满速", account.label.as_deref().unwrap_or("会员")),
                Language::En => format!("{} full speed", account.label.as_deref().unwrap_or("member")),
            }),
            Speed::Boosted => Some(match lang {
                Language::Zh => account.label.clone().unwrap_or_else(|| "会员".into()),
                Language::En => account.label.unwrap_or_else(|| "member".into()),
            }),
            _ => None,
        }
    };
    let tail = |l: Language| match l {
        Language::Zh => format!("已加入 {count} 个文件"),
        Language::En => format!("queued {count} file(s)"),
    };
    match who {
        Some(w) => match lang {
            Language::Zh => format!("{name}：以{w}身份加速，{}", tail(lang)),
            Language::En => format!("{name}: accelerated as {w}, {}", tail(lang)),
        },
        None if !account.logged_in && account.speed == Speed::Throttled => match lang {
            Language::Zh => format!(
                "{name}：游客身份（限速）。在浏览器登录后可提速，{}",
                tail(lang)
            ),
            Language::En => format!(
                "{name}: guest (throttled). Log in via the browser to speed up; {}",
                tail(lang)
            ),
        },
        None => match lang {
            Language::Zh => format!("{name}：{}", tail(lang)),
            Language::En => format!("{name}: {}", tail(lang)),
        },
    }
}
pub fn tr_netdisk_unsupported(lang: Language, name: &str) -> String {
    match lang {
        Language::Zh => format!("暂不支持该网盘链接（{name}）。"),
        Language::En => format!("Unsupported netdisk link ({name})."),
    }
}
pub fn tr_netdisk_fail(lang: Language, name: &str, msg: &str) -> String {
    match lang {
        Language::Zh => format!("{name}解析失败：{msg}"),
        Language::En => format!("{name} resolve failed: {msg}"),
    }
}
pub fn tr_status_paused(lang: Language, name: &str) -> String {
    match lang {
        Language::Zh => format!("已暂停：{name}"),
        Language::En => format!("Paused: {name}"),
    }
}
pub fn tr_status_paused_all(lang: Language, n: usize) -> String {
    match lang {
        Language::Zh => format!("已暂停 {n} 项"),
        Language::En => format!("Paused {n} items"),
    }
}
pub fn tr_status_resumed_all(lang: Language, n: usize) -> String {
    match lang {
        Language::Zh => format!("已继续 {n} 项"),
        Language::En => format!("Resumed {n} items"),
    }
}
pub fn tr_status_held_captures(lang: Language, n: usize) -> String {
    match lang {
        Language::Zh => format!("已加入 {n} 项，先暂停"),
        Language::En => format!("Added {n} items, paused"),
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
pub fn tr_clipboard_hint_n(lang: Language, n: usize) -> String {
    match lang {
        Language::Zh => format!("剪贴板中有 {n} 条链接，是否下载？"),
        Language::En => format!("Clipboard has {n} URLs. Download them?"),
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
pub fn tr_settings_limit(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "全局限速 (KiB/s，0 不限)",
        Language::En => "Global limit (KiB/s, 0 = unlimited)",
    }
}
pub fn tr_settings_proxy(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "代理（从剪贴板粘贴）",
        Language::En => "Proxy (paste from clipboard)",
    }
}
pub fn tr_netdisk_already(lang: Language, provider: &str, count: usize) -> String {
    match lang {
        Language::Zh => format!("{provider}：{count} 个文件已在任务列表中"),
        Language::En => format!("{provider}: {count} file(s) already in the task list"),
    }
}
pub fn tr_btn_clear(lang: Language) -> &'static str {
    match lang {
        Language::Zh => "清除",
        Language::En => "Clear",
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
        Language::Zh => "支持 HTTP / HTTPS / FTP / HLS / BT。多连接分段下载，支持断点续传。",
        Language::En => {
            "HTTP, HTTPS, FTP, HLS, BitTorrent. Multi-connection segmented download with resume."
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_line_idle_connecting_and_split() {
        assert_eq!(
            tr_detail_peers(Language::Zh, BtPeers::default()),
            "还没有节点"
        );
        assert_eq!(
            tr_detail_peers(
                Language::Zh,
                BtPeers {
                    connecting: 3,
                    seen: 15,
                    ..BtPeers::default()
                }
            ),
            "正在连接 3 · 已知 15"
        );
        assert_eq!(
            tr_detail_peers(
                Language::Zh,
                BtPeers {
                    live: 12,
                    live_tcp: 10,
                    live_utp: 2,
                    seen: 80,
                    ..BtPeers::default()
                }
            ),
            "连接 12（TCP 10 / uTP 2）· 已知 80"
        );
        assert_eq!(
            tr_detail_peers(
                Language::En,
                BtPeers {
                    live: 8,
                    live_tcp: 8,
                    seen: 20,
                    ..BtPeers::default()
                }
            ),
            "8 live · 20 seen"
        );
    }

    #[test]
    fn confirm_copy_mentions_trash_and_keeps_files() {
        assert!(tr_confirm_delete_body(Language::Zh, "a.zip").contains("废纸篓"));
        assert!(tr_confirm_delete_body(Language::En, "a.zip").contains("Trash"));
        assert!(tr_confirm_clear_body(Language::Zh, 3).contains("3"));
        assert!(tr_confirm_clear_body(Language::En, 3).contains("download folder"));
    }

    #[test]
    fn bulk_add_confirm_mentions_count() {
        assert!(tr_confirm_add_title(Language::Zh, 12).contains("12"));
        assert!(tr_confirm_add_body(Language::Zh, 12).contains("12"));
        assert!(tr_confirm_add_body(Language::En, 12).contains("12"));
        assert_eq!(tr_confirm_add_more(Language::Zh, 4), "还有 4 个");
        assert_eq!(
            tr_clipboard_hint_n(Language::Zh, 5),
            "剪贴板中有 5 条链接，是否下载？"
        );
        assert_eq!(tr_btn_pause_all(Language::Zh), "全部暂停");
        assert_eq!(tr_btn_resume_all(Language::En), "Resume all");
        assert_eq!(
            tr_status_held_captures(Language::Zh, 3),
            "已加入 3 项，先暂停"
        );
        assert_eq!(
            tr_status_held_captures(Language::En, 3),
            "Added 3 items, paused"
        );
    }
}
