//! 传输编排（协议 v2）：把「传输连接」与「直连流帧」组合成 tx / rx 两个动作。
//!
//! 帧（跑在一条 QUIC 双向流上；载荷长度与文件名来自端点交换阶段的
//! [`TransferOffer`]，流内不再承载元数据）：
//!
//! - rx（`dial`/`open_bi`，往 tx 方向）先写 **1 字节 GO（0x01）**，让 tx 侧
//!   `listen`/`accept_bi` 尽快返回（QUIC 服务器在收到首字节之前不会同意流）；
//! - tx 读 GO → 精确写 `size` 字节载荷（边写边算 SHA-256）；
//! - rx 读满 `size` 字节（边读边写 dest，边算 SHA-256）→ 写裁决帧：
//!   `[status: u8][sha256: 32][size: u64 LE]`，status=0 表示本地核算通过；
//! - tx 读裁决后返回 [`ServeOutcome`];rx 返回 [`Receipt`]
//!   （临时文件 → 校验 → 原子改名仍由 rx 侧完成）。
//!
//! 对端可控的 offer 早经 [`TransferOffer::validate`]（含文件名卫生）校验；
//! 对端取消统一映射为 [`OrchestrateError::PeerCancelled`]。

use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::transfer::TransferOffer;
use sha2::{Digest, Sha256};

/// rx 首字节：同意接收（触发 tx 的 accept_bi）。
const GO: u8 = 0x01;
/// 裁决状态：核算通过。
const VERDICT_OK: u8 = 0;
/// 裁决帧字节长度：status(1) + sha256(32) + size(u64 LE)。
const VERDICT_LEN: usize = 1 + 32 + 8;
/// 载荷搬运块大小（QUIC/TCP 下都减少 read/write 往返次数）。
const CHUNK: usize = 64 * 1024;

/// 编排错误：传输层、IO、元数据或裁决。
#[derive(Debug, thiserror::Error)]
pub enum OrchestrateError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("io: {0}")]
    Io(io::Error),
    #[error("transfer: {0}")]
    Transfer(#[from] crate::transfer::TransferError),
    #[error("digest mismatch: rx 提交的核算结果未被 tx 接受")]
    DigestMismatch,
    /// 对端中止了连接（Ctrl+C / CONNECTION_CLOSE / 提前 EOF）。
    #[error("peer cancelled the transfer")]
    PeerCancelled,
    #[error("protocol violation: {0}")]
    Protocol(String),
}

/// 把 IO 错误归类：显式取消标记或常见断连都按"对端取消"呈现。
fn classify_io(error: io::Error) -> OrchestrateError {
    if crate::transport::iface::is_peer_cancelled(&error) {
        return OrchestrateError::PeerCancelled;
    }
    match error.kind() {
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::UnexpectedEof => OrchestrateError::PeerCancelled,
        _ => OrchestrateError::Io(error),
    }
}

fn hex(bytes: &[u8]) -> String {
    crate::transfer::hex(bytes)
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

/// tx 侧载荷来源与元数据（size/name/kind 必须真实，且已经
/// [`TransferOffer::validate`] 校验过——session 层在端点交换时做的）。
pub struct ServeOptions {
    /// 载荷类型（回执与目录展示用）。
    pub kind: crate::transfer::Kind,
    /// 原始文件名（对端落盘时使用）。
    pub name: String,
    /// 将写入载荷的精确字节数（与 offer 一致）。
    pub size: u64,
    /// 载荷来源（文件 / 内存），边发边读，不整体进内存。
    pub body: Box<dyn AsyncRead + Unpin + Send>,
    /// 进度回调：每发一块调用一次，参数是**累计已发字节数**。
    pub progress: Option<Box<dyn FnMut(u64) + Send>>,
}

/// 服务一条连接的结果：rx 裁决是否与本地一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOutcome {
    /// rx 提交的 SHA-256/大小是否与 tx 本地一致。
    pub verified: bool,
    /// tx 边发边算出的 SHA-256（十六进制小写）。
    pub sha256: String,
}

/// rx 侧接收参数。
#[derive(Default)]
pub struct ReceiveOptions<'a> {
    /// 端点交换阶段宣告过的载荷（提供 size/name/kind；v2 下必填）。
    pub offer: Option<&'a TransferOffer>,
    /// 进度回调：每读一块调用一次，参数是**累计已收字节数**。
    pub progress: Option<&'a mut (dyn FnMut(u64) + Send)>,
}

/// tx：在传输内监听、写完载荷并读完裁决。
///
/// 对应 [`send_file`]/[`send_text`] 的组合体；供自定义载荷来源使用。
pub async fn serve_file<C>(
    conn: &mut C,
    opts: ServeOptions,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    if let Err(error) = crate::transfer::validate_filename(&opts.name) {
        return Err(OrchestrateError::Protocol(error.to_string()));
    }
    let stream = conn
        .listen()
        .await
        .map_err(|e| OrchestrateError::Transport(e.to_string()))?;
    serve(stream, opts).await
}

