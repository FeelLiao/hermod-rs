//! 验收：真实 Rust 信令 server ↔ hermod-rs 客户端。
//!
//! 与 `e2e_full_chain.rs`（进程内哑中继）互补：这一层起**真 server**（TLS +
//! WS 路由 + 限流 + blob 中继 + /cert），保证「Rust server 可与现有客户端
//! 互换部署」的口径成立。信令、TLS 钉扎、QUIC 数据面全部走真实路径。

use hermod_core::{
    config::Config,
    crypto::wordlist,
    orchestrate::{self, ReceiveOptions},
    server::{self, Server, ServerLimits, cert},
    session::{self, SessionOptions},
    transfer::{Kind, TransferOffer, hash_bytes},
    transport::TransportConnection,
};
use rand_core::UnwrapErr;
use std::time::Duration;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("hermod-server-e2e-{tag}-{}", std::process::id(),));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 起一个真 server（TLS + /cert + 反射），返回客户端用的 Config 与任务句柄。
async fn start_server() -> (Config, tokio::task::JoinHandle<()>, std::path::PathBuf) {
    let dir = temp_dir("srv");
    let cert = cert::ensure(&dir, cert::CERT_AUTO_RENEW_THRESHOLD).unwrap();

    let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let acceptor = server::tls_acceptor(&cert).unwrap();
    let store = std::sync::Arc::new(server::MemoryStore::new(
        server::DEFAULT_MAX_CHANNELS_PER_IP,
    ));
    let server = Server::new(
        ServerLimits {
            ttl: Duration::from_secs(600),
            max_blobs_per_channel: server::DEFAULT_MAX_BLOBS_PER_CHANNEL,
            max_cpace_failures: server::DEFAULT_MAX_CPACE_FAILURES,
        },
        store,
        1000.0,
        3000.0, // 测试内放宽限流，避免并发抖动
        cert.cert_pem.clone(),
    );
    // 同端口起 UDP 反射（回环可达；顺带覆盖与 TCP 共存）。
    let reflector = server::reflector::start(addr).unwrap();
    let task = tokio::spawn(async move {
        let _ = server.run(listener, acceptor, Some(reflector)).await;
    });

    let url = format!("wss://{addr}");
    let mut config = Config {
        server_url: url.clone(),
        ..Config::default()
    };
    config
        .trusted_servers
        .insert(url, hex(&cert.spki_fingerprint));
    (config, task, dir)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SPKI 钉扎链路：`fetch_server_spki`（TOFU 抓取）必须与服务端自报指纹一致。
#[tokio::test]
async fn cert_endpoint_and_pinning_agree() {
    let (config, task, dir) = start_server().await;
    let (_, expected_fp) = hermod_core::session::expected_spki(&config).unwrap();
    // 客户端 TOFU 抓到的指纹 == 服务端证书指纹 → trust 流程闭环
    let observed = hermod_core::signal::tls::fetch_server_spki(&config.server_url)
        .await
        .unwrap();
    assert_eq!(hex(&observed), hex(&expected_fp));
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 完整链路：传输码 → 真信令 → CPace → QUIC 直连 → 文件传输 → SHA-256 裁决。
#[tokio::test]
async fn full_chain_through_real_server() {
    let (config, task, dir) = start_server().await;
    let mut rng = UnwrapErr(getrandom::SysRng);

    // 载荷与预告
    let mut data = vec![0u8; 256 * 1024];
    use rand_core::Rng;
    rng.fill_bytes(&mut data);
    let source = dir.join("payload.bin");
    std::fs::write(&source, &data).unwrap();
    let offer = TransferOffer {
        kind: Kind::File,
        name: "payload.bin".into(),
        size: data.len() as u64,
    };

    // 双方会话选项：回环 UDP，发送端带 offer + 文件路径源（多流能力）
    let mut sender_opts = SessionOptions {
        transport: Some(hermod_core::transport::TransportKind::Quic),
        udp_bind: Some("[::1]:0".parse().unwrap()),
        multi_stream_capable: true,
        ..Default::default()
    };
    sender_opts.offer = Some(offer.clone());
    let receiver_opts = SessionOptions {
        udp_bind: Some("[::1]:0".parse().unwrap()),
        ..Default::default()
    };

    let (_, code) = wordlist::generate_transfer_code(&mut rng, 3);
    let sender = session::sender_session_with_ready(&config, &code, &sender_opts, || {});
    let receiver = session::receiver_session_with(&config, &code, &receiver_opts);
    let (tx, rx) = tokio::time::timeout(
        Duration::from_secs(90),
        futures_util::future::join(sender, receiver),
    )
    .await
    .expect("session deadline")
    .pipe(|(t, r)| (t.unwrap(), r.unwrap()));
    let (mut tx, mut rx) = (tx, rx);
    assert_eq!(rx.offer.as_ref(), Some(&offer));
    assert!(rx.peer_multi_stream, "发送端 bundle 应声明多流能力");

    let dest_dir = dir.join("out");
    let (sent, receipt) = tokio::time::timeout(
        Duration::from_secs(90),
        futures_util::future::join(
            async {
                orchestrate::send_file(&mut tx.conn, &source, None)
                    .await
                    .unwrap()
            },
            async {
                orchestrate::receive(
                    &mut rx.conn,
                    &dest_dir,
                    ReceiveOptions {
                        offer: rx.offer.as_ref(),
                        profile: hermod_core::options::LinkProfile::Lan,
                        peer_multi_stream: rx.peer_multi_stream,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
            },
        ),
    )
    .await
    .expect("transfer deadline");
    assert!(sent.verified);
    assert_eq!(receipt.sha256, hash_bytes(&data));
    assert_eq!(std::fs::read(&receipt.path).unwrap(), data);

    tx.conn.shutdown().await.unwrap();
    rx.conn.shutdown().await.unwrap();
    task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 小工具：把 (A, B) 的元组结果拆开（比直接 unwrap 更好读）。
trait Pipe: Sized {
    fn pipe<F, Out>(self, f: F) -> Out
    where
        F: FnOnce(Self) -> Out,
    {
        f(self)
    }
}
impl<T> Pipe for T {}
