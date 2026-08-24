//! 顶部地址栏。这是整个应用的入口，也是 Downie 那一挂的手感。

use crate::ui::components::buttons::{ghost_button, primary_button};
use crate::ui::components::icons::icon_link;
use crate::ui::i18n::*;
use crate::ui::text_input::{clamp_to_boundary, index_for_mouse_x, paint_search_text, SearchTextPaint};
use crate::ui::theme::*;
use crate::ui::Root;
use gpui::{
    div, prelude::*, px, rgb, Bounds, Context, DispatchPhase, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, SharedString, Window,
};

pub fn render_url_bar(root: &Root, window: &Window, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let input = &root.url_input;
    let focused = input.focus_handle.is_focused(window);
    let text = input.text.clone();
    let placeholder = SharedString::from(tr_url_placeholder(lang));
    let sel = clamp_to_boundary(&text, input.sel.clone());
    let marked = input.marked.clone();
    let cursor_visible = root.cursor_blink_visible;
    let font_size = 14.0;
    let cursor_h = 16.0;
    let fh = input.focus_handle.clone();

    let paint_text = text.clone();
    let paint_placeholder = placeholder.clone();
    let paint_sel = sel.clone();
    let paint_marked = marked.clone();
    let text_entity = cx.entity();
    let text_content = div()
        .relative()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .overflow_hidden()
        .child(
            gpui::canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let hit = paint_search_text(
                        window,
                        cx,
                        bounds,
                        SearchTextPaint {
                            text: &paint_text,
                            placeholder: paint_placeholder.as_ref(),
                            sel: &paint_sel,
                            marked: paint_marked.as_ref(),
                            font_size,
                            cursor_h,
                            focused,
                            cursor_visible,
                        },
                    );
                    text_entity.update(cx, |this, _| {
                        this.url_input.text_hit = Some(hit);
                    });
                },
            )
            .size_full(),
        );

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
        .when(!text.is_empty(), |d| {
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
        .on_mouse_down(MouseButton::Left, {
            let fh = fh.clone();
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                fh.focus(window);
                if event.click_count >= 2 {
                    let input = &mut this.url_input;
                    input.sel = 0..input.text.len();
                    input.text_drag = None;
                    input.marked = None;
                    this.poke_cursor_blink(cx);
                    return;
                }
                let mouse_x: f32 = event.position.x.into();
                let idx = index_for_mouse_x(
                    &this.url_input.text,
                    mouse_x,
                    this.url_input.text_hit.as_ref(),
                    font_size,
                    window,
                );
                this.url_input.sel = idx..idx;
                this.url_input.text_drag = Some(idx);
                this.url_input.marked = None;
                this.poke_cursor_blink(cx);
            })
        })
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _window, cx| {
            let ctrl = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
            let shift = event.keystroke.modifiers.shift;
            match event.keystroke.key.as_str() {
                "backspace" => {
                    let input = &mut this.url_input;
                    input.sel = crate::ui::text_input::delete_backward(
                        &mut input.text,
                        input.sel.clone(),
                    );
                    input.marked = None;
                    cx.notify();
                }
                "delete" => {
                    let input = &mut this.url_input;
                    input.sel =
                        crate::ui::text_input::delete_forward(&mut input.text, input.sel.clone());
                    input.marked = None;
                    cx.notify();
                }
                "escape" => {
                    this.url_input.clear();
                    cx.notify();
                }
                "enter" | "return" => {
                    this.submit_url(cx);
                }
                "left" => {
                    let input = &mut this.url_input;
                    input.sel =
                        crate::ui::text_input::move_left(&input.text, input.sel.clone(), shift);
                    input.marked = None;
                    cx.notify();
                }
                "right" => {
                    let input = &mut this.url_input;
                    input.sel =
                        crate::ui::text_input::move_right(&input.text, input.sel.clone(), shift);
                    input.marked = None;
                    cx.notify();
                }
                "home" => {
                    let input = &mut this.url_input;
                    input.sel =
                        crate::ui::text_input::move_home(&input.text, input.sel.clone(), shift);
                    input.marked = None;
                    cx.notify();
                }
                "end" => {
                    let input = &mut this.url_input;
                    input.sel =
                        crate::ui::text_input::move_end(&input.text, input.sel.clone(), shift);
                    input.marked = None;
                    cx.notify();
                }
                "a" if ctrl => {
                    let input = &mut this.url_input;
                    input.sel = 0..input.text.len();
                    cx.notify();
                }
                "c" if ctrl => {
                    let input = &this.url_input;
                    let text = input.text[input.selection()].to_string();
                    if !text.is_empty() {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    }
                }
                "x" if ctrl => {
                    let input = &mut this.url_input;
                    let sel = input.selection();
                    let cut = input.text[sel.clone()].to_string();
                    if !cut.is_empty() {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(cut));
                        input.text.replace_range(sel.clone(), "");
                        input.sel = sel.start..sel.start;
                        input.marked = None;
                        cx.notify();
                    }
                }
                "v" if ctrl => {
                    if let Some(item) = cx.read_from_clipboard() {
                        if let Some(pasted) = item.text() {
                            let input = &mut this.url_input;
                            let sel = input.selection();
                            input.text.replace_range(sel.clone(), &pasted);
                            let caret = sel.start + pasted.len();
                            input.sel = caret..caret;
                            input.marked = None;
                            cx.notify();
                        }
                    }
                }
                _ => {}
            }
            this.poke_cursor_blink(cx);
        }))
        .child({
            let handle = fh.clone();
            let entity = cx.entity();
            gpui::canvas(move |bounds, _window, _cx| bounds, {
                move |_, bounds: Bounds<Pixels>, window, cx| {
                    entity.update(cx, |this, _| {
                        this.url_input.bounds = Some(bounds);
                    });
                    window.handle_input(
                        &handle,
                        gpui::ElementInputHandler::new(bounds, entity.clone()),
                        cx,
                    );
                    let ent = entity.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        if event.pressed_button != Some(MouseButton::Left) {
                            return;
                        }
                        let mouse_x: f32 = event.position.x.into();
                        ent.update(cx, |this, cx| {
                            let Some(anchor) = this.url_input.text_drag else {
                                return;
                            };
                            let cur = index_for_mouse_x(
                                &this.url_input.text,
                                mouse_x,
                                this.url_input.text_hit.as_ref(),
                                font_size,
                                window,
                            );
                            this.url_input.sel = cur.min(anchor)..cur.max(anchor);
                            cx.notify();
                        });
                    });
                }
            })
            .absolute()
            .size_full()
        });

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
