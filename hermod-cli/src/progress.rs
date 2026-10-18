//! 进度条与空闲超时（indicatif）。

use indicatif::{ProgressBar, ProgressStyle};
use std::future::Future;
use std::time::Duration;
use tokio::sync::mpsc;

/// 已知总字节数的进度条；非 tty 或 quiet 时隐藏。
pub fn bar(total: u64, quiet: bool) -> ProgressBar {
    if quiet || !crate::tty::stderr_is_tty() {
        return ProgressBar::hidden();
    }
    let pb = ProgressBar::new(total);
    let style = ProgressStyle::with_template("{bar:40} {bytes}/{total_bytes} ({eta})")
        .map(|s| s.progress_chars("=>-"))
        .unwrap_or_else(|_| ProgressStyle::default_bar());
    pb.set_style(style);
    pb
}

/// 未知总字节数的滚动进度条（rx 在收到响应头前不知道大小）。
pub fn spinner(quiet: bool) -> ProgressBar {
    if quiet || !crate::tty::stderr_is_tty() {
        return ProgressBar::hidden();
    }
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(Duration::from_millis(120));
    pb
}

/// 在 `fut` 运行期间做**空闲超时**：每次 `ticks` 收到一个信号就重置计时；
/// 超过 `idle` 没有任何进展则返回 `Err("payload idle timeout")`。
pub async fn with_idle_timeout<F, T>(
    fut: F,
    mut ticks: mpsc::Receiver<()>,
    idle: Duration,
) -> Result<T, &'static str>
where
    F: Future<Output = T>,
{
    let mut fut = Box::pin(fut);
    let mut sleep = Box::pin(tokio::time::sleep(idle));
    loop {
        tokio::select! {
            r = &mut fut => return Ok(r),
            tick = ticks.recv() => {
                // 通道关闭（发送端 drop）后不再重置，让超时生效
                if tick.is_some() {
                    sleep.as_mut().reset(tokio::time::Instant::now() + idle);
                }
            }
            _ = &mut sleep => return Err("payload idle timeout"),
        }
    }
}
