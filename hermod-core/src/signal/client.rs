//! 信令客户端连接层（M2.2）：经 WebSocket 连 server，可做 SPKI pinning。
//!
//! 对应 Go `internal/network/signaling.go` 的连接层（消息流程见 M2.3）：
//! - [`connect_pinned`]：连 `wss://host:4376/ws`，用 M2.2 的 SPKI pinning 验证服务器；
//! - [`send_message`] / [`recv_message`]：收发 M2.1 定义的 JSON 信封。

use crate::signal::protocol::{Message, MsgType};
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
    #[error("cpace error: {0}")]
    Cpace(#[from] crate::crypto::cpace::CpaceError),
    #[error("server error: {0}")]
    Server(String),
    #[error("invalid blob: expected a 65-byte CPace point, got {0} bytes")]
    InvalidBlob(usize),
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

/// server 观测到的源地址（ok 应答 payload 的 JSON 解码）。
///
/// Go 版会同时给 public_ipv4/public_ipv6；Rust 版只采纳 IPv6（D7，
/// 与 [`crate::transport::wireguard::addr`] 的规则一致），v4 字段保留以兼容 Go server。
#[derive(Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct ObservedAddrs {
    #[serde(default)]
    pub public_ipv4: String,
    #[serde(default)]
    pub public_ipv6: String,
}

/// 创建频道（tx → server）。处理 ok / ready（竞态）/ error 三种应答
/// （对应 Go `SignalingClient.Allocate`）。
pub async fn allocate(
    stream: &mut WsStream,
    channel_id: u16,
) -> Result<ObservedAddrs, ClientError> {
    send_message(stream, &Message::allocate(channel_id)).await?;
    loop {
        let resp = recv_message(stream).await?;
        match resp.msg_type {
            MsgType::Ok => return parse_observed(&resp.payload),
            // 竞态：receiver 先 join 了，server 直接回 ready
            MsgType::Ready => return Ok(ObservedAddrs::default()),
            MsgType::Error => return Err(ClientError::Server(resp.error)),
            _ => continue,
        }
    }
}

/// 加入频道（rx → server）。对应 Go `SignalingClient.Join`。
pub async fn join(stream: &mut WsStream, channel_id: u16) -> Result<ObservedAddrs, ClientError> {
    send_message(stream, &Message::join(channel_id)).await?;
    let resp = recv_message(stream).await?;
    if resp.msg_type == MsgType::Error {
        return Err(ClientError::Server(resp.error));
    }
    parse_observed(&resp.payload)
}

/// 等待 ready（server 通知 tx：对端已加入）。对应 Go `WaitReady`。
pub async fn wait_ready(stream: &mut WsStream) -> Result<(), ClientError> {
    loop {
        let msg = recv_message(stream).await?;
        match msg.msg_type {
            MsgType::Ready => return Ok(()),
            MsgType::Error => return Err(ClientError::Server(msg.error)),
            _ => continue,
        }
    }
}

/// 经中继发送一个二进制 blob（对应 Go `SendBlob`）。
pub async fn send_blob(
    stream: &mut WsStream,
    channel_id: u16,
    blob: &[u8],
) -> Result<(), ClientError> {
    send_message(stream, &Message::blob(channel_id, blob.to_vec())).await
}

/// 等待中继转发来的 blob（跳过 ready，error 报错；对应 Go `RecvBlob`）。
pub async fn recv_blob(stream: &mut WsStream) -> Result<Vec<u8>, ClientError> {
    loop {
        let msg = recv_message(stream).await?;
        match msg.msg_type {
            MsgType::Blob => return Ok(msg.payload),
            MsgType::Ready => continue,
            MsgType::Error => return Err(ClientError::Server(msg.error)),
            _ => continue,
        }
    }
}

/// 发起方（tx）经中继完成 CPace（M2.3）：
/// allocate → 等 ready → 发 blob1（CPace 公钥）→ 收 blob2 → 得到共享密钥 K。
///
/// `password` 是传输码去掉 `channelID-` 前缀后的单词部分（如 `acid-acorn-acre`）。
pub async fn sender_handshake(
    stream: &mut WsStream,
    channel_id: u16,
    password: &str,
) -> Result<([u8; 32], ObservedAddrs), ClientError> {
    let observed = allocate(stream, channel_id).await?;
    wait_ready(stream).await?;

    let (session, my_pub) =
        crate::crypto::cpace::init(password, channel_id, crate::crypto::cpace::Role::Sender)?;
    // blob1：本端 CPace 公钥（只发一次；多发会污染后续的 bundle 交换）
    send_blob(stream, channel_id, &my_pub).await?;

    // blob2：对端 CPace 公钥 → 得到 K
    let peer_pub = recv_blob(stream).await?;
    let peer = blob_to_point(&peer_pub)?;
    Ok((session.finish(&peer)?, observed))
}

