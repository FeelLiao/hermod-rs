//! Flutter 桥接 crate：GUI 应用层（app）、稳定门面（gui_api）与
//! flutter_rust_bridge 生成胶水（frb_generated / api）的唯一归属地。
//!
//! [`hermod_core`] 只保留协议、传输与编排等 CLI/GUI 共用的底层能力；
//! 本 crate 编译为 cdylib（`hermod_flutter_bridge.dll`），经 FFI 直接被
//! Flutter 加载，不再使用独立的 JSON 行协议子进程。

#[cfg(windows)]
mod windows;

pub mod api;
pub mod app;
mod frb_generated;
pub mod gui_api; /* flutter_rust_bridge codegen placeholder marker */
