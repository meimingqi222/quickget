use crate::core::i18n::Language;
use crate::core::model::{fmt_duration, fmt_eta, fmt_size, fmt_speed, truncate, Task, TaskStatus};
use crate::core::urlx::Protocol;
use crate::ui::components::buttons::small_button;
use crate::ui::components::sidebar::View;
use crate::ui::i18n::*;
use crate::ui::theme::*;
use crate::ui::Root;
use chrono::{Local, TimeZone};
use gpui::{div, prelude::*, px, relative, rgb, Context, IntoElement, SharedString};

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
        .min_w(px(0.))
        .w_full()
        .overflow_y_scroll()
        .overflow_x_hidden()
        .px_3()
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
        for t in &items {
            list = list.child(render_task_row(root, t, cx));
        }
    }

    let visible_ids: Vec<String> = items.iter().map(|t| t.id.clone()).collect();
    let selected_visible_count = visible_ids
        .iter()
        .filter(|id| root.is_selected(id))
        .count();
    let all_selected = !visible_ids.is_empty() && selected_visible_count == visible_ids.len();
    let total_selected = root.selected_tasks.len();

    let toolbar = if total_selected > 0 {
        // 批量操作工具栏
        let ids_for_toggle = visible_ids.clone();
        div()
            .flex_none()
            .px_4()
            .py_2()
            .bg(rgba(PRIMARY_FIXED, 0.35))
            .border_t_1()
            .border_color(rgba(PRIMARY, 0.3))
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        small_button(
                            if all_selected {
                                tr_btn_unselect_all(lang).into()
                            } else {
                                tr_btn_select_all(lang).into()
                            },
                            SURF_LOW,
                            TEXT,
                            true,
                        )
                        .id("batch-toggle-all")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_select_all_visible(&ids_for_toggle, cx);
                        })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(PRIMARY))
                            .px_1()
                            .child(tr_selected_count(lang, total_selected)),
                    ),
            )
            .child(div().flex_1())
            .child(
                small_button(tr_btn_pause_selected(lang).into(), SURF_LOW, TEXT, true)
                    .id("batch-pause")
                    .on_click(cx.listener(|this, _, _, cx| this.pause_selected(cx))),
            )
            .child(
                small_button(tr_btn_resume_selected(lang).into(), PRIMARY_FIXED, PRIMARY, true)
                    .id("batch-resume")
                    .on_click(cx.listener(|this, _, _, cx| this.resume_selected(cx))),
            )
            .child(
                small_button(tr_btn_copy_links(lang).into(), SURF_LOW, TEXT, true)
                    .id("batch-copy-links")
                    .on_click(cx.listener(|this, _, _, cx| this.copy_selected_urls(cx))),
            )
            .child(
                small_button(tr_btn_remove_selected(lang).into(), ERROR_CONTAINER, ERROR, true)
                    .id("batch-remove")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.request_remove_selected(window, cx)
                    })),
            )
            .child(
                small_button(tr_btn_cancel(lang).into(), SURF_LOW, MUTED, true)
                    .id("batch-cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.clear_selection(cx))),
            )
    } else {
        // 普通全局工具栏
        let pausable = root.count_pausable();
        let paused = root.count_paused();
        let ids_for_select_all = visible_ids.clone();
        let can_select_all = !visible_ids.is_empty();

        div()
            .flex_none()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(rgba(OUTLINE_VAR, 0.4))
            .flex()
            .items_center()
            .gap_2()
            .child(
                small_button(
                    tr_btn_select_all(lang).into(),
                    SURF_LOW,
                    TEXT,
                    can_select_all,
                )
                .id("select-all-btn")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_all_visible(&ids_for_select_all, cx);
                })),
            )
            .child(
                small_button(tr_btn_pause_all(lang).into(), SURF_LOW, TEXT, pausable > 0)
                    .id("pause-all")
                    .on_click(cx.listener(|this, _, _, cx| this.pause_all(cx))),
            )
            .child(
                small_button(tr_btn_resume_all(lang).into(), SURF_LOW, TEXT, paused > 0)
                    .id("resume-all")
                    .on_click(cx.listener(|this, _, _, cx| this.resume_all(cx))),
            )
            .child(div().flex_1())
            .child(
                small_button(
                    tr_btn_clear_done(lang).into(),
                    SURF_LOW,
                    MUTED,
                    root.count_done() > 0,
                )
                .id("clear-done")
                .on_click(cx.listener(|this, _, window, cx| this.request_clear_done(window, cx))),
            )
    };

    let inspector_target = root
        .inspector_task
        .as_deref()
        .and_then(|id| root.tasks.iter().find(|t| t.id == id))
        .cloned();

    let mut main_split = div()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .w_full()
        .overflow_hidden()
        .flex()
        .flex_row()
        .child(list);

    if let Some(target) = inspector_target {
        main_split = main_split.child(render_task_inspector(root, &target, cx));
    }

    div()
        .flex_1()
        .min_w(px(0.))
        .w_full()
        .h_full()
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(main_split)
        .child(toolbar)
}

