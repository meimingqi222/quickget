//! 输入法支持。中文输入必须走 EntityInputHandler，不能靠 on_key_down。

use gpui::{
    fill, point, px, rgb, size, App, Bounds, Context, EntityInputHandler, FontWeight, Hsla,
    LineLayout, Pixels, ShapedLine, SharedString, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use std::ops::Range;
use std::sync::Arc;

use crate::ui::state::SearchTextHit;
use crate::ui::theme::{rgba, OUTLINE, PRIMARY, TEXT};
use crate::ui::Root;

pub fn search_box_font(window: &Window) -> gpui::Font {
    let mut font = window.text_style().font();
    font.weight = FontWeight::MEDIUM;
    font
}

fn search_text_run(len: usize, font: gpui::Font, color: Hsla, underline: bool) -> TextRun {
    TextRun {
        len,
        font,
        color,
        background_color: None,
        underline: underline.then_some(UnderlineStyle {
            color: Some(color),
            thickness: px(1.0),
            wavy: false,
        }),
        strikethrough: None,
    }
}

pub fn shape_search_line(
    text: &str,
    font_size: f32,
    color: Hsla,
    marked: Option<&Range<usize>>,
    window: &Window,
) -> ShapedLine {
    let display = SharedString::from(text.replace('\n', " "));
    let font = search_box_font(window);
    let runs = if display.is_empty() {
        Vec::new()
    } else if let Some(marked) = marked {
        let marked = clamp_to_boundary(display.as_ref(), marked.clone());
        let mut runs = Vec::new();
        if marked.start > 0 {
            runs.push(search_text_run(marked.start, font.clone(), color, false));
        }
        if marked.end > marked.start {
            runs.push(search_text_run(
                marked.end - marked.start,
                font.clone(),
                color,
                true,
            ));
        }
        if marked.end < display.len() {
            runs.push(search_text_run(
                display.len() - marked.end,
                font,
                color,
                false,
            ));
        }
        runs
    } else {
        vec![search_text_run(display.len(), font, color, false)]
    };
    window
        .text_system()
        .shape_line(display, px(font_size), &runs, None)
}

pub fn layout_single_line_window(
    text: &str,
    font_size: f32,
    window: &mut Window,
) -> Arc<LineLayout> {
    let runs = if text.is_empty() {
        Vec::new()
    } else {
        vec![TextRun {
            len: text.len(),
            font: search_box_font(window),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }]
    };
    window
        .text_system()
        .layout_line(text, px(font_size), &runs, None)
}

pub fn closest_boundary_for_x(
    text: &str,
    rel_x: f32,
    x_for_index: impl Fn(usize) -> f32,
    line_width: f32,
) -> usize {
    if text.is_empty() || rel_x <= 0.0 {
        return 0;
    }
    let mut best_idx = 0;
    let mut best_dist = rel_x.abs();
    for (idx, _) in text.char_indices().skip(1) {
        let dist = (x_for_index(idx) - rel_x).abs();
        if dist < best_dist {
            best_dist = dist;
            best_idx = idx;
        }
    }
    if (line_width - rel_x).abs() < best_dist {
        text.len()
    } else {
        clamp_to_boundary(text, best_idx..best_idx).start
    }
}

pub fn closest_index_for_x_layout(
    text: &str,
    rel_x: f32,
    font_size: f32,
    window: &mut Window,
) -> usize {
    if text.is_empty() || rel_x <= 0.0 {
        return 0;
    }
    let layout = layout_single_line_window(text, font_size, window);
    closest_boundary_for_x(
        text,
        rel_x,
        |idx| f32::from(layout.x_for_index(idx)),
        f32::from(layout.width),
    )
}

pub fn index_for_mouse_x(
    text: &str,
    mouse_x: f32,
    hit: Option<&SearchTextHit>,
    font_size: f32,
    window: &mut Window,
) -> usize {
    if text.is_empty() {
        return 0;
    }
    let Some(hit) = hit else {
        return text.len();
    };
    let rel_x = mouse_x - f32::from(hit.bounds.origin.x);
    if rel_x <= 0.0 {
        return 0;
    }
    if hit.line.text.as_ref() == text {
        closest_boundary_for_x(
            text,
            rel_x,
            |idx| f32::from(hit.line.x_for_index(idx)),
            f32::from(hit.line.width),
        )
    } else {
        closest_index_for_x_layout(text, rel_x, font_size, window)
    }
}

pub struct SearchTextPaint<'a> {
    pub text: &'a str,
    pub placeholder: &'a str,
    pub sel: &'a Range<usize>,
    pub marked: Option<&'a Range<usize>>,
    pub font_size: f32,
    pub cursor_h: f32,
    pub focused: bool,
    pub cursor_visible: bool,
}

