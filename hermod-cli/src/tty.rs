//! 终端检测（决定是否画进度条/颜色）。

use std::io::IsTerminal;

/// stderr 是否是终端（进度条画在 stderr）。
pub fn stderr_is_tty() -> bool {
    std::io::stderr().is_terminal()
}

/// stdin 是否被管道/重定向（判断 `-` 的语义）。
pub fn stdin_is_piped() -> bool {
    !std::io::stdin().is_terminal()
}
