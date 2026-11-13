//! 设备控制协议：独立的 `/device` 认证、短期呼叫路由与单所有者客户端。

use super::client::{self, ObservedAddrs, WsStream};
use crate::{
    config::Config,
    identity::{DeviceId, Identity},
    options::IpFamily,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Message as Frame;

/// 控制协议版本。
pub const VERSION: u8 = 1;
/// JSON 消息最大字节数。
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024;
/// 注册签名域；不与传输认证签名混用。
pub const REGISTER_DOMAIN: &[u8] = b"hermod/device/register/v1";

/// 控制协议消息；拒绝未知字段，避免解释分歧。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeviceMessage {
    Challenge {
        version: u8,
        challenge: String,
        capabilities: Vec<String>,
    },
    Register {
        version: u8,
        id: DeviceId,
        pubkey: String,
        sig: String,
        register: bool,
    },
    Registered {
        version: u8,
    },
    Call {
        call_id: String,
        target_id: DeviceId,
        channel_id: u16,
        channel_token: String,
    },
    CallPending {
        call_id: String,
    },
    Incoming {
        caller_id: DeviceId,
        call_id: String,
        channel_id: u16,
    },
    Accept {
        call_id: String,
    },
    Decline {
        call_id: String,
    },
    Cancel {
        call_id: String,
    },
    Accepted {
        call_id: String,
    },
    Declined {
        call_id: String,
    },
    Cancelled {
        call_id: String,
    },
    Error {
        code: String,
    },
}

/// 控制客户端错误；不包含密钥或频道能力。
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error(transparent)]
    Client(#[from] client::ClientError),
    #[error("device protocol error")]
    Protocol,
    #[error("device operation rejected: {0}")]
    Rejected(String),
    #[error("device operation timed out")]
    Timeout,
}

