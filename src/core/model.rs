//! 任务模型与格式化工具。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::core::urlx::Protocol;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Queued,
    Probing,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn is_active(self) -> bool {
        matches!(self, TaskStatus::Probing | TaskStatus::Downloading)
    }

    pub fn is_open(self) -> bool {
        matches!(
            self,
            TaskStatus::Queued | TaskStatus::Probing | TaskStatus::Downloading | TaskStatus::Paused
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub save_dir: PathBuf,
    pub protocol: Protocol,
    pub status: TaskStatus,
    pub size: u64,
    pub downloaded: u64,
    pub connections: u32,
    pub error: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub referer: Option<String>,
    /// 浏览器扩展带过来的 Cookie 头，登录态下载靠它。
    #[serde(default)]
    pub cookies: Option<String>,
    /// 覆盖设置里的 UA。扩展会带上浏览器真实 UA。
    #[serde(default)]
    pub user_agent: Option<String>,
}

impl Task {
    pub fn dest_path(&self) -> PathBuf {
        self.save_dir.join(&self.filename)
    }

    pub fn part_path(&self) -> PathBuf {
        let mut p = self.dest_path().into_os_string();
        p.push(".part");
        PathBuf::from(p)
    }

    pub fn meta_path(&self) -> PathBuf {
        let mut p = self.dest_path().into_os_string();
        p.push(".qg.json");
        PathBuf::from(p)
    }

    /// 删除任务时送进废纸篓的路径：成品、`.part`、`.qg.json`、HLS 临时目录。
    pub fn leftover_paths(&self) -> Vec<PathBuf> {
        let dest = self.dest_path();
        let mut out = vec![dest.clone(), self.part_path(), self.meta_path()];
        if !self.filename.to_ascii_lowercase().ends_with(".zip") {
            let zip = self.save_dir.join(format!("{}.zip", self.filename));
            let mut part = zip.clone().into_os_string();
            part.push(".part");
            let mut meta = zip.clone().into_os_string();
            meta.push(".qg.json");
            out.push(zip);
            out.push(PathBuf::from(part));
            out.push(PathBuf::from(meta));
        }
        let mut hls = dest.as_os_str().to_os_string();
        hls.push(".hls-tmp");
        out.push(PathBuf::from(hls));
        if let Some(stem) = dest.file_stem() {
            out.push(self.save_dir.join(format!("{}.hls-tmp", stem.to_string_lossy())));
        }
        out
    }

    pub fn fraction(&self) -> f32 {
        if self.size == 0 {
            if self.status == TaskStatus::Completed {
                1.0
            } else {
                0.0
            }
        } else {
            (self.downloaded as f64 / self.size as f64).clamp(0.0, 1.0) as f32
        }
    }
}

/// 格式化字节大小。macOS 用 1000 进制，Windows 用 1024。
pub fn fmt_size(bytes: u64) -> String {
    const BASE: f64 = if cfg!(windows) { 1024.0 } else { 1000.0 };
    const KB: f64 = BASE;
    const MB: f64 = KB * BASE;
    const GB: f64 = MB * BASE;
    const TB: f64 = GB * BASE;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.2} TB", b / TB)
    } else if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn fmt_speed(bps: u64) -> String {
    format!("{}/s", fmt_size(bps))
}

pub fn fmt_eta(remaining: u64, bps: u64) -> String {
    if bps == 0 {
        return "--".into();
    }
    let secs = remaining / bps;
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}

/// 根据文件大小建议连接数：小文件少开，大文件吃满上限。
pub fn suggested_connections(size: u64, max: u32) -> u32 {
    let max = max.clamp(1, 64);
    let want = if size == 0 {
        max.min(8)
    } else if size < 1_000_000 {
        1
    } else if size < 8_000_000 {
        4
    } else if size < 50_000_000 {
        8
    } else {
        max
    };
    want.min(max).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("短名", 22), "短名");
        assert_eq!(truncate("一二三四", 2), "一二…");
    }

    #[test]
    fn suggested_connections_scales() {
        assert_eq!(suggested_connections(100, 16), 1);
        assert_eq!(suggested_connections(3_000_000, 16), 4);
        assert_eq!(suggested_connections(20_000_000, 16), 8);
        assert_eq!(suggested_connections(200_000_000, 16), 16);
        assert_eq!(suggested_connections(200_000_000, 4), 4);
    }

    #[test]
    fn leftover_paths_include_zip_sidecars() {
        let t = Task {
            id: "1".into(),
            url: "https://ex.com/a.SAFE".into(),
            filename: "a.SAFE".into(),
            save_dir: PathBuf::from("/tmp"),
            protocol: crate::core::urlx::Protocol::Http,
            status: TaskStatus::Failed,
            size: 1,
            downloaded: 0,
            connections: 1,
            error: None,
            created_at: 0,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
        };
        let paths = t.leftover_paths();
        let s: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert!(s.iter().any(|p| p.ends_with("a.SAFE")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.part")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.qg.json")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.zip")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.zip.part")));
        assert!(s.iter().any(|p| p.ends_with("a.SAFE.zip.qg.json")));
    }
}
