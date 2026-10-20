//! 传输编排（M4.4）：把「传输连接」与「传输内 HTTP」组合成 tx / rx 两个动作。
//!
//! - tx：[`serve_file`] 先 `listen`，再用 HTTP server 发文件并收裁决；
//! - rx：[`fetch_file`] 先 `dial`，再用 HTTP client 拉文件并提交 SHA-256。
//!
//! 上层（CLI / GUI）只需提供 `ServeOptions` 与目标写入端，不关心底层是
//! WireGuard 还是其它传输。

use crate::transport::http::HttpError;
use crate::transport::http::client::{FetchOutcome, fetch};
use crate::transport::http::server::{ServeOptions, ServeOutcome, serve};
use crate::transport::iface::TransportConnection;
use tokio::io::AsyncWrite;

/// 编排错误：传输层或 HTTP 层。
#[derive(Debug, thiserror::Error)]
pub enum OrchestrateError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("http: {0}")]
    Http(#[from] HttpError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("transfer: {0}")]
    Transfer(#[from] crate::transfer::TransferError),
    #[error("digest mismatch: rx 提交的 SHA-256 未被 tx 接受")]
    DigestMismatch,
    /// 对端中止了连接（Ctrl+C / RST / 提前 EOF）。
    #[error("peer cancelled the transfer")]
    PeerCancelled,
}

/// 把「对端中止连接」类错误（RST / 提前 EOF / BrokenPipe）映射为友好提示。
fn map_http(e: HttpError) -> OrchestrateError {
    if matches!(e, HttpError::PeerCancelled) {
        return OrchestrateError::PeerCancelled;
    }
    let cancelled = match &e {
        HttpError::PeerCancelled => false,
        HttpError::Malformed(s) => s.contains("body shorter than Content-Length"),
        HttpError::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::UnexpectedEof
        ),
        HttpError::Meta(_) => false,
    };
    if cancelled {
        OrchestrateError::PeerCancelled
    } else {
        OrchestrateError::Http(e)
    }
}

/// rx 成功落盘后的回执。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    /// 最终落盘路径（已去重）。
    pub path: std::path::PathBuf,
    /// SHA-256（十六进制小写）。
    pub sha256: String,
    /// 字节数。
    pub size: u64,
    /// 载荷类型。
    pub kind: crate::transfer::Kind,
}

