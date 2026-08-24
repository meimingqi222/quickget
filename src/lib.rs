//! QuickGet 核心库
//!
//! 三层结构，依赖方向严格自上而下：
//!
//! - [`ui`] —— 基于 GPUI 的视图与控件。只依赖 `core` 的领域类型和
//!   `platform` 的门面函数。
//! - [`core`] —— 领域模型与下载引擎。不知道 UI 的存在。
//! - [`platform`] —— 操作系统适配层。

pub mod core;
pub mod platform;
pub mod ui;
