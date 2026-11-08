//! CLI 与 GUI 共用的连接选项；按实际协议校验，禁止静默忽略。
use crate::{bandwidth::BandwidthLimit, session::SessionOptions, transport::TransportKind};
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
    pub limit_mbps: String,
}
impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            transport: "wireguard".into(),
            mtu: Mtu::Auto,
            congestion: Congestion::Auto,
            limit_mbps: "0".into(),
        }
    }
}
impl ConnectionSettings {
    pub fn session_options(&self, receiving: bool) -> Result<SessionOptions, String> {
        let transport = if self.transport == "auto" && receiving {
            None
        } else {
            Some(
                TransportKind::parse(&self.transport)
                    .ok_or("Choose wireguard or quic (auto is only available when receiving)")?,
            )
        };
        let options = SessionOptions {
            transport,
            mtu: match self.mtu {
                Mtu::Auto => None,
                Mtu::Value(v) => Some(usize::from(v)),
            },
            congestion: self.congestion,
            bandwidth_limit: self
                .limit_mbps
                .parse::<BandwidthLimit>()
                .map_err(str::to_string)?,
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
            TransportKind::WireGuard => 576..=65488,
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
            TransportKind::WireGuard => matches!(
                self.congestion,
                Congestion::Auto | Congestion::Cubic | Congestion::Reno
            ),
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
            Some(TransportKind::WireGuard)
        );
        settings.congestion = Congestion::Bbr;
        assert!(settings.session_options(false).is_err());
        settings.transport = "quic".into();
        assert!(settings.session_options(false).is_ok());
        settings.mtu = Mtu::Value(1100);
        assert!(settings.session_options(false).is_err());
        settings.transport = "auto".into();
        assert!(settings.session_options(false).is_err());
        let opts = settings.session_options(true).unwrap();
        assert!(opts.validate_for(TransportKind::Quic).is_err());
        assert!(opts.validate_for(TransportKind::WireGuard).is_err());
        for value in ["0", "-1", "65535", "NaN"] {
            assert!(value.parse::<Mtu>().is_err());
        }
    }
}
