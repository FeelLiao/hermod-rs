//! 极简日志钩子：core 负责打日志，输出目标由上层（CLI/GUI）决定。
//!
//! 用法：
//! ```ignore
//! hermod_core::log::set_logger(|level, msg| { /* stderr、文件、GUI … */ });
//! ```
//! 然后 core 内部用 [`log_error!`](crate::log_error!) 等宏打日志。
//! 级别与 CLI 的 `--verbose`（none/error/warning/info/debug）一一对应。

use std::sync::OnceLock;

/// 日志级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 0,
    Warning = 1,
    Info = 2,
    Debug = 3,
}

type LoggerFn = fn(Level, &str);
static LOGGER: OnceLock<LoggerFn> = OnceLock::new();

/// 安装全局 logger（进程内只生效一次；重复安装会被忽略）。
pub fn set_logger(f: LoggerFn) {
    let _ = LOGGER.set(f);
}

/// 把一条日志交给已安装的 logger（未安装则丢弃）。
pub fn emit(level: Level, msg: &str) {
    if let Some(f) = LOGGER.get() {
        f(level, msg);
    }
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::log::emit($crate::log::Level::Error, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::log::emit($crate::log::Level::Warning, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::log::emit($crate::log::Level::Info, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { $crate::log::emit($crate::log::Level::Debug, &format!($($arg)*)) };
}
