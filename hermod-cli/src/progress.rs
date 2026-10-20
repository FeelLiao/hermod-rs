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
    let style =
        ProgressStyle::with_template("{bar:40} {bytes}/{total_bytes} {bytes_per_sec} ({eta})")
            .map(|s| s.progress_chars("=>-"))
            .unwrap_or_else(|_| ProgressStyle::default_bar());
    pb.set_style(style);
    pb
}

/// 发送端 QUIC 用的进度条：位置是本地写入字节，速度由网络统计消息覆盖。
pub fn bar_net(total: u64, quiet: bool) -> ProgressBar {
    if quiet || !crate::tty::stderr_is_tty() {
        return ProgressBar::hidden();
    }
    let pb = ProgressBar::new(total);
    let style = ProgressStyle::with_template("{bar:40} {bytes}/{total_bytes} {msg}")
        .map(|s| s.progress_chars("=>-"))
        .unwrap_or_else(|_| ProgressStyle::default_bar());
    pb.set_style(style);
    pb.set_message("0 B/s (--)");
    pb
}

/// 把字节/秒格式化成 `7.2 MiB/s` 这样的字符串。
pub fn format_rate(bytes_per_sec: f64) -> String {
    const K: f64 = 1024.0;
    if !bytes_per_sec.is_finite() || bytes_per_sec < 1.0 {
        "0 B/s".to_string()
    } else if bytes_per_sec < K {
        format!("{bytes_per_sec:.0} B/s")
    } else if bytes_per_sec < K * K {
        format!("{:.1} KiB/s", bytes_per_sec / K)
    } else if bytes_per_sec < K * K * K {
        format!("{:.1} MiB/s", bytes_per_sec / (K * K))
    } else {
        format!("{:.2} GiB/s", bytes_per_sec / (K * K * K))
    }
}

/// 把剩余秒数格式化成 `(5m)` 风格。
pub fn format_eta(seconds: f64) -> String {
    if !seconds.is_finite() || seconds <= 0.0 {
        return "--".to_string();
    }
    if seconds < 60.0 {
        format!("{}s", seconds.ceil() as u64)
    } else if seconds < 3600.0 {
        format!("{}m", (seconds / 60.0).ceil() as u64)
    } else {
        format!("{}h", (seconds / 3600.0).ceil() as u64)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rate_in_binary_units() {
        assert_eq!(format_rate(0.0), "0 B/s");
        assert_eq!(format_rate(512.0), "512 B/s");
        assert_eq!(format_rate(1536.0), "1.5 KiB/s");
        assert_eq!(format_rate(7.2 * 1024.0 * 1024.0), "7.2 MiB/s");
        assert_eq!(format_rate(2.0 * 1024.0 * 1024.0 * 1024.0), "2.00 GiB/s");
    }

    #[test]
    fn formats_eta() {
        assert_eq!(format_eta(0.0), "--");
        assert_eq!(format_eta(9.2), "10s");
        assert_eq!(format_eta(5.0 * 60.0), "5m");
        assert_eq!(format_eta(2.0 * 3600.0), "2h");
    }
}