/// 接收方（rx）经中继完成 CPace（M2.3）：
/// join → 收 blob1 → 发 blob2（CPace 公钥）→ 得到共享密钥 K。
pub async fn receiver_handshake(
    stream: &mut WsStream,
    channel_id: u16,
    password: &str,
) -> Result<([u8; 32], ObservedAddrs), ClientError> {
    let observed = join(stream, channel_id).await?;

    let (session, my_pub) =
        crate::crypto::cpace::init(password, channel_id, crate::crypto::cpace::Role::Receiver)?;

    // blob1：对端 CPace 公钥；blob2：本端 CPace 公钥 → 得到 K
    let peer_pub = recv_blob(stream).await?;
    send_blob(stream, channel_id, &my_pub).await?;

    let peer = blob_to_point(&peer_pub)?;
    Ok((session.finish(&peer)?, observed))
}

/// 把 blob 转成 65 字节 CPace 未压缩点。
fn blob_to_point(blob: &[u8]) -> Result<[u8; 65], ClientError> {
    blob.try_into()
        .map_err(|_| ClientError::InvalidBlob(blob.len()))
}

/// 解析 ok 应答里的观测地址 JSON。
fn parse_observed(payload: &[u8]) -> Result<ObservedAddrs, ClientError> {
    serde_json::from_slice(payload).map_err(ClientError::from)
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

    /// 最小中继：照 Go server 的 allocate/join/ready/blob 语义实现，
    /// 让两个客户端在本地完成 CPace 会合（M2.3）。
    /// 服务端 accept 出的是 `WebSocketStream<TcpStream>`（无 TLS 包装）。
    type ServerWs = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

    async fn run_test_relay(mut tx: ServerWs, mut rx: ServerWs) {
        // tx 的 allocate → 回 ok
        let alloc: Message = serde_json::from_str(
            tx.next().await.unwrap().unwrap().to_text().unwrap(),
        )
        .unwrap();
        let channel_id = alloc.channel_id;
        let ok_json = serde_json::to_string(&Message::ok(channel_id, b"{}".to_vec())).unwrap();
        tx.send(WsMessage::text(ok_json)).await.unwrap();

        // rx 的 join → 回 ok，并通知 tx ready
        let join: Message = serde_json::from_str(
            rx.next().await.unwrap().unwrap().to_text().unwrap(),
        )
        .unwrap();
        assert_eq!(join.channel_id, channel_id);
        let ok_json = serde_json::to_string(&Message::ok(join.channel_id, b"{}".to_vec())).unwrap();
        rx.send(WsMessage::text(ok_json)).await.unwrap();
        let ready_json = serde_json::to_string(&Message::ready(join.channel_id)).unwrap();
        tx.send(WsMessage::text(ready_json)).await.unwrap();

        // 转发 blob（双向）
        loop {
            tokio::select! {
                Some(Ok(msg)) = tx.next() => { rx.send(msg).await.unwrap(); }
                Some(Ok(msg)) = rx.next() => { tx.send(msg).await.unwrap(); }
                else => break,
            }
        }
    }

    /// 两个客户端经本地中继跑完整 CPace，双方得到同一个 K。
    #[tokio::test]
    async fn cpace_rendezvous_through_relay() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let relay = tokio::spawn(async move {
            let (tcp_tx, _) = listener.accept().await.unwrap();
            let tx = tokio_tungstenite::accept_async(tcp_tx).await.unwrap();
            let (tcp_rx, _) = listener.accept().await.unwrap();
            let rx = tokio_tungstenite::accept_async(tcp_rx).await.unwrap();
            run_test_relay(tx, rx).await;
        });

        let url = format!("ws://{addr}");
        let (mut tx_client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let (mut rx_client, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        let channel_id = 0x1234;
        let password = "acid-acorn-acre";
        let (k_tx, k_rx) = tokio::join!(
            sender_handshake(&mut tx_client, channel_id, password),
            receiver_handshake(&mut rx_client, channel_id, password),
        );
        let (k_tx, _) = k_tx.unwrap();
        let (k_rx, _) = k_rx.unwrap();
        assert_eq!(k_tx, k_rx, "双方 CPace 密钥必须一致");

        // 关闭客户端连接，让中继循环退出（否则 relay.await 永远等不到流关闭）
        drop(tx_client);
        drop(rx_client);
        relay.await.unwrap();
    }
}
