//! UI 状态。

use crate::core::model::Task;
use crate::core::progress::{Control, LiveMeta, LiveProgress};
use gpui::Task as GpuiTask;
pub use gpui_app_widgets::input::TextInputState;
use std::collections::HashMap;
use std::sync::Arc;

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
            s.ctrl
                .pause
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn pause_all(&self) {
        for s in self.inner.values() {
            s.ctrl
                .pause
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn resume_all(&self) {
        for s in self.inner.values() {
            s.ctrl
                .pause
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn stop(&self, id: &str) {
        if let Some(s) = self.inner.get(id) {
            s.ctrl
                .stop
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn stop_all(&self) {
        for s in self.inner.values() {
            s.ctrl
                .stop
                .store(true, std::sync::atomic::Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::progress::{Control, LiveMeta, LiveProgress};

    fn slot() -> RuntimeSlot {
        RuntimeSlot {
            ctrl: Control::new(),
            progress: LiveProgress::new(0, 0),
            meta: LiveMeta::new(),
            task: None,
        }
    }

    #[test]
    fn resume_all_clears_in_flight_pause_flags() {
        let mut map = RuntimeMap::default();
        map.insert("a".into(), slot());
        map.insert("b".into(), slot());
        map.pause_all();
        assert!(map.get("a").unwrap().ctrl.is_pause());
        assert!(map.get("b").unwrap().ctrl.is_pause());
        map.resume_all();
        assert!(!map.get("a").unwrap().ctrl.is_pause());
        assert!(!map.get("b").unwrap().ctrl.is_pause());
    }
}
