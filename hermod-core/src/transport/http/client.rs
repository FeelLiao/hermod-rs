//! 传输内 HTTP 客户端（rx）：一次 GET 拉文件 + `POST /done` 提交裁决。
//!
//! 对应 DEVELOPMENT.md §3.4/§3.6：rx 拿一次性 token 发 `GET /f/<token>`，
//! 边收边算 SHA-256，把 body 写入目标；随后 `POST /done` 提交
//! `{"sha256":..., "size":N}`，由 tx 比对裁决。

use crate::bandwidth::{BandwidthLimit, Pacer, RECEIVE_LIMIT_HEADER};
use crate::http_meta;
use crate::transfer::{self, Kind};
use crate::transport::http::{HttpError, ResponseHead, write_request};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// 拉取结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOutcome {
    /// 载荷类型（来自 `X-Hermod-Kind`）。
    pub kind: Kind,
    /// 原始文件名（来自 `Content-Disposition`）。
    pub name: String,
    /// 载荷字节数。
    pub size: u64,
    /// rx 本地算出的 SHA-256（十六进制小写）。
    pub sha256: String,
    /// tx 是否接受 rx 提交的摘要（POST /done 返回 200）。
    pub verified: bool,
}

/// 用一次性 token 从传输内的 tx 拉取文件并提交裁决。
pub async fn fetch<S, D>(
    stream: S,
    token: &[u8; 16],
    dest: &mut D,
    progress: Option<&mut (dyn FnMut(u64) + Send)>,
) -> Result<FetchOutcome, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    D: AsyncWrite + Unpin,
{
    fetch_with_limit(stream, token, dest, progress, BandwidthLimit::UNLIMITED).await
}

/// 向发送端声明接收预算；本地读端限速也兼容不认识新请求头的旧发送端。
pub async fn fetch_with_limit<S, D>(
    stream: S,
    token: &[u8; 16],
    dest: &mut D,
    mut progress: Option<&mut (dyn FnMut(u64) + Send)>,
    limit: BandwidthLimit,
) -> Result<FetchOutcome, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    D: AsyncWrite + Unpin,
{
    let pacer = Pacer::new(limit);
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r);
    let path = http_meta::token_path(token);

    // 1) GET /f/<token>
    crate::log_info!(path = %path, "GET");
    let receive_limit = limit.bytes_per_second().to_string();
    write_request(
        &mut w,
        "GET",
        &path,
        &[("Host", "hermod"), (RECEIVE_LIMIT_HEADER, &receive_limit)],
        &[],
    )
    .await?;
    let head = ResponseHead::read(&mut reader).await?;
    if head.status != 200 {
        return Err(HttpError::Malformed(format!(
            "GET {path} status {}",
            head.status
        )));
    }
    let kind = head
        .header(http_meta::X_HERMOD_KIND)
        .and_then(http_meta::parse_x_hermod_kind)
        .ok_or_else(|| HttpError::Malformed("missing/invalid X-Hermod-Kind".into()))?;
    let name = head
        .header(http_meta::CONTENT_DISPOSITION)
        .and_then(http_meta::parse_content_disposition)
        .ok_or_else(|| HttpError::Malformed("missing/invalid Content-Disposition".into()))?;
    let size =
        head.content_length()
            .ok_or_else(|| HttpError::Malformed("missing Content-Length".into()))? as u64;
    crate::log_info!(status = head.status, size, filename = %name, "response headers");

    // 2) 边收边算 SHA-256，写入目标
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut remaining = size;
    // 64 KiB：QUIC/TCP 下都减少 read/write 往返次数。
    let mut buf = [0u8; 64 * 1024];
    let mut last_log = std::time::Instant::now();
    let mut last_bytes = 0u64;
    let mut write_time = std::time::Duration::ZERO;
    let started = std::time::Instant::now();
    while remaining > 0 {
        let want = (pacer.chunk_len() as u64).min(remaining) as usize;
        let n = reader.read(&mut buf[..want]).await?;
        if n == 0 {
            return Err(HttpError::Malformed(
                "body shorter than Content-Length".into(),
            ));
        }
        pacer.wait(n).await;
        hasher.update(&buf[..n]);
        let w = std::time::Instant::now();
        dest.write_all(&buf[..n]).await?;
        write_time += w.elapsed();
        remaining -= n as u64;
        received += n as u64;
        if let Some(p) = progress.as_mut() {
            p(received);
        }
        let elapsed = last_log.elapsed().as_secs_f64();
        if elapsed >= 2.0 {
            let mib = received as f64 / 1_048_576.0;
            let rate = (received - last_bytes) as f64 / elapsed / 1_048_576.0;
            let total = started.elapsed().as_secs_f64();
            let write_ratio = if total > 0.0 {
                write_time.as_secs_f64() / total * 100.0
            } else {
                0.0
            };
            crate::log_info!(
                received_mib = format!("{mib:.1}"),
                rate_mib_s = format!("{rate:.1}"),
                write_pct = format!("{write_ratio:.1}"),
                "receive progress"
            );
            last_log = std::time::Instant::now();
            last_bytes = received;
        }
    }
    dest.flush().await?;
    let sha256 = transfer::hex(&hasher.finalize()[..]);

    // 3) POST /done 提交裁决
    crate::log_info!("body received, submitting verdict");
    let meta = transfer::Metadata {
        kind,
        name: name.clone(),
        size: size as i64,
        sha256: sha256.clone(),
    };
    let body = transfer::encode_metadata(&meta).map_err(|e| HttpError::Malformed(e.to_string()))?;
    write_request(&mut w, "POST", "/done", &[], &body).await?;
    let verdict = ResponseHead::read(&mut reader).await?;
    if verdict.status != 200 && verdict.status != 409 {
        return Err(HttpError::Malformed(format!(
            "POST /done status {}",
            verdict.status
        )));
    }

    // 主动发送流结束；QUIC 会等到 tx 确认收到全部数据。
    w.shutdown().await?;
    crate::log_info!(verified = verdict.status == 200, "verdict");
    Ok(FetchOutcome {
        kind,
        name,
        size,
        sha256,
        verified: verdict.status == 200,
    })
}
