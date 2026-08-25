//! GPUI 根视图。

pub mod components;
pub mod i18n;
pub mod state;
pub mod text_input;
pub mod theme;
pub mod views;

pub use state::*;

use crate::core::capture::CaptureJob;
use crate::core::engine::run_task;
use crate::core::i18n::{bilingual, Language, Text};
use crate::core::model::{adopt_existing_task, fmt_speed, truncate, AdoptResult, Task, TaskStatus};
use crate::core::progress::{Control, JobOutcome, LiveMeta, LiveProgress};
use crate::core::settings::Settings;
use crate::core::urlx::{
    detect_protocol, extract_urls, filename_from_source, filename_from_url, is_bt_placeholder,
    unique_path, Protocol,
};
use crate::ui::components::{render_sidebar, render_url_bar, View};
use crate::ui::i18n::*;
use crate::ui::theme::*;
use crate::ui::views::{render_settings_view, render_tasks_view};
use gpui::{
    div, prelude::*, px, rgb, Context, FocusHandle, IntoElement, KeyDownEvent, MouseButton, Render,
    ScrollHandle, Task as GpuiTask, Window,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 短时间进来这么多项就先确认再下，避免页面接管/粘贴列表把队列灌满。
const BULK_ADD_THRESHOLD: usize = 3;
/// 扩展投递是一条条进收件箱的，等这一小会儿看是不是一批。
const CAPTURE_BURST_QUIET_MS: u64 = 800;
const BULK_PREVIEW_LIMIT: usize = 8;

pub struct Root {
    pub language: Language,
    pub settings: Settings,
    pub view: View,
    pub status: Text,
    pub tasks: Vec<Task>,
    pub runtime: RuntimeMap,
    pub url_input: TextInputState,
    pub tick_task: Option<GpuiTask<()>>,
    pub clip_task: Option<GpuiTask<()>>,
    pub inbox_task: Option<GpuiTask<()>>,
    pub cleanup_tasks: Vec<GpuiTask<()>>,
    pub last_clipboard: String,
    pub clip_hint: Option<String>,
    pub anim_phase: usize,
    pub cursor_blink_visible: bool,
    pub cursor_blink_task: Option<GpuiTask<()>>,
    pub cursor_blink_wanted: bool,
    pub expanded_task: Option<String>,
    pub detail_scroll: ScrollHandle,
    /// 详情文件列表滚动条拖拽：(按下时鼠标 y, 当时的滚动偏移)。
    pub detail_scroll_drag: Option<(f32, f32)>,
    pub pending_confirm: Option<PendingConfirm>,
    confirm_focus: FocusHandle,
    pending_captures: Vec<CaptureJob>,
    capture_flush_task: Option<GpuiTask<()>>,
    capture_flush_gen: u64,
    /// 网盘直链解析中的后台任务句柄，保持存活用。
    netdisk_tasks: Vec<GpuiTask<()>>,
}

pub enum PendingConfirm {
    RemoveTask { id: String, filename: String },
    ClearDone { count: usize },
    AddUrls { urls: Vec<String> },
    AddCaptures { jobs: Vec<CaptureJob> },
}

impl Root {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
        let mut tasks = crate::core::store::load();
        // 上次没正常退出时，进行中的任务变成暂停，下次还能接着拉。
        for t in &mut tasks {
            if t.status.is_active() {
                t.status = TaskStatus::Paused;
            }
        }
        let url_input = TextInputState::new(cx.focus_handle());
        let mut root = Self {
            language: settings.language,
            settings,
            view: View::All,
            status: bilingual(|l| tr_status_ready(l)),
            tasks,
            runtime: RuntimeMap::default(),
            url_input,
            tick_task: None,
            clip_task: None,
            inbox_task: None,
            cleanup_tasks: Vec::new(),
            last_clipboard: String::new(),
            clip_hint: None,
            anim_phase: 0,
            cursor_blink_visible: true,
            cursor_blink_task: None,
            cursor_blink_wanted: false,
            expanded_task: None,
            detail_scroll: ScrollHandle::new(),
            detail_scroll_drag: None,
            pending_confirm: None,
            confirm_focus: cx.focus_handle(),
            pending_captures: Vec::new(),
            capture_flush_task: None,
            capture_flush_gen: 0,
            netdisk_tasks: Vec::new(),
        };
        crate::core::capture::write_pid();
        crate::core::capture::install_native_host();
        crate::core::http_api::start();
        root.start_tick(cx);
        root.start_clipboard_watch(cx);
        root.start_inbox_watch(cx);
        root.pump_queue(cx);
        root
    }

    pub fn toggle_language(&mut self, cx: &mut Context<Self>) {
        self.language = self.language.toggle();
        self.settings.language = self.language;
        self.settings.save();
        cx.notify();
    }

    pub fn count_active(&self) -> usize {
        self.tasks.iter().filter(|t| t.status.is_open()).count()
    }
    pub fn count_done(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .count()
    }
    pub fn count_failed(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Failed | TaskStatus::Cancelled))
            .count()
    }
    pub fn count_pausable(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| {
                matches!(
                    t.status,
                    TaskStatus::Queued | TaskStatus::Probing | TaskStatus::Downloading
                )
            })
            .count()
    }
    pub fn count_paused(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| {
                if t.status == TaskStatus::Paused {
                    return true;
                }
                matches!(t.status, TaskStatus::Probing | TaskStatus::Downloading)
                    && self.runtime.get(&t.id).is_some_and(|s| s.ctrl.is_pause())
            })
            .count()
    }

    pub fn persist(&self) {
        crate::core::store::save(&self.tasks);
    }

    pub fn submit_url(&mut self, cx: &mut Context<Self>) {
        if self.pending_confirm.is_some() {
            return;
        }
        let raw = self.url_input.text.clone();
        self.add_urls(&raw, cx);
        if matches!(self.pending_confirm, Some(PendingConfirm::AddUrls { .. })) {
            return;
        }
        if !self.url_input.text.is_empty() {
            // 成功吃进去才清空
            let added = extract_urls(&raw);
            if !added.is_empty() {
                self.url_input.clear();
            }
        }
    }

    pub fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard() {
            if let Some(text) = item.text() {
                self.url_input.set_text(text);
                self.clip_hint = None;
                cx.notify();
            }
        }
    }

    pub fn add_urls(&mut self, raw: &str, cx: &mut Context<Self>) {
        if self.pending_confirm.is_some() {
            return;
        }
        let urls = collect_downloadable_urls(raw);
        if urls.is_empty() {
            self.status = bilingual(|l| tr_status_bad_url(l));
            cx.notify();
            return;
        }
        if should_confirm_bulk(urls.len()) {
            self.pending_confirm = Some(PendingConfirm::AddUrls { urls });
            cx.notify();
            return;
        }
        self.commit_urls(urls, cx);
    }

    fn commit_urls(&mut self, urls: Vec<String>, cx: &mut Context<Self>) {
        let mut last_name = String::new();
        let mut last_kind: Option<AdoptResult> = None;
        for url in urls {
            let proto = detect_protocol(&url);
            if proto == Protocol::Unknown {
                continue;
            }
            if let Some(adopted) = self.adopt_existing(&url, None, None, None) {
                last_name = adopted.name().to_string();
                last_kind = Some(adopted);
                continue;
            }
            let filename_raw = if proto == Protocol::Hls {
                crate::core::hls::default_hls_filename(&url)
            } else if proto == Protocol::Magnet {
                filename_from_source(&url)
            } else {
                filename_from_url(&url)
            };
            let (filename, dest) = unique_path(&self.settings.save_dir, &filename_raw);
            let now = now_secs();
            let task = Task {
                id: uuid::Uuid::new_v4().to_string(),
                url,
                filename: filename.clone(),
                save_dir: self.settings.save_dir.clone(),
                protocol: proto,
                status: TaskStatus::Queued,
                size: 0,
                downloaded: 0,
                connections: self.settings.connections_clamped(),
                error: None,
                created_at: now,
                finished_at: None,
                referer: None,
                cookies: None,
                user_agent: None,
                files: Vec::new(),
                output_dir: if proto == Protocol::Magnet {
                    Some(dest)
                } else {
                    None
                },
                max_part_size: None,
                cleanup_paths: None,
                cleanup_cookies: None,
                peers: Default::default(),
            };
            last_name = filename;
            last_kind = None;
            self.tasks.insert(0, task);
        }
        if !last_name.is_empty() {
            self.status = match last_kind {
                Some(AdoptResult::Completed(_)) => {
                    bilingual(|l| tr_status_already_done(l, &last_name))
                }
                Some(AdoptResult::Active(_)) => bilingual(|l| tr_status_already(l, &last_name)),
                Some(AdoptResult::Requeued(_)) | None => {
                    bilingual(|l| tr_status_added(l, &last_name))
                }
            };
        }
        self.persist();
        self.pump_queue(cx);
        cx.notify();
    }

    pub fn add_capture(&mut self, job: CaptureJob, cx: &mut Context<Self>) {
        self.ingest_capture(job, TaskStatus::Queued, cx);
    }

    fn ingest_capture(&mut self, job: CaptureJob, status: TaskStatus, cx: &mut Context<Self>) {
        if let Some(nd) = job.netdisk.clone() {
            let ua = job.ua.clone();
            let cookies = job.cookies.unwrap_or_default();
            self.ingest_netdisk(nd, ua, cookies, cx);
            return;
        }
        let url = job.url.trim().to_string();
        if url.is_empty() {
            return;
        }
        let proto = detect_protocol(&url);
        if proto == Protocol::Unknown {
            self.status = bilingual(|l| tr_status_bad_url(l));
            cx.notify();
            return;
        }
        if let Some(adopted) = self.adopt_existing(
            &url,
            job.cookies.clone(),
            job.referer.clone(),
            job.ua.clone(),
        ) {
            let name = adopted.name().to_string();
            self.status = match adopted {
                AdoptResult::Requeued(_) => bilingual(|l| tr_status_added(l, &name)),
                AdoptResult::Active(_) => bilingual(|l| tr_status_already(l, &name)),
                AdoptResult::Completed(_) => bilingual(|l| tr_status_already_done(l, &name)),
            };
            self.persist();
            self.pump_queue(cx);
            raise_gui(cx);
            cx.notify();
            return;
        }
        let filename_raw = job
            .filename
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if proto == Protocol::Hls {
                    crate::core::hls::default_hls_filename(&url)
                } else if proto == Protocol::Magnet {
                    filename_from_source(&url)
                } else {
                    filename_from_url(&url)
                }
            });
        let (filename, dest) = unique_path(&self.settings.save_dir, &filename_raw);
        let task = Task {
            id: uuid::Uuid::new_v4().to_string(),
            url,
            filename: filename.clone(),
            save_dir: self.settings.save_dir.clone(),
            protocol: proto,
            status,
            size: 0,
            downloaded: 0,
            connections: self.settings.connections_clamped(),
            error: None,
            created_at: now_secs(),
            finished_at: None,
            referer: job.referer,
            cookies: job.cookies,
            user_agent: job.ua,
            files: Vec::new(),
            output_dir: if proto == Protocol::Magnet {
                Some(dest)
            } else {
                None
            },
            max_part_size: job.max_part_size,
            cleanup_paths: job.cleanup_paths,
            cleanup_cookies: job.cleanup_cookies,
            peers: Default::default(),
        };
        self.tasks.insert(0, task);
        self.status = if status == TaskStatus::Paused {
            bilingual(|l| tr_status_paused(l, &filename))
        } else {
            bilingual(|l| tr_status_added(l, &filename))
        };
        self.persist();
        self.pump_queue(cx);
        raise_gui(cx);
        cx.notify();
    }

    /// 网盘分享任务：交给 providers 注册表里对应的插件，先用页面无关的 API
    /// 换明文直链，再把每个文件按普通 HTTP 任务入队。解析在后台线程跑。
    fn ingest_netdisk(
        &mut self,
        nd: crate::core::providers::NetdiskRequest,
        ua: Option<String>,
        cookies: String,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) =
            crate::core::providers::find(nd.provider.as_deref(), &nd.share_url)
        else {
            self.status = bilingual(|l| tr_netdisk_unsupported(l, nd.share_url.trim()));
            cx.notify();
            return;
        };
        self.status = bilingual(|l| tr_netdisk_resolving(l, provider.display_name()));
        cx.notify();
        let ua = ua.unwrap_or_else(|| self.settings.user_agent.clone());
        let api_ua = ua.clone();
        let share_referer = nd.share_url.clone();
        let netdisk_cookies = cookies.clone();
        let work = cx.background_executor().spawn(async move {
            // 先探登录档位（决定直链的服务器限速），再解析直链；失败互不阻断。
            let account = provider.check_login(cookies.trim(), &api_ua);
            let files = provider.resolve(&nd, cookies.trim(), &api_ua);
            (provider, share_referer, account, files, netdisk_cookies)
        });
        let task_ua = ua.clone();
        let task = cx.spawn(async move |this, cx| {
            let (provider, referer, account, result, netdisk_cookies) = work.await;
            this.update(cx, move |this, cx| {
                this.apply_netdisk_result(provider, referer, account, result, task_ua, netdisk_cookies, cx);
            })
            .ok();
        });
        self.netdisk_tasks.push(task);
    }

    fn apply_netdisk_result(
        &mut self,
        provider: &'static dyn crate::core::providers::ShareProvider,
        referer: String,
        account: Result<crate::core::providers::AccountInfo, String>,
        result: Result<Vec<crate::core::providers::ResolvedFile>, String>,
        ua: String,
        netdisk_cookies: String,
        cx: &mut Context<Self>,
    ) {
        // 登录档位决定 dlink 的服务端限速：顶级会员满速、游客严格限速。
        let account = account.unwrap_or_default();
        match result {
            Err(e) => {
                let name = provider.display_name();
                self.status = bilingual(|l| tr_netdisk_fail(l, name, &e));
            }
            Ok(files) if files.is_empty() => {
                let name = provider.display_name();
                self.status =
                    bilingual(|l| tr_netdisk_fail(l, name, "分享里没有可下载的文件"));
            }
            Ok(files) => {
                // 直链任务默认不带网盘 Cookie：签名已含账号权益，
                // 登录凭证留在网盘域内，不流向 CDN。
                let count = files.len();
                let file_ua = provider
                    .download_ua()
                    .map(|s| s.to_string())
                    .unwrap_or(ua);
                let max_part = provider.max_part_size();
                for f in files {
                    // 收集转存路径用于下载后清理（仅百度转存方式有 transfer_path）。
                    let cleanup = f.transfer_path.as_ref().map(|p| vec![p.clone()]);
                    let cleanup_ck = cleanup.as_ref().map(|_| netdisk_cookies.clone());
                    self.add_capture(
                        CaptureJob {
                            url: f.dlink,
                            referer: Some(referer.clone()),
                            ua: Some(file_ua.clone()),
                            filename: Some(f.filename),
                            max_part_size: max_part,
                            cleanup_paths: cleanup,
                            cleanup_cookies: cleanup_ck,
                            ..CaptureJob::default()
                        },
                        cx,
                    );
                }
                let name = provider.display_name();
                self.status = bilingual(|l| tr_netdisk_added(l, name, count, account.clone()));
            }
        }
        cx.notify();
    }

    /// 同一资源（去掉 sid 等 query）已在列表里：更新凭证，失败的重新排队。
    fn adopt_existing(
        &mut self,
        url: &str,
        cookies: Option<String>,
        referer: Option<String>,
        ua: Option<String>,
    ) -> Option<AdoptResult> {
        adopt_existing_task(&mut self.tasks, url, cookies, referer, ua)
    }

    fn buffer_captures(&mut self, jobs: Vec<CaptureJob>, cx: &mut Context<Self>) {
        if jobs.is_empty() {
            return;
        }
        if let Some(PendingConfirm::AddCaptures { jobs: pending }) = &mut self.pending_confirm {
            pending.extend(jobs);
            raise_gui(cx);
            cx.notify();
            return;
        }
        self.pending_captures.extend(jobs);
        if self.pending_captures.len() >= BULK_ADD_THRESHOLD && self.pending_confirm.is_none() {
            let jobs = std::mem::take(&mut self.pending_captures);
            self.pending_confirm = Some(PendingConfirm::AddCaptures { jobs });
            raise_gui(cx);
            cx.notify();
            return;
        }
        self.schedule_capture_flush(cx);
    }

    fn schedule_capture_flush(&mut self, cx: &mut Context<Self>) {
        self.capture_flush_gen = self.capture_flush_gen.wrapping_add(1);
        let gen = self.capture_flush_gen;
        self.capture_flush_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(CAPTURE_BURST_QUIET_MS))
                .await;
            this.update(cx, |this, cx| {
                if this.capture_flush_gen != gen {
                    return;
                }
                this.flush_captures(cx);
            })
            .ok();
        }));
    }

    fn flush_captures(&mut self, cx: &mut Context<Self>) {
        if self.pending_confirm.is_some() {
            return;
        }
        let jobs = std::mem::take(&mut self.pending_captures);
        if jobs.is_empty() {
            return;
        }
        if should_confirm_bulk(jobs.len()) {
            self.pending_confirm = Some(PendingConfirm::AddCaptures { jobs });
            raise_gui(cx);
            cx.notify();
            return;
        }
        for job in jobs {
            self.add_capture(job, cx);
        }
    }

    pub fn start_inbox_watch(&mut self, cx: &mut Context<Self>) {
        if self.inbox_task.is_some() {
            return;
        }
        self.inbox_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            this.update(cx, |this, cx| {
                let raise = crate::core::capture::take_raise();
                let jobs = crate::core::capture::drain();
                if raise && jobs.is_empty() && this.pending_confirm.is_none() {
                    raise_gui(cx);
                }
                if !jobs.is_empty() {
                    this.buffer_captures(jobs, cx);
                }
            })
            .ok();
        }));
    }

    pub fn pump_queue(&mut self, cx: &mut Context<Self>) {
        let cap = self.settings.max_concurrent_clamped() as usize;
        while self.runtime.active_count() < cap {
            let next = self
                .tasks
                .iter()
                .find(|t| t.status == TaskStatus::Queued)
                .map(|t| t.id.clone());
            let Some(id) = next else { break };
            self.start_task(&id, cx);
        }
        self.start_tick(cx);
    }

    fn start_task(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(idx) = self.tasks.iter().position(|t| t.id == id) else {
            return;
        };
        if self.runtime.get(id).is_some() {
            return;
        }
        self.tasks[idx].status = TaskStatus::Downloading;
        self.tasks[idx].error = None;
        let task = self.tasks[idx].clone();
        let ua = task
            .user_agent
            .clone()
            .unwrap_or_else(|| self.settings.user_agent.clone());
        let ctrl = Control::new();
        let progress = LiveProgress::new(task.downloaded, task.size);
        let meta = LiveMeta::new();
        if !task.files.is_empty() {
            meta.set_files(task.files.clone());
        }
        let ctrl_bg = ctrl.clone();
        let progress_bg = progress.clone();
        let meta_bg = meta.clone();

        let work = cx
            .background_executor()
            .spawn(async move { run_task(&task, &ua, progress_bg, ctrl_bg, meta_bg) });
        let id_owned = id.to_string();
        let gpui_task = cx.spawn(async move |this, cx| {
            let outcome = work.await;
            this.update(cx, |this, cx| {
                this.apply_outcome(&id_owned, outcome, cx);
                this.runtime.remove(&id_owned);
                this.persist();
                this.pump_queue(cx);
                cx.notify();
            })
            .ok();
        });
        self.runtime.insert(
            id.to_string(),
            RuntimeSlot {
                ctrl,
                progress,
                meta,
                task: Some(gpui_task),
            },
        );
        self.persist();
    }

    fn apply_outcome(&mut self, id: &str, outcome: JobOutcome, cx: &mut Context<Self>) {
        let snap = self
            .runtime
            .get(id)
            .map(|s| s.meta.snapshot())
            .unwrap_or_default();
        let pause_still_wanted = self
            .runtime
            .get(id)
            .map(|s| s.ctrl.is_pause())
            .unwrap_or(true);
        let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if let Some(name) = snap.filename {
            if !name.is_empty() {
                t.filename = name;
            }
        }
        if !snap.files.is_empty() {
            t.files = snap.files;
        }
        if let Some(dir) = snap.output_dir {
            t.output_dir = Some(dir);
        }
        t.peers = snap.peers;
        match outcome {
            JobOutcome::Completed { size, path } => {
                t.status = TaskStatus::Completed;
                t.size = size;
                t.downloaded = size;
                t.finished_at = Some(now_secs());
                t.error = None;
                if t.protocol == Protocol::Magnet {
                    t.output_dir = Some(path.clone());
                }
                if let Some(name) = path.file_name() {
                    let n = name.to_string_lossy().into_owned();
                    if !is_bt_placeholder(&n) || is_bt_placeholder(&t.filename) {
                        t.filename = n;
                    }
                }
                for f in &mut t.files {
                    f.downloaded = f.size;
                }
                t.peers = Default::default();
                let name = t.filename.clone();
                self.status = bilingual(|l| tr_status_done(l, &name));
                // 下载成功后清理网盘转存的临时文件（尽力而为，不阻断）。
                if let (Some(paths), Some(ck)) = (t.cleanup_paths.take(), t.cleanup_cookies.take()) {
                    if !paths.is_empty() {
                        cx.background_executor()
                            .spawn(async move {
                                crate::core::providers::baidu::delete_files(&ck, &paths);
                            })
                            .detach();
                    }
                }
            }
            JobOutcome::Paused { downloaded, size } => {
                t.downloaded = downloaded;
                if size > 0 {
                    t.size = size;
                }
                t.status = status_after_engine_pause(pause_still_wanted);
                if pause_still_wanted {
                    let name = t.filename.clone();
                    self.status = bilingual(|l| tr_status_paused(l, &name));
                } else {
                    t.error = None;
                }
            }
            JobOutcome::Cancelled => {
                t.status = TaskStatus::Cancelled;
                t.finished_at = Some(now_secs());
                t.peers = Default::default();
            }
            JobOutcome::Failed(msg) => {
                t.status = TaskStatus::Failed;
                t.error = Some(msg.clone());
                t.finished_at = Some(now_secs());
                self.status = bilingual(|l| tr_status_fail(l, &msg));
            }
        }
    }

    pub fn pause_task(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
            if t.status == TaskStatus::Queued {
                t.status = TaskStatus::Paused;
                self.persist();
                cx.notify();
                return;
            }
        }
        self.runtime.pause(id);
        cx.notify();
    }

    pub fn pause_all(&mut self, cx: &mut Context<Self>) {
        let mut queued = 0usize;
        let mut active = 0usize;
        for t in &mut self.tasks {
            match t.status {
                TaskStatus::Queued => {
                    t.status = TaskStatus::Paused;
                    queued += 1;
                }
                TaskStatus::Probing | TaskStatus::Downloading => {
                    active += 1;
                }
                _ => {}
            }
        }
        self.runtime.pause_all();
        let n = queued + active;
        if n == 0 {
            return;
        }
        if queued > 0 {
            self.persist();
        }
        self.status = bilingual(|l| tr_status_paused_all(l, n));
        cx.notify();
    }

    pub fn resume_task(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
            t.status = TaskStatus::Queued;
            t.error = None;
        }
        self.persist();
        self.pump_queue(cx);
        cx.notify();
    }

    pub fn resume_all(&mut self, cx: &mut Context<Self>) {
        let pausing_active = self
            .tasks
            .iter()
            .filter(|t| {
                matches!(t.status, TaskStatus::Probing | TaskStatus::Downloading)
                    && self.runtime.get(&t.id).is_some_and(|s| s.ctrl.is_pause())
            })
            .count();
        let mut n = pausing_active;
        for t in &mut self.tasks {
            if t.status == TaskStatus::Paused {
                t.status = TaskStatus::Queued;
                t.error = None;
                n += 1;
            }
        }
        self.runtime.resume_all();
        if n == 0 {
            return;
        }
        self.status = bilingual(|l| tr_status_resumed_all(l, n));
        self.persist();
        self.pump_queue(cx);
        cx.notify();
    }

    pub fn request_remove_task(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tasks.iter().find(|t| t.id == id) else {
            return;
        };
        self.pending_confirm = Some(PendingConfirm::RemoveTask {
            id: t.id.clone(),
            filename: t.filename.clone(),
        });
        self.confirm_focus.focus(window);
        cx.notify();
    }

    pub fn request_clear_done(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.count_done();
        if count == 0 {
            return;
        }
        self.pending_confirm = Some(PendingConfirm::ClearDone { count });
        self.confirm_focus.focus(window);
        cx.notify();
    }

    pub fn dismiss_confirm(&mut self, cx: &mut Context<Self>) {
        match self.pending_confirm.take() {
            Some(PendingConfirm::AddCaptures { jobs }) => {
                let n = jobs.len();
                for job in jobs {
                    self.ingest_capture(job, TaskStatus::Paused, cx);
                }
                if n > 0 {
                    self.status = bilingual(|l| tr_status_held_captures(l, n));
                }
                if !self.pending_captures.is_empty() {
                    self.schedule_capture_flush(cx);
                }
                cx.notify();
            }
            Some(_) => {
                if !self.pending_captures.is_empty() {
                    self.schedule_capture_flush(cx);
                }
                cx.notify();
            }
            None => {}
        }
    }

    pub fn confirm_pending(&mut self, cx: &mut Context<Self>) {
        match self.pending_confirm.take() {
            Some(PendingConfirm::RemoveTask { id, .. }) => self.remove_task(&id, cx),
            Some(PendingConfirm::ClearDone { .. }) => self.clear_done(cx),
            Some(PendingConfirm::AddUrls { urls }) => {
                self.commit_urls(urls, cx);
                self.url_input.clear();
            }
            Some(PendingConfirm::AddCaptures { jobs }) => {
                for job in jobs {
                    self.add_capture(job, cx);
                }
            }
            None => {}
        }
        if !self.pending_captures.is_empty() {
            self.schedule_capture_flush(cx);
        }
    }

    pub fn remove_task(&mut self, id: &str, cx: &mut Context<Self>) {
        self.runtime.stop(id);
        let task = self.tasks.iter().find(|t| t.id == id).cloned();
        let live_meta = self.runtime.get(id).map(|s| s.meta.clone());
        let wait_for_stop = self.runtime.get(id).is_some();
        self.tasks.retain(|t| t.id != id);
        if self.expanded_task.as_deref() == Some(id) {
            self.expanded_task = None;
        }
        self.status = bilingual(|l| tr_status_trashed(l));
        self.persist();
        cx.notify();
        if !wait_for_stop {
            if let Some(task) = task {
                crate::core::io::trash_paths(&leftover_paths_for_remove(task, live_meta.as_ref()));
            }
            self.runtime.remove(id);
            return;
        }
        let id = id.to_string();
        self.cleanup_tasks.push(cx.spawn(async move |this, cx| {
            for _ in 0..50 {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let gone = this
                    .update(cx, |this, _| this.runtime.get(&id).is_none())
                    .unwrap_or(true);
                if gone {
                    break;
                }
            }
            let paths = task
                .map(|t| leftover_paths_for_remove(t, live_meta.as_ref()))
                .unwrap_or_default();
            cx.background_executor()
                .spawn(async move {
                    crate::core::io::trash_paths(&paths);
                })
                .await;
            this.update(cx, |this, _| {
                this.runtime.remove(&id);
            })
            .ok();
        }));
    }

    pub fn clear_done(&mut self, cx: &mut Context<Self>) {
        self.tasks.retain(|t| t.status != TaskStatus::Completed);
        self.persist();
        cx.notify();
    }

    pub fn open_task(&mut self, id: &str, _cx: &mut Context<Self>) {
        if let Some(t) = self.tasks.iter().find(|t| t.id == id) {
            crate::platform::open_in_default_app(&t.dest_path());
        }
    }

    pub fn reveal_task(&mut self, id: &str, _cx: &mut Context<Self>) {
        if let Some(t) = self.tasks.iter().find(|t| t.id == id) {
            crate::platform::reveal_in_explorer(&t.dest_path());
        }
    }

    pub fn pick_save_dir(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.settings.save_dir = path;
            self.settings.save();
            cx.notify();
        }
    }

    pub fn start_tick(&mut self, cx: &mut Context<Self>) {
        if self.tick_task.is_some() {
            return;
        }
        self.tick_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let keep = this
                .update(cx, |this, cx| {
                    this.anim_phase = this.anim_phase.wrapping_add(1);
                    this.runtime.tick_all(200);
                    if this.runtime.sync_into(&mut this.tasks) {
                        this.persist();
                    }
                    let n = this.tasks.iter().filter(|t| t.status.is_active()).count();
                    if n > 0 {
                        let speed = fmt_speed(this.runtime.total_speed());
                        this.status = bilingual(|l| tr_status_speed(l, &speed, n));
                    }
                    cx.notify();
                    n > 0 || this.runtime.active_count() > 0
                })
                .unwrap_or(false);
            if !keep {
                this.update(cx, |this, _| {
                    this.tick_task = None;
                })
                .ok();
                break;
            }
        }));
    }

    pub fn start_clipboard_watch(&mut self, cx: &mut Context<Self>) {
        if self.clip_task.is_some() {
            return;
        }
        self.clip_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(900))
                .await;
            this.update(cx, |this, cx| {
                if !this.settings.watch_clipboard {
                    return;
                }
                let Some(item) = cx.read_from_clipboard() else {
                    return;
                };
                let Some(text) = item.text() else { return };
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() || trimmed == this.last_clipboard {
                    return;
                }
                this.last_clipboard = trimmed.clone();
                if extract_urls(&trimmed).is_empty() {
                    return;
                }
                if this.url_input.text.trim() == trimmed {
                    return;
                }
                this.clip_hint = Some(trimmed);
                cx.notify();
            })
            .ok();
        }));
    }

    fn accept_clip_hint(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.clip_hint.take() {
            self.add_urls(&url, cx);
        }
    }

    pub fn toggle_task(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.expanded_task.as_deref() == Some(id) {
            self.expanded_task = None;
            self.detail_scroll_drag = None;
        } else {
            self.expanded_task = Some(id.to_string());
            self.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
            self.detail_scroll_drag = None;
        }
        cx.notify();
    }

    pub fn reveal_task_file(&mut self, id: &str, file_path: &str, _cx: &mut Context<Self>) {
        let Some(t) = self.tasks.iter().find(|t| t.id == id) else {
            return;
        };
        if let Some(file) = t.files.iter().find(|f| f.path == file_path) {
            let p = t.file_disk_path(file);
            if p.exists() {
                crate::platform::reveal_in_explorer(&p);
                return;
            }
        }
        crate::platform::reveal_in_explorer(&t.dest_path());
    }
}

