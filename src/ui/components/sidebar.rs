use crate::core::i18n::Language;
use crate::ui::components::icons::*;
use crate::ui::i18n::*;
use crate::ui::theme::*;
use crate::ui::Root;
use gpui::{div, prelude::*, px, rgb, AnyElement, Context, IntoElement, SharedString};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    All,
    Active,
    Done,
    Failed,
    Settings,
}

impl View {
    pub const ALL: [View; 5] = [
        View::All,
        View::Active,
        View::Done,
        View::Failed,
        View::Settings,
    ];

    pub fn title_lang(&self, lang: Language) -> &'static str {
        match self {
            View::All => tr_view_all(lang),
            View::Active => tr_view_active(lang),
            View::Done => tr_view_done(lang),
            View::Failed => tr_view_failed(lang),
            View::Settings => tr_view_settings(lang),
        }
    }

    pub fn render_icon(&self, fg: u32) -> AnyElement {
        match self {
            View::All => icon_list(fg, 16.),
            View::Active => icon_bolt(fg, 16.),
            View::Done => icon_check(fg, 16.),
            View::Failed => icon_warn(fg, 16.),
            View::Settings => icon_gear(fg, 16.),
        }
    }
}

pub fn render_sidebar(root: &Root, cx: &mut Context<Root>) -> impl IntoElement {
    let current = root.view;
    let lang = root.language;

    let nav_item = |v: View, count: usize, cx: &mut Context<Root>| {
        let active = current == v;
        let fg_color = if active { PRIMARY } else { MUTED };
        div()
            .id(SharedString::from(format!("nav-{v:?}")))
            .h(px(44.))
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .rounded_xl()
            .cursor_pointer()
            .when(active, |d| d.bg(rgb(PRIMARY_FIXED)))
            .when(!active, |d| d.hover(|h| h.bg(rgb(SURF_LOW))))
            .child(
                div()
                    .w(px(24.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(v.render_icon(fg_color)),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .font_weight(if active {
                        gpui::FontWeight::BOLD
                    } else {
                        gpui::FontWeight::MEDIUM
                    })
                    .text_color(rgb(fg_color))
                    .child(v.title_lang(lang)),
            )
            .when(count > 0 && v != View::Settings, |d| {
                d.child(
                    div()
                        .px_2()
                        .py(px(1.))
                        .rounded_full()
                        .bg(if active {
                            rgb(PRIMARY)
                        } else {
                            rgb(SURF_HIGH)
                        })
                        .text_xs()
                        .text_color(if active {
                            rgb(ON_PRIMARY)
                        } else {
                            rgb(MUTED)
                        })
                        .child(count.to_string()),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.view = v;
                cx.notify();
            }))
    };

    div()
        .flex_none()
        .w(px(196.))
        .h_full()
        .bg(rgb(BG))
        .border_r_1()
        .border_color(rgba(OUTLINE_VAR, 0.6))
        .px_3()
        .py_4()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .px_2()
                .pb_4()
                .flex()
                .items_center()
                .gap_2()
                .child(icon_app_logo(28.))
                .child(
                    div()
                        .text_base()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(TEXT))
                        .child(tr_app(lang)),
                ),
        )
        .child(nav_item(View::All, root.tasks.len(), cx))
        .child(nav_item(View::Active, root.count_active(), cx))
        .child(nav_item(View::Done, root.count_done(), cx))
        .child(nav_item(View::Failed, root.count_failed(), cx))
        .child(div().flex_1())
        .child(nav_item(View::Settings, 0, cx))
        .child(
            div()
                .id("lang-toggle")
                .mt_2()
                .h(px(32.))
                .rounded_full()
                .bg(rgb(SURF))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|h| h.bg(rgb(SURF_HIGH)))
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(MUTED))
                .child(root.language.short_name().to_string())
                .on_click(cx.listener(|this, _, _, cx| this.toggle_language(cx))),
        )
}
