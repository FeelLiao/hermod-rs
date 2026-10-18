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
    let style = ProgressStyle::with_template("{bar:40} {bytes}/{total_bytes} {bytes_per_sec} ({eta})")
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


/// 估算瞬时速率（字节/秒），用于滚动进度条（rx 不知道总大小）。
pub struct Speed {
    last_t: std::time::Instant,
    last_n: u64,
    rate: f64,
}

impl Speed {
    /// 新建。
    pub fn new() -> Self {
        Self {
            last_t: std::time::Instant::now(),
            last_n: 0,
            rate: 0.0,
        }
    }

    /// 用累计字节数更新，返回当前估算速率（至少 200ms 更新一次）。
    pub fn update(&mut self, n: u64) -> f64 {
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last_t).as_secs_f64();
        if dt >= 0.2 {
            self.rate = n.saturating_sub(self.last_n) as f64 / dt;
            self.last_t = now;
            self.last_n = n;
        }
        self.rate
    }
}

impl Default for Speed {
    fn default() -> Self {
        Self::new()
    }
}
