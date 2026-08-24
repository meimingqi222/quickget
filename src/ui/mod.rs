//! GPUI 根视图。

pub mod components;
pub mod i18n;
pub mod state;
pub mod text_input;
pub mod theme;
pub mod views;

pub use state::*;

use crate::core::engine::run_task;
use crate::core::i18n::{bilingual, Language, Text};
use crate::core::model::{fmt_speed, Task, TaskStatus};
use crate::core::progress::{Control, JobOutcome, LiveMeta, LiveProgress};
use crate::core::settings::Settings;
use crate::core::urlx::{
    detect_protocol, extract_urls, filename_from_source, filename_from_url, is_bt_placeholder,
    unique_path, url_identity, Protocol,
};
use crate::ui::components::{render_sidebar, render_url_bar, View};
use crate::ui::i18n::*;
use crate::ui::theme::*;
use crate::ui::views::{render_settings_view, render_tasks_view};
use gpui::{
    div, prelude::*, px, rgb, Context, IntoElement, Render, ScrollHandle, Task as GpuiTask, Window,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        };
        crate::core::capture::write_pid();
        crate::core::capture::install_native_host();
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

    pub fn persist(&self) {
        crate::core::store::save(&self.tasks);
    }

    pub fn submit_url(&mut self, cx: &mut Context<Self>) {
        let raw = self.url_input.text.clone();
        self.add_urls(&raw, cx);
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
        let urls = extract_urls(raw);
        if urls.is_empty() {
            self.status = bilingual(|l| tr_status_bad_url(l));
            cx.notify();
            return;
        }
        let mut last_name = String::new();
        let mut last_already = false;
        for url in urls {
            let proto = detect_protocol(&url);
            if proto == Protocol::Unknown {
                continue;
            }
            if let Some((name, fresh)) = self.adopt_existing(&url, None, None, None) {
                last_name = name;
                last_already = !fresh;
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
                peers: Default::default(),
            };
            last_name = filename;
            last_already = false;
            self.tasks.insert(0, task);
        }
        if !last_name.is_empty() {
            self.status = if last_already {
                bilingual(|l| tr_status_already(l, &last_name))
            } else {
                bilingual(|l| tr_status_added(l, &last_name))
            };
        }
        self.persist();
        self.pump_queue(cx);
        cx.notify();
    }

    pub fn add_capture(&mut self, job: crate::core::capture::CaptureJob, cx: &mut Context<Self>) {
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
        if let Some((name, fresh)) =
            self.adopt_existing(&url, job.cookies.clone(), job.referer.clone(), job.ua.clone())
        {
            self.status = if fresh {
                bilingual(|l| tr_status_added(l, &name))
            } else {
                bilingual(|l| tr_status_already(l, &name))
            };
            self.persist();
            self.pump_queue(cx);
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
            status: TaskStatus::Queued,
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
            peers: Default::default(),
        };
        self.tasks.insert(0, task);
        self.status = bilingual(|l| tr_status_added(l, &filename));
        self.persist();
        self.pump_queue(cx);
        cx.notify();
    }

    /// 同一资源（去掉 sid 等 query）已在列表里：更新凭证，失败的重新排队。
    /// 返回 `(文件名, 是否重新入队)`。
    fn adopt_existing(
        &mut self,
        url: &str,
        cookies: Option<String>,
        referer: Option<String>,
        ua: Option<String>,
    ) -> Option<(String, bool)> {
        let key = url_identity(url);
        let idx = self.tasks.iter().position(|t| {
            t.status != TaskStatus::Completed && url_identity(&t.url) == key
        })?;
        let t = &mut self.tasks[idx];
        t.url = url.to_string();
        if cookies.as_ref().is_some_and(|s| !s.is_empty()) {
            t.cookies = cookies;
        }
        if referer.as_ref().is_some_and(|s| !s.is_empty()) {
            t.referer = referer;
        }
        if ua.as_ref().is_some_and(|s| !s.is_empty()) {
            t.user_agent = ua;
        }
        let name = t.filename.clone();
        match t.status {
            TaskStatus::Paused | TaskStatus::Failed | TaskStatus::Cancelled => {
                t.status = TaskStatus::Queued;
                t.error = None;
                t.finished_at = None;
                Some((name, true))
            }
            _ => Some((name, false)),
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
                for job in crate::core::capture::drain() {
                    this.add_capture(job, cx);
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

        let work = cx.background_executor().spawn(async move {
            run_task(&task, &ua, progress_bg, ctrl_bg, meta_bg)
        });
        let id_owned = id.to_string();
        let gpui_task = cx.spawn(async move |this, cx| {
            let outcome = work.await;
            this.update(cx, |this, cx| {
                this.apply_outcome(&id_owned, outcome);
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

    fn apply_outcome(&mut self, id: &str, outcome: JobOutcome) {
        let snap = self
            .runtime
            .get(id)
            .map(|s| s.meta.snapshot())
            .unwrap_or_default();
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
            }
            JobOutcome::Paused { downloaded, size } => {
                t.status = TaskStatus::Paused;
                t.downloaded = downloaded;
                if size > 0 {
                    t.size = size;
                }
                let name = t.filename.clone();
                self.status = bilingual(|l| tr_status_paused(l, &name));
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

    pub fn resume_task(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
            t.status = TaskStatus::Queued;
            t.error = None;
        }
        self.persist();
        self.pump_queue(cx);
        cx.notify();
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
                crate::core::io::trash_paths(&leftover_paths_for_remove(
                    task,
                    live_meta.as_ref(),
                ));
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
                    let n = this
                        .tasks
                        .iter()
                        .filter(|t| t.status.is_active())
                        .count();
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

        if let Some(_hint) = self.clip_hint.as_ref() {
            let lang = self.language;
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
                            .child(tr_clipboard_hint(lang)),
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
                        crate::ui::components::small_button(
                            "✕".into(),
                            SURF_LOW,
                            MUTED,
                            true,
                        )
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
    }
}
