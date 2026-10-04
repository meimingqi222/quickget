//! 自动更新控制器：检查、下载、校验、安装交接
//!
//! 更新源为 GitHub Releases；开发态（cargo target）与用户关闭自动检查时
//! 不发起定时网络请求。

use crate::core::updater::{
    candidate_asset_names, current_target, download_to_file, evaluate_release,
    extract_update_zip, fetch_latest_release, fetch_text, is_skipped, parse_checksum_sidecar,
    sha256_file, DownloadProgress, UpdateOperation, UpdateStatus, CHECK_INTERVAL_MS,
    FIRST_CHECK_DELAY_MS,
};
use crate::ui::i18n::*;
use crate::ui::Root;
use gpui::{Context, Task as GpuiTask};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct UpdateState {
    pub status: UpdateStatus,
    pub checking: bool,
    pub show_dialog: bool,
    pub live_progress: Option<Arc<Mutex<DownloadProgress>>>,
    pub task: Option<GpuiTask<()>>,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self {
            status: UpdateStatus::Idle {
                current_version: env!("CARGO_PKG_VERSION").to_string(),
            },
            checking: false,
            show_dialog: false,
            live_progress: None,
            task: None,
        }
    }
}

impl Root {
    /// 启动延迟检查 + 周期检查。幂等。
    pub fn start_update_scheduler(&mut self, cx: &mut Context<Self>) {
        if crate::platform::is_packaged_install() {
            crate::platform::cleanup_previous_update_leftovers();
        }
        if self.update.task.is_some() || !crate::platform::is_packaged_install() {
            return;
        }
        if !self.settings.auto_check_updates {
            return;
        }
        self.update.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(FIRST_CHECK_DELAY_MS))
                .await;
            loop {
                let keep = this
                    .update(cx, |this, cx| {
                        this.check_for_updates_internal(false, cx);
                        this.settings.auto_check_updates
                    })
                    .unwrap_or(false);
                if !keep {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(CHECK_INTERVAL_MS))
                    .await;
            }
        }));
    }

    /// 用户手动「检查更新」。
    pub fn check_for_updates_manual(&mut self, cx: &mut Context<Self>) {
        if !crate::platform::is_packaged_install() {
            let lang = self.language;
            self.status = crate::core::i18n::bilingual(move |_| {
                match lang {
                    crate::core::i18n::Language::Zh => "开发环境中已跳过版本更新检测",
                    crate::core::i18n::Language::En => "Update check skipped in dev environment",
                }
                .to_string()
            });
            cx.notify();
            return;
        }
        self.check_for_updates_internal(true, cx);
    }

    fn check_for_updates_internal(&mut self, manual: bool, cx: &mut Context<Self>) {
        if !crate::platform::is_packaged_install() {
            return;
        }
        if !manual && !self.settings.auto_check_updates {
            return;
        }
        if self.update.checking {
            return;
        }
        if matches!(
            self.update.status,
            UpdateStatus::Downloading { .. }
                | UpdateStatus::Verifying { .. }
                | UpdateStatus::Downloaded { .. }
                | UpdateStatus::Installing { .. }
        ) {
            if manual {
                self.open_update_dialog(cx);
            }
            return;
        }

        let current_version = env!("CARGO_PKG_VERSION").to_string();
        let skipped = self.settings.skipped_update_version.clone();
        self.update.checking = true;
        self.update.status = UpdateStatus::Checking {
            current_version: current_version.clone(),
        };
        cx.notify();

        let work = cx.background_executor().spawn(async move {
            let release = fetch_latest_release(15_000)?;
            let candidates = candidate_asset_names(current_target());
            let status = evaluate_release(&current_version, &release, candidates)?;
            Ok::<UpdateStatus, String>(status)
        });

        cx.spawn(async move |this, cx| {
            let result = work.await;
            let applied = this.update(cx, |this, cx| {
                this.update.checking = false;
                this.settings.last_update_check_at = Some(chrono::Utc::now().timestamp());
                this.settings.save();
                match result {
                    Ok(status) => match &status {
                        UpdateStatus::Available { latest_version, .. } => {
                            if !manual && is_skipped(latest_version, skipped.as_deref()) {
                                this.update.status = UpdateStatus::NotAvailable {
                                    current_version: env!("CARGO_PKG_VERSION").to_string(),
                                };
                            } else {
                                this.update.status = status;
                                if manual {
                                    this.open_update_dialog(cx);
                                }
                            }
                        }
                        UpdateStatus::NotAvailable { .. } => {
                            this.update.status = status;
                            if manual {
                                let lang = this.language;
                                this.status = crate::core::i18n::bilingual(move |_| {
                                    tr_update_latest(lang).to_string()
                                });
                            }
                        }
                        _ => {
                            this.update.status = status;
                        }
                    },
                    Err(message) => {
                        crate::core::log::write(format_args!("更新检查失败: {message}"));
                        this.update.status = UpdateStatus::Error {
                            current_version: env!("CARGO_PKG_VERSION").to_string(),
                            operation: UpdateOperation::Check,
                            message: message.clone(),
                        };
                        if manual {
                            let lang = this.language;
                            this.status = crate::core::i18n::bilingual(move |_| {
                                format!("{}: {message}", tr_update_failed(lang))
                            });
                            this.open_update_dialog(cx);
                        }
                    }
                }
                cx.notify();
            });
            let _ = applied;
        })
        .detach();
    }

    pub fn open_update_dialog(&mut self, cx: &mut Context<Self>) {
        if !self.update.status.wants_attention() {
            return;
        }
        self.update.show_dialog = true;
        cx.notify();
    }

    pub fn dismiss_update_dialog(&mut self, cx: &mut Context<Self>) {
        self.update.show_dialog = false;
        cx.notify();
    }

    pub fn skip_this_update_version(&mut self, cx: &mut Context<Self>) {
        if let Some(v) = self.update.status.latest_version().map(|s| s.to_string()) {
            self.settings.skipped_update_version = Some(v);
            self.settings.save();
        }
        self.update.show_dialog = false;
        if matches!(self.update.status, UpdateStatus::Available { .. }) {
            self.update.status = UpdateStatus::NotAvailable {
                current_version: env!("CARGO_PKG_VERSION").to_string(),
            };
        }
        cx.notify();
    }

    /// 开始下载更新包（Available → Downloading → Verifying → Downloaded）。
    pub fn start_update_download(&mut self, cx: &mut Context<Self>) {
        let UpdateStatus::Available {
            current_version,
            latest_version,
            asset_name,
            asset_url,
            checksum_url,
            release_url,
            notes,
        } = self.update.status.clone()
        else {
            return;
        };
        let cache = match crate::platform::update_cache_dir() {
            Some(c) => c,
            None => {
                self.update.status = UpdateStatus::Error {
                    current_version,
                    operation: UpdateOperation::Download,
                    message: "无法获取更新缓存目录".into(),
                };
                cx.notify();
                return;
            }
        };

        self.update.status = UpdateStatus::Downloading {
            current_version: current_version.clone(),
            latest_version: latest_version.clone(),
            progress: Default::default(),
            asset_name: asset_name.clone(),
            asset_url: asset_url.clone(),
            checksum_url: checksum_url.clone(),
            release_url: release_url.clone(),
            notes: notes.clone(),
        };
        let progress = Arc::new(Mutex::new(DownloadProgress::default()));
        self.update.live_progress = Some(progress.clone());
        cx.notify();

        // 定时轮询通知 UI 刷新下载进度
        cx.spawn(async move |this, cx| loop {
            let downloading = this
                .update(cx, |this, _| {
                    matches!(this.update.status, UpdateStatus::Downloading { .. })
                })
                .unwrap_or(false);
            if !downloading {
                return;
            }
            let _ = this.update(cx, |_, cx| cx.notify());
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
        })
        .detach();

        let work = cx.background_executor().spawn(async move {
            let zip_path = cache.join(&asset_name);
            let sidecar_path = cache.join(format!("{asset_name}.sha256"));
            {
                let progress = progress.clone();
                download_to_file(&asset_url, &zip_path, &mut |transferred, total| {
                    if let Ok(mut p) = progress.lock() {
                        p.transferred = transferred;
                        p.total = total;
                        p.percent = match total {
                            Some(t) if t > 0 => {
                                (transferred as f32 / t as f32 * 100.0).clamp(0.0, 100.0)
                            }
                            _ => 0.0,
                        };
                    }
                })?;
            }
            let sidecar = fetch_text(&checksum_url, 20_000)?;
            let _ = std::fs::write(&sidecar_path, &sidecar);
            let expected = parse_checksum_sidecar(&sidecar)
                .ok_or_else(|| "校验文件格式异常".to_string())?;
            let actual = sha256_file(&zip_path).map_err(|e| format!("计算 SHA-256 失败: {e}"))?;
            if actual != expected {
                let _ = std::fs::remove_file(&zip_path);
                return Err(format!(
                    "SHA-256 校验不匹配（期望 {expected}，实际 {actual}）"
                ));
            }

            let extract_dir = cache.join("extracted");
            let payload = extract_update_zip(&zip_path, &extract_dir)?;
            Ok::<PathBuf, String>(payload)
        });

        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.update.live_progress = None;
                match result {
                    Ok(payload) => {
                        this.update.status = UpdateStatus::Downloaded {
                            current_version: env!("CARGO_PKG_VERSION").to_string(),
                            latest_version: latest_version.clone(),
                            payload,
                        };
                    }
                    Err(message) => {
                        crate::core::log::write(format_args!("更新下载/校验失败: {message}"));
                        this.update.status = UpdateStatus::Error {
                            current_version: env!("CARGO_PKG_VERSION").to_string(),
                            operation: UpdateOperation::Download,
                            message,
                        };
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 执行重启替换（Downloaded → Installing → 优雅暂停任务 → 替换重启）。
    pub fn apply_update_now(&mut self, cx: &mut Context<Self>) {
        let UpdateStatus::Downloaded {
            current_version,
            latest_version,
            payload,
        } = self.update.status.clone()
        else {
            return;
        };

        self.update.status = UpdateStatus::Installing {
            current_version,
            latest_version,
        };
        cx.notify();

        // 1. 优雅暂停所有进行中的任务，保存进度与断点
        self.runtime.stop_all();
        for t in &mut self.tasks {
            if t.status.is_active() {
                t.status = crate::core::model::TaskStatus::Paused;
            }
        }
        crate::core::store::save(&self.tasks);

        // 2. 拉起平台自替换 helper 脚本
        if let Err(e) = crate::platform::apply_update_and_restart(&payload) {
            crate::core::log::write(format_args!("更新替换失败: {e}"));
            self.update.status = UpdateStatus::Error {
                current_version: env!("CARGO_PKG_VERSION").to_string(),
                operation: UpdateOperation::Install,
                message: e,
            };
            cx.notify();
            return;
        }

        // 3. 正常退出当前进程
        cx.quit();
    }
}
