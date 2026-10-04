use crate::ui::components::buttons::{ghost_button, small_button};
use crate::ui::components::scroll::{
    drag_capture, drag_to_offset, scroll_metrics, scrollbar, SCROLLBAR_W,
};
use crate::ui::i18n::*;
use crate::ui::theme::*;
use crate::ui::Root;
use gpui::{div, prelude::*, px, rgb, Context, Div, IntoElement};

pub fn render_settings_view(root: &Root, cx: &mut Context<Root>) -> impl IntoElement {
    const EST_VIEWPORT_H: f32 = 600.0;
    const EST_CONTENT_H: f32 = 800.0;
    let lang = root.language;
    let folder = root.settings.save_dir.display().to_string();
    let conn = root.settings.connections_clamped();
    let conc = root.settings.max_concurrent_clamped();
    let limit_kib = root.settings.download_limit_kib();
    let proxy = root.settings.proxy().unwrap_or_else(|| tr_off(lang));
    let handle = root.settings_scroll.clone();
    let metrics = scroll_metrics(&handle, EST_VIEWPORT_H, EST_CONTENT_H);

    let content = div()
        .id("settings-scroll")
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .overflow_y_scroll()
        .track_scroll(&handle)
        .when(metrics.is_some(), |d| d.pr(px(SCROLLBAR_W)))
        .px_8()
        .py_6()
        .flex()
        .flex_col()
        .gap_5()
        .child(
            div()
                .text_xl()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(TEXT))
                .child(tr_view_settings(lang)),
        )
        // 1. 下载与存储
        .child(section_card(
            tr_settings_sec_download(lang),
            vec![
                setting_row(
                    tr_settings_folder(lang),
                    Some(folder),
                    ghost_button(tr_btn_browse(lang).into(), true)
                        .id("browse-dir")
                        .on_click(cx.listener(|this, _, _, cx| this.pick_save_dir(cx))),
                    false,
                ),
                setting_row(
                    tr_settings_conc(lang),
                    None,
                    stepper(
                        conc,
                        1,
                        8,
                        |this, v, cx| {
                            this.settings.max_concurrent = v;
                            this.settings.save();
                            cx.notify();
                        },
                        cx,
                    ),
                    false,
                ),
                setting_row(
                    tr_settings_conn(lang),
                    None,
                    stepper(
                        conn,
                        1,
                        64,
                        |this, v, cx| {
                            this.settings.connections = v;
                            this.settings.save();
                            cx.notify();
                        },
                        cx,
                    ),
                    true,
                ),
            ],
        ))
        // 2. 网络与传输
        .child(section_card(
            tr_settings_sec_network(lang),
            vec![
                setting_row(
                    tr_settings_limit(lang),
                    Some(tr_settings_limit_sub(lang).to_string()),
                    stepper(
                        limit_kib,
                        0,
                        100_000,
                        |this, v, cx| {
                            this.settings.download_limit_bps = u64::from(v) * 1024;
                            this.limiter.set_limit(this.settings.download_limit_bps);
                            this.settings.save();
                            cx.notify();
                        },
                        cx,
                    ),
                    false,
                ),
                setting_row(
                    tr_settings_proxy(lang),
                    Some(proxy.to_string()),
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            ghost_button(tr_btn_paste(lang).into(), true)
                                .id("proxy-paste")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.set_proxy_from_clipboard(cx)),
                                ),
                        )
                        .child(
                            ghost_button(
                                tr_btn_clear(lang).into(),
                                root.settings.proxy().is_some(),
                            )
                            .id("proxy-clear")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_proxy(cx))),
                        ),
                    true,
                ),
            ],
        ))
        // 3. 常规与偏好
        .child(section_card(
            tr_settings_sec_general(lang),
            vec![
                {
                    let on = root.settings.watch_clipboard;
                    setting_row(
                        tr_settings_clip(lang),
                        Some(tr_settings_clip_sub(lang).to_string()),
                        small_button(
                            if on {
                                tr_on(lang).into()
                            } else {
                                tr_off(lang).into()
                            },
                            if on { PRIMARY_FIXED } else { SURF },
                            if on { PRIMARY } else { MUTED },
                            true,
                        )
                        .id("clip-toggle")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings.watch_clipboard = !this.settings.watch_clipboard;
                            this.settings.save();
                            cx.notify();
                        })),
                        false,
                    )
                },
                {
                    let auto = root.settings.auto_check_updates;
                    setting_row(
                        tr_update_auto_check(lang),
                        Some(tr_update_auto_check_sub(lang).to_string()),
                        small_button(
                            if auto {
                                tr_on(lang).into()
                            } else {
                                tr_off(lang).into()
                            },
                            if auto { PRIMARY_FIXED } else { SURF },
                            if auto { PRIMARY } else { MUTED },
                            true,
                        )
                        .id("auto-update-toggle")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings.auto_check_updates = !this.settings.auto_check_updates;
                            this.settings.save();
                            cx.notify();
                        })),
                        false,
                    )
                },
                setting_row(
                    tr_settings_lang(lang),
                    None,
                    small_button(
                        root.language.short_name().into(),
                        PRIMARY_FIXED,
                        PRIMARY,
                        true,
                    )
                    .id("lang-settings")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_language(cx))),
                    true,
                ),
            ],
        ))
        // 4. 关于与说明
        .child(
            div()
                .w_full()
                .max_w(px(720.))
                .p_3p5()
                .rounded_xl()
                .bg(rgb(SURF_LOW))
                .border_1()
                .border_color(rgba(OUTLINE_VAR, 0.45))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(TEXT))
                                        .child(tr_settings_sec_about(lang)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(MUTED))
                                        .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                                ),
                        )
                        .child(
                            ghost_button(
                                if root.update.checking {
                                    tr_update_checking(lang).into()
                                } else {
                                    tr_update_check(lang).into()
                                },
                                !root.update.checking,
                            )
                            .id("check-update-btn")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.check_for_updates_manual(cx);
                            })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(tr_settings_blurb(lang)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(tr_settings_ext(lang)),
                ),
        );

    let scrollbar_el = metrics.map(|m| {
        scrollbar("settings-scroll-thumb", m, |thumb| {
            thumb.on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    let mouse_y: f32 = event.position.y.into();
                    let top: f32 = (-this.settings_scroll.offset().y).into();
                    this.settings_scroll_drag = Some((mouse_y, top.max(0.0)));
                    cx.notify();
                }),
            )
        })
    });

    div()
        .relative()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .child(content)
        .children(scrollbar_el)
        .child(drag_capture(
            cx.entity(),
            |this, mouse_y, cx| {
                let Some(start) = this.settings_scroll_drag else { return };
                if let Some(top) = drag_to_offset(&this.settings_scroll, start, mouse_y) {
                    this.settings_scroll.set_offset(gpui::point(px(0.0), px(-top)));
                    cx.notify();
                }
            },
            |this, cx| {
                if this.settings_scroll_drag.take().is_some() {
                    cx.notify();
                }
            },
        ))
}

