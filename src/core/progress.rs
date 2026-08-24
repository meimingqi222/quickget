//! 跨线程进度。UI 只读原子量，引擎只写。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct Control {
    pub stop: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
}

impl Control {
    pub fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn is_pause(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }

    pub fn interrupted(&self) -> bool {
        self.is_stop() || self.is_pause()
    }
}

impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LiveProgress {
    pub downloaded: AtomicU64,
    pub total: AtomicU64,
    /// 上一拍的 downloaded，用来算瞬时速度。
    pub last_bytes: AtomicU64,
    pub speed_bps: AtomicU64,
}

impl LiveProgress {
    pub fn new(downloaded: u64, total: u64) -> Arc<Self> {
        Arc::new(Self {
            downloaded: AtomicU64::new(downloaded),
            total: AtomicU64::new(total),
            last_bytes: AtomicU64::new(downloaded),
            speed_bps: AtomicU64::new(0),
        })
    }

    pub fn add(&self, n: u64) {
        self.downloaded.fetch_add(n, Ordering::Relaxed);
    }

    pub fn set_total(&self, n: u64) {
        self.total.store(n, Ordering::Relaxed);
    }

    pub fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    pub fn speed(&self) -> u64 {
        self.speed_bps.load(Ordering::Relaxed)
    }

    /// `dt_ms` 是距上一拍的毫秒。返回当前速度。
    pub fn tick_speed(&self, dt_ms: u64) -> u64 {
        let now = self.downloaded.load(Ordering::Relaxed);
        let last = self.last_bytes.swap(now, Ordering::Relaxed);
        let delta = now.saturating_sub(last);
        let bps = if dt_ms == 0 {
            0
        } else {
            delta.saturating_mul(1000) / dt_ms
        };
        // 轻微平滑，避免进度条上的速度数字乱跳。
        let prev = self.speed_bps.load(Ordering::Relaxed);
        let smoothed = if prev == 0 {
            bps
        } else {
            (prev * 3 + bps) / 4
        };
        self.speed_bps.store(smoothed, Ordering::Relaxed);
        smoothed
    }
}

#[derive(Debug)]
pub enum JobOutcome {
    Completed { size: u64, path: std::path::PathBuf },
    Paused { downloaded: u64, size: u64 },
    Cancelled,
    Failed(String),
}

impl JobOutcome {
    pub fn from_interrupt(ctrl: &Control, downloaded: u64, size: u64) -> Self {
        if ctrl.is_stop() {
            JobOutcome::Cancelled
        } else {
            JobOutcome::Paused { downloaded, size }
        }
    }
}