async fn serve<S>(mut stream: S, mut opts: ServeOptions) -> Result<ServeOutcome, OrchestrateError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    // 1) GO 帧
    let mut go = [0u8; 1];
    stream.read_exact(&mut go).await.map_err(classify_io)?;
    if go[0] != GO {
        return Err(OrchestrateError::Protocol(format!(
            "unexpected start byte {:#04x}",
            go[0]
        )));
    }

    // 2) 精确写 size 字节载荷，边发边算 SHA-256
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    let mut remaining = opts.size;
    let mut buf = vec![0u8; CHUNK];
    let mut body = std::mem::replace(&mut opts.body, Box::new(tokio::io::empty()));
    while remaining > 0 {
        let want = buf.len().min(remaining as usize);
        let n = body.read(&mut buf[..want]).await.map_err(classify_io)?;
        if n == 0 {
            return Err(OrchestrateError::Protocol(
                "payload shorter than announced size".into(),
            ));
        }
        hasher.update(&buf[..n]);
        stream.write_all(&buf[..n]).await.map_err(classify_io)?;
        written += n as u64;
        remaining -= n as u64;
        if let Some(progress) = opts.progress.as_mut() {
            progress(written);
        }
    }
    stream.flush().await.map_err(classify_io)?;
    let mut local_sha_bytes = [0u8; 32];
    local_sha_bytes.copy_from_slice(&hasher.finalize());
    let local_sha = hex(&local_sha_bytes);

    // 3) 读裁决：status(1) + sha256(32) + size(8)
    let mut verdict = [0u8; VERDICT_LEN];
    stream.read_exact(&mut verdict).await.map_err(classify_io)?;
    let verified = verdict[0] == VERDICT_OK
        && verdict[1..33] == local_sha_bytes
        && u64::from_le_bytes(verdict[33..41].try_into().unwrap()) == opts.size;
    Ok(ServeOutcome {
        verified,
        sha256: local_sha,
    })
}

/// rx 方向的帧协议本体：GO → 读满 offer.size → 裁决。
/// 供 [`receive`]（落盘到临时文件）与测试使用。
pub async fn fetch_with_options<S, D>(
    mut stream: S,
    offer: &TransferOffer,
    dest: &mut D,
    mut progress: Option<&mut (dyn FnMut(u64) + Send)>,
) -> Result<(String, bool), OrchestrateError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    D: AsyncWrite + Unpin,
{
    // 1) GO 帧
    stream.write_all(&[GO]).await.map_err(classify_io)?;

    // 2) 精确读 offer.size 字节，边读边写、边算 SHA-256
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut remaining = offer.size;
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        let want = buf.len().min(remaining as usize);
        let n = stream.read(&mut buf[..want]).await.map_err(classify_io)?;
        if n == 0 {
            return Err(OrchestrateError::Protocol(
                "payload shorter than announced size".into(),
            ));
        }
        dest.write_all(&buf[..n]).await.map_err(classify_io)?;
        hasher.update(&buf[..n]);
        received += n as u64;
        remaining -= n as u64;
        if let Some(progress) = progress.as_mut() {
            progress(received);
        }
    }
    dest.flush().await.map_err(classify_io)?;
    let mut local_sha_bytes = [0u8; 32];
    local_sha_bytes.copy_from_slice(&hasher.finalize());
    let local_sha = hex(&local_sha_bytes);

    // 3) 裁决帧：status + sha256 + size（诚实 rx：按本地核算提交）
    let mut verdict = [0u8; VERDICT_LEN];
    verdict[0] = VERDICT_OK;
    verdict[1..33].copy_from_slice(&local_sha_bytes);
    verdict[33..41].copy_from_slice(&received.to_le_bytes());
    stream.write_all(&verdict).await.map_err(classify_io)?;
    stream.shutdown().await.map_err(classify_io)?;
    Ok((local_sha, received == offer.size))
}

/// tx：把本地文件经传输发给 rx（边发边算 SHA-256）。
pub async fn send_file<C>(
    conn: &mut C,
    path: &std::path::Path,
    progress: Option<Box<dyn FnMut(u64) + Send>>,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    let file = tokio::fs::File::open(path).await.map_err(classify_io)?;
    let size = file.metadata().await.map_err(classify_io)?.len();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    crate::log_info!(path = %path.display().to_string(), size, "sending file");
    serve_file(
        conn,
        ServeOptions {
            kind: crate::transfer::Kind::File,
            name,
            size,
            body: Box::new(file),
            progress,
        },
    )
    .await
}

/// tx：把一段文本经传输发给 rx（`Kind::Text`）。
pub async fn send_text<C>(
    conn: &mut C,
    text: &str,
    progress: Option<Box<dyn FnMut(u64) + Send>>,
) -> Result<ServeOutcome, OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    let bytes = text.as_bytes().to_vec();
    serve_file(
        conn,
        ServeOptions {
            kind: crate::transfer::Kind::Text,
            name: "message.txt".to_string(),
            size: bytes.len() as u64,
            body: Box::new(std::io::Cursor::new(bytes)),
            progress,
        },
    )
    .await
}

