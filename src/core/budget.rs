//! 进程级网络 worker 预算，防止多个任务各自开满连接而争抢资源。

use crate::core::progress::Control;
use std::sync::OnceLock;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const WAIT_SLICE: Duration = Duration::from_millis(25);

#[derive(Clone)]
pub struct WorkerBudget {
    inner: Arc<Inner>,
}

struct Inner {
    limit: usize,
    active: Mutex<usize>,
    available: Condvar,
}

pub struct WorkerPermit {
    inner: Arc<Inner>,
}

impl WorkerBudget {
    pub fn new(limit: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                limit: limit.max(1),
                active: Mutex::new(0),
                available: Condvar::new(),
            }),
        }
    }

    pub fn acquire(&self, ctrl: &Control) -> Option<WorkerPermit> {
        let mut active = self.inner.active.lock().unwrap_or_else(|p| p.into_inner());
        while *active >= self.inner.limit {
            if ctrl.interrupted() {
                return None;
            }
            let (next, _) = self
                .inner
                .available
                .wait_timeout(active, WAIT_SLICE)
                .unwrap_or_else(|p| p.into_inner());
            active = next;
        }
        *active += 1;
        Some(WorkerPermit {
            inner: self.inner.clone(),
        })
    }
}

impl Drop for WorkerPermit {
    fn drop(&mut self) {
        let mut active = self.inner.active.lock().unwrap_or_else(|p| p.into_inner());
        *active = active.saturating_sub(1);
        self.inner.available.notify_one();
    }
}

/// 所有 HTTP/HLS/FTP 传输共享的进程级预算。默认按 CPU 调整，避免桌面端
/// 多任务同时启动时把数百个阻塞 worker 堆到系统调度器上。
pub fn acquire_network_worker(ctrl: &Control) -> Option<WorkerPermit> {
    static BUDGET: OnceLock<WorkerBudget> = OnceLock::new();
    let budget = BUDGET.get_or_init(|| {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        WorkerBudget::new((cpus * 4).clamp(8, 32))
    });
    budget.acquire(ctrl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_makes_a_slot_available() {
        let budget = WorkerBudget::new(1);
        let ctrl = Control::new();
        let permit = budget.acquire(&ctrl).unwrap();
        drop(permit);
        assert!(budget.acquire(&ctrl).is_some());
    }

    #[test]
    fn interrupted_wait_returns_without_a_permit() {
        let budget = WorkerBudget::new(1);
        let held = budget.acquire(&Control::new()).unwrap();
        let ctrl = Control::new();
        let stop = ctrl.stop.clone();
        let waiting = std::thread::spawn(move || budget.acquire(&ctrl));
        std::thread::sleep(Duration::from_millis(20));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(waiting.join().unwrap().is_none());
        drop(held);
    }
}
