//! 应用更新对话框组件

use crate::core::updater::UpdateStatus;
use crate::ui::components::buttons::{ghost_button, primary_button};
use crate::ui::i18n::*;
use crate::ui::theme::*;
use crate::ui::Root;
use gpui::{div, prelude::*, px, rgb, Context, IntoElement};

pub fn render_update_dialog(root: &Root, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let status = root.update.status.clone();

    let (title, body, primary_label, primary_enabled) = match &status {
        UpdateStatus::Available {
            latest_version,
            notes,
            release_url,
            ..
        } => {
            let note = if notes.trim().is_empty() {
                release_url.clone()
            } else {
                crate::core::model::truncate(notes.trim(), 280)
            };
            (
                format!("v{latest_version}"),
                note,
                tr_update_download(lang).to_string(),
                true,
            )
        }
        UpdateStatus::Downloading { progress, .. } => {
            let live = root
                .update
                .live_progress
                .as_ref()
                .and_then(|p| p.lock().ok().map(|p| p.percent));
            let percent = live.unwrap_or(progress.percent);
            (
                tr_update_downloading(lang).to_string(),
                format!("{percent:.0}%"),
                tr_update_downloading(lang).to_string(),
                false,
            )
        }
        UpdateStatus::Verifying { .. } => (
            tr_update_verifying(lang).to_string(),
            String::new(),
            tr_update_verifying(lang).to_string(),
            false,
        ),
        UpdateStatus::Downloaded { latest_version, .. } => (
            format!("v{latest_version}"),
            tr_update_ready_hint(lang).to_string(),
            tr_update_restart_install(lang).to_string(),
            true,
        ),
        UpdateStatus::Installing { .. } => (
            tr_update_installing(lang).to_string(),
            String::new(),
            tr_update_installing(lang).to_string(),
            false,
        ),
        UpdateStatus::Error { message, .. } => (
            tr_update_failed(lang).to_string(),
            message.clone(),
            tr_update_retry(lang).to_string(),
            true,
        ),
        _ => (
            tr_update_checking(lang).to_string(),
            String::new(),
            tr_update_later(lang).to_string(),
            false,
        ),
    };

    let show_skip = matches!(status, UpdateStatus::Available { .. });
    let release_url = match &status {
        UpdateStatus::Available { release_url, .. }
        | UpdateStatus::Downloading { release_url, .. } => Some(release_url.clone()),
        _ => None,
    };

    let active_task_count = root
        .tasks
        .iter()
        .filter(|t| t.status.is_active())
        .count();

    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(rgba(0x000000, 0.45))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(460.))
                .p_6()
                .rounded_2xl()
                .bg(rgb(CARD))
                .border_1()
                .border_color(rgba(OUTLINE_VAR, 0.5))
                .shadow_lg()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(TEXT))
                                        .child(tr_update_dialog_title(lang)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(PRIMARY))
                                        .child(title),
                                ),
                        )
                        .child(
                            div()
                                .id("update-close")
                                .p_1()
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|h| h.bg(rgb(SURF_LOW)))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.dismiss_update_dialog(cx);
                                }))
                                .child(crate::ui::components::icons::icon_close(MUTED, 14.)),
                        ),
                )
                // 进度条（下载中呈现）
                .when(
                    matches!(status, UpdateStatus::Downloading { .. }),
                    |d| {
                        let live = root
                            .update
                            .live_progress
                            .as_ref()
                            .and_then(|p| p.lock().ok().map(|p| p.percent))
                            .unwrap_or(0.0);
                        d.child(
                            div()
                                .w_full()
                                .h(px(6.))
                                .rounded_full()
                                .bg(rgb(SURF_HIGH))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(live / 100.0))
                                        .bg(rgb(PRIMARY)),
                                ),
                        )
                    },
                )
                // 活跃任务保护提示
                .when(active_task_count > 0, |d| {
                    d.child(
                        div()
                            .w_full()
                            .p_2p5()
                            .rounded_lg()
                            .bg(rgb(CAUTION_CONTAINER))
                            .text_xs()
                            .text_color(rgb(CAUTION))
                            .child(tr_update_active_tasks_warn(lang, active_task_count)),
                    )
                })
                // 更新日志 / 描述正文
                .when(!body.is_empty(), |d| {
                    d.child(
                        div()
                            .w_full()
                            .max_h(px(140.))
                            .overflow_hidden()
                            .p_3()
                            .rounded_xl()
                            .bg(rgb(SURF_LOW))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(body),
                    )
                })
                // 底部操作区
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .pt_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(show_skip, |d| {
                                    d.child(
                                        ghost_button(tr_update_skip(lang).into(), true)
                                            .id("update-skip")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.skip_this_update_version(cx);
                                            })),
                                    )
                                })
                                .children(release_url.map(|url| {
                                    ghost_button("Release Notes".into(), true)
                                        .id("update-url")
                                        .on_click(cx.listener(move |_, _, _, _| {
                                            crate::platform::open_url(&url);
                                        }))
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    ghost_button(tr_update_later(lang).into(), true)
                                        .id("update-later")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.dismiss_update_dialog(cx);
                                        })),
                                )
                                .child({
                                    let status_for_action = status.clone();
                                    primary_button(
                                        primary_label.into(),
                                        primary_enabled,
                                    )
                                    .id("update-action")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match &status_for_action {
                                            UpdateStatus::Available { .. }
                                            | UpdateStatus::Error { .. } => {
                                                this.start_update_download(cx);
                                            }
                                            UpdateStatus::Downloaded { .. } => {
                                                this.apply_update_now(cx);
                                            }
                                            _ => {}
                                        }
                                    }))
                                }),
                        ),
                ),
        )
}