pub fn paint_search_text(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    spec: SearchTextPaint<'_>,
) -> SearchTextHit {
    let SearchTextPaint {
        text,
        placeholder,
        sel,
        marked,
        font_size,
        cursor_h,
        focused,
        cursor_visible,
    } = spec;
    let line_height = bounds.size.height;
    let is_empty = text.is_empty();
    let color = Hsla::from(rgb(if is_empty { OUTLINE } else { TEXT }));
    let paint_src = if is_empty { placeholder } else { text };
    let marked = if is_empty { None } else { marked };
    let line = shape_search_line(paint_src, font_size, color, marked, window);

    if focused && !is_empty && sel.start < sel.end {
        let x1 = f32::from(line.x_for_index(sel.start.min(line.len())));
        let x2 = f32::from(line.x_for_index(sel.end.min(line.len())));
        let sel_h = font_size + 4.0;
        let y_off = ((f32::from(line_height) - sel_h) / 2.0).max(0.0);
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x + px(x1), bounds.origin.y + px(y_off)),
                size(px((x2 - x1).max(2.0)), px(sel_h)),
            ),
            rgba(PRIMARY, 0.28),
        ));
    }

    let _ = line.paint(bounds.origin, line_height, window, cx);

    if focused && sel.start == sel.end && cursor_visible {
        let caret_x = if is_empty {
            0.0
        } else {
            f32::from(line.x_for_index(sel.start.min(line.len())))
        };
        let y_off = ((f32::from(line_height) - cursor_h) / 2.0).max(0.0);
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x + px(caret_x), bounds.origin.y + px(y_off)),
                size(px(1.5), px(cursor_h)),
            ),
            rgb(PRIMARY),
        ));
    }

    let stored = if is_empty {
        shape_search_line("", font_size, color, None, window)
    } else {
        line
    };
    SearchTextHit {
        bounds,
        line: stored,
    }
}

pub fn offset_from_utf16(s: &str, utf16_offset: usize) -> usize {
    let mut utf16_count = 0usize;
    for (byte_idx, ch) in s.char_indices() {
        if utf16_count >= utf16_offset {
            return byte_idx;
        }
        utf16_count += ch.len_utf16();
    }
    s.len()
}

pub fn offset_to_utf16(s: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(s.len());
    s[..clamped].chars().map(char::len_utf16).sum()
}

pub fn range_from_utf16(s: &str, r: &Range<usize>) -> Range<usize> {
    let start = offset_from_utf16(s, r.start);
    let end = offset_from_utf16(s, r.end);
    start.min(end)..start.max(end)
}

pub fn range_to_utf16(s: &str, r: &Range<usize>) -> Range<usize> {
    offset_to_utf16(s, r.start)..offset_to_utf16(s, r.end)
}

pub fn clamp_to_boundary(s: &str, r: Range<usize>) -> Range<usize> {
    let mut start = r.start.min(s.len());
    let mut end = r.end.min(s.len());
    while start > 0 && !s.is_char_boundary(start) {
        start -= 1;
    }
    while end < s.len() && !s.is_char_boundary(end) {
        end += 1;
    }
    start.min(end)..start.max(end)
}

pub fn move_left(text: &str, sel: Range<usize>, shift: bool) -> Range<usize> {
    let sel = clamp_to_boundary(text, sel);
    if !shift {
        if sel.start != sel.end {
            sel.start..sel.start
        } else if sel.start > 0 {
            let prev = text[..sel.start]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            prev..prev
        } else {
            0..0
        }
    } else {
        let prev = if sel.start > 0 {
            text[..sel.start]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0)
        } else {
            0
        };
        prev..sel.end
    }
}

pub fn move_right(text: &str, sel: Range<usize>, shift: bool) -> Range<usize> {
    let sel = clamp_to_boundary(text, sel);
    if !shift {
        if sel.start != sel.end {
            sel.end..sel.end
        } else if sel.end < text.len() {
            let next = text[sel.end..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| sel.end + i)
                .unwrap_or(text.len());
            next..next
        } else {
            text.len()..text.len()
        }
    } else {
        let next = if sel.end < text.len() {
            text[sel.end..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| sel.end + i)
                .unwrap_or(text.len())
        } else {
            text.len()
        };
        sel.start..next
    }
}

pub fn move_home(_text: &str, sel: Range<usize>, shift: bool) -> Range<usize> {
    if shift {
        0..sel.end
    } else {
        0..0
    }
}

pub fn move_end(text: &str, sel: Range<usize>, shift: bool) -> Range<usize> {
    if shift {
        sel.start..text.len()
    } else {
        text.len()..text.len()
    }
}

