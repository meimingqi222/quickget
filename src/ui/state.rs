//! UI 状态。

use crate::core::model::Task;
use crate::core::progress::{Control, LiveMeta, LiveProgress};
use gpui::{Bounds, FocusHandle, Pixels, ShapedLine, Task as GpuiTask};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

pub struct SearchTextHit {
    pub bounds: Bounds<Pixels>,
    pub line: ShapedLine,
}

pub struct TextInputState {
    pub text: String,
    pub sel: Range<usize>,
    pub marked: Option<Range<usize>>,
    pub bounds: Option<Bounds<Pixels>>,
    pub text_hit: Option<SearchTextHit>,
    pub text_drag: Option<usize>,
    pub focus_handle: FocusHandle,
}

impl TextInputState {
    pub fn new(focus_handle: FocusHandle) -> Self {
        Self {
            text: String::new(),
            sel: 0..0,
            marked: None,
            bounds: None,
            text_hit: None,
            text_drag: None,
            focus_handle,
        }
    }

    pub fn selection(&self) -> Range<usize> {
        crate::ui::text_input::clamp_to_boundary(&self.text, self.sel.clone())
    }

    pub fn reset_caret(&mut self) {
        let end = self.text.len();
        self.sel = end..end;
        self.marked = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.reset_caret();
    }

    pub fn replace(&mut self, range_utf16: Option<&Range<usize>>, new_text: &str) {
        let (sel, marked) = crate::ui::text_input::replace_text(
            &mut self.text,
            &self.sel,
            &self.marked,
            range_utf16,
            new_text,
        );
        self.sel = sel;
        self.marked = marked;
    }

    pub fn replace_and_mark(
        &mut self,
        range_utf16: Option<&Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<&Range<usize>>,
    ) {
        let (sel, marked) = crate::ui::text_input::replace_and_mark_text(
            &mut self.text,
            &self.sel,
            &self.marked,
            range_utf16,
            new_text,
            new_selected_range_utf16,
        );
        self.sel = sel;
        self.marked = marked;
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.reset_caret();
    }
}

pub struct RuntimeSlot {
    pub ctrl: Control,
    pub progress: Arc<LiveProgress>,
    pub meta: LiveMeta,
    pub task: Option<GpuiTask<()>>,
}

#[derive(Default)]
pub struct RuntimeMap {
    inner: HashMap<String, RuntimeSlot>,
}

impl RuntimeMap {
    pub fn get(&self, id: &str) -> Option<&RuntimeSlot> {
        self.inner.get(id)
    }

    pub fn insert(&mut self, id: String, slot: RuntimeSlot) {
        self.inner.insert(id, slot);
    }

    pub fn remove(&mut self, id: &str) -> Option<RuntimeSlot> {
        self.inner.remove(id)
    }

    pub fn pause(&self, id: &str) {
        if let Some(s) = self.inner.get(id) {
            s.ctrl.pause.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn stop(&self, id: &str) {
        if let Some(s) = self.inner.get(id) {
            s.ctrl.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn active_count(&self) -> usize {
        self.inner.len()
    }

    pub fn total_speed(&self) -> u64 {
        self.inner.values().map(|s| s.progress.speed()).sum()
    }

    pub fn tick_all(&self, dt_ms: u64) {
        for s in self.inner.values() {
            s.progress.tick_speed(dt_ms);
        }
    }

    /// 把引擎进度、真实文件名和文件清单写回任务。有新标题或新清单时返回 true，该落盘。
    pub fn sync_into(&self, tasks: &mut [Task]) -> bool {
        let mut dirty = false;
        for t in tasks {
            if let Some(s) = self.inner.get(&t.id) {
                t.downloaded = s.progress.downloaded();
                let total = s.progress.total();
                if total > 0 {
                    t.size = total;
                }
                let snap = s.meta.snapshot();
                if let Some(name) = snap.filename {
                    if !name.is_empty() && t.filename != name {
                        t.filename = name;
                        dirty = true;
                    }
                }
                if !snap.files.is_empty() {
                    if t.files.is_empty() {
                        dirty = true;
                    }
                    t.files = snap.files;
                }
                if let Some(dir) = snap.output_dir {
                    if t.output_dir.as_ref() != Some(&dir) {
                        t.output_dir = Some(dir);
                        dirty = true;
                    }
                }
                t.peers = snap.peers;
            }
        }
        dirty
    }
}
