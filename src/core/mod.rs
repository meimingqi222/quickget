//! 下载引擎与领域模型。这一层不依赖 GPUI。

pub mod bt;
pub mod capture;
pub mod engine;
pub mod ftp;
pub mod hls;
pub mod http;
pub mod http_api;
pub mod i18n;
pub mod io;
pub mod log;
pub mod model;
pub mod native_host;
pub mod probe;
pub mod providers;
pub mod progress;
pub mod settings;
pub mod store;
pub mod urlx;
pub mod verify;

pub use i18n::Language;
pub use model::{fmt_size, fmt_speed, Task, TaskStatus};
pub use settings::Settings;
pub use urlx::Protocol;
