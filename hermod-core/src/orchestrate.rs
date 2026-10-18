//! 传输编排（M4.4）：把「传输连接」与「隧道内 HTTP」组合成 tx / rx 两个动作。
//!
//! - tx：[`serve_file`] 先 `listen`，再用 HTTP server 发文件并收裁决；
//! - rx：[`fetch_file`] 先 `dial`，再用 HTTP client 拉文件并提交 SHA-256。
//!
//! 上层（CLI / GUI）只需提供 `ServeOptions` 与目标写入端，不关心底层是
//! WireGuard 还是其它传输。

use crate::transport::iface::TransportConnection;
use crate::transport::wireguard::http::HttpError;
use crate::transport::wireguard::http::client::{FetchOutcome, fetch};
use crate::transport::wireguard::http::server::{ServeOptions, ServeOutcome, serve};
use crate::transport::wireguard::transport::{WgConnection, WgError};
use tokio::io::AsyncWrite;

/// 编排错误：传输层或 HTTP 层。
#[derive(Debug, thiserror::Error)]
pub enum OrchestrateError {
    #[error("transport: {0}")]
    Transport(#[from] WgError),
    #[error("http: {0}")]
    Http(#[from] HttpError),
}

/// tx：在隧道内监听，服务一次 `GET /f/<token>` + `POST /done`。
pub async fn serve_file(
    conn: &mut WgConnection,
    opts: ServeOptions,
) -> Result<ServeOutcome, OrchestrateError> {
    let stream = conn.listen().await?;
    Ok(serve(stream, opts).await?)
}

/// rx：向隧道内 tx 拨号，拉文件到 `dest` 并提交裁决。
pub async fn fetch_file<D: AsyncWrite + Unpin>(
    conn: &mut WgConnection,
    token: &[u8; 16],
    dest: &mut D,
) -> Result<FetchOutcome, OrchestrateError> {
    let stream = conn.dial().await?;
    Ok(fetch(stream, token, dest).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::cpace::Role;
    use crate::crypto::kdf::derive_wg_psk;
    use crate::transport::iface::Transport;
    use crate::transport::wireguard::transport::WireGuardTransport;
    use crate::transfer::{self, Kind};

    /// 终极 e2e：两个 WireGuardTransport + 隧道内 HTTP，完整搬运一个文件并裁决。
    /// 覆盖 WireGuard 握手 → smoltcp TCP → HTTP → SHA-256 全链路。
    #[tokio::test]
    async fn end_to_end_file_transfer() {
        let mut tx = WireGuardTransport::new();
        let mut rx = WireGuardTransport::new();
        let tx_ep = tx.prepare().await.unwrap();
        let rx_ep = rx.prepare().await.unwrap();

        let psk = derive_wg_psk(&[0x44u8; 32], 7);
        let mut tx_conn = tx.establish(Role::Sender, &psk, &rx_ep).await.unwrap();
        let mut rx_conn = rx.establish(Role::Receiver, &psk, &tx_ep).await.unwrap();

        let data = b"hermod end-to-end file transfer!".to_vec();
        let token = [0x11u8; 16];
        let sha = transfer::hash_bytes(&data);
        let opts = ServeOptions {
            token,
            kind: Kind::File,
            name: "hello.txt".into(),
            size: data.len() as u64,
            sha256: sha.clone(),
            body: Box::new(std::io::Cursor::new(data.clone())),
        };

        let mut dest = Vec::new();
        let (serve_out, fetch_out) = tokio::join!(
            serve_file(&mut tx_conn, opts),
            fetch_file(&mut rx_conn, &token, &mut dest),
        );

        let serve_out = serve_out.unwrap();
        let fetch_out = fetch_out.unwrap();
        assert_eq!(dest, data);
        assert_eq!(fetch_out.name, "hello.txt");
        assert_eq!(fetch_out.size, data.len() as u64);
        assert_eq!(fetch_out.sha256, sha);
        assert!(fetch_out.verified, "tx 应接受 rx 的摘要");
        assert!(serve_out.verified, "tx 本地裁决通过");
    }
}
