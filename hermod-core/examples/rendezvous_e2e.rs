//! 端到端会合测试（逐步打印版）：两个客户端经真实 signaling server 跑 CPace。
//!
//! 关键顺序（与真实使用一致）：**tx 先 allocate（拿到传输码），rx 再 join**，
//! 然后双方并发完成 CPace。若 rx 的 join 先于 tx 的 allocate，Go server 会拒绝
//! （频道不存在）。
//!
//! ```bash
//! SPKI_FP=<hex> cargo run -p hermod-core --example rendezvous_e2e
//! ```

use hermod_core::crypto::cpace;
use hermod_core::crypto::wordlist;
use hermod_core::signal::client::{self, connect_pinned};
use std::time::Duration;

#[tokio::main]
async fn main() {
    let url = std::env::var("SERVER_URL")
        .unwrap_or_else(|_| "wss://oracle.stariverfeel.eu.org:4376/ws".to_string());
    let fp_hex = std::env::var("SPKI_FP").expect("set SPKI_FP to the server's SPKI fingerprint");
    let expected = parse_fp(&fp_hex).expect("SPKI_FP must be 64 hex chars");

    // tx 侧生成传输码；口令 = 去掉 channelID- 前缀的单词部分
    let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
    let (channel_id, code) = wordlist::generate_transfer_code(&mut rng, 3);
    let password = code.split_once('-').map(|(_, w)| w).unwrap();
    eprintln!("transfer code : {code}");
    eprintln!("channel / pwd : {channel_id} / {password}");
    eprintln!("server        : {url}");

    let mut tx_conn = connect_pinned(&url, expected).await.expect("tx connect");
    eprintln!("tx: connected + pinned");
    let mut rx_conn = connect_pinned(&url, expected).await.expect("rx connect");
    eprintln!("rx: connected + pinned");

    // 1) tx 先 allocate，确认频道已创建
    eprintln!("tx: allocate");
    client::allocate(&mut tx_conn, channel_id)
        .await
        .expect("tx allocate");
    eprintln!("tx: allocated");

    // 2) rx 再 join
    eprintln!("rx: join");
    client::join(&mut rx_conn, channel_id).await.expect("rx join");
    eprintln!("rx: joined");

    // 3) 并发完成 CPace（tx 等 ready 后发 blob1；rx 收 blob1 后发 blob2）
    let tx_fut = async {
        eprintln!("tx: wait_ready");
        client::wait_ready(&mut tx_conn).await.expect("tx wait_ready");
        eprintln!("tx: started");
        let (session, my_pub) =
            cpace::init(password, channel_id, cpace::Role::Sender).expect("tx cpace init");
        eprintln!("tx: send blob1");
        client::send_blob(&mut tx_conn, channel_id, &my_pub)
            .await
            .expect("tx send blob1");
        eprintln!("tx: recv blob2");
        let peer: [u8; 65] = client::recv_blob(&mut tx_conn)
            .await
            .expect("tx recv blob2")
            .try_into()
            .expect("blob2 length");
        session.finish(&peer).expect("tx finish")
    };

    let rx_fut = async {
        eprintln!("rx: started");
        let (session, my_pub) =
            cpace::init(password, channel_id, cpace::Role::Receiver).expect("rx cpace init");
        eprintln!("rx: recv blob1");
        let peer: [u8; 65] = client::recv_blob(&mut rx_conn)
            .await
            .expect("rx recv blob1")
            .try_into()
            .expect("blob1 length");
        eprintln!("rx: send blob2");
        client::send_blob(&mut rx_conn, channel_id, &my_pub)
            .await
            .expect("rx send blob2");
        session.finish(&peer).expect("rx finish")
    };

    match tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(tx_fut, rx_fut)
    })
    .await
    {
        Ok((k_tx, k_rx)) => {
            assert_eq!(k_tx, k_rx, "双方 K 不一致");
            eprintln!("OK: both sides derived the same K = {}", hex(&k_tx));
        }
        Err(_) => {
            eprintln!("TIMEOUT after 30s — 看上面最后一条打印就是卡住的步骤");
            std::process::exit(1);
        }
    }
}

fn parse_fp(hex_str: &str) -> Result<[u8; 32], &'static str> {
    let bytes: Vec<u8> = (0..hex_str.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).map_err(|_| "bad hex"))
        .collect::<Result<_, _>>()?;
    bytes.try_into().map_err(|_| "need 32 bytes")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
