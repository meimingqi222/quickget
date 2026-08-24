//! 任务模型与格式化工具。

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

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
    /// BT 种子里的文件。HTTP/FTP/HLS 通常为空，界面会用主文件名顶上。
    #[serde(default)]
    pub files: Vec<TaskFile>,
    /// BT 实际落盘目录。标题换成种子真名后 dest_path 仍指向这个文件夹。
    #[serde(default)]
    pub output_dir: Option<PathBuf>,
    /// 当前 Peer 快照。只给界面看，不写任务文件。
    #[serde(default, skip)]
    pub peers: BtPeers,
}

/// BT 节点汇总。Tracker / DHT / 入站都算进这里。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BtPeers {
    pub live: u32,
    pub live_tcp: u32,
    pub live_utp: u32,
    pub connecting: u32,
    pub seen: u32,
}

impl BtPeers {
    pub fn is_idle(self) -> bool {
        self.live == 0 && self.connecting == 0 && self.seen == 0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskFile {
    pub path: String,
    pub size: u64,
    #[serde(default)]
    pub downloaded: u64,
}

impl Task {
    pub fn dest_path(&self) -> PathBuf {
        if self.protocol == Protocol::Magnet {
            if let Some(dir) = &self.output_dir {
                if !dir.as_os_str().is_empty() {
                    return dir.clone();
                }
            }
        }
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
    /// BT 只收任务自己的文件夹，不扫下载根目录里的同名文件。
    pub fn leftover_paths(&self) -> Vec<PathBuf> {
        let dest = self.dest_path();
        let mut out = Vec::new();
        self.push_leftover(&mut out, dest.clone());
        if let Some(dir) = &self.output_dir {
            self.push_leftover(&mut out, dir.clone());
        }

        if self.protocol == Protocol::Magnet {
            return out;
        }

        self.push_leftover(&mut out, self.save_dir.join(&self.filename));

        self.push_leftover(&mut out, self.part_path());
        self.push_leftover(&mut out, self.meta_path());
        if !self.filename.to_ascii_lowercase().ends_with(".zip") {
            let zip = self.save_dir.join(format!("{}.zip", self.filename));
            let mut part = zip.clone().into_os_string();
            part.push(".part");
            let mut meta = zip.clone().into_os_string();
            meta.push(".qg.json");
            self.push_leftover(&mut out, zip);
            self.push_leftover(&mut out, PathBuf::from(part));
            self.push_leftover(&mut out, PathBuf::from(meta));
        }
        let mut hls = dest.as_os_str().to_os_string();
        hls.push(".hls-tmp");
        self.push_leftover(&mut out, PathBuf::from(hls));
        if let Some(stem) = dest.file_stem() {
            self.push_leftover(
                &mut out,
                self.save_dir.join(format!("{}.hls-tmp", stem.to_string_lossy())),
            );
        }
        out
    }

    fn push_leftover(&self, out: &mut Vec<PathBuf>, path: PathBuf) {
        if path.as_os_str().is_empty() {
            return;
        }
        if path == self.save_dir {
            return;
        }
        if !path.starts_with(&self.save_dir) {
            return;
        }
        if out.iter().any(|p| p == &path) {
            return;
        }
        out.push(path);
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

    /// 详情面板用的文件列表。BT 用种子清单；其它协议就是目标文件本身。
    pub fn display_files(&self) -> Vec<TaskFile> {
        if !self.files.is_empty() {
            return self.files.clone();
        }
        vec![TaskFile {
            path: self.filename.clone(),
            size: self.size,
            downloaded: self.downloaded,
        }]
    }

    pub fn file_disk_path(&self, file: &TaskFile) -> PathBuf {
        let Some(rel) = safe_relative(&file.path) else {
            return self.dest_path();
        };
        if self.protocol == Protocol::Magnet {
            let nested = self.dest_path().join(&rel);
            if nested.exists() {
                return nested;
            }
            let flat = self.save_dir.join(&rel);
            if flat.exists() {
                return flat;
            }
            nested
        } else if self.files.len() <= 1 {
            self.dest_path()
        } else {
            self.dest_path().join(rel)
        }
    }
}

/// 种子相对路径：拒绝绝对路径和 `..`，避免删到保存目录外面。
fn safe_relative(path: &str) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let p = Path::new(path);
    if p.is_absolute() {
        return None;
    }
    if p.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return None;
    }
    Some(p.to_path_buf())
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
            files: Vec::new(),
            output_dir: None,
            peers: BtPeers::default(),
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

    #[test]
    fn display_files_falls_back_to_filename() {
        let t = Task {
            id: "1".into(),
            url: "https://ex.com/a.bin".into(),
            filename: "a.bin".into(),
            save_dir: PathBuf::from("/tmp"),
            protocol: crate::core::urlx::Protocol::Http,
            status: TaskStatus::Downloading,
            size: 10,
            downloaded: 3,
            connections: 1,
            error: None,
            created_at: 0,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files: Vec::new(),
            output_dir: None,
            peers: BtPeers::default(),
        };
        let files = t.display_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a.bin");
        assert_eq!(files[0].size, 10);
        assert_eq!(files[0].downloaded, 3);
    }

    #[test]
    fn file_disk_path_joins_multi_file() {
        let t = Task {
            id: "1".into(),
            url: "magnet:?xt=urn:btih:abc".into(),
            filename: "Album".into(),
            save_dir: PathBuf::from("/tmp"),
            protocol: crate::core::urlx::Protocol::Magnet,
            status: TaskStatus::Downloading,
            size: 20,
            downloaded: 0,
            connections: 1,
            error: None,
            created_at: 0,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files: vec![
                TaskFile {
                    path: "a.flac".into(),
                    size: 10,
                    downloaded: 0,
                },
                TaskFile {
                    path: "b.flac".into(),
                    size: 10,
                    downloaded: 0,
                },
            ],
            output_dir: None,
            peers: BtPeers::default(),
        };
        let p = t.file_disk_path(&t.files[1]);
        assert!(p.ends_with("Album/b.flac") || p.ends_with(r"Album\b.flac"));
    }

    fn magnet_task(
        filename: &str,
        save_dir: &str,
        files: Vec<TaskFile>,
        output_dir: Option<PathBuf>,
    ) -> Task {
        Task {
            id: "1".into(),
            url: "magnet:?xt=urn:btih:abc".into(),
            filename: filename.into(),
            save_dir: PathBuf::from(save_dir),
            protocol: crate::core::urlx::Protocol::Magnet,
            status: TaskStatus::Completed,
            size: 20,
            downloaded: 20,
            connections: 1,
            error: None,
            created_at: 0,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files,
            output_dir,
            peers: BtPeers::default(),
        }
    }

    #[test]
    fn peers_idle_when_all_zero() {
        assert!(BtPeers::default().is_idle());
        assert!(!BtPeers {
            live: 1,
            ..BtPeers::default()
        }
        .is_idle());
    }

    #[test]
    fn dest_path_prefers_bt_output_dir() {
        let t = magnet_task(
            "Ubuntu",
            "/tmp",
            vec![],
            Some(PathBuf::from("/tmp/BT")),
        );
        assert_eq!(t.dest_path(), PathBuf::from("/tmp/BT"));
    }

    #[test]
    fn leftover_paths_include_bt_folder_and_files() {
        let t = magnet_task(
            "Ubuntu",
            "/tmp",
            vec![
                TaskFile {
                    path: "a.iso".into(),
                    size: 10,
                    downloaded: 10,
                },
                TaskFile {
                    path: "docs/readme.txt".into(),
                    size: 10,
                    downloaded: 10,
                },
            ],
            Some(PathBuf::from("/tmp/BT")),
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(s.iter().any(|p| p == "/tmp/BT"));
        assert!(!s.iter().any(|p| p == "/tmp/Ubuntu"));
        assert!(!s.iter().any(|p| p == "/tmp/a.iso"));
        assert!(!s.iter().any(|p| p == "/tmp/docs"));
        assert!(!s.iter().any(|p| p == "/tmp"));
        assert!(!s.iter().any(|p| p.ends_with(".zip")));
    }

    #[test]
    fn leftover_paths_do_not_trash_title_sibling() {
        let t = magnet_task(
            "Ubuntu",
            "/tmp",
            vec![TaskFile {
                path: "disk.iso".into(),
                size: 1,
                downloaded: 1,
            }],
            Some(PathBuf::from("/tmp/BT (1)")),
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(s.iter().any(|p| p == "/tmp/BT (1)"));
        assert!(!s.iter().any(|p| p == "/tmp/Ubuntu"));
        assert!(!s.iter().any(|p| p == "/tmp/disk.iso"));
    }

    #[test]
    fn leftover_paths_include_flattened_bt_files() {
        let t = magnet_task(
            "Album",
            "/tmp",
            vec![
                TaskFile {
                    path: "Album/a.flac".into(),
                    size: 10,
                    downloaded: 10,
                },
                TaskFile {
                    path: "Album/b.flac".into(),
                    size: 10,
                    downloaded: 10,
                },
            ],
            None,
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(s.iter().any(|p| p == "/tmp/Album"));
        assert!(!s.iter().any(|p| p == "/tmp/a.flac"));
        assert!(!s.iter().any(|p| p == "/tmp"));
    }

    #[test]
    fn leftover_ignores_loose_files_in_save_dir() {
        let root = std::env::temp_dir().join(format!("qg-left-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.iso"), b"x").unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let t = magnet_task(
            "Album",
            root.to_str().unwrap(),
            vec![
                TaskFile {
                    path: "a.iso".into(),
                    size: 1,
                    downloaded: 1,
                },
                TaskFile {
                    path: "docs/readme.txt".into(),
                    size: 1,
                    downloaded: 1,
                },
            ],
            None,
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        let a = root.join("a.iso").to_string_lossy().replace('\\', "/");
        let docs = root.join("docs").to_string_lossy().replace('\\', "/");
        let dest = root.join("Album").to_string_lossy().replace('\\', "/");
        assert!(s.iter().any(|p| p == &dest));
        assert!(!s.iter().any(|p| p == &a));
        assert!(!s.iter().any(|p| p == &docs));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn leftover_paths_reject_parent_dir_escape() {
        let t = magnet_task(
            "Album",
            "/tmp",
            vec![TaskFile {
                path: "../secret.txt".into(),
                size: 1,
                downloaded: 1,
            }],
            Some(PathBuf::from("/tmp/Album")),
        );
        let s: Vec<String> = t
            .leftover_paths()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(s.iter().all(|p| p.starts_with("/tmp/")));
        assert!(!s.iter().any(|p| p.contains("secret")));
    }
}
