//! CLI 与 GUI 共用的连接选项；按实际协议校验，禁止静默忽略。
use crate::{session::SessionOptions, transport::TransportKind};
use serde::{Deserialize, Serialize};
use std::{fmt, net::IpAddr, str::FromStr, time::Duration};

/// 本次运行的地址族约束；显式限制不得跨族回退。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IpFamily {
    #[default]
    Auto,
    V4,
    V6,
}
impl IpFamily {
    /// IPv4 映射地址按原始 IPv4 家族处理。
    pub fn allows(self, ip: IpAddr) -> bool {
        let ipv4 = match ip {
            IpAddr::V4(_) => true,
            IpAddr::V6(ip) => ip.to_ipv4_mapped().is_some(),
        };
        match self {
            Self::Auto => true,
            Self::V4 => ipv4,
            Self::V6 => !ipv4,
        }
    }
}

/// 非持久化的本端传输预算；CLI 和会话入口在联网前统一校验。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferTuning {
    pub streams: Option<usize>,
    pub chunk: usize,
    pub window: u64,
    pub udp_buffer: usize,
    pub slice_size: u64,
    pub prefetch: usize,
    pub keepalive: Duration,
    pub idle_timeout: Duration,
}
impl Default for TransferTuning {
    fn default() -> Self {
        Self {
            streams: None,
            chunk: 1024 * 1024,
            window: 64 * 1024 * 1024,
            udp_buffer: 16 * 1024 * 1024,
            slice_size: 4 * 1024 * 1024,
            prefetch: 16,
            keepalive: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(30),
        }
    }
}
impl TransferTuning {
    /// 预算最大为 8 条流、2 GiB 连接窗和 256 MiB 预读队列，不按输入截断。
    pub fn validate(&self) -> Result<(), String> {
        if self.streams.is_some_and(|n| !(1..=8).contains(&n)) {
            return Err("streams must be auto or 1..=8".into());
        }
        for (name, value, min, max) in [
            ("chunk", self.chunk as u64, 256 * 1024, 4 * 1024 * 1024),
            ("window", self.window, 8 * 1024 * 1024, 256 * 1024 * 1024),
            (
                "udp-buffer",
                self.udp_buffer as u64,
                1024 * 1024,
                64 * 1024 * 1024,
            ),
            (
                "slice-size",
                self.slice_size,
                4 * 1024 * 1024,
                64 * 1024 * 1024,
            ),
            ("prefetch", self.prefetch as u64, 4, 64),
        ] {
            if !(min..=max).contains(&value) {
                return Err(format!(
                    "{name} must be {min}..={max}{}",
                    if name == "prefetch" { "" } else { " bytes" }
                ));
            }
        }
        if self.keepalive > Duration::from_secs(60) {
            return Err("keepalive must be 0..=60 seconds".into());
        }
        if !(Duration::from_secs(10)..=Duration::from_secs(300)).contains(&self.idle_timeout) {
            return Err("idle-timeout must be 10..=300 seconds".into());
        }
        if self.idle_timeout <= self.keepalive {
            return Err("idle-timeout must be greater than keepalive".into());
        }
        Ok(())
    }

    /// 连接总窗口按允许的八条数据流预留，避免两端并发偏好不同导致流控饥饿。
    pub(crate) fn connection_window(&self) -> u64 {
        self.window * 8
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Mtu {
    #[default]
    Auto,
    Value(u16),
}
impl FromStr for Mtu {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        if value.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        let value: u16 = value
            .parse()
            .map_err(|_| "MTU must be auto or an integer")?;
        if !(576..=65488).contains(&value) {
            return Err("MTU must be between 576 and 65488 bytes".into());
        }
        Ok(Self::Value(value))
    }
}
impl TryFrom<String> for Mtu {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        value.parse()
    }
}
impl From<Mtu> for String {
    fn from(value: Mtu) -> Self {
        value.to_string()
    }
}
impl fmt::Display for Mtu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Value(v) => v.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Congestion {
    #[default]
    Auto,
    Bbr,
    Cubic,
    Reno,
    Newreno,
}