fn raise_gui(cx: &mut Context<Root>) {
    cx.activate(true);
    let handles = cx.windows();
    cx.defer(move |cx| {
        for handle in handles {
            let _ = handle.update(cx, |_, window, _| {
                window.activate_window();
            });
        }
    });
}

fn status_after_engine_pause(pause_still_wanted: bool) -> TaskStatus {
    if pause_still_wanted {
        TaskStatus::Paused
    } else {
        TaskStatus::Queued
    }
}

fn leftover_paths_for_remove(mut task: Task, meta: Option<&LiveMeta>) -> Vec<std::path::PathBuf> {
    if let Some(meta) = meta {
        let snap = meta.snapshot();
        if let Some(name) = snap.filename {
            if !name.is_empty() {
                task.filename = name;
            }
        }
        if !snap.files.is_empty() {
            task.files = snap.files;
        }
        if let Some(dir) = snap.output_dir {
            task.output_dir = Some(dir);
        }
    }
    task.leftover_paths()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.cursor_blink_wanted = self.url_input.focus_handle.is_focused(window);
        if self.cursor_blink_wanted {
            self.ensure_cursor_blink(cx);
        }
        if self.pending_confirm.is_some() && !self.confirm_focus.is_focused(window) {
            self.confirm_focus.focus(window);
        }

        let content = if self.view == View::Settings {
            render_settings_view(self, cx).into_any_element()
        } else {
            render_tasks_view(self, cx).into_any_element()
        };

        let mut main = div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(rgb(CARD))
            .flex()
            .flex_col()
            .child(render_url_bar(self, window, cx));

        if let Some(hint) = self.clip_hint.as_ref() {
            let lang = self.language;
            let n = extract_urls(hint).len();
            let hint_text = if n > 1 {
                tr_clipboard_hint_n(lang, n)
            } else {
                tr_clipboard_hint(lang).to_string()
            };
            main = main.child(
                div()
                    .flex_none()
                    .px_5()
                    .py_2()
                    .bg(rgb(PRIMARY_FIXED))
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(rgb(PRIMARY))
                            .child(hint_text),
                    )
                    .child(
                        crate::ui::components::small_button(
                            tr_btn_add(lang).into(),
                            PRIMARY,
                            ON_PRIMARY,
                            true,
                        )
                        .id("clip-yes")
                        .on_click(cx.listener(|this, _, _, cx| this.accept_clip_hint(cx))),
                    )
                    .child(
                        crate::ui::components::small_button("✕".into(), SURF_LOW, MUTED, true)
                            .id("clip-no")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clip_hint = None;
                                cx.notify();
                            })),
                    ),
            );
        }

        main = main.child(div().flex_1().min_h(px(0.)).flex().child(content));

        div()
            .size_full()
            .min_w(px(0.))
            .relative()
            .overflow_hidden()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .child(render_sidebar(self, cx))
                    .child(main),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .px_8()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .bg(rgb(BG))
                    .border_t_1()
                    .border_color(rgba(OUTLINE_VAR, 0.6))
                    .child(self.status.get(self.language).to_string()),
            )
            .when(self.pending_confirm.is_some(), |d| {
                d.child(render_confirm_overlay(self, cx))
            })
    }
}

