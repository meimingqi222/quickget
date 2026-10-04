//! 进程内共享的下载限速器。`0` 表示不限速。

use crate::core::progress::Control;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 单次最多授予的字节数。太小会让高限速下 grant 次数过多、线程 sleep 等待损耗大，
/// 导致实际吞吐只有目标的一半左右。取 4MiB，保证 40 万 MB/s 以内每次 grant 都能
/// 给足 100ms 的额度（limit/10），不被二次压小。
const MAX_GRANT: u64 = 4 * 1024 * 1024;
/// 把限速等待切成小片；这里只保证额度等待的轮询间隔，网络读取仍受请求超时限制。
const MAX_SLEEP_SLICE: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct DownloadLimiter {
    limit_bps: Arc<AtomicU64>,
    bucket: Arc<Mutex<Bucket>>,
}

struct Bucket {
    available: f64,
    updated_at: Instant,
}

impl DownloadLimiter {
    pub fn new(limit_bps: u64) -> Self {
        Self {
            limit_bps: Arc::new(AtomicU64::new(limit_bps)),
            bucket: Arc::new(Mutex::new(Bucket {
                available: 0.0,
                updated_at: Instant::now(),
            })),
        }
    }

    pub fn set_limit(&self, limit_bps: u64) {
        self.limit_bps.store(limit_bps, Ordering::Relaxed);
        if let Ok(mut bucket) = self.bucket.lock() {
            bucket.available = 0.0;
            bucket.updated_at = Instant::now();
        }
    }

    pub fn limit(&self) -> u64 {
        self.limit_bps.load(Ordering::Relaxed)
    }

    /// 在读取网络数据前申请最多 `wanted` 字节的额度。暂停或取消时返回 0。
    ///
    /// 凭证桶按实时速率补充，grant 到调用方真正想要的量为止。多 worker 并发时
    /// 尽量一次给足可用的额度，减少反复 sleep 的调度损耗。
    pub fn acquire(&self, wanted: usize, ctrl: &Control) -> usize {
        let wanted = (wanted as u64).min(MAX_GRANT);
        while !ctrl.interrupted() {
            let limit = self.limit_bps.load(Ordering::Relaxed);
            if limit == 0 {
                return wanted as usize;
            }
            let (granted, sleep) = {
                let mut bucket = self.bucket.lock().expect("下载限速器锁损坏");
                let now = Instant::now();
                let elapsed = now.duration_since(bucket.updated_at).as_secs_f64();
                bucket.updated_at = now;
                bucket.available = (bucket.available + elapsed * limit as f64).min(limit as f64);
                if bucket.available >= wanted as f64 {
                    bucket.available -= wanted as f64;
                    (wanted as usize, None)
                } else {
                    let granted = bucket.available.floor() as u64;
                    bucket.available -= granted as f64;
                    let needed = (wanted as f64 - granted as f64).max(0.0);
                    (
                        granted as usize,
                        Some(Duration::from_secs_f64(needed / limit as f64)),
                    )
                }
            };
            if granted > 0 {
                return granted as usize;
            }
            match sleep {
                Some(d) => {
                    // 精确等待；太短则不睡，避免高频唤醒抖动。
                    if d < Duration::from_micros(200) {
                        std::thread::yield_now();
                    } else {
                        std::thread::sleep(d.min(MAX_SLEEP_SLICE));
                    }
                }
                None => return wanted as usize,
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_limit_does_not_throttle() {
        let limiter = DownloadLimiter::new(0);
        // 不限速时应返回调用方要的量（不超过 MAX_GRANT）。
        assert_eq!(limiter.acquire(100_000, &Control::new()), 100_000);
        assert_eq!(
            limiter.acquire(MAX_GRANT as usize + 1, &Control::new()),
            MAX_GRANT as usize
        );
    }

    #[test]
    fn limit_can_be_changed() {
        let limiter = DownloadLimiter::new(0);
        limiter.set_limit(1024);
        assert_eq!(limiter.limit_bps.load(Ordering::Relaxed), 1024);
    }

    #[test]
    fn interrupted_acquire_returns_promptly() {
        let limiter = DownloadLimiter::new(1);
        let ctrl = Control::new();
        let stop = ctrl.stop.clone();
        let worker = std::thread::spawn(move || limiter.acquire(1024, &ctrl));
        std::thread::sleep(Duration::from_millis(20));
        stop.store(true, Ordering::Relaxed);
        assert_eq!(worker.join().unwrap(), 0);
    }
}