fn status_badge(lang: Language, status: TaskStatus) -> impl IntoElement {
    let (bg_c, text_c, label) = match status {
        TaskStatus::Completed => (OK_CONTAINER, OK, tr_status_label(lang, status)),
        TaskStatus::Downloading => (PRIMARY_FIXED, PRIMARY, tr_status_label(lang, status)),
        TaskStatus::Probing => (PRIMARY_FIXED, PRIMARY, tr_status_label(lang, status)),
        TaskStatus::Paused => (CAUTION_CONTAINER, CAUTION, tr_status_label(lang, status)),
        TaskStatus::Failed | TaskStatus::Cancelled => {
            (ERROR_CONTAINER, ERROR, tr_status_label(lang, status))
        }
        TaskStatus::Queued => (SURF_HIGH, MUTED, tr_status_label(lang, status)),
    };
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.5))
        .rounded_full()
        .bg(rgb(bg_c))
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(rgb(text_c))
        .child(label.to_string())
}

fn protocol_badge(proto: Protocol) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(5.))
        .py(px(1.5))
        .rounded(px(4.))
        .bg(rgb(SURF_HIGH))
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(rgb(MUTED))
        .child(proto.label().to_string())
}

fn render_task_row(root: &Root, t: &Task, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let speed = root
        .runtime
        .get(&t.id)
        .map(|s| s.progress.speed())
        .unwrap_or(0);
    let frac = t.fraction();
    let is_selected = root.is_selected(&t.id);
    let is_inspecting = root.inspector_task.as_deref() == Some(t.id.as_str());

    let id = t.id.clone();
    let id_select = t.id.clone();
    let id_inspect = t.id.clone();
    let id_pause = t.id.clone();
    let id_retry = t.id.clone();
    let id_remove = t.id.clone();
    let id_open = t.id.clone();
    let id_reveal = t.id.clone();

    let size_text = if t.size > 0 {
        fmt_size(t.size)
    } else if t.downloaded > 0 {
        fmt_size(t.downloaded)
    } else {
        "--".into()
    };

    let meta_element = match t.status {
        TaskStatus::Completed => {
            let mut parts = vec![size_text];
            if let Some(elapsed) = t.elapsed_secs() {
                parts.push(format!(
                    "{} {}",
                    tr_detail_duration(lang),
                    fmt_duration(elapsed)
                ));
            }
            if let Some(avg) = t.average_speed() {
                parts.push(format!(
                    "{} {}",
                    tr_detail_average_speed(lang),
                    fmt_speed(avg)
                ));
            }
            if let Some(finished) = t.finished_at {
                parts.push(fmt_timestamp(finished));
            }
            div().text_xs().text_color(rgb(MUTED)).child(parts.join(" · "))
        }
        TaskStatus::Downloading | TaskStatus::Probing => {
            let prog = if t.size > 0 {
                format!(
                    "{} / {} ({:.0}%)",
                    fmt_size(t.downloaded),
                    fmt_size(t.size),
                    frac * 100.0
                )
            } else {
                fmt_size(t.downloaded)
            };
            let speed_str = if speed > 0 {
                fmt_speed(speed)
            } else if t.status == TaskStatus::Probing {
                if lang == Language::Zh {
                    "探测中…".into()
                } else {
                    "Probing…".into()
                }
            } else {
                "0 B/s".into()
            };
            let eta = if t.size > t.downloaded && speed > 0 {
                format!(
                    " · {} {}",
                    tr_detail_duration(lang),
                    fmt_eta(t.size - t.downloaded, speed)
                )
            } else {
                String::new()
            };
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(format!("{prog} · {speed_str}{eta}"))
        }
        TaskStatus::Paused => {
            let prog = if t.size > 0 {
                format!(
                    "{} / {} ({:.0}%)",
                    fmt_size(t.downloaded),
                    fmt_size(t.size),
                    frac * 100.0
                )
            } else {
                fmt_size(t.downloaded)
            };
            div().text_xs().text_color(rgb(MUTED)).child(prog)
        }
        TaskStatus::Failed | TaskStatus::Cancelled => {
            let err_text = t.error.as_deref().unwrap_or("下载失败");
            div()
                .text_xs()
                .text_color(rgb(ERROR))
                .child(format!("{} · {}", size_text, err_text))
        }
        TaskStatus::Queued => {
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(format!("{} · 排队中", size_text))
        }
    };

    let mut actions = div().flex_none().flex().items_center().gap_1();
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
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.reveal_task(&id_reveal, cx)),
                        ),
                );
        }
    }
    actions = actions.child(
        small_button(tr_btn_remove(lang).into(), SURF_LOW, ERROR, true)
            .id(SharedString::from(format!("rm-{id_remove}")))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.request_remove_task(&id_remove, window, cx)
            })),
    );

    let progress_color = match t.status {
        TaskStatus::Downloading | TaskStatus::Probing => PRIMARY,
        TaskStatus::Paused => CAUTION,
        TaskStatus::Failed | TaskStatus::Cancelled => ERROR,
        _ => MUTED,
    };

    let show_progress_bar = matches!(
        t.status,
        TaskStatus::Downloading | TaskStatus::Probing | TaskStatus::Paused | TaskStatus::Failed
    );

    let card_bg = if is_inspecting {
        rgba(SURF_LOW, 1.0)
    } else if is_selected {
        rgba(PRIMARY_FIXED, 0.4)
    } else {
        rgba(CARD, 1.0)
    };
    let border_c = if is_inspecting {
        rgba(PRIMARY, 1.0)
    } else if is_selected {
        rgba(PRIMARY, 0.7)
    } else {
        rgba(OUTLINE_VAR, 0.55)
    };

    div()
        .id(SharedString::from(format!("task-{id}")))
        .w_full()
        .min_w(px(0.))
        .overflow_hidden()
        .px_3()
        .py_2()
        .rounded_lg()
        .bg(card_bg)
        .border_1()
        .border_color(border_c)
        .flex()
        .flex_col()
        .gap(px(6.))
        .cursor_pointer()
        .hover(|h| {
            if !is_inspecting && !is_selected {
                h.border_color(rgba(PRIMARY, 0.45)).bg(rgba(SURF_LOW, 1.0))
            } else {
                h
            }
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_inspector(&id_inspect, cx);
        }))
        .child(
            div()
                .w_full()
                .min_w(px(0.))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .id(SharedString::from(format!("chk-{id_select}")))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_select_task(&id_select, cx);
                        }))
                        .child(crate::ui::components::icons::checkbox(is_selected)),
                )
                .child(protocol_badge(t.protocol))
                .child(status_badge(lang, t.status))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT))
                        .overflow_hidden()
                        .child(truncate(&t.filename, 28)),
                )
                .when(t.status.is_active(), |d| {
                    let sp = if speed > 0 {
                        fmt_speed(speed)
                    } else if t.status == TaskStatus::Probing {
                        if lang == Language::Zh {
                            "探测中…".into()
                        } else {
                            "Probing…".into()
                        }
                    } else {
                        "0 B/s".into()
                    };
                    d.child(
                        div()
                            .flex_none()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(PRIMARY))
                            .child(sp),
                    )
                })
                .child(actions),
        )
        .child(
            div()
                .w_full()
                .min_w(px(0.))
                .overflow_hidden()
                .pl(px(24.))
                .child(meta_element),
        )
        .when(show_progress_bar, |d| {
            d.child(
                div()
                    .pl(px(24.))
                    .child(
                        div()
                            .w_full()
                            .h(px(4.))
                            .rounded_full()
                            .bg(rgb(SURF_HIGHEST))
                            .overflow_hidden()
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(frac))
                                    .rounded_full()
                                    .bg(rgb(progress_color)),
                            ),
                    ),
            )
        })
}