fn render_confirm_overlay(root: &Root, cx: &mut Context<Root>) -> impl IntoElement {
    let lang = root.language;
    let (title, body, action, danger, names, extra) = match root.pending_confirm.as_ref() {
        Some(PendingConfirm::RemoveTask { filename, .. }) => (
            tr_confirm_delete_title(lang).to_string(),
            tr_confirm_delete_body(lang, &truncate(filename, 48)),
            tr_confirm_delete_action(lang).to_string(),
            true,
            Vec::new(),
            0usize,
        ),
        Some(PendingConfirm::ClearDone { count }) => (
            tr_confirm_clear_title(lang).to_string(),
            tr_confirm_clear_body(lang, *count),
            tr_confirm_clear_action(lang).to_string(),
            true,
            Vec::new(),
            0,
        ),
        Some(PendingConfirm::AddUrls { urls }) => {
            let (names, extra) = bulk_preview(urls.iter().map(|u| preview_name_for_url(u)));
            (
                tr_confirm_add_title(lang, urls.len()),
                tr_confirm_add_body(lang, urls.len()),
                tr_confirm_add_action(lang).to_string(),
                false,
                names,
                extra,
            )
        }
        Some(PendingConfirm::AddCaptures { jobs }) => {
            let (names, extra) = bulk_preview(jobs.iter().map(preview_name_for_job));
            (
                tr_confirm_add_title(lang, jobs.len()),
                tr_confirm_add_body(lang, jobs.len()),
                tr_confirm_add_action(lang).to_string(),
                false,
                names,
                extra,
            )
        }
        None => (
            String::new(),
            String::new(),
            String::new(),
            true,
            Vec::new(),
            0,
        ),
    };
    let fh = root.confirm_focus.clone();
    let ok_bg = if danger { ERROR } else { PRIMARY };

    div()
        .id("confirm-mask")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(TEXT, 0.32))
        .occlude()
        .track_focus(&fh)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if matches!(
                    this.pending_confirm,
                    Some(PendingConfirm::AddCaptures { .. })
                ) {
                    return;
                }
                this.dismiss_confirm(cx);
            }),
        )
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            match event.keystroke.key.as_str() {
                "escape" => this.dismiss_confirm(cx),
                "enter" | "return" => this.confirm_pending(cx),
                _ => {}
            }
        }))
        .child(
            div()
                .id("confirm-dialog")
                .w(px(440.))
                .rounded_xl()
                .bg(rgb(CARD))
                .border_1()
                .border_color(rgb(OUTLINE_VAR))
                .px_5()
                .py_4()
                .flex()
                .flex_col()
                .gap_3()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT))
                        .child(title),
                )
                .child(div().text_sm().text_color(rgb(MUTED)).child(body))
                .when(!names.is_empty(), |d| {
                    let mut list = div()
                        .id("confirm-add-list")
                        .w_full()
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(2.));
                    for name in names {
                        list = list.child(div().text_xs().text_color(rgb(TEXT)).child(name));
                    }
                    if extra > 0 {
                        list = list.child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(tr_confirm_add_more(lang, extra)),
                        );
                    }
                    d.child(list)
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            crate::ui::components::ghost_button(tr_btn_cancel(lang).into(), true)
                                .id("confirm-cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.dismiss_confirm(cx))),
                        )
                        .child(
                            crate::ui::components::small_button(action, ok_bg, ON_PRIMARY, true)
                                .id("confirm-ok")
                                .on_click(cx.listener(|this, _, _, cx| this.confirm_pending(cx))),
                        ),
                ),
        )
}

