//! 顶部地址栏。这是整个应用的入口，也是 Downie 那一挂的手感。

use crate::ui::components::buttons::{ghost_button, primary_button};
use crate::ui::components::icons::icon_link;
use crate::ui::i18n::*;
use crate::ui::text_input::{url_paint_style, URL_INPUT};
use crate::ui::theme::*;
use crate::ui::Root;
use gpui::{
    div, prelude::*, px, rgb, Context, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    SharedString, Window,
};
use gpui_app_widgets::input::InputPaintState;

pub fn render_url_bar(root: &Root, window: &Window, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let input = &root.url_input;
    let focused = input.focus_handle.is_focused(window);
    let placeholder = SharedString::from(tr_url_placeholder(lang));
    let paint = InputPaintState::new(input, placeholder, 16., focused, root.cursor_blink_visible);
    let fh = input.focus_handle.clone();

    let text_content = div()
        .relative()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .overflow_hidden()
        .child(URL_INPUT.text_element(cx.entity(), url_paint_style(), paint));

    let box_el = div()
        .id(SharedString::from("url-bar"))
        .track_focus(&fh)
        .relative()
        .flex_1()
        .h(px(40.))
        .px_3()
        .rounded_full()
        .bg(rgb(SURF_LOW))
        .border_1()
        .when(focused, |d| d.border_color(rgb(PRIMARY)).bg(rgb(CARD)))
        .when(!focused, |d| {
            d.border_color(rgba(OUTLINE_VAR, 0.6))
                .hover(|h| h.bg(rgb(SURF_HIGH)))
        })
        .flex()
        .items_center()
        .gap_2()
        .cursor_text()
        .child(icon_link(if focused { PRIMARY } else { OUTLINE }, 16.))
        .child(text_content)
        .when(!input.text.is_empty(), |d| {
            d.child(
                div()
                    .id("url-clear")
                    .px_1()
                    .rounded_full()
                    .text_xs()
                    .text_color(rgb(OUTLINE))
                    .cursor_pointer()
                    .hover(|h| h.text_color(rgb(ERROR)))
                    .child("✕")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.url_input.clear();
                        this.poke_cursor_blink(cx);
                    })),
            )
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                URL_INPUT.mouse_down(this, event, window, cx);
            }),
        )
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
            match event.keystroke.key.as_str() {
                "escape" => {
                    this.url_input.clear();
                    cx.notify();
                }
                "enter" | "return" => this.submit_url(cx),
                _ => {
                    URL_INPUT.edit_key(this, &event.keystroke, cx);
                }
            }
            this.poke_cursor_blink(cx);
        }))
        .child(URL_INPUT.platform_element(cx.entity()));

    let can_add = !root.url_input.text.trim().is_empty();

    div()
        .flex_none()
        .w_full()
        .px_5()
        .py_3()
        .bg(rgb(CARD))
        .border_b_1()
        .border_color(rgba(OUTLINE_VAR, 0.55))
        .flex()
        .items_center()
        .gap_3()
        .child(box_el)
        .child(
            ghost_button(tr_btn_paste(lang).into(), true)
                .id("paste-btn")
                .on_click(cx.listener(|this, _, _, cx| this.paste_clipboard(cx))),
        )
        .child(
            primary_button(tr_btn_add(lang).into(), can_add)
                .id("add-btn")
                .on_click(cx.listener(|this, _, _, cx| this.submit_url(cx))),
        )
}
