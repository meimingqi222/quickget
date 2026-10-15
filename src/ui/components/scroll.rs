//! 产品滚动条配色；度量、拖拽和窗口级捕获由共享控件维护。

use crate::ui::theme::{rgba, OUTLINE, OUTLINE_VAR};
use gpui::{rgb, Div, SharedString, Stateful};
use gpui_app_widgets::scroll::{self, ScrollbarStyle};
pub use gpui_app_widgets::scroll::{
    drag_capture, drag_to_offset, scroll_metrics, ScrollMetrics, SCROLLBAR_W,
};

pub fn scrollbar(
    thumb_id: impl Into<SharedString>,
    metrics: ScrollMetrics,
    on_thumb_down: impl FnOnce(Stateful<Div>) -> Stateful<Div>,
) -> Div {
    scroll::scrollbar(
        ScrollbarStyle {
            track: rgba(OUTLINE_VAR, 0.14),
            thumb: rgb(OUTLINE).into(),
        },
        thumb_id,
        metrics,
        on_thumb_down,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::ScrollHandle;

    /// 布局尚未跑过时用估算值，保证滚动条首帧就能出现。
    #[test]
    fn falls_back_to_estimates_before_first_layout() {
        let handle = ScrollHandle::new();
        let m = scroll_metrics(&handle, 340.0, 340.0 * 4.0).expect("内容溢出时应有滚动条");
        assert_eq!(m.track_h, 340.0);
        assert_eq!(m.max_offset, 340.0 * 3.0);
        // 视口占内容 1/4，滑块也该是轨道的 1/4
        assert!((m.thumb_h - 85.0).abs() < 0.5);
    }

    #[test]
    fn no_scrollbar_when_content_fits() {
        let handle = ScrollHandle::new();
        assert!(scroll_metrics(&handle, 340.0, 200.0).is_none());
    }

    #[test]
    fn thumb_never_shrinks_below_grabbable_size() {
        let handle = ScrollHandle::new();
        // 5 万像素内容配 300 像素视口，等比算出来的滑块不到 2px
        let m = scroll_metrics(&handle, 300.0, 50_000.0).unwrap();
        assert!(m.thumb_h >= 28.0);
        assert!(m.thumb_top + m.thumb_h <= m.track_h + 0.01);
    }

    #[test]
    fn drag_needs_real_layout() {
        let handle = ScrollHandle::new();
        // 没有真实布局就没法换算行程，必须返回 None 而不是瞎猜
        assert!(drag_to_offset(&handle, (0.0, 0.0), 50.0).is_none());
    }
}