fn section_card(title: &str, rows: Vec<Div>) -> impl IntoElement {
    div()
        .w_full()
        .max_w(px(720.))
        .flex()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(MUTED))
                .px_1()
                .child(title.to_string()),
        )
        .child(
            div()
                .w_full()
                .rounded_xl()
                .bg(rgb(CARD))
                .border_1()
                .border_color(rgba(OUTLINE_VAR, 0.5))
                .overflow_hidden()
                .children(rows),
        )
}

fn setting_row(
    label: &str,
    subtitle: Option<String>,
    right: impl IntoElement,
    is_last: bool,
) -> Div {
    div()
        .w_full()
        .px_4()
        .py_2p5()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .when(!is_last, |d| {
            d.border_b_1().border_color(rgba(OUTLINE_VAR, 0.35))
        })
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(TEXT))
                        .child(label.to_string()),
                )
                .children(subtitle.map(|sub| {
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .overflow_hidden()
                        .child(sub)
                })),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .child(right.into_any_element()),
        )
}

fn stepper(
    value: u32,
    min: u32,
    max: u32,
    on_change: impl Fn(&mut Root, u32, &mut Context<Root>) + Clone + 'static,
    cx: &mut Context<Root>,
) -> impl IntoElement {
    let down = on_change.clone();
    let up = on_change;
    let v_down = value.saturating_sub(1).max(min);
    let v_up = (value + 1).min(max);
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            small_button("−".into(), SURF, TEXT, value > min)
                .id(gpui::SharedString::from(format!("step-down-{value}-{min}")))
                .on_click(cx.listener(move |this, _, _, cx| down(this, v_down, cx))),
        )
        .child(
            div()
                .min_w(px(36.))
                .px_1()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .flex()
                .justify_center()
                .child(value.to_string()),
        )
        .child(
            small_button("+".into(), SURF, TEXT, value < max)
                .id(gpui::SharedString::from(format!("step-up-{value}-{max}")))
                .on_click(cx.listener(move |this, _, _, cx| up(this, v_up, cx))),
        )
}
