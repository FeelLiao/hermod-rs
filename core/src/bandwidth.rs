//! 应用层平滑限速：在写入 QUIC/TCP 前等待，不把整文件堆进传输窗口。
//! 单位为十进制 Mbps；0 表示不限速。收发两端声明的限制取较小值。

use std::time::Duration;

pub const RECEIVE_LIMIT_HEADER: &str = "X-Hermod-Receive-Limit";

/// 文件载荷速率上限，不包含加密、重传和协议开销。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BandwidthLimit(u64);

impl BandwidthLimit {
    pub const UNLIMITED: Self = Self(0);

    /// 从 Mbps 换算为字节/秒；避免 NaN、负数和浮点溢出。
    pub fn from_mbps(mbps: f64) -> Result<Self, &'static str> {
        if !mbps.is_finite() || !(0.0..=1_000_000.0).contains(&mbps) || (mbps > 0.0 && mbps < 0.001)
        {
            return Err("limit must be 0 (unlimited) or 0.001..=1000000 Mbps");
        }
        Ok(Self((mbps * 125_000.0).round() as u64))
    }

    pub fn from_bytes_per_second(bytes: u64) -> Self {
        Self(bytes)
    }
    pub fn bytes_per_second(self) -> u64 {
        self.0
    }
    pub fn mbps(self) -> f64 {
        self.0 as f64 / 125_000.0
    }

    /// 0 作为无穷大处理，而非暂停。
    pub fn stricter(self, peer: Self) -> Self {
        match (self.0, peer.0) {
            (0, _) => peer,
            (_, 0) => self,
            (a, b) => Self(a.min(b)),
        }
    }

    /// 限速时将 QUIC 应用队列控制在约 250ms；不限速仍保留大窗口。
    /// 注意：上限参数可能小，必须先抬下限再钳上限，避免 clamp(min > max) panic。
    pub(crate) fn window(self, unlimited: u64) -> u64 {
        if self.0 == 0 {
            unlimited
        } else {
            (self.0 / 4).max(256 * 1024).min(unlimited)
        }
    }
}

impl std::str::FromStr for BandwidthLimit {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_mbps(value.parse().map_err(|_| "expected a number in Mbps")?)
    }
}

impl std::fmt::Display for BandwidthLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.mbps().fmt(f)
    }
}

/// 每块不超过 20ms 的预算；磁盘停顿后不积攒信用，不以突发流量追赶。
pub(crate) struct Pacer(BandwidthLimit);

impl Pacer {
    pub(crate) fn new(limit: BandwidthLimit) -> Self {
        Self(limit)
    }
    pub(crate) fn chunk_len(&self) -> usize {
        if self.0.0 == 0 {
            64 * 1024
        } else {
            (self.0.0 / 50).clamp(1, 64 * 1024) as usize
        }
    }
    pub(crate) async fn wait(&self, bytes: usize) {
        if self.0.0 != 0 {
            tokio::time::sleep(Duration::from_secs_f64(bytes as f64 / self.0.0 as f64)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_validate_units_and_choose_the_lower_peer_budget() {
        let limit: BandwidthLimit = "8".parse().unwrap();
        assert_eq!(limit.bytes_per_second(), 1_000_000);
        assert_eq!(limit.stricter(BandwidthLimit::UNLIMITED), limit);
        assert_eq!(BandwidthLimit::UNLIMITED.stricter(limit), limit);
        assert_eq!(limit.stricter("2".parse().unwrap()).mbps(), 2.0);
        for input in ["-1", "NaN", "inf", "0.00001", "1000001"] {
            assert!(input.parse::<BandwidthLimit>().is_err());
        }
    }

    #[tokio::test]
    async fn pacing_has_a_bounded_burst_and_does_not_bank_idle_time() {
        let pacer = Pacer::new("0.8".parse().unwrap());
        assert_eq!(pacer.chunk_len(), 2_000);
        tokio::time::sleep(Duration::from_millis(30)).await;
        let start = tokio::time::Instant::now();
        for _ in 0..5 {
            pacer.wait(pacer.chunk_len()).await;
        }
        assert!(start.elapsed() >= Duration::from_millis(100));
    }

    /// 队列窗口：0 = 完全跟随调用方给的 unlimited；受限时先抬下限（256 KiB）
    /// 再钳上限，即使上限比下限小也不得 panic（clamp 语义陷阱）。
    #[test]
    fn window_clamps_between_floor_and_caller_ceiling() {
        let slow: BandwidthLimit = "0.002".parse().unwrap(); // 256 B/s
        let fast: BandwidthLimit = "8".parse().unwrap(); // 1 MiB/s
        let huge: BandwidthLimit = "1000".parse().unwrap(); // 125 MiB/s
        assert_eq!(BandwidthLimit::UNLIMITED.window(64 * 1024), 64 * 1024);
        assert_eq!(slow.window(1 << 30), 256 * 1024);
        assert_eq!(fast.window(1 << 30), 256 * 1024);
        assert_eq!(huge.window(1 << 30), 125_000_000 / 4);
        // 上限参数小于下限：必须返回上限而不是 panic
        assert_eq!(slow.window(100_000), 100_000);
        // 速度低于下限衍生值（250 KiB < 256 KiB）时先抬下限，再满足不超上限
        assert_eq!(fast.window(300_000), 256 * 1024);
    }
}
