//! 用户设置。读不出来就退回默认值，绝不打断启动。

use crate::core::i18n::Language;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DIR_NAME: &str = "QuickGet";
const FILE_NAME: &str = "settings.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    /// 默认保存目录。空则用系统下载文件夹。
    pub save_dir: PathBuf,
    /// 单个任务的最大连接数。
    pub connections: u32,
    /// 同时进行的任务数。
    pub max_concurrent: u32,
    pub user_agent: String,
    /// 剪贴板出现新链接时是否提示。
    pub watch_clipboard: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: crate::platform::detect_system_language(),
            save_dir: default_download_dir(),
            connections: 16,
            max_concurrent: 3,
            user_agent: default_ua().into(),
            watch_clipboard: true,
        }
    }
}

pub fn default_ua() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
    }
    #[cfg(target_os = "windows")]
    {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
    }
}

pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::merge_json(&text)
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                return;
            }
        }
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, &text).is_err() {
            return;
        }
        if std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&path);
            if std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::remove_file(&tmp);
                let _ = std::fs::write(&path, &text);
            }
        }
    }

    pub fn merge_json(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn path() -> Option<PathBuf> {
        Some(Self::dir()?.join(FILE_NAME))
    }

    pub fn dir() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(DIR_NAME))
    }

    pub fn connections_clamped(&self) -> u32 {
        self.connections.clamp(1, 64)
    }

    pub fn max_concurrent_clamped(&self) -> u32 {
        self.max_concurrent.clamp(1, 16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_file_falls_back() {
        for junk in ["", "{", "not json", "[1]"] {
            let s = Settings::merge_json(junk);
            assert_eq!(s, Settings::default(), "{junk:?}");
        }
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let s = Settings::merge_json(r#"{"language":"En","future":1}"#);
        assert_eq!(s.language, Language::En);
    }

    #[test]
    fn connections_round_trip() {
        let s = Settings::merge_json(r#"{"language":"Zh","connections":32,"max_concurrent":5}"#);
        assert_eq!(s.connections, 32);
        assert_eq!(s.max_concurrent, 5);
    }
}
