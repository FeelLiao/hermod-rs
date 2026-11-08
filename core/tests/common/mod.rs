//! 集成测试共享组件：本地最小信令中继 + 免费的测试 `Config`。
//!
//! 线上语义与 `cli/tests/interop.rs`、`flutter-bridge` worker 测试完全一致：
//! 先 accept 两条 WS（第一连接发 allocate→ok，第二连接发 join→ok，并向 tx
//! 回 ready），然后把两条连接之间的所有 blob 原样双向转发。
//! 每个测试用例起一份独立中继，互不串扰。

use futures_util::{SinkExt, StreamExt};
use hermod_core::config::Config;
use hermod_core::signal::protocol::Message;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message as Frame};

/// 每个测试用例独立计数，避免临时目录冲突。
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// 启动一个一次性的哑信令中继：返回可直接喂给
/// [`session::sender_session_with_ready`](hermod_core::session) /
/// [`session::receiver_session_with`](hermod_core::session) 的 `Config`，
/// 以及中继任务句柄（测试收尾时 `abort()`）。
pub async fn start_relay() -> (Config, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local = listener.local_addr().unwrap();
    let mut config = Config {
        server_url: format!("ws://{local}"),
        ..Config::default()
    };
    // SPKI pinning 对 ws:// 明文连接不生效（connector 只用于 TLS 升级），
    // 但会话层仍要求"服务器已信任"，这里插入一条占位指纹即可。
    config
        .trusted_servers
        .insert(config.server_url.clone(), "00".repeat(32));

    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tx = accept_async(tcp).await.unwrap();
        let allocate: Message =
            serde_json::from_str(tx.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        tx.send(Frame::text(
            serde_json::to_string(&Message::ok(allocate.channel_id, b"{}".to_vec())).unwrap(),
        ))
        .await
        .unwrap();

        let (tcp, _) = listener.accept().await.unwrap();
        let mut rx = accept_async(tcp).await.unwrap();
        let join: Message =
            serde_json::from_str(rx.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(join.channel_id, allocate.channel_id);
        rx.send(Frame::text(
            serde_json::to_string(&Message::ok(join.channel_id, b"{}".to_vec())).unwrap(),
        ))
        .await
        .unwrap();
        tx.send(Frame::text(
            serde_json::to_string(&Message::ready(join.channel_id)).unwrap(),
        ))
        .await
        .unwrap();

        // blob 双向转发（CPace 公钥、加密端点信封全走这里）
        loop {
            tokio::select! {
                Some(Ok(frame)) = tx.next() => { if rx.send(frame).await.is_err() { break; } }
                Some(Ok(frame)) = rx.next() => { if tx.send(frame).await.is_err() { break; } }
                else => break,
            }
        }
    });
    (config, task)
}

/// 每个测试一个独立临时目录（放系统 temp 下，避免污染工作区）。
pub fn temp_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!(
        "hermod-core-tests-{}-{tag}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 供个别测试对临时目录做收尾。
#[allow(dead_code)]
pub fn cleanup_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}