fn render_task_inspector(root: &Root, t: &Task, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let id_open = t.id.clone();
    let id_reveal = t.id.clone();
    let path_copy = t.dest_path().display().to_string();

    let speed = root
        .runtime
        .get(&t.id)
        .map(|s| s.progress.speed())
        .unwrap_or(0);

    let waiting_bt = t.protocol == Protocol::Magnet && t.files.is_empty() && t.status.is_open();
    let files = if waiting_bt {
        Vec::new()
    } else {
        t.display_files()
    };
    let is_multi_file = files.len() > 1 || t.protocol == Protocol::Magnet;

    let handle = root.inspector_scroll.clone();

    let mut body = div()
        .id(SharedString::from(format!("inspector-body-{}", t.id)))
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .overflow_y_scroll()
        .track_scroll(&handle)
        .px_3()
        .py_3()
        .pb(px(40.))
        .flex()
        .flex_col()
        .gap_3();

    let header = div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .pb_2()
        .border_b_1()
        .border_color(rgba(OUTLINE_VAR, 0.5))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(TEXT))
                .child(tr_inspector_title(lang)),
        )
        .child(
            div()
                .id("inspector-close")
                .p_1()
                .rounded_md()
                .cursor_pointer()
                .hover(|h| h.bg(rgb(SURF_LOW)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_inspector(cx);
                }))
                .child(crate::ui::components::icons::icon_close(MUTED, 12.)),
        );

    let summary_card = div()
        .w_full()
        .min_w(px(0.))
        .p_3()
        .rounded_lg()
        .bg(rgb(SURF_LOW))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .w_full()
                .min_w(px(0.))
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .overflow_hidden()
                .child(t.filename.clone()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(protocol_badge(t.protocol))
                .child(status_badge(lang, t.status))
                .when(t.status.is_active(), |d| {
                    let cur_speed = if speed > 0 {
                        fmt_speed(speed)
                    } else {
                        "0 B/s".into()
                    };
                    d.child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(PRIMARY))
                            .child(cur_speed),
                    )
                })
                .when(t.status == TaskStatus::Completed, |d| {
                    if let Some(avg) = t.average_speed() {
                        d.child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(OK))
                                .child(format!(
                                    "{} {}",
                                    tr_detail_average_speed(lang),
                                    fmt_speed(avg)
                                )),
                        )
                    } else {
                        d
                    }
                }),
        );
    body = body.child(summary_card);

    let mut quick_actions = div().flex().items_center().gap_2();
    if t.status == TaskStatus::Completed {
        quick_actions = quick_actions
            .child(
                small_button(tr_btn_open(lang).into(), PRIMARY, ON_PRIMARY, true)
                    .id("inspector-open")
                    .on_click(cx.listener(move |this, _, _, cx| this.open_task(&id_open, cx))),
            )
            .child(
                small_button(tr_btn_reveal(lang).into(), SURF_LOW, TEXT, true)
                    .id("inspector-reveal")
                    .on_click(cx.listener(move |this, _, _, cx| this.reveal_task(&id_reveal, cx))),
            );
    } else {
        quick_actions = quick_actions.child(
            small_button(tr_btn_reveal(lang).into(), SURF_LOW, TEXT, true)
                .id("inspector-reveal")
                .on_click(cx.listener(move |this, _, _, cx| this.reveal_task(&id_reveal, cx))),
        );
    }
    body = body.child(quick_actions);

    let mut details_box = div()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(rgba(OUTLINE_VAR, 0.4))
        .bg(rgb(CARD))
        .w_full()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(px(6.));

    let size_display = if t.size > 0 {
        format!("{} / {}", fmt_size(t.downloaded), fmt_size(t.size))
    } else {
        fmt_size(t.downloaded)
    };
    details_box = details_box.child(inspector_item(tr_detail_size(lang), size_display));

    if t.status.is_active() {
        let cur_speed = if speed > 0 {
            fmt_speed(speed)
        } else {
            "0 B/s".into()
        };
        details_box = details_box.child(inspector_item(tr_detail_speed(lang), cur_speed));
    }

    if t.connections > 0 {
        details_box = details_box.child(inspector_item(
            tr_detail_threads(lang),
            tr_detail_threads_val(lang, t.connections),
        ));
    }

    if t.created_at > 0 {
        details_box = details_box.child(inspector_item(
            tr_detail_created(lang),
            fmt_timestamp(t.created_at),
        ));
    }
    if let Some(started) = t.started_at {
        details_box = details_box.child(inspector_item(
            tr_detail_started(lang),
            fmt_timestamp(started),
        ));
    }
    if let Some(finished) = t.finished_at {
        details_box = details_box.child(inspector_item(
            tr_detail_finished(lang),
            fmt_timestamp(finished),
        ));
    }
    if let Some(elapsed) = t.elapsed_secs() {
        details_box = details_box.child(inspector_item(
            tr_detail_duration(lang),
            fmt_duration(elapsed),
        ));
    }
    if let Some(avg) = t.average_speed() {
        details_box = details_box.child(inspector_item(
            tr_detail_average_speed(lang),
            fmt_speed(avg),
        ));
    }

    if t.protocol == Protocol::Magnet
        && (t.status.is_active() || t.status == TaskStatus::Paused || !t.peers.is_idle())
    {
        details_box = details_box.child(inspector_item(
            tr_detail_peers_label(lang),
            tr_detail_peers(lang, t.peers),
        ));
    }

    body = body.child(details_box);

    if let Some(ref err) = t.error {
        let err_to_copy = err.clone();
        body = body.child(
            div()
                .w_full()
                .min_w(px(0.))
                .p_2()
                .rounded_md()
                .bg(rgb(ERROR_CONTAINER))
                .border_1()
                .border_color(rgba(ERROR, 0.4))
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(ERROR))
                                .child(tr_detail_error(lang)),
                        )
                        .child(
                            div()
                                .id("copy-btn-error")
                                .flex_none()
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(ERROR))
                                .cursor_pointer()
                                .hover(|h| h.opacity(0.8))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let notice = tr_copied_to_clipboard(this.language);
                                    this.copy_text(err_to_copy.clone(), notice, cx);
                                }))
                                .child(tr_btn_copy_error(lang)),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .text_xs()
                        .text_color(rgb(ERROR))
                        .overflow_hidden()
                        .child(err.clone()),
                ),
        );
    }

    // 块级展示长文本：URL、保存路径、Referer、User-Agent
    body = body.child(inspector_block(
        tr_detail_url(lang),
        t.url.clone(),
        Some((tr_btn_copy_link(lang), t.url.clone())),
        cx,
    ));

    body = body.child(inspector_block(
        tr_detail_save(lang),
        path_copy.clone(),
        Some((tr_btn_copy_path(lang), path_copy)),
        cx,
    ));

    if let Some(ref ref_url) = t.referer {
        body = body.child(inspector_block(
            tr_detail_referer(lang),
            ref_url.clone(),
            Some((tr_btn_copy_link(lang), ref_url.clone())),
            cx,
        ));
    }

    if let Some(ref ua) = t.user_agent {
        body = body.child(inspector_block(
            tr_detail_user_agent(lang),
            ua.clone(),
            Some((tr_btn_copy_link(lang), ua.clone())),
            cx,
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
    } else if is_multi_file {
        body = body.child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(MUTED))
                .pt_1()
                .child(tr_detail_files(lang, files.len())),
        );

        let mut file_list = div()
            .id(SharedString::from(format!("inspector-files-{}", t.id)))
            .w_full()
            .max_h(px(160.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.));

        for (i, f) in files.into_iter().enumerate() {
            let task_id = t.id.clone();
            let file_path = f.path.clone();
            let size_txt = if f.size > 0 {
                fmt_size(f.size)
            } else {
                String::new()
            };
            file_list = file_list.child(
                div()
                    .id(SharedString::from(format!("inspector-file-{}-{i}", t.id)))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py(px(3.))
                    .rounded_md()
                    .bg(rgb(SURF_LOW))
                    .cursor_pointer()
                    .hover(|h| h.bg(rgb(SURF)))
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
                    .child(div().text_xs().text_color(rgb(MUTED)).child(size_txt)),
            );
        }
        body = body.child(file_list);
    }

    div()
        .id("task-inspector")
        .flex_none()
        .w(px(320.))
        .min_w(px(320.))
        .max_w(px(320.))
        .h_full()
        .bg(rgb(CARD))
        .border_l_1()
        .border_color(rgba(OUTLINE_VAR, 0.6))
        .flex()
        .flex_col()
        .p_3()
        .overflow_hidden()
        .child(header)
        .child(body)
}

