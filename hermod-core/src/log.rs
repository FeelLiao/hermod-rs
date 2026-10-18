//! 极简日志钩子：core 负责打日志，输出目标由上层（CLI/GUI）决定。
//!
//! 用法：
//! ```ignore
//! hermod_core::log::set_logger(|level, msg| { /* stderr、文件、GUI … */ });
//! ```
//! 然后 core 内部用 [`log_error!`](crate::log_error!) 等宏打日志。
//! 级别与 CLI 的 `--verbose`（none/error/warning/info/debug）一一对应。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

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
/// 最高允许级别（默认只放 Error，避免热路径上的 `format!` 开销）。
static MAX_LEVEL: AtomicU8 = AtomicU8::new(Level::Error as u8);

/// 安装全局 logger（进程内只生效一次；重复安装会被忽略）。
pub fn set_logger(f: LoggerFn) {
    let _ = LOGGER.set(f);
}

/// 设置最高输出级别：低于它的日志连 `format!` 都不会执行。
pub fn set_max_level(level: Level) {
    MAX_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// 该级别是否会被输出（宏用它避免无谓的 `format!`）。
pub fn enabled(level: Level) -> bool {
    (level as u8) <= MAX_LEVEL.load(Ordering::Relaxed)
}

/// 把一条日志交给已安装的 logger（未安装则丢弃）。
pub fn emit(level: Level, msg: &str) {
    if enabled(level)
        && let Some(f) = LOGGER.get()
    {
        f(level, msg);
    }
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Error) {
            $crate::log::emit($crate::log::Level::Error, &format!($($arg)*))
        }
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Warning) {
            $crate::log::emit($crate::log::Level::Warning, &format!($($arg)*))
        }
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Info) {
            $crate::log::emit($crate::log::Level::Info, &format!($($arg)*))
        }
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        if $crate::log::enabled($crate::log::Level::Debug) {
            $crate::log::emit($crate::log::Level::Debug, &format!($($arg)*))
        }
    };
}
