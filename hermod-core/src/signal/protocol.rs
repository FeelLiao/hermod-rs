//! 信令线格式（M2.1）：与 Go server 共享的 WebSocket JSON 消息。
//!
//! 对应 Go `hermod-go/internal/server/server.go`：
//! - `MsgType`（第 30 行） → [`MsgType`]
//! - `Message`（第 43 行）  → [`Message`]
//! - `maxMessageSize`（第 21 行）→ [`MAX_MESSAGE_SIZE`]
//!
//! **一个重要的坑**：Go 的 `encoding/json` 把 `[]byte` 序列化成 **base64 字符串**，
//! 而 `serde_json` 默认会把 `Vec<u8>` 输出成数字数组 `[1,2,3,…]`——两者不互通！
//! 所以 `payload` 字段必须用自定义 serialize/deserialize 走 base64
//! （用依赖树里已有的 `base64ct`，标准字母表 + padding，与 Go 一致）。

use base64ct::{Base64, Encoding as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 单条信令消息上限：64 KiB（对应 Go `maxMessageSize`）。
pub const MAX_MESSAGE_SIZE: usize = 65536;

/// 信令消息类型（对应 Go `MsgType`）。JSON 里是小写字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MsgType {
    /// 创建频道（tx → server），携带自选的 channel_id。
    Allocate,
    /// 加入频道（rx → server），携带 channel_id。
    Join,
    /// Go 定义了但未使用；保留以保证能解析历史/未来报文。
    Handshake,
    /// 经 server 中继给对端的二进制 blob（CPace 消息、加密 bundle 等）。
    Blob,
    /// 错误（server → 任一客户端）。
    Error,
    /// 成功应答（如 allocate 的应答，payload 携带 server 观测到的源地址）。
    Ok,
    /// 频道就绪（server → tx，对端已加入）。
    Ready,
}

/// 信令消息（对应 Go `Message`）。
///
/// JSON 字段序 `type / channel_id / payload / error` 与 Go 一致；
/// 零值字段省略（对应 Go 的 `omitempty`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// 消息类型。Rust 关键字 `type` 不能做字段名，serde 里改回 `"type"`。
    #[serde(rename = "type")]
    pub msg_type: MsgType,
    #[serde(rename = "channel_id", default, skip_serializing_if = "is_zero_u16")]
    pub channel_id: u16,
    /// 二进制负载。序列化为 base64 字符串，与 Go 互通。
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_payload",
        deserialize_with = "deserialize_payload"
    )]
    pub payload: Vec<u8>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

impl Message {
    /// 构造 allocate（对应 Go `Message{Type: MsgAllocate, ChannelID: …}`）。
    pub fn allocate(channel_id: u16) -> Self {
        Self::new(MsgType::Allocate, channel_id)
    }

    /// 构造 join（对应 Go `Message{Type: MsgJoin, ChannelID: …}`）。
    pub fn join(channel_id: u16) -> Self {
        Self::new(MsgType::Join, channel_id)
    }

    /// 构造 blob（对应 Go `Message{Type: MsgBlob, ChannelID: …, Payload: …}`）。
    pub fn blob(channel_id: u16, payload: Vec<u8>) -> Self {
        Self {
            msg_type: MsgType::Blob,
            channel_id,
            payload,
            error: String::new(),
        }
    }

    /// 构造 ok（allocate 的应答，payload 带 server 观测到的源地址）。
    pub fn ok(channel_id: u16, payload: Vec<u8>) -> Self {
        Self {
            msg_type: MsgType::Ok,
            channel_id,
            payload,
            error: String::new(),
        }
    }

    /// 构造 ready（server → tx：对端已加入）。
    pub fn ready(channel_id: u16) -> Self {
        Self::new(MsgType::Ready, channel_id)
    }

    /// 构造 error。
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            msg_type: MsgType::Error,
            channel_id: 0,
            payload: Vec::new(),
            error: message.into(),
        }
    }

    fn new(msg_type: MsgType, channel_id: u16) -> Self {
        Self {
            msg_type,
            channel_id,
            payload: Vec::new(),
            error: String::new(),
        }
    }
}

