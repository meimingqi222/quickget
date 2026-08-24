//! 下载引擎与领域模型。这一层不依赖 GPUI。

pub mod bt;
pub mod capture;
pub mod engine;
pub mod ftp;
pub mod hls;
pub mod http;
pub mod native_host;
pub mod i18n;
pub mod io;
pub mod log;
pub mod model;
pub mod probe;
pub mod progress;
pub mod settings;
pub mod store;
pub mod urlx;
pub mod verify;

pub use i18n::Language;
pub use model::{fmt_size, fmt_speed, Task, TaskStatus};
pub use urlx::Protocol;
pub use settings::Settings;
