//! 日志级别与输出（`--verbose` / `--quiet`），带 tty 感知的颜色。

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// 日志级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    None = 0,
    Error = 1,
    Warning = 2,
    Info = 3,
    Debug = 4,
}

static LEVEL: AtomicU8 = AtomicU8::new(0);
static QUIET: AtomicBool = AtomicBool::new(false);

/// 由 `main` 在启动时设置一次。
pub fn set(level: Level, quiet: bool) {
    LEVEL.store(level as u8, Ordering::Relaxed);
    QUIET.store(quiet, Ordering::Relaxed);
}

/// `--quiet` 是否开启（状态输出被抑制，错误仍显示）。
pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

fn enabled(level: Level) -> bool {
    LEVEL.load(Ordering::Relaxed) >= level as u8
}

/// 仅当 stderr 是终端时上色。
fn paint(s: &str, color: console::Color) -> String {
    if crate::tty::stderr_is_tty() {
        console::style(s).fg(color).to_string()
    } else {
        s.to_string()
    }
}

/// 调试信息。
pub fn debug(msg: impl AsRef<str>) {
    if enabled(Level::Debug) {
        eprintln!("{}", msg.as_ref());
    }
}

/// 普通信息（`--quiet` 时抑制）。
pub fn info(msg: impl AsRef<str>) {
    if enabled(Level::Info) && !is_quiet() {
        eprintln!("{}", msg.as_ref());
    }
}

/// 成功信息（绿色，`--quiet` 时抑制）。
pub fn success(msg: impl AsRef<str>) {
    if enabled(Level::Info) && !is_quiet() {
        eprintln!("{}", paint(msg.as_ref(), console::Color::Green));
    }
}

/// 警告（`--quiet` 时抑制）。
pub fn warn(msg: impl AsRef<str>) {
    if enabled(Level::Warning) && !is_quiet() {
        eprintln!("{}", paint(&format!("warning: {}", msg.as_ref()), console::Color::Yellow));
    }
}

/// 错误（**永不被抑制**，红色）。
pub fn error(msg: impl AsRef<str>) {
    eprintln!("{}", paint(&format!("error: {}", msg.as_ref()), console::Color::Red));
}