fn inspector_item(label: &str, value: String) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .w_full()
        .min_w(px(0.))
        .child(
            div()
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
                .text_right()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(TEXT))
                .overflow_hidden()
                .child(value),
        )
}

fn inspector_block(
    label: &str,
    content: String,
    copy_btn: Option<(&str, String)>,
    cx: &mut Context<Root>,
) -> impl IntoElement {
    let mut header = div().w_full().min_w(px(0.)).flex().items_center().justify_between();
    header = header.child(
        div()
            .flex_none()
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(MUTED))
            .child(label.to_string()),
    );
    if let Some((btn_label, to_copy)) = copy_btn {
        let id_btn = format!("copy-btn-{}", label);
        header = header.child(
            div()
                .id(SharedString::from(id_btn))
                .flex_none()
                .text_xs()
                .text_color(rgb(PRIMARY))
                .cursor_pointer()
                .hover(|h| h.opacity(0.8))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let notice = tr_copied_to_clipboard(this.language);
                    this.copy_text(to_copy.clone(), notice, cx);
                }))
                .child(btn_label.to_string()),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .w_full()
        .min_w(px(0.))
        .child(header)
        .child(
            div()
                .w_full()
                .min_w(px(0.))
                .p_2()
                .rounded_md()
                .bg(rgb(SURF_LOW))
                .text_xs()
                .text_color(rgb(TEXT))
                .overflow_hidden()
                .child(content),
        )
}

fn fmt_timestamp(secs: i64) -> String {
    Local
        .timestamp_opt(secs, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "--".into())
}