fn should_confirm_bulk(count: usize) -> bool {
    count >= BULK_ADD_THRESHOLD
}

fn collect_downloadable_urls(raw: &str) -> Vec<String> {
    extract_urls(raw)
        .into_iter()
        .filter(|u| detect_protocol(u) != Protocol::Unknown)
        .collect()
}

fn bulk_preview<I, S>(names: I) -> (Vec<String>, usize)
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let all: Vec<String> = names.into_iter().map(Into::into).collect();
    let extra = all.len().saturating_sub(BULK_PREVIEW_LIMIT);
    let shown = all.into_iter().take(BULK_PREVIEW_LIMIT).collect();
    (shown, extra)
}

fn preview_name_for_url(url: &str) -> String {
    let proto = detect_protocol(url);
    let raw = if proto == Protocol::Hls {
        crate::core::hls::default_hls_filename(url)
    } else if proto == Protocol::Magnet {
        filename_from_source(url)
    } else {
        filename_from_url(url)
    };
    truncate(&raw, 56)
}

fn preview_name_for_job(job: &CaptureJob) -> String {
    if let Some(name) = job.filename.as_ref() {
        let t = name.trim();
        if !t.is_empty() {
            return truncate(t, 56);
        }
    }
    preview_name_for_url(&job.url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_threshold_is_three() {
        assert!(!should_confirm_bulk(0));
        assert!(!should_confirm_bulk(1));
        assert!(!should_confirm_bulk(2));
        assert!(should_confirm_bulk(3));
        assert!(should_confirm_bulk(20));
    }

    #[test]
    fn collect_urls_skips_junk_lines() {
        let raw = "https://a.com/1.zip\nnot a url\nhttps://b.com/2.mp4\nftp://c.com/3.bin\n";
        let v = collect_downloadable_urls(raw);
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn bulk_preview_caps_list_and_counts_rest() {
        let names = (0..12).map(|i| format!("f{i}.bin"));
        let (shown, extra) = bulk_preview(names);
        assert_eq!(shown.len(), BULK_PREVIEW_LIMIT);
        assert_eq!(extra, 4);
        assert_eq!(shown[0], "f0.bin");
    }

    #[test]
    fn engine_pause_is_cancellable() {
        assert_eq!(status_after_engine_pause(true), TaskStatus::Paused);
        assert_eq!(status_after_engine_pause(false), TaskStatus::Queued);
    }
}
