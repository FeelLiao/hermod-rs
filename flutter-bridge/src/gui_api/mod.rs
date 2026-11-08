//! 面向 Flutter GUI 的稳定门面。
//!
//! GUI 只应依赖本模块，不直接操作会话、传输实现或应用私有路径。
mod controller;
pub mod error;
pub mod models;

pub use controller::GuiController;
pub use error::{GuiError, GuiErrorCode};
pub use models::*;
// Flutter 桥接所需的稳定 DTO；不导出 Engine 或私有存储模块。
pub use crate::app::{Bootstrap, Selection, Settings, Status};
