use crate::core::model::{fmt_eta, fmt_size, fmt_speed, truncate, Task, TaskStatus};
use crate::core::urlx::Protocol;
use crate::ui::components::buttons::small_button;
use crate::ui::components::scroll::{
    drag_capture, drag_to_offset, scroll_metrics, scrollbar, SCROLLBAR_W,
};
use crate::ui::components::sidebar::View;
use crate::ui::i18n::*;
use crate::ui::theme::*;
use crate::ui::Root;
use gpui::{div, prelude::*, px, rgb, relative, Context, IntoElement, SharedString};

/// 展开详情里文件列表的最大高度。超出这条才出滚动条。
const DETAIL_LIST_MAX_H: f32 = 240.0;
const FILE_ROW_H: f32 = 26.0;

pub fn render_tasks_view(root: &Root, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let items: Vec<Task> = root
        .tasks
        .iter()
        .filter(|t| match root.view {
            View::All => true,
            View::Active => t.status.is_open(),
            View::Done => t.status == TaskStatus::Completed,
            View::Failed => matches!(t.status, TaskStatus::Failed | TaskStatus::Cancelled),
            View::Settings => false,
        })
        .cloned()
        .collect();

    let empty = match root.view {
        View::All => tr_empty_all(lang),
        View::Active => tr_empty_active(lang),
        View::Done => tr_empty_done(lang),
        View::Failed => tr_empty_failed(lang),
        View::Settings => "",
    };

    let mut list = div()
        .id("task-list")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .px_5()
        .py_3()
        .flex()
        .flex_col()
        .gap_2();

    if items.is_empty() {
        list = list.child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(empty.to_string()),
        );
    } else {
        for t in items {
            list = list.child(render_task_row(root, &t, cx));
        }
    }

    let toolbar = div()
        .flex_none()
        .px_5()
        .py_2()
        .flex()
        .items_center()
        .justify_end()
        .child(
            small_button(
                tr_btn_clear_done(lang).into(),
                SURF_LOW,
                MUTED,
                root.count_done() > 0,
            )
            .id("clear-done")
            .on_click(cx.listener(|this, _, _, cx| this.clear_done(cx))),
        );

    div()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .flex()
        .flex_col()
        .child(list)
        .child(toolbar)
}

fn render_task_row(root: &Root, t: &Task, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let speed = root
        .runtime
        .get(&t.id)
        .map(|s| s.progress.speed())
        .unwrap_or(0);
    let frac = t.fraction();
    let status_color = match t.status {
        TaskStatus::Completed => OK,
        TaskStatus::Failed | TaskStatus::Cancelled => ERROR,
        TaskStatus::Paused => CAUTION,
        TaskStatus::Downloading | TaskStatus::Probing => PRIMARY,
        TaskStatus::Queued => MUTED,
    };

    let size_line = if t.protocol == Protocol::Hls && t.status.is_active() && t.size > 0 && t.size < 10_000
    {
        format!("{}/{}", t.downloaded, t.size)
    } else if t.size > 0 {
        format!("{} / {}", fmt_size(t.downloaded), fmt_size(t.size))
    } else if t.downloaded > 0 {
        fmt_size(t.downloaded)
    } else {
        "…".into()
    };

    let eta = if t.status.is_active() && t.size > t.downloaded {
        format!("  {}", fmt_eta(t.size - t.downloaded, speed))
    } else {
        String::new()
    };

    let meta = format!(
        "{} · {} · {}{}",
        t.protocol.label(),
        tr_status_label(lang, t.status),
        size_line,
        eta
    );

    let expanded = root.expanded_task.as_deref() == Some(t.id.as_str());
    let id = t.id.clone();
    let id_toggle = t.id.clone();
    let id_pause = t.id.clone();
    let id_retry = t.id.clone();
    let id_remove = t.id.clone();
    let id_open = t.id.clone();
    let id_reveal = t.id.clone();

    let mut actions = div().flex().items_center().gap_1();
    match t.status {
        TaskStatus::Downloading | TaskStatus::Probing | TaskStatus::Queued => {
            actions = actions.child(
                small_button(tr_btn_pause(lang).into(), SURF_LOW, TEXT, true)
                    .id(SharedString::from(format!("pause-{id_pause}")))
                    .on_click(cx.listener(move |this, _, _, cx| this.pause_task(&id_pause, cx))),
            );
        }
        TaskStatus::Paused | TaskStatus::Failed | TaskStatus::Cancelled => {
            actions = actions.child(
                small_button(tr_btn_retry(lang).into(), PRIMARY_FIXED, PRIMARY, true)
                    .id(SharedString::from(format!("retry-{id_retry}")))
                    .on_click(cx.listener(move |this, _, _, cx| this.resume_task(&id_retry, cx))),
            );
        }
        TaskStatus::Completed => {
            actions = actions
                .child(
                    small_button(tr_btn_open(lang).into(), SURF_LOW, TEXT, true)
                        .id(SharedString::from(format!("open-{id_open}")))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_task(&id_open, cx))),
                )
                .child(
                    small_button(tr_btn_reveal(lang).into(), SURF_LOW, TEXT, true)
                        .id(SharedString::from(format!("reveal-{id_reveal}")))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.reveal_task(&id_reveal, cx)
                        })),
                );
        }
    }
    actions = actions.child(
        small_button(tr_btn_remove(lang).into(), SURF_LOW, ERROR, true)
            .id(SharedString::from(format!("rm-{id_remove}")))
            .on_click(cx.listener(move |this, _, _, cx| this.remove_task(&id_remove, cx))),
    );

    div()
        .id(SharedString::from(format!("task-{id}")))
        .w_full()
        .px_4()
        .py_3()
        .rounded_xl()
        .bg(rgb(CARD))
        .border_1()
        .border_color(if expanded {
            rgba(PRIMARY, 0.45)
        } else {
            rgba(OUTLINE_VAR, 0.55)
        })
        .flex()
        .flex_col()
        .gap_2()
        .hover(|h| h.border_color(rgba(PRIMARY, 0.35)))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .id(SharedString::from(format!("task-toggle-{id_toggle}")))
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_task(&id_toggle, cx)))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(TEXT))
                                .overflow_hidden()
                                .child(t.filename.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(meta),
                        ),
                )
                .when(t.status.is_active() && speed > 0, |d| {
                    d.child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(PRIMARY))
                            .child(fmt_speed(speed)),
                    )
                })
                .child(actions),
        )
        .child(
            div()
                .w_full()
                .h(px(6.))
                .rounded_full()
                .bg(rgb(SURF_HIGHEST))
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(relative(frac))
                        .rounded_full()
                        .bg(rgb(status_color)),
                ),
        )
        .when(
            t.status == TaskStatus::Failed && t.error.as_ref().is_some(),
            |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(rgb(ERROR))
                        .child(t.error.clone().unwrap_or_default()),
                )
            },
        )
        .when(expanded, |d| d.child(render_task_detail(root, t, cx)))
}

