//! 传输内 HTTP 服务端（tx）：`GET /f/<token>` 发文件、`POST /done` 裁决。
//!
//! 对应 DEVELOPMENT.md §3.4/§3.6：tx 起 HTTP server，rx 一次 GET 拉完文件，
//! 再 POST /done 提交自己的 SHA-256；tx 比对后才允许改名。
//!
//! 一条传输流、keep-alive：GET 与 POST 复用同一条流。

use crate::bandwidth::{BandwidthLimit, Pacer, RECEIVE_LIMIT_HEADER};
use crate::http_meta;
use crate::transfer::{self, Kind};
use crate::transport::http::{HttpError, Request, write_response, write_response_head};
use sha2::{Digest, Sha256};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// 服务端配置。
pub struct ServeOptions {
    /// 一次性 URL token（128 位；tx 生成、经 URL 传给 rx）。
    pub token: [u8; 16],
    /// 载荷类型。
    pub kind: Kind,
    /// 原始文件名（`Kind::File` 时用于 Content-Disposition）。
    pub name: String,
    /// 载荷字节数（Content-Length）。
    pub size: u64,
    /// 载荷来源（文件 / 内存），边发边读，不整体进内存。
    pub body: Box<dyn AsyncRead + Unpin + Send>,
    /// 进度回调：每发一块调用一次，参数是**累计已发字节数**。
    pub progress: Option<Box<dyn FnMut(u64) + Send>>,
}

/// 服务一条连接的结果：rx 提交的摘要是否与本地一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOutcome {
    /// rx 提交的 SHA-256/大小是否与 tx 本地一致。
    pub verified: bool,
    /// tx 边发边算出的 SHA-256（十六进制小写）。
    pub sha256: String,
}

/// 服务一条 TCP 连接：一次 GET 拉文件 + 一次 POST /done 裁决，然后返回。
pub async fn serve<S>(stream: S, opts: ServeOptions) -> Result<ServeOutcome, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    serve_with_limit(stream, opts, BandwidthLimit::UNLIMITED).await
}

