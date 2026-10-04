//! 几何图标，跟 QuickCleaner 同一套手法：div 拼形，不用 emoji。

use crate::ui::theme::*;
use gpui::{div, img, prelude::*, px, rgb, AnyElement, Image, ImageFormat, ImageSource};
use std::sync::Arc;

pub fn icon_app_logo(size: f32) -> AnyElement {
    static PNG_BYTES: &[u8] = include_bytes!("../../../assets/icon.png");
    let image = Arc::new(Image::from_bytes(ImageFormat::Png, PNG_BYTES.to_vec()));
    img(ImageSource::from(image))
        .w(px(size))
        .h(px(size))
        .into_any_element()
}

pub fn icon_list(fg: u32, size: f32) -> AnyElement {
    let w = size;
    div()
        .w(px(size))
        .h(px(size))
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(3.))
        .child(div().w(px(w)).h(px(2.)).rounded_full().bg(rgb(fg)))
        .child(
            div()
                .w(px(w * 0.75))
                .h(px(2.))
                .rounded_full()
                .bg(rgba(fg, 0.7)),
        )
        .child(div().w(px(w * 0.9)).h(px(2.)).rounded_full().bg(rgb(fg)))
        .into_any_element()
}

pub fn icon_bolt(fg: u32, size: f32) -> AnyElement {
    div()
        .w(px(size))
        .h(px(size))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .child(
            div()
                .w(px(2.5))
                .h(px(size * 0.38))
                .rounded_full()
                .bg(rgb(fg)),
        )
        .child(
            div()
                .w(px(size * 0.55))
                .h(px(size * 0.22))
                .rounded_full()
                .bg(rgb(fg)),
        )
        .into_any_element()
}

pub fn icon_check(fg: u32, size: f32) -> AnyElement {
    div()
        .w(px(size))
        .h(px(size))
        .rounded_full()
        .border_2()
        .border_color(rgb(fg))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(fg))
                .child("✓"),
        )
        .into_any_element()
}

pub fn icon_warn(fg: u32, size: f32) -> AnyElement {
    div()
        .w(px(size))
        .h(px(size))
        .rounded_full()
        .border_2()
        .border_color(rgb(fg))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(fg))
                .child("!"),
        )
        .into_any_element()
}

pub fn icon_gear(fg: u32, size: f32) -> AnyElement {
    div()
        .w(px(size))
        .h(px(size))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(size * 0.72))
                .h(px(size * 0.72))
                .rounded_full()
                .border_2()
                .border_color(rgb(fg))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(size * 0.28))
                        .h(px(size * 0.28))
                        .rounded_full()
                        .bg(rgb(fg)),
                ),
        )
        .into_any_element()
}

pub fn icon_link(fg: u32, size: f32) -> AnyElement {
    div()
        .w(px(size))
        .h(px(size))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(size * 0.7))
                .h(px(size * 0.38))
                .rounded_full()
                .border_2()
                .border_color(rgb(fg)),
        )
        .into_any_element()
}

pub fn icon_close(fg: u32, size: f32) -> AnyElement {
    div()
        .w(px(size))
        .h(px(size))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(fg))
                .child("✕"),
        )
        .into_any_element()
}

pub fn checkbox(checked: bool) -> AnyElement {
    if checked {
        div()
            .w(px(16.))
            .h(px(16.))
            .rounded(px(4.))
            .bg(rgb(PRIMARY))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(ON_PRIMARY))
                    .child("✓"),
            )
            .into_any_element()
    } else {
        div()
            .w(px(16.))
            .h(px(16.))
            .rounded(px(4.))
            .border_1()
            .border_color(rgb(OUTLINE_VAR))
            .bg(rgb(CARD))
            .hover(|h| h.border_color(rgb(PRIMARY)))
            .into_any_element()
    }
}