/// 临时文件 RAII 清理：Drop 时删除（含 future 被取消/panic 的情况）。
struct TempGuard(std::path::PathBuf);

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// rx：按 offer 收满字节数 → 临时文件 → 核算 → 原子改名。
pub async fn receive<C>(
    conn: &mut C,
    dest_dir: &std::path::Path,
    options: ReceiveOptions<'_>,
) -> Result<Receipt, OrchestrateError>
where
    C: crate::transport::TransportConnection,
{
    let offer = options.offer.ok_or_else(|| {
        OrchestrateError::Protocol(
            "payload size unknown (the sender never announced a transfer offer)".into(),
        )
    })?;
    offer.validate().map_err(OrchestrateError::Protocol)?;
    std::fs::create_dir_all(dest_dir).map_err(classify_io)?;
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
    let std_file = crate::transfer::create_temp(&temp).map_err(classify_io)?;
    // 守卫在函数返回/被取消时删除临时文件；成功改名后 forget 掉。
    let guard = TempGuard(temp.clone());
    let mut file = tokio::fs::File::from_std(std_file);

    let stream = conn
        .dial()
        .await
        .map_err(|e| OrchestrateError::Transport(e.to_string()))?;
    let (sha256, verified) =
        match fetch_with_options(stream, offer, &mut file, options.progress).await {
            Ok((sha256, verified)) => (sha256, verified),
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                return Err(e);
            }
        };
    file.sync_all().await.map_err(classify_io)?;
    drop(file);

    if !verified {
        crate::log_warn!(
            "size mismatch vs announced offer, removing temp file {}",
            temp.display()
        );
        let _ = std::fs::remove_file(&temp);
        return Err(OrchestrateError::DigestMismatch);
    }

    // 校验通过才原子改名（safe_destination_path 负责剥目录分量 + 去重）
    let final_path = crate::transfer::safe_destination_path(dest_dir, &offer.name)?;
    crate::log_info!(
        from = %temp.display().to_string(),
        to = %final_path.display().to_string(),
        "verified, renaming"
    );
    std::fs::rename(&temp, &final_path).map_err(classify_io)?;
    std::mem::forget(guard);
    Ok(Receipt {
        path: final_path,
        sha256,
        size: offer.size,
        kind: offer.kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::cpace::Role;
    use crate::crypto::kdf::derive_wg_psk;
    use crate::transport::QuicConnection;
    use crate::transport::iface::Transport;
    use crate::transport::quic::transport::QuicTransport;
    use std::net::{Ipv6Addr, SocketAddrV6};

    /// 建一对已建立、可 listen/dial 的连接（QUIC 回环）。
    async fn connected_pair() -> (QuicConnection, QuicConnection) {
        let mut tx = QuicTransport::new();
        let mut rx = QuicTransport::new();
        let bind: std::net::SocketAddr = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0).into();
        tx.set_udp_bind(Some(bind));
        rx.set_udp_bind(Some(bind));
        tx.set_credential_fingerprint([0x66; 32]);
        rx.set_credential_fingerprint([0x66; 32]);
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();
        let psk = derive_wg_psk(&[0x44u8; 32], 7);
        let tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();
        (tx_conn, rx_conn)
    }

    /// 终极 e2e：两个 QuicTransport + 直连流帧，完整搬运一个文件并裁决。
    #[tokio::test]
    async fn end_to_end_file_transfer() {
        let (mut tx_conn, mut rx_conn) = connected_pair().await;
        let data = b"hermod end-to-end file transfer!".to_vec();
        let offer = TransferOffer {
            kind: crate::transfer::Kind::File,
            name: "hello.txt".into(),
            size: data.len() as u64,
        };

        let dir = std::env::temp_dir().join(format!("hermod-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest_dir = dir.join("out");

        let (serve_out, receipt) = tokio::join!(
            async {
                serve_file(
                    &mut tx_conn,
                    ServeOptions {
                        kind: crate::transfer::Kind::File,
                        name: offer.name.clone(),
                        size: offer.size,
                        body: Box::new(std::io::Cursor::new(data.clone())),
                        progress: None,
                    },
                )
                .await
                .unwrap()
            },
            async {
                receive(
                    &mut rx_conn,
                    &dest_dir,
                    ReceiveOptions {
                        offer: Some(&offer),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
            },
        );

        assert!(serve_out.verified);
        assert_eq!(std::fs::read(&receipt.path).unwrap(), data);
        assert_eq!(receipt.sha256, crate::transfer::hash_bytes(&data));
        assert_eq!(receipt.path.file_name().unwrap(), "hello.txt");
        let _ = std::fs::remove_dir_all(&dir);
        crate::transport::iface::TransportConnection::shutdown(&mut tx_conn)
            .await
            .unwrap();
        crate::transport::iface::TransportConnection::shutdown(&mut rx_conn)
            .await
            .unwrap();
    }
}
