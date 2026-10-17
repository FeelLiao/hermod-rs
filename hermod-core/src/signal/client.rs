//! 信令客户端连接层（M2.2）：经 WebSocket 连 server，可做 SPKI pinning。
//!
//! 对应 Go `internal/network/signaling.go` 的连接层（消息流程见 M2.3）：
//! - [`connect_pinned`]：连 `wss://host:4376/ws`，用 M2.2 的 SPKI pinning 验证服务器；
//! - [`send_message`] / [`recv_message`]：收发 M2.1 定义的 JSON 信封。

use crate::signal::protocol::Message;
use crate::signal::tls::pinned_client_config;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream};

/// 信令 WebSocket 流类型（明文或 TLS 由 URL scheme 决定）。
pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 信令客户端错误。
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("websocket error: {0}")]
    Ws(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("connection closed unexpectedly")]
    Closed,
    #[error("unexpected binary websocket message")]
    Binary,
}

/// 连接信令 server（`wss://host:4376/ws`），并用 SPKI 指纹 pin 服务器证书。
///
/// `expected_spki` 来自 `hermod trust` 记录的指纹（见 [`crate::signal::tls`]）。
pub async fn connect_pinned(url: &str, expected_spki: [u8; 32]) -> Result<WsStream, ClientError> {
    let config = pinned_client_config(expected_spki);
    let connector = Connector::Rustls(config);
    let (stream, _response) =
        connect_async_tls_with_config(url, None, false, Some(connector)).await?;
    Ok(stream)
}

/// 发送一条信令消息（JSON 文本帧，对应 Go `conn.WriteJSON`）。
pub async fn send_message(stream: &mut WsStream, msg: &Message) -> Result<(), ClientError> {
    let json = serde_json::to_string(msg)?;
    stream.send(WsMessage::text(json)).await?;
    Ok(())
}

/// 接收并解析一条信令消息。
///
/// - 跳过 Ping/Pong；
/// - 服务器只会发文本帧（JSON），收到二进制帧视为协议错误；
/// - 连接关闭返回 [`ClientError::Closed`]。
pub async fn recv_message(stream: &mut WsStream) -> Result<Message, ClientError> {
    loop {
        match stream.next().await {
            Some(Ok(WsMessage::Text(t))) => {
                return serde_json::from_str(t.as_str()).map_err(ClientError::from);
            }
            Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => continue,
            Some(Ok(WsMessage::Binary(_))) => return Err(ClientError::Binary),
            Some(Ok(WsMessage::Close(_))) | None => return Err(ClientError::Closed),
            Some(Ok(WsMessage::Frame(_))) => continue,
            Some(Err(e)) => return Err(ClientError::Ws(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::protocol::MsgType;

    /// 本地起一个 WebSocket echo server，验证消息收发往返（不涉及 TLS，
    /// TLS pinning 的正确性已在 `signal::tls` 的测试里覆盖）。
    #[tokio::test]
    async fn message_roundtrip_over_ws() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            while let Some(Ok(msg)) = ws.next().await {
                if let WsMessage::Text(t) = msg {
                    ws.send(WsMessage::Text(t)).await.unwrap();
                }
            }
        });

        let url = format!("ws://{addr}");
        let (mut client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // 带 base64 payload 的 blob 消息
        let m = Message::blob(42, vec![0xde, 0xad, 0xbe, 0xef]);
        send_message(&mut client, &m).await.unwrap();
        assert_eq!(recv_message(&mut client).await.unwrap(), m);

        // allocate 消息（无 payload）
        let m = Message::allocate(7);
        send_message(&mut client, &m).await.unwrap();
        let got = recv_message(&mut client).await.unwrap();
        assert_eq!(got.msg_type, MsgType::Allocate);
        assert_eq!(got, m);

        client.close(None).await.unwrap();
        server.await.unwrap();
    }
}
