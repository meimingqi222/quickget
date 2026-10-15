//! 地址栏的输入适配。中文输入必须走平台输入处理器，不能靠 on_key_down。
//!
//! 排版、命中、拖选、编辑键和 IME 转发由 `gpui_app_widgets::input` 统一实现；
//! 这里只保留地址栏的字号、字重、配色与光标闪烁。

use crate::ui::theme::{rgba, OUTLINE, PRIMARY, TEXT};
use crate::ui::Root;
use gpui::{rgb, Context, FontWeight, Hsla};
use gpui_app_widgets::input::{InputBinding, InputPaintStyle};

/// 地址栏字段。文本变化只需重绘；提交仍由 Enter / 添加按钮触发。
pub const URL_INPUT: InputBinding<Root> = InputBinding {
    read: |root| &root.url_input,
    write: |root| &mut root.url_input,
    on_change: |_, cx| cx.notify(),
    on_activity: Root::poke_cursor_blink,
    on_key: |_, _, _, _| false,
    font_size: 14.,
    font_weight: FontWeight::MEDIUM,
};

pub fn url_paint_style() -> InputPaintStyle {
    InputPaintStyle {
        text: Hsla::from(rgb(TEXT)),
        placeholder: Hsla::from(rgb(OUTLINE)),
        selection: rgba(PRIMARY, 0.28),
        caret: Hsla::from(rgb(PRIMARY)),
    }
}

impl Root {
    pub fn poke_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_blink_visible = true;
        cx.notify();
        self.ensure_cursor_blink(cx);
    }

    pub fn ensure_cursor_blink(&mut self, cx: &mut Context<Self>) {
        if self.cursor_blink_task.is_some() {
            return;
        }
        self.cursor_blink_visible = true;
        self.cursor_blink_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(530))
                .await;
            let should_continue = this
                .update(cx, |this, cx| {
                    if !this.cursor_blink_wanted {
                        return false;
                    }
                    this.cursor_blink_visible = !this.cursor_blink_visible;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !should_continue {
                this.update(cx, |this, _| {
                    this.cursor_blink_task = None;
                })
                .ok();
                break;
            }
        }));
    }
}