/// 限制写入传输层的速率，同时遵守接收端在加密请求中声明的预算。
pub async fn serve_with_limit<S>(
    stream: S,
    opts: ServeOptions,
    limit: BandwidthLimit,
) -> Result<ServeOutcome, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r);
    let path = http_meta::token_path(&opts.token);
    let mut body = opts.body;
    let mut progress = opts.progress;
    let mut computed_sha = String::new();

    loop {
        let req = match Request::read(&mut reader).await {
            Ok(req) => req,
            // rx 关闭连接（正常结束）→ 结束服务
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                return Err(HttpError::Malformed(e.to_string()));
            }
            Err(e) => return Err(e.into()),
        };

        crate::log_info!(method = %req.method, path = %req.path, "request received");
        match (req.method.as_str(), req.path.as_str()) {
            ("GET", p) if p == path => {
                let peer_limit = req
                    .header(RECEIVE_LIMIT_HEADER)
                    .map(str::parse::<u64>)
                    .transpose()
                    .map_err(|_| HttpError::Malformed("invalid receive bandwidth limit".into()))?
                    .map(BandwidthLimit::from_bytes_per_second)
                    .unwrap_or_default();
                let effective = limit.stricter(peer_limit);
                let pacer = Pacer::new(effective);
                crate::log_info!(
                    limit_mbps = effective.mbps(),
                    "payload bandwidth limit (0 = unlimited)"
                );
                let cd = http_meta::content_disposition(&opts.name)?;
                let kind = http_meta::x_hermod_kind_value(opts.kind);
                let headers = [
                    (http_meta::X_HERMOD_KIND, kind),
                    (http_meta::CONTENT_DISPOSITION, cd.as_str()),
                    ("Content-Type", "application/octet-stream"),
                ];
                crate::log_info!(size = opts.size, "sending body, hashing SHA-256 on the fly");
                write_response_head(&mut w, 200, "OK", &headers, opts.size).await?;

                // 流式发送 body + **边发边算** SHA-256（不整体进内存）
                let mut hasher = Sha256::new();
                let mut sent: u64 = 0;
                let mut remaining = opts.size;
                // 64 KiB：QUIC/TCP 下都减少 read/write 往返次数。
                let mut buf = [0u8; 64 * 1024];
                while remaining > 0 {
                    let want = (pacer.chunk_len() as u64).min(remaining) as usize;
                    let n = body.read(&mut buf[..want]).await?;
                    if n == 0 {
                        return Err(HttpError::Malformed(
                            "body shorter than Content-Length".into(),
                        ));
                    }
                    hasher.update(&buf[..n]);
                    pacer.wait(n).await;
                    w.write_all(&buf[..n]).await?;
                    remaining -= n as u64;
                    sent += n as u64;
                    if let Some(p) = progress.as_mut() {
                        p(sent);
                    }
                }
                w.flush().await?;
                computed_sha = transfer::hex(&hasher.finalize()[..]);
            }
            ("POST", "/done") => {
                let n = req.content_length().ok_or_else(|| {
                    HttpError::Malformed("POST /done missing content-length".into())
                })?;
                let mut body = vec![0u8; n];
                reader.read_exact(&mut body).await?;
                let meta = transfer::decode_metadata(&body)
                    .map_err(|e| HttpError::Malformed(e.to_string()))?;
                crate::log_info!(
                    rx_sha256 = %meta.sha256,
                    rx_size = meta.size,
                    local_sha256 = %computed_sha,
                    local_size = opts.size,
                    "received digest verdict"
                );
                let verified = meta.sha256 == computed_sha && meta.size as u64 == opts.size;
                let (status, reason) = if verified {
                    (200, "OK")
                } else {
                    (409, "Conflict")
                };
                write_response(&mut w, status, reason, &[], b"").await?;
                // 让传输层把最后的状态码真正送出去；QUIC 会等到对端 ACK 流数据。
                w.shutdown().await?;
                return Ok(ServeOutcome {
                    verified,
                    sha256: computed_sha,
                });
            }
            _ => {
                write_response(&mut w, 404, "Not Found", &[], b"").await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::http::client::fetch;

    /// 大容量 duplex 避免读端反压伪装成发送端限速；测量最后一次写入回调。
    #[tokio::test]
    async fn sender_paces_payload_and_honors_receiver_budget() {
        use crate::transport::http::client::fetch_with_limit;
        use std::{
            sync::{Arc, Mutex},
            time::{Duration, Instant},
        };
        for (tx_limit, rx_limit) in [("0.8", "0"), ("0", "0.8"), ("1.6", "0.8")] {
            let data = vec![0x42; 50_000];
            let final_write = Arc::new(Mutex::new(None));
            let observed = final_write.clone();
            let options = ServeOptions {
                token: [7; 16],
                kind: Kind::File,
                name: "paced.bin".into(),
                size: data.len() as u64,
                body: Box::new(std::io::Cursor::new(data.clone())),
                progress: Some(Box::new(move |n| {
                    if n == 50_000 {
                        *observed.lock().unwrap() = Some(Instant::now());
                    }
                })),
            };
            let (rx, tx) = tokio::io::duplex(1024 * 1024);
            let started = Instant::now();
            let mut dest = Vec::new();
            let (sent, received) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::try_join!(
                    serve_with_limit(tx, options, tx_limit.parse().unwrap()),
                    fetch_with_limit(rx, &[7; 16], &mut dest, None, rx_limit.parse().unwrap())
                )
            })
            .await
            .unwrap()
            .unwrap();
            assert!(
                final_write.lock().unwrap().unwrap().duration_since(started)
                    >= Duration::from_millis(500)
            );
            assert_eq!(dest, data);
            assert!(sent.verified && received.verified);
            assert_eq!(sent.sha256, received.sha256);
        }
    }

    /// 服务端 + 客户端（duplex）：GET 拉文件 + POST /done 裁决通过。
    #[tokio::test]
    async fn serve_and_fetch_roundtrip() {
        let data = b"hello in-tunnel http".to_vec();
        let token = [0x7au8; 16];
        let sha = transfer::hash_bytes(&data);
        let opts = ServeOptions {
            token,
            kind: Kind::File,
            name: "greeting.txt".into(),
            size: data.len() as u64,
            body: Box::new(std::io::Cursor::new(data.clone())),
            progress: None,
        };
        let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let sent2 = sent.clone();
        let mut opts = opts;
        opts.progress = Some(Box::new(move |n| {
            sent2.store(n, std::sync::atomic::Ordering::SeqCst);
        }));

        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(serve(server_io, opts));

        let mut dest = Vec::new();
        let mut got: u64 = 0;
        let out = fetch(client_io, &token, &mut dest, Some(&mut |n| got = n))
            .await
            .unwrap();
        assert_eq!(dest, data);
        assert_eq!(out.kind, Kind::File);
        assert_eq!(out.name, "greeting.txt");
        assert_eq!(out.size, data.len() as u64);
        assert_eq!(out.sha256, sha);
        assert!(out.verified, "rx 的摘要应被 tx 接受");

        let outcome = server.await.unwrap().unwrap();
        assert!(outcome.verified);
        assert_eq!(outcome.sha256, sha, "tx 边发边算的摘要应与期望一致");
        assert_eq!(
            sent.load(std::sync::atomic::Ordering::SeqCst),
            data.len() as u64
        );
        assert_eq!(got, data.len() as u64);
    }

    /// 错误的 token → 404，客户端报错。
    #[tokio::test]
    async fn wrong_token_is_404() {
        let data = b"x".to_vec();
        let opts = ServeOptions {
            token: [1u8; 16],
            kind: Kind::File,
            name: "a".into(),
            size: 1,
            body: Box::new(std::io::Cursor::new(data)),
            progress: None,
        };
        let (client_io, server_io) = tokio::io::duplex(4096);
        let _server = tokio::spawn(serve(server_io, opts));
        let mut dest = Vec::new();
        let err = fetch(client_io, &[2u8; 16], &mut dest, None)
            .await
            .unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)));
    }
}