/// 注册签名正文：固定端点、版本、完整设备 ID 与随机挑战。
pub fn registration_message(challenge: &[u8; 32], id: DeviceId, register: bool) -> Vec<u8> {
    let mut message = b"/device\0".to_vec();
    message.push(VERSION);
    message.push(u8::from(register));
    message.extend_from_slice(id.to_string().as_bytes());
    message.push(0);
    message.extend_from_slice(challenge);
    message
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn unhex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut result = [0; N];
    for (i, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(result)
}

/// 控制连接唯一拥有读写流；监听与发起呼叫使用不同连接。
pub struct DeviceControl {
    stream: WsStream,
}

impl DeviceControl {
    /// 认证控制连接；`register=true` 占用设备监听 ID，false 仅证明 caller 身份。
    pub async fn connect(
        config: &Config,
        family: IpFamily,
        identity: &Identity,
        register: bool,
    ) -> Result<Self, DeviceError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            Self::connect_inner(config, family, identity, register),
        )
        .await
        .map_err(|_| DeviceError::Timeout)?
    }

    async fn connect_inner(
        config: &Config,
        family: IpFamily,
        identity: &Identity,
        register: bool,
    ) -> Result<Self, DeviceError> {
        let url = crate::config::normalize_server_url(&config.server_url)
            .map_err(|_| DeviceError::Protocol)?;
        if !url.starts_with("wss://") {
            return Err(DeviceError::Protocol);
        }
        let pin = config
            .trusted_servers
            .get(&url)
            .and_then(|v| unhex::<32>(v))
            .ok_or(DeviceError::Protocol)?;
        let endpoint = format!(
            "{}/device",
            url.trim_end_matches('/').trim_end_matches("/ws")
        );
        let stream = client::connect_endpoint_pinned_with_family(&endpoint, pin, family).await?;
        let mut control = Self { stream };
        let challenge = match control.next_event().await? {
            DeviceMessage::Challenge {
                version: VERSION,
                challenge,
                capabilities,
            } if capabilities.iter().any(|c| c == "channel_owner_token") => {
                unhex::<32>(&challenge).ok_or(DeviceError::Protocol)?
            }
            _ => return Err(DeviceError::Protocol),
        };
        let id = identity.device_id();
        let sig = identity
            .sign(
                REGISTER_DOMAIN,
                &registration_message(&challenge, id, register),
            )
            .map_err(|_| DeviceError::Protocol)?;
        control
            .send(DeviceMessage::Register {
                version: VERSION,
                id,
                pubkey: hex(&identity.public_key()),
                sig: hex(&sig),
                register,
            })
            .await?;
        match control.next_event().await? {
            DeviceMessage::Registered { version: VERSION } => Ok(control),
            _ => Err(DeviceError::Protocol),
        }
    }

    async fn send(&mut self, message: DeviceMessage) -> Result<(), DeviceError> {
        let json = serde_json::to_string(&message).map_err(|_| DeviceError::Protocol)?;
        if json.len() > MAX_MESSAGE_SIZE {
            return Err(DeviceError::Protocol);
        }
        self.stream
            .send(Frame::text(json))
            .await
            .map_err(client::ClientError::from)?;
        Ok(())
    }

    /// 产生随机 128 位呼叫 ID；调用后由 `next_event` 读取终态。
    pub async fn call(
        &mut self,
        target_id: DeviceId,
        channel_id: u16,
        channel_token: String,
    ) -> Result<String, DeviceError> {
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| DeviceError::Protocol)?;
        let call_id = hex(&random);
        self.send(DeviceMessage::Call {
            call_id: call_id.clone(),
            target_id,
            channel_id,
            channel_token,
        })
        .await?;
        Ok(call_id)
    }
    /// 仅目标连接可接受呼叫。
    pub async fn accept(&mut self, call_id: &str) -> Result<(), DeviceError> {
        self.send(DeviceMessage::Accept {
            call_id: call_id.into(),
        })
        .await
    }
    /// 仅目标连接可拒绝呼叫；连接仍可继续监听。
    pub async fn decline(&mut self, call_id: &str) -> Result<(), DeviceError> {
        self.send(DeviceMessage::Decline {
            call_id: call_id.into(),
        })
        .await
    }
    /// 仅发起连接可取消呼叫。
    pub async fn cancel(&mut self, call_id: &str) -> Result<(), DeviceError> {
        self.send(DeviceMessage::Cancel {
            call_id: call_id.into(),
        })
        .await
    }
    /// 接收下一事件；一次只能有一个读者。
    pub async fn next_event(&mut self) -> Result<DeviceMessage, DeviceError> {
        loop {
            match self.stream.next().await {
                Some(Ok(Frame::Text(text))) if text.len() <= MAX_MESSAGE_SIZE => {
                    let message = serde_json::from_str(&text).map_err(|_| DeviceError::Protocol)?;
                    return match message {
                        DeviceMessage::Error { code } => Err(DeviceError::Rejected(code)),
                        other => Ok(other),
                    };
                }
                Some(Ok(Frame::Ping(_) | Frame::Pong(_))) => {
                    self.stream
                        .flush()
                        .await
                        .map_err(client::ClientError::from)?;
                }
                _ => return Err(DeviceError::Protocol),
            }
        }
    }
}

/// 在旧频道分配完成后提取仅该发送连接持有的一次性呼叫能力。
pub async fn allocate_channel(
    stream: &mut WsStream,
    channel_id: u16,
) -> Result<(ObservedAddrs, String), DeviceError> {
    client::send_message(stream, &super::protocol::Message::allocate(channel_id)).await?;
    let response = client::recv_message(stream).await?;
    if response.msg_type != super::protocol::MsgType::Ok {
        return Err(DeviceError::Protocol);
    }
    let value: serde_json::Value =
        serde_json::from_slice(&response.payload).map_err(|_| DeviceError::Protocol)?;
    let token = value
        .get("device_call_token")
        .and_then(|v| v.as_str())
        .filter(|v| unhex::<32>(v).is_some())
        .ok_or(DeviceError::Protocol)?
        .to_owned();
    let observed = serde_json::from_value(value).map_err(|_| DeviceError::Protocol)?;
    Ok((observed, token))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_hex_and_unknown_fields() {
        assert_eq!(unhex::<2>("01ff"), Some([1, 255]));
        assert!(unhex::<2>("01fg").is_none());
        assert!(unhex::<2>("01").is_none());
        assert!(
            serde_json::from_str::<DeviceMessage>(r#"{"type":"accept","call_id":"x","extra":1}"#)
                .is_err()
        );
    }
}
