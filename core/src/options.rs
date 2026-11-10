//! CLI 与 GUI 共用的连接选项；按实际协议校验，禁止静默忽略。
use crate::{session::SessionOptions, transport::TransportKind};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

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
        if value == "auto" {
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
/// - `Auto`：按探测到的 RTT 在线选择（局域网用激进参数，弱网保守）；
/// - `Lan`：强制局域网档（多流并行 + 更低的初始 RTT）；
/// - `Compatible`：强制兼容档（单流大块流水线，最保守）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkProfile {
    #[default]
    Auto,
    Lan,
    Compatible,
}
impl LinkProfile {
    /// 字符串值："auto" | "lan" | "compatible"。
    pub fn as_str(self) -> &'static str {
        match self {
            LinkProfile::Auto => "auto",
            LinkProfile::Lan => "lan",
            LinkProfile::Compatible => "compatible",
        }
    }
}
impl FromStr for LinkProfile {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "lan" => Ok(Self::Lan),
            "compatible" => Ok(Self::Compatible),
            _ => Err("Profile must be auto, lan or compatible".into()),
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
        match value {
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
        if let Some(kind) = transport {
            options.validate_for(kind)?;
        }
        Ok(options)
    }
}
impl SessionOptions {
    pub fn validate_for(&self, kind: TransportKind) -> Result<(), String> {
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
