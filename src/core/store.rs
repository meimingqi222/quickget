//! 任务列表持久化。和设置一样：坏文件退回空列表。

use crate::core::model::Task;
use std::path::PathBuf;

const FILE_NAME: &str = "tasks.json";

pub fn load() -> Vec<Task> {
    let Some(path) = path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(tasks: &[Task]) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    let Ok(text) = serde_json::to_string_pretty(tasks) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &text).is_err() {
        return;
    }
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn path() -> Option<PathBuf> {
    crate::core::settings::Settings::dir().map(|d| d.join(FILE_NAME))
}