/// tx：在传输内监听，服务一次 `GET /f/<token>` + `POST /done`。
pub async fn serve_file<C>(
    conn: &mut C,
    opts: ServeOptions,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: TransportConnection,
{
    let stream = conn
        .listen()
        .await
        .map_err(|e| OrchestrateError::Transport(e.to_string()))?;
    serve(stream, opts).await.map_err(map_http)
}

/// rx：向传输内 tx 拨号，拉文件到 `dest` 并提交裁决。
pub async fn fetch_file<C, D>(
    conn: &mut C,
    token: &[u8; 16],
    dest: &mut D,
    progress: Option<&mut dyn FnMut(u64)>,
) -> Result<FetchOutcome, OrchestrateError>
where
    C: TransportConnection,
    D: AsyncWrite + Unpin,
{
    let stream = conn
        .dial()
        .await
        .map_err(|e| OrchestrateError::Transport(e.to_string()))?;
    fetch(stream, token, dest, progress).await.map_err(map_http)
}

/// tx：把本地文件经传输发给 rx（边发边算 SHA-256）。
pub async fn send_file<C>(
    conn: &mut C,
    token: [u8; 16],
    path: &std::path::Path,
    progress: Option<Box<dyn FnMut(u64) + Send>>,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: TransportConnection,
{
    let file = tokio::fs::File::open(path).await?;
    let size = file.metadata().await?.len();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    crate::log_info!(path = %path.display().to_string(), size, "sending file");
    let opts = ServeOptions {
        token,
        kind: crate::transfer::Kind::File,
        name,
        size,
        body: Box::new(file),
        progress,
    };
    serve_file(conn, opts).await
}

/// tx：把一段文本经传输发给 rx（`Kind::Text`）。
pub async fn send_text<C>(
    conn: &mut C,
    token: [u8; 16],
    text: &str,
    progress: Option<Box<dyn FnMut(u64) + Send>>,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: TransportConnection,
{
    let data = text.as_bytes().to_vec();
    let size = data.len() as u64;
    let opts = ServeOptions {
        token,
        kind: crate::transfer::Kind::Text,
        name: "message.txt".to_string(),
        size,
        body: Box::new(std::io::Cursor::new(data)),
        progress,
    };
    serve_file(conn, opts).await
}

/// 临时文件 RAII 清理：Drop 时删除（含 future 被取消/panic 的情况）。
struct TempGuard(std::path::PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// rx：拉取文件到 `dest_dir`，写临时文件 → 校验 → 原子改名。
///
/// 对应 M4.4：临时文件 O_EXCL + 0600；摘要不一致则删临时文件并报错。
pub async fn receive<C>(
    conn: &mut C,
    token: &[u8; 16],
    dest_dir: &std::path::Path,
    progress: Option<&mut dyn FnMut(u64)>,
) -> Result<Receipt, OrchestrateError>
where
    C: TransportConnection,
{
    std::fs::create_dir_all(dest_dir)?;
    // 唯一临时名（避免并发/重名）；create_temp 用 O_EXCL + 0600
    let mut rnd = [0u8; 8];
    {
        use rand_core::Rng;
        let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
        rng.fill_bytes(&mut rnd);
    }
    let temp = dest_dir.join(format!(
        ".hermod_{:016x}.hermod_tmp",
        u64::from_le_bytes(rnd)
    ));
    let std_file = crate::transfer::create_temp(&temp)?;
    // 守卫在函数返回/被取消时删除临时文件；成功改名后 forget 掉。
    let guard = TempGuard(temp.clone());
    let mut file = tokio::fs::File::from_std(std_file);

    let out = match fetch_file(conn, token, &mut file, progress).await {
        Ok(out) => out,
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            return Err(e);
        }
    };
    file.sync_all().await?;
    drop(file);

    if !out.verified {
        crate::log_warn!("digest mismatch, removing temp file {}", temp.display());
        let _ = std::fs::remove_file(&temp);
        return Err(OrchestrateError::DigestMismatch);
    }

    // 校验通过才原子改名（safe_destination_path 负责剥目录分量 + 去重）
    let final_path = crate::transfer::safe_destination_path(dest_dir, &out.name)?;
    crate::log_info!(
        from = %temp.display().to_string(),
        to = %final_path.display().to_string(),
        "verified, renaming"
    );
    std::fs::rename(&temp, &final_path)?;
    std::mem::forget(guard);
    Ok(Receipt {
        path: final_path,
        sha256: out.sha256,
        size: out.size,
        kind: out.kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::cpace::Role;
    use crate::crypto::kdf::derive_wg_psk;
    use crate::transfer::{self, Kind};
    use crate::transport::WgConnection;
    use crate::transport::iface::Transport;
    use crate::transport::wireguard::transport::WireGuardTransport;

    /// 建一对已建立、可 listen/dial 的连接。
    async fn connected_pair() -> (WgConnection, WgConnection) {
        let mut tx = WireGuardTransport::new();
        let mut rx = WireGuardTransport::new();
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();
        let psk = derive_wg_psk(&[0x44u8; 32], 7);
        let tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();
        (tx_conn, rx_conn)
    }

    /// 终极 e2e：两个 WireGuardTransport + 传输内 HTTP，完整搬运一个文件并裁决。
    /// 覆盖 WireGuard 握手 → smoltcp TCP → HTTP → SHA-256 全链路。
    #[tokio::test]
    async fn end_to_end_file_transfer() {
        let (mut tx_conn, mut rx_conn) = connected_pair().await;

        let data = b"hermod end-to-end file transfer!".to_vec();
        let token = [0x11u8; 16];
        let sha = transfer::hash_bytes(&data);
        let opts = ServeOptions {
            token,
            kind: Kind::File,
            name: "hello.txt".into(),
            size: data.len() as u64,
            body: Box::new(std::io::Cursor::new(data.clone())),
            progress: None,
        };

        let mut dest = Vec::new();
        let (serve_out, fetch_out) = tokio::join!(
            serve_file(&mut tx_conn, opts),
            fetch_file(&mut rx_conn, &token, &mut dest, None),
        );

        let serve_out = serve_out.unwrap();
        let fetch_out = fetch_out.unwrap();
        assert_eq!(dest, data);
        assert_eq!(fetch_out.name, "hello.txt");
        assert_eq!(fetch_out.size, data.len() as u64);
        assert_eq!(fetch_out.sha256, sha);
        assert!(fetch_out.verified, "tx 应接受 rx 的摘要");
        assert!(serve_out.verified, "tx 本地裁决通过");
        assert_eq!(serve_out.sha256, sha, "tx 边发边算的摘要");
    }

    /// 真实文件 e2e：send_file → receive，落盘内容/SHA-256 一致。
    #[tokio::test]
    async fn end_to_end_real_file_transfer() {
        let (mut tx_conn, mut rx_conn) = connected_pair().await;

        let dir = std::env::temp_dir().join(format!("hermod-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("payload.bin");
        let data: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let dest_dir = dir.join("out");
        let token = [0x22u8; 16];

        let (serve_out, receipt) = tokio::join!(
            send_file(&mut tx_conn, token, &src, None),
            receive(&mut rx_conn, &token, &dest_dir, None),
        );
        let serve_out = serve_out.unwrap();
        let receipt = receipt.unwrap();

        assert_eq!(std::fs::read(&receipt.path).unwrap(), data);
        assert_eq!(receipt.sha256, transfer::hash_bytes(&data));
        assert_eq!(receipt.size, data.len() as u64);
        assert_eq!(receipt.path.file_name().unwrap(), "payload.bin");
        assert!(serve_out.verified);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 取消发生在 HTTP 传输过程中时，另一端必须得到 PeerCancelled 而不是裸 io 错误。
    #[tokio::test]
    async fn quic_peer_cancel_during_http_is_clean() {
        use crate::transport::quic::transport::QuicTransport;
        use std::net::{Ipv6Addr, SocketAddrV6};

        let mut tx = QuicTransport::new();
        let mut rx = QuicTransport::new();
        let bind = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0);
        tx.set_udp_bind(Some(bind));
        rx.set_udp_bind(Some(bind));
        tx.set_credential_fingerprint([0x91; 32]);
        rx.set_credential_fingerprint([0x91; 32]);
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();
        let psk = derive_wg_psk(&[0x92u8; 32], 13);
        let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let mut rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();

        let data = vec![0x5a_u8; 64 * 1024 * 1024];
        let token = [0x93u8; 16];
        let opts = ServeOptions {
            token,
            kind: Kind::File,
            name: "cancel.bin".into(),
            size: data.len() as u64,
            body: Box::new(std::io::Cursor::new(data)),
            progress: None,
        };
        let mut dest = Vec::new();
        let abort = rx_conn.abort_handle();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            abort.abort();
        });

        let (serve_out, _fetch_out) =
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                tokio::join!(
                    serve_file(&mut tx_conn, opts),
                    fetch_file(&mut rx_conn, &token, &mut dest, None),
                )
            })
            .await
            .expect("取消测试超时");
        assert!(
            matches!(serve_out, Err(OrchestrateError::PeerCancelled)),
            "tx 应得到 PeerCancelled，实际: {serve_out:?}"
        );
    }

    /// QUIC 全链路：QuicTransport + 传输内 HTTP + SHA-256 裁决。
    #[tokio::test]
    async fn quic_end_to_end_file_transfer() {
        use crate::transport::quic::transport::QuicTransport;
        use std::net::{Ipv6Addr, SocketAddrV6};

        let mut tx = QuicTransport::new();
        let mut rx = QuicTransport::new();
        let bind = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0);
        tx.set_udp_bind(Some(bind));
        rx.set_udp_bind(Some(bind));
        tx.set_credential_fingerprint([0x55; 32]);
        rx.set_credential_fingerprint([0x55; 32]);
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();
        let psk = derive_wg_psk(&[0x66u8; 32], 11);
        let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let mut rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();

        let data: Vec<u8> = (0..4 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
        let token = [0x77u8; 16];
        let sha = transfer::hash_bytes(&data);
        let opts = ServeOptions {
            token,
            kind: Kind::File,
            name: "quic.bin".into(),
            size: data.len() as u64,
            body: Box::new(std::io::Cursor::new(data.clone())),
            progress: None,
        };
        let mut dest = Vec::new();
        let (serve_out, fetch_out) =
            tokio::time::timeout(std::time::Duration::from_secs(60), async {
                tokio::join!(
                    serve_file(&mut tx_conn, opts),
                    fetch_file(&mut rx_conn, &token, &mut dest, None),
                )
            })
            .await
            .expect("QUIC HTTP 传输超时");

        let serve_out = serve_out.unwrap();
        let fetch_out = fetch_out.unwrap();
        assert_eq!(dest, data);
        assert_eq!(fetch_out.name, "quic.bin");
        assert_eq!(fetch_out.sha256, sha);
        assert!(fetch_out.verified);
        assert!(serve_out.verified);
        assert_eq!(serve_out.sha256, sha);

        tx_conn.shutdown().await.unwrap();
        rx_conn.shutdown().await.unwrap();
    }
}