pub fn delete_forward(text: &mut String, sel: Range<usize>) -> Range<usize> {
    let sel = clamp_to_boundary(text, sel);
    if sel.start != sel.end {
        text.replace_range(sel.clone(), "");
        sel.start..sel.start
    } else if sel.start < text.len() {
        let next = text[sel.start..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| sel.start + i)
            .unwrap_or(text.len());
        text.replace_range(sel.start..next, "");
        sel.start..sel.start
    } else {
        sel
    }
}

pub fn delete_backward(text: &mut String, sel: Range<usize>) -> Range<usize> {
    let sel = clamp_to_boundary(text, sel);
    if sel.start != sel.end {
        text.replace_range(sel.clone(), "");
        sel.start..sel.start
    } else if sel.start > 0 {
        let prev = text[..sel.start]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        text.replace_range(prev..sel.start, "");
        prev..prev
    } else {
        sel
    }
}

fn resolve_replace_range(
    text: &str,
    sel: &Range<usize>,
    marked: &Option<Range<usize>>,
    range_utf16: Option<&Range<usize>>,
) -> Range<usize> {
    let range = range_utf16
        .map(|r| range_from_utf16(text, r))
        .or_else(|| marked.clone())
        .unwrap_or_else(|| clamp_to_boundary(text, sel.clone()));
    clamp_to_boundary(text, range)
}

pub fn replace_text(
    text: &mut String,
    sel: &Range<usize>,
    marked: &Option<Range<usize>>,
    range_utf16: Option<&Range<usize>>,
    new_text: &str,
) -> (Range<usize>, Option<Range<usize>>) {
    let range = resolve_replace_range(text, sel, marked, range_utf16);
    text.replace_range(range.clone(), new_text);
    let caret = range.start + new_text.len();
    (caret..caret, None)
}

pub fn replace_and_mark_text(
    text: &mut String,
    sel: &Range<usize>,
    marked: &Option<Range<usize>>,
    range_utf16: Option<&Range<usize>>,
    new_text: &str,
    new_selected_range_utf16: Option<&Range<usize>>,
) -> (Range<usize>, Option<Range<usize>>) {
    let range = resolve_replace_range(text, sel, marked, range_utf16);
    text.replace_range(range.clone(), new_text);
    let new_marked = if new_text.is_empty() {
        None
    } else {
        Some(range.start..range.start + new_text.len())
    };
    let composed = &text[range.start..range.start + new_text.len()];
    let new_sel = match new_selected_range_utf16 {
        Some(r) => {
            let inner = range_from_utf16(composed, r);
            range.start + inner.start..range.start + inner.end
        }
        None => {
            let caret = range.start + new_text.len();
            caret..caret
        }
    };
    (new_sel, new_marked)
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

impl EntityInputHandler for Root {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let input = &self.url_input;
        let range = clamp_to_boundary(&input.text, range_from_utf16(&input.text, &range_utf16));
        actual_range.replace(range_to_utf16(&input.text, &range));
        Some(input.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let input = &self.url_input;
        Some(UTF16Selection {
            range: range_to_utf16(&input.text, &input.selection()),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.url_input
            .marked
            .as_ref()
            .map(|r| range_to_utf16(&self.url_input.text, r))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.url_input.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.url_input.replace(range_utf16.as_ref(), new_text);
        self.poke_cursor_blink(cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.url_input.replace_and_mark(
            range_utf16.as_ref(),
            new_text,
            new_selected_range_utf16.as_ref(),
        );
        self.poke_cursor_blink(cx);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let input = &self.url_input;
        if let Some(hit) = input.text_hit.as_ref() {
            let range = clamp_to_boundary(&input.text, range_from_utf16(&input.text, &range_utf16));
            let x1 = hit.line.x_for_index(range.start.min(hit.line.len()));
            let x2 = hit.line.x_for_index(range.end.min(hit.line.len()));
            return Some(Bounds::from_corners(
                point(hit.bounds.origin.x + x1, hit.bounds.origin.y),
                point(
                    hit.bounds.origin.x + x2.max(x1 + px(1.0)),
                    hit.bounds.origin.y + hit.bounds.size.height,
                ),
            ));
        }
        Some(input.bounds.unwrap_or(element_bounds))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let input = &self.url_input;
        let byte_idx = index_for_mouse_x(
            &input.text,
            f32::from(point.x),
            input.text_hit.as_ref(),
            14.0,
            window,
        );
        Some(offset_to_utf16(&input.text, byte_idx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_inserts_at_caret() {
        let mut text = "ab".to_string();
        let (sel, marked) = replace_text(&mut text, &(1..1), &None, None, "X");
        assert_eq!(text, "aXb");
        assert_eq!(sel, 2..2);
        assert_eq!(marked, None);
    }
}
