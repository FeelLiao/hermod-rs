//! 端到端会合测试：两个客户端经真实 signaling server 跑 CPace（M2.2/M2.3）。
//!
//! 前提：能访问部署好的 Go server（默认 `wss://oracle.stariverfeel.eu.org:4376/ws`）。
//!
//! 1) 先算出服务器证书的 **SPKI 指纹**（在能连服务器的机器上执行）：
//!
//! ```bash
//! openssl s_client -connect oracle.stariverfeel.eu.org:4376 </dev/null 2>/dev/null \
//!   | openssl x509 -pubkey -noout \
//!   | openssl pkey -pubin -outform DER \
//!   | openssl dgst -sha256 | awk '{print $2}'
//! ```
//!
//! 2) 运行（用上一步得到的 hex）：
//!
//! ```bash
//! SPKI_FP=<hex> cargo run -p hermod-core --example rendezvous_e2e
//! ```
//!
//! 可用 `SERVER_URL` 环境变量覆盖服务器地址。
//!
//! 预期输出：两个客户端都打印「connected + pinned」，最后打印双方派生出**同一个 K**。

use hermod_core::crypto::wordlist;
use hermod_core::signal::client::{self, connect_pinned};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("SERVER_URL")
        .unwrap_or_else(|_| "wss://oracle.stariverfeel.eu.org:4376/ws".to_string());
    let fp_hex = std::env::var("SPKI_FP")
        .map_err(|_| "set SPKI_FP to the server's SPKI fingerprint (64 hex chars)")?;
    let expected = parse_fp(&fp_hex)?;

    // tx 侧生成传输码；口令 = 去掉 channelID- 前缀的单词部分（与 Go 一致）
    let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
    let (channel_id, code) = wordlist::generate_transfer_code(&mut rng, 3);
    let password = code
        .split_once('-')
        .map(|(_, words)| words)
        .ok_or("malformed transfer code")?;
    eprintln!("transfer code : {code}");
    eprintln!("channel / pwd : {channel_id} / {password}");
    eprintln!("server        : {url}  (SPKI {fp_hex})");

    let mut tx_conn = connect_pinned(&url, expected).await?;
    let mut rx_conn = connect_pinned(&url, expected).await?;
    eprintln!("both clients connected + pinned");

    let (k_tx, k_rx) = tokio::join!(
        client::sender_handshake(&mut tx_conn, channel_id, password),
        client::receiver_handshake(&mut rx_conn, channel_id, password),
    );
    let k_tx = k_tx?;
    let k_rx = k_rx?;
    if k_tx != k_rx {
        return Err("CPace keys differ between sender and receiver".into());
    }
    eprintln!("OK: both sides derived the same K = {}", hex(&k_tx));
    Ok(())
}

/// 把十六进制字符串解析成 32 字节指纹。
fn parse_fp(hex_str: &str) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let bytes: Vec<u8> = (0..hex_str.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16))
        .collect::<Result<_, _>>()
        .map_err(|_| "SPKI_FP must be hex")?;
    bytes
        .try_into()
        .map_err(|_| "SPKI_FP must be 32 bytes (64 hex chars)".into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