/// 链路档位：决定数据面并发（单流/多流切片）与拥塞启动参数。
///
/// - `Auto`：按实际对端地址判断；有有效 RTT 时还须不超过 25 ms；
/// - `Lan`：强制局域网档（多流并行 + 更低的初始 RTT）；
/// - `Wan`：强制广域网档（默认单流大块流水线）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkProfile {
    #[default]
    Auto,
    Lan,
    Wan,
}
impl LinkProfile {
    /// 字符串值："auto" | "lan" | "wan"。
    pub fn as_str(self) -> &'static str {
        match self {
            LinkProfile::Auto => "auto",
            LinkProfile::Lan => "lan",
            LinkProfile::Wan => "wan",
        }
    }
}
impl FromStr for LinkProfile {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "lan" => Ok(Self::Lan),
            "wan" => Ok(Self::Wan),
            _ => Err("Profile must be auto, lan or wan".into()),
        }
    }
}
impl TryFrom<String> for LinkProfile {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        value.parse()
    }
}
impl From<LinkProfile> for String {
    fn from(value: LinkProfile) -> Self {
        value.as_str().into()
    }
}
impl fmt::Display for LinkProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl FromStr for Congestion {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "bbr" => Ok(Self::Bbr),
            "cubic" => Ok(Self::Cubic),
            "reno" => Ok(Self::Reno),
            "newreno" => Ok(Self::Newreno),
            _ => Err("Congestion must be auto, cubic, reno, bbr or newreno".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionSettings {
    pub transport: String,
    pub mtu: Mtu,
    pub congestion: Congestion,
    /// 链路档位（数据面并发与拥塞启动参数）。
    pub profile: LinkProfile,
}
impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            transport: "quic".into(),
            mtu: Mtu::Auto,
            congestion: Congestion::Auto,
            profile: LinkProfile::Auto,
        }
    }
}
impl ConnectionSettings {
    pub fn session_options(&self, receiving: bool) -> Result<SessionOptions, String> {
        let transport = if self.transport == "auto" && receiving {
            None
        } else {
            Some(TransportKind::parse(&self.transport).ok_or(
                "Choose quic (auto is only available when receiving; \
                 the WireGuard transport was removed — ask the peer to upgrade)",
            )?)
        };
        let options = SessionOptions {
            transport,
            mtu: match self.mtu {
                Mtu::Auto => None,
                Mtu::Value(v) => Some(usize::from(v)),
            },
            congestion: self.congestion,
            profile: self.profile,
            ..Default::default()
        };
        options.validate_for(transport.unwrap_or(TransportKind::Quic))?;
        Ok(options)
    }
}
impl SessionOptions {
    pub fn validate_for(&self, kind: TransportKind) -> Result<(), String> {
        self.tuning.validate()?;
        if self
            .udp_bind
            .is_some_and(|addr| !self.family.allows(addr.ip()))
        {
            return Err("UDP bind address conflicts with the selected IP family".into());
        }
        let valid_mtu = match kind {
            TransportKind::Quic => 1200..=1452,
        };
        if self.mtu.is_some_and(|v| !valid_mtu.contains(&v)) {
            return Err(format!(
                "MTU for {kind} must be {}..={} bytes",
                valid_mtu.start(),
                valid_mtu.end()
            ));
        }
        let supported = match kind {
            TransportKind::Quic => matches!(
                self.congestion,
                Congestion::Auto | Congestion::Bbr | Congestion::Cubic | Congestion::Newreno
            ),
        };
        if !supported {
            return Err(format!(
                "Congestion {:?} is not supported by {kind}; select auto or a supported algorithm",
                self.congestion
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tuning_rejects_invalid_budgets_and_accepts_boundaries() {
        let defaults = TransferTuning::default();
        assert!(defaults.validate().is_ok());
        assert_eq!(defaults.connection_window(), 512 * 1024 * 1024);
        for streams in [0, 9, usize::MAX] {
            assert!(
                TransferTuning {
                    streams: Some(streams),
                    ..defaults
                }
                .validate()
                .is_err()
            );
        }
        for chunk in [0, 256 * 1024 - 1, 4 * 1024 * 1024 + 1] {
            assert!(TransferTuning { chunk, ..defaults }.validate().is_err());
        }
        for window in [0, 8 * 1024 * 1024 - 1, 256 * 1024 * 1024 + 1] {
            assert!(TransferTuning { window, ..defaults }.validate().is_err());
        }
        for udp_buffer in [0, 1024 * 1024 - 1, 64 * 1024 * 1024 + 1] {
            assert!(
                TransferTuning {
                    udp_buffer,
                    ..defaults
                }
                .validate()
                .is_err()
            );
        }
        for slice_size in [0, 4 * 1024 * 1024 - 1, 64 * 1024 * 1024 + 1] {
            assert!(
                TransferTuning {
                    slice_size,
                    ..defaults
                }
                .validate()
                .is_err()
            );
        }
        for prefetch in [0, 3, 65] {
            assert!(
                TransferTuning {
                    prefetch,
                    ..defaults
                }
                .validate()
                .is_err()
            );
        }
        for idle_timeout in [
            Duration::ZERO,
            Duration::from_secs(9),
            Duration::from_secs(301),
        ] {
            assert!(
                TransferTuning {
                    idle_timeout,
                    ..defaults
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            TransferTuning {
                keepalive: Duration::from_secs(61),
                ..defaults
            }
            .validate()
            .is_err()
        );
        assert!(
            TransferTuning {
                keepalive: Duration::from_secs(30),
                ..defaults
            }
            .validate()
            .is_err()
        );
        assert!(
            TransferTuning {
                keepalive: Duration::ZERO,
                idle_timeout: Duration::from_secs(10),
                ..defaults
            }
            .validate()
            .is_ok()
        );
        assert!(
            TransferTuning {
                streams: Some(8),
                chunk: 4 * 1024 * 1024,
                window: 256 * 1024 * 1024,
                udp_buffer: 64 * 1024 * 1024,
                slice_size: 64 * 1024 * 1024,
                prefetch: 64,
                keepalive: Duration::from_secs(60),
                idle_timeout: Duration::from_secs(300)
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn clean_profiles_and_family_constraints() {
        assert_eq!("WaN".parse::<LinkProfile>().unwrap(), LinkProfile::Wan);
        assert!("compatible".parse::<LinkProfile>().is_err());
        assert_eq!(serde_json::to_string(&LinkProfile::Wan).unwrap(), "\"wan\"");
        assert_eq!("BBR".parse::<Congestion>().unwrap(), Congestion::Bbr);
        assert_eq!("AUTO".parse::<Mtu>().unwrap(), Mtu::Auto);
        assert!(IpFamily::V4.allows("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!IpFamily::V6.allows("::ffff:127.0.0.1".parse().unwrap()));
        let options = SessionOptions {
            family: IpFamily::V6,
            udp_bind: Some("127.0.0.1:0".parse().unwrap()),
            ..Default::default()
        };
        assert!(options.validate_for(TransportKind::Quic).is_err());
    }

    #[test]
    fn settings_validate_protocol_semantics_and_receiver_auto() {
        let mut settings = ConnectionSettings::default();
        assert_eq!(
            settings.session_options(false).unwrap().transport,
            Some(TransportKind::Quic)
        );
        // 发送端不接受 auto
        settings.transport = "auto".into();
        assert!(settings.session_options(false).is_err());
        // 接收端 auto 有效
        assert!(settings.session_options(true).is_ok());
        // 已移除的 wireguard → 明确拒绝（提示升级对端或改设置）
        settings.transport = "wireguard".into();
        let error = settings.session_options(true).unwrap_err();
        assert!(error.contains("removed") || error.contains("quic"));
        // QUIC 的 MTU/拥塞合法性仍然按协议校验
        settings.transport = "quic".into();
        settings.congestion = Congestion::Reno;
        assert!(settings.session_options(false).is_err());
        settings.congestion = Congestion::Bbr;
        assert!(settings.session_options(false).is_ok());
        // 链路档位随设置进入会话选项
        settings.profile = LinkProfile::Lan;
        assert_eq!(
            settings.session_options(false).unwrap().profile,
            LinkProfile::Lan
        );
        settings.mtu = Mtu::Value(1100);
        assert!(settings.session_options(false).is_err());
        for value in ["0", "-1", "65535", "NaN"] {
            assert!(value.parse::<Mtu>().is_err());
        }
    }
}