fn render_task_detail(root: &Root, t: &Task, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let waiting_bt = t.protocol == Protocol::Magnet
        && t.files.is_empty()
        && t.status.is_open();
    let files = if waiting_bt {
        Vec::new()
    } else {
        t.display_files()
    };
    let total_files = files.len();

    let mut body = div()
        .w_full()
        .mt_1()
        .pt_2()
        .border_t_1()
        .border_color(rgba(OUTLINE_VAR, 0.5))
        .flex()
        .flex_col()
        .gap_1();

    body = body
        .child(detail_row(tr_detail_url(lang), truncate(&t.url, 72)))
        .child(detail_row(
            tr_detail_save(lang),
            t.dest_path().display().to_string(),
        ));

    if t.protocol == Protocol::Magnet
        && (t.status.is_active() || t.status == TaskStatus::Paused || !t.peers.is_idle())
    {
        body = body.child(detail_row(
            tr_detail_peers_label(lang),
            tr_detail_peers(lang, t.peers),
        ));
    }

    if waiting_bt {
        body = body.child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .pt_1()
                .child(tr_detail_waiting(lang).to_string()),
        );
        return body;
    }

    body = body.child(
        div()
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(MUTED))
            .pt_1()
            .child(tr_detail_files(lang, total_files)),
    );

    let handle = root.detail_scroll.clone();
    let est_content = total_files as f32 * FILE_ROW_H;
    let metrics = scroll_metrics(&handle, DETAIL_LIST_MAX_H, est_content);

    let mut list = div()
        .id(SharedString::from(format!("task-files-{}", t.id)))
        .w_full()
        .max_h(px(DETAIL_LIST_MAX_H))
        .overflow_y_scroll()
        .track_scroll(&handle)
        .flex()
        .flex_col()
        .when(metrics.is_some(), |d| d.pr(px(SCROLLBAR_W)));

    for (i, f) in files.into_iter().enumerate() {
        let task_id = t.id.clone();
        let file_path = f.path.clone();
        let size_txt = if f.size > 0 && f.downloaded > 0 && f.downloaded < f.size {
            format!("{} / {}", fmt_size(f.downloaded), fmt_size(f.size))
        } else if f.size > 0 {
            fmt_size(f.size)
        } else {
            String::new()
        };
        list = list.child(
            div()
                .id(SharedString::from(format!("file-{}-{i}", t.id)))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py(px(3.))
                .rounded_md()
                .cursor_pointer()
                .hover(|h| h.bg(rgb(SURF_LOW)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.reveal_task_file(&task_id, &file_path, cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_xs()
                        .text_color(rgb(TEXT))
                        .overflow_hidden()
                        .child(f.path),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(size_txt),
                ),
        );
    }

    let scrollbar_el = metrics.map(|m| {
        scrollbar(
            SharedString::from(format!("task-files-thumb-{}", t.id)),
            m,
            |thumb| {
                thumb.on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                        let mouse_y: f32 = event.position.y.into();
                        let start_top: f32 = (-this.detail_scroll.offset().y).into();
                        this.detail_scroll_drag = Some((mouse_y, start_top.max(0.0)));
                        cx.notify();
                    }),
                )
            },
        )
    });

    body.child(
        div()
            .relative()
            .w_full()
            .max_h(px(DETAIL_LIST_MAX_H))
            .child(list)
            .children(scrollbar_el)
            .child(drag_capture(
                cx.entity(),
                |this, mouse_y, cx| {
                    let Some(start) = this.detail_scroll_drag else {
                        return;
                    };
                    if let Some(new_top) = drag_to_offset(&this.detail_scroll, start, mouse_y) {
                        this.detail_scroll
                            .set_offset(gpui::point(px(0.0), px(-new_top)));
                        cx.notify();
                    }
                },
                |this, cx| {
                    if this.detail_scroll_drag.take().is_some() {
                        cx.notify();
                    }
                },
            )),
    )
}

fn detail_row(label: &str, value: String) -> impl IntoElement {
    div()
        .flex()
        .gap_2()
        .child(
            div()
                .w(px(52.))
                .flex_none()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_xs()
                .text_color(rgb(TEXT))
                .child(value),
        )
}
