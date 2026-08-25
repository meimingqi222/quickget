use crate::ui::components::buttons::{ghost_button, small_button};
use crate::ui::i18n::*;
use crate::ui::theme::*;
use crate::ui::Root;
use gpui::{div, prelude::*, px, rgb, Context, IntoElement};

pub fn render_settings_view(root: &Root, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let folder = root.settings.save_dir.display().to_string();
    let conn = root.settings.connections_clamped();
    let conc = root.settings.max_concurrent_clamped();

    div()
        .id("settings-scroll")
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .overflow_y_scroll()
        .px_8()
        .py_6()
        .flex()
        .flex_col()
        .gap_6()
        .child(
            div()
                .text_xl()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(TEXT))
                .child(tr_view_settings(lang)),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(tr_settings_blurb(lang)),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(tr_settings_ext(lang)),
        )
        .child(row(
            tr_settings_folder(lang),
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_sm()
                        .text_color(rgb(TEXT))
                        .child(folder),
                )
                .child(
                    ghost_button(tr_btn_browse(lang).into(), true)
                        .id("browse-dir")
                        .on_click(cx.listener(|this, _, _, cx| this.pick_save_dir(cx))),
                ),
        ))
        .child(row(
            tr_settings_conn(lang),
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
        ))
        .child(row(
            tr_settings_conc(lang),
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
        ))
        .child(row(tr_settings_clip(lang), {
            let on = root.settings.watch_clipboard;
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
            }))
        }))
        .child(row(
            tr_settings_lang(lang),
            small_button(
                root.language.short_name().into(),
                PRIMARY_FIXED,
                PRIMARY,
                true,
            )
            .id("lang-settings")
            .on_click(cx.listener(|this, _, _, cx| this.toggle_language(cx))),
        ))
}

fn row(label: &str, right: impl IntoElement) -> impl IntoElement {
    div()
        .w_full()
        .max_w(px(720.))
        .px_4()
        .py_3()
        .rounded_xl()
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgba(OUTLINE_VAR, 0.55))
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .w(px(160.))
                .flex_none()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(label.to_string()),
        )
        .child(div().flex_1().min_w(px(0.)).child(right))
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
                .w(px(36.))
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