fn is_zero_u16(v: &u16) -> bool {
    *v == 0
}

/// 把 payload 序列化成 base64 字符串（Go 的 encoding/json 对 []byte 的行为）。
fn serialize_payload<S: Serializer>(payload: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&Base64::encode_string(payload))
}

/// 从 base64 字符串反序列化 payload。
fn deserialize_payload<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let s = String::deserialize(d)?;
    Base64::decode_vec(&s).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 序列化字节与 Go `encoding/json` 逐字节一致（字段序 + omitempty + base64）。
    #[test]
    fn serialize_matches_go() {
        assert_eq!(
            serde_json::to_string(&Message::allocate(42)).unwrap(),
            r#"{"type":"allocate","channel_id":42}"#
        );
        assert_eq!(
            serde_json::to_string(&Message::join(7)).unwrap(),
            r#"{"type":"join","channel_id":7}"#
        );
        // payload 是 base64（独立基准：Python 算出 3q2+7w==）
        assert_eq!(
            serde_json::to_string(&Message::blob(42, vec![0xde, 0xad, 0xbe, 0xef])).unwrap(),
            r#"{"type":"blob","channel_id":42,"payload":"3q2+7w=="}"#
        );
        assert_eq!(
            serde_json::to_string(&Message::error("boom")).unwrap(),
            r#"{"type":"error","error":"boom"}"#
        );
        // 空 payload 省略（omitempty）
        assert_eq!(
            serde_json::to_string(&Message::ready(9)).unwrap(),
            r#"{"type":"ready","channel_id":9}"#
        );
    }

    /// 解析 Go 写出的 JSON（含 base64 payload）→ Message。
    #[test]
    fn deserialize_go_message() {
        let m: Message = serde_json::from_str(r#"{"type":"blob","channel_id":42,"payload":"aGVsbG8="}"#)
            .unwrap();
        assert_eq!(m.msg_type, MsgType::Blob);
        assert_eq!(m.channel_id, 42);
        assert_eq!(m.payload, b"hello");

        // 没有 channel_id / payload 的 error 消息
        let m: Message = serde_json::from_str(r#"{"type":"error","error":"channel terminated"}"#)
            .unwrap();
        assert_eq!(m.msg_type, MsgType::Error);
        assert_eq!(m.channel_id, 0);
        assert!(m.payload.is_empty());
    }

    /// 序列化 → 解析 往返一致。
    #[test]
    fn roundtrip() {
        let cases = [
            Message::allocate(1),
            Message::join(2),
            Message::blob(3, b"\x00\x01\xff".to_vec()),
            Message::ok(4, b"".to_vec()),
            Message::ready(5),
            Message::error("x"),
        ];
        for m in cases {
            let json = serde_json::to_string(&m).unwrap();
            assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), m);
        }
    }

    /// base64 边界：空串（被 omitempty 省略）、1/2/3 字节、0-15 递增
    /// （独立基准来自 Python）。
    #[test]
    fn base64_payload_edges() {
        // 空 payload → omitempty 省略字段（与 Go 一致）
        let json = serde_json::to_string(&Message::blob(1, Vec::new())).unwrap();
        assert!(!json.contains("payload"), "{json}");
        for (bytes, expected) in [
            (vec![0x00, 0x00, 0x00], "AAAA"),
            (b"hello".to_vec(), "aGVsbG8="),
            ((0u8..16).collect(), "AAECAwQFBgcICQoLDA0ODw=="),
        ] {
            let json = serde_json::to_string(&Message::blob(1, bytes)).unwrap();
            assert!(json.contains(&format!("\"payload\":\"{expected}\"")), "{json}");
        }
    }

    /// 非法输入：未知类型 / 非法 base64 必须报错。
    #[test]
    fn rejects_invalid() {
        assert!(serde_json::from_str::<Message>(r#"{"type":"bogus"}"#).is_err());
        assert!(serde_json::from_str::<Message>(r#"{"type":"blob","payload":"!!!"}"#).is_err());
        // 缺少 type 字段
        assert!(serde_json::from_str::<Message>(r#"{"channel_id":1}"#).is_err());
    }
}
