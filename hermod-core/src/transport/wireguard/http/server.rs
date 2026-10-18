//! 隧道内 HTTP 服务端（tx）：`GET /f/<token>` 发文件、`POST /done` 裁决。
//!
//! 对应 DEVELOPMENT.md §3.4/§3.6：tx 起 HTTP server，rx 一次 GET 拉完文件，
//! 再 POST /done 提交自己的 SHA-256；tx 比对后才允许改名。
//!
//! 一条 TCP 连接、keep-alive：GET 与 POST 复用同一条连接。

use crate::http_meta;
use crate::transport::wireguard::http::{
    HttpError, Request, write_response, write_response_head,
};
use crate::transfer::{self, Kind};
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
    /// tx 预先算好的 SHA-256（十六进制小写）。
    pub sha256: String,
    /// 载荷来源（文件 / 内存），边发边读，不整体进内存。
    pub body: Box<dyn AsyncRead + Unpin + Send>,
}

/// 服务一条连接的结果：rx 提交的摘要是否与本地一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServeOutcome {
    /// rx 提交的 SHA-256/大小是否与 tx 本地一致。
    pub verified: bool,
}

/// 服务一条 TCP 连接：一次 GET 拉文件 + 一次 POST /done 裁决，然后返回。
pub async fn serve<S>(stream: S, opts: ServeOptions) -> Result<ServeOutcome, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r);
    let path = http_meta::token_path(&opts.token);
    let mut body = opts.body;

    loop {
        let req = match Request::read(&mut reader).await {
            Ok(req) => req,
            // rx 关闭连接（正常结束）→ 结束服务
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(HttpError::Io(e));
            }
            Err(e) => return Err(HttpError::Malformed(e.to_string())),
        };

        match (req.method.as_str(), req.path.as_str()) {
            ("GET", p) if p == path => {
                let cd = http_meta::content_disposition(&opts.name)?;
                let kind = http_meta::x_hermod_kind_value(opts.kind);
                let headers = [
                    (http_meta::X_HERMOD_KIND, kind),
                    (http_meta::CONTENT_DISPOSITION, cd.as_str()),
                    ("Content-Type", "application/octet-stream"),
                ];
                write_response_head(&mut w, 200, "OK", &headers, opts.size).await?;

                // 流式发送 body（不整体进内存）
                let mut remaining = opts.size;
                let mut buf = [0u8; 16 * 1024];
                while remaining > 0 {
                    let want = (buf.len() as u64).min(remaining) as usize;
                    let n = body.read(&mut buf[..want]).await?;
                    if n == 0 {
                        return Err(HttpError::Malformed("body shorter than Content-Length".into()));
                    }
                    w.write_all(&buf[..n]).await?;
                    remaining -= n as u64;
                }
                w.flush().await?;
            }
            ("POST", "/done") => {
                let n = req
                    .content_length()
                    .ok_or_else(|| HttpError::Malformed("POST /done missing content-length".into()))?;
                let mut body = vec![0u8; n];
                reader.read_exact(&mut body).await?;
                let meta = transfer::decode_metadata(&body)
                    .map_err(|e| HttpError::Malformed(e.to_string()))?;
                let verified = meta.sha256 == opts.sha256 && meta.size as u64 == opts.size;
                let (status, reason) = if verified {
                    (200, "OK")
                } else {
                    (409, "Conflict")
                };
                write_response(&mut w, status, reason, &[], b"").await?;
                return Ok(ServeOutcome { verified });
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
    use crate::transport::wireguard::http::client::fetch;

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
            sha256: sha.clone(),
            body: Box::new(std::io::Cursor::new(data.clone())),
        };
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(serve(server_io, opts));

        let mut dest = Vec::new();
        let out = fetch(client_io, &token, &mut dest).await.unwrap();
        assert_eq!(dest, data);
        assert_eq!(out.kind, Kind::File);
        assert_eq!(out.name, "greeting.txt");
        assert_eq!(out.size, data.len() as u64);
        assert_eq!(out.sha256, sha);
        assert!(out.verified, "rx 的摘要应被 tx 接受");

        let outcome = server.await.unwrap().unwrap();
        assert!(outcome.verified);
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
            sha256: transfer::hash_bytes(&data),
            body: Box::new(std::io::Cursor::new(data)),
        };
        let (client_io, server_io) = tokio::io::duplex(4096);
        let _server = tokio::spawn(serve(server_io, opts));
        let mut dest = Vec::new();
        let err = fetch(client_io, &[2u8; 16], &mut dest).await.unwrap_err();
        assert!(matches!(err, HttpError::Malformed(_)));
    }
}
