//! 隧道内 HTTP/1.1（M4.3）：极简请求/响应编解码。
//!
//! 对应 DEVELOPMENT.md §3.4/§3.6（D11）：tx 起 HTTP server，rx 用一次
//! `GET /f/<token>` 拉文件，再 `POST /done` 回裁决。这里手写极简解析
//! （S4 要求自研头解析限长度），不引入 hyper。
//!
//! - [`Request`] / [`ResponseHead`]：从 `AsyncBufRead` 读请求行/状态行 + 头；
//! - [`write_request`] / [`write_response`]：写请求/响应（自动补 Content-Length）。

use std::io;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub mod client;
pub mod server;

/// 头部最大字节数（防恶意长度）。
pub const MAX_HEADER_BYTES: usize = 64 * 1024;
/// 头部最大行数。
pub const MAX_HEADER_LINES: usize = 200;

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// 从 `AsyncBufRead` 读一行（返回去掉行尾的字符串），并累计字节数。
async fn read_line_bounded<R: AsyncBufRead + Unpin>(
    r: &mut R,
    total: &mut usize,
) -> io::Result<Option<String>> {
    let mut line = String::new();
    let n = r.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    *total += n;
    if *total > MAX_HEADER_BYTES {
        return Err(invalid("header too large"));
    }
    Ok(Some(
        line.trim_end_matches(['\r', '\n']).to_string(),
    ))
}

/// 解析出的 HTTP 请求头（body 由调用方按 `Content-Length` 读）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Request {
    /// 取某头（大小写不敏感）。
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Content-Length（缺失/非法返回 None）。
    pub fn content_length(&self) -> Option<usize> {
        self.header("content-length")
            .and_then(|v| v.trim().parse().ok())
    }

    /// 从 reader 读一个请求（请求行 + 头，遇到空行结束）。
    pub async fn read<R: AsyncBufRead + Unpin>(r: &mut R) -> io::Result<Request> {
        let mut total = 0usize;
        let line = read_line_bounded(r, &mut total)
            .await?
            .ok_or_else(|| invalid("empty request"))?;
        let mut parts = line.split_whitespace();
        let method = parts
            .next()
            .ok_or_else(|| invalid("missing method"))?
            .to_string();
        let path = parts
            .next()
            .ok_or_else(|| invalid("missing path"))?
            .to_string();
        let version = parts.next().ok_or_else(|| invalid("missing version"))?;
        if !version.starts_with("HTTP/1.") {
            return Err(invalid("unsupported HTTP version"));
        }

        let mut headers = Vec::new();
        while let Some(line) = read_line_bounded(r, &mut total).await? {
            if line.is_empty() {
                break; // 头结束
            }
            if headers.len() >= MAX_HEADER_LINES {
                return Err(invalid("too many headers"));
            }
            match line.split_once(':') {
                Some((k, v)) => headers.push((k.trim().to_string(), v.trim().to_string())),
                None => return Err(invalid("malformed header")),
            }
        }
        Ok(Request {
            method,
            path,
            headers,
        })
    }
}

/// 解析出的 HTTP 响应头。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl ResponseHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn content_length(&self) -> Option<usize> {
        self.header("content-length")
            .and_then(|v| v.trim().parse().ok())
    }

    /// 从 reader 读响应状态行 + 头。
    pub async fn read<R: AsyncBufRead + Unpin>(r: &mut R) -> io::Result<ResponseHead> {
        let mut total = 0usize;
        let line = read_line_bounded(r, &mut total)
            .await?
            .ok_or_else(|| invalid("empty response"))?;
        let mut parts = line.split_whitespace();
        let version = parts.next().ok_or_else(|| invalid("missing version"))?;
        if !version.starts_with("HTTP/1.") {
            return Err(invalid("unsupported HTTP version"));
        }
        let status: u16 = parts
            .next()
            .ok_or_else(|| invalid("missing status"))?
            .parse()
            .map_err(|_| invalid("bad status"))?;
        let mut headers = Vec::new();
        while let Some(line) = read_line_bounded(r, &mut total).await? {
            if line.is_empty() {
                break;
            }
            if headers.len() >= MAX_HEADER_LINES {
                return Err(invalid("too many headers"));
            }
            match line.split_once(':') {
                Some((k, v)) => headers.push((k.trim().to_string(), v.trim().to_string())),
                None => return Err(invalid("malformed header")),
            }
        }
        Ok(ResponseHead { status, headers })
    }
}

/// 写一个 HTTP 请求（带可选 body，自动补 Content-Length）。
pub async fn write_request<W: AsyncWrite + Unpin>(
    w: &mut W,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> io::Result<()> {
    let mut head = format!("{method} {path} HTTP/1.1\r\n");
    for (k, v) in headers {
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
    {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        w.write_all(body).await?;
    }
    w.flush().await
}

/// 只写响应头（用于流式 body：由调用方随后的字节填充 Content-Length）。
pub async fn write_response_head<W: AsyncWrite + Unpin>(
    w: &mut W,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    content_length: u64,
) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (k, v) in headers {
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    head.push_str(&format!("Content-Length: {content_length}\r\n\r\n"));
    w.write_all(head.as_bytes()).await?;
    w.flush().await
}

/// HTTP 层错误（编解码 / IO / 元数据）。
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("malformed http: {0}")]
    Malformed(String),
    #[error("metadata: {0}")]
    Meta(#[from] crate::http_meta::MetaError),
}

/// 写一个 HTTP 响应（头 + body，自动补 Content-Length）。
pub async fn write_response<W: AsyncWrite + Unpin>(
    w: &mut W,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (k, v) in headers {
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    if !headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
    {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        w.write_all(body).await?;
    }
    w.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, BufReader, duplex};

    /// 请求写入 → 读回，字段一致。
    #[tokio::test]
    async fn request_roundtrip() {
        let (mut tx, rx) = duplex(4096);
        write_request(
            &mut tx,
            "GET",
            "/f/abc123",
            &[("Host", "10.66.66.1"), ("X-Hermod-Kind", "file")],
            &[],
        )
        .await
        .unwrap();
        drop(tx);

        let mut reader = BufReader::new(rx);
        let req = Request::read(&mut reader).await.unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/f/abc123");
        assert_eq!(req.header("host"), Some("10.66.66.1"));
        assert_eq!(req.header("X-HERMOD-KIND"), Some("file"));
        assert_eq!(req.content_length(), Some(0));
    }

    /// 响应写入 → 读回，状态码/头/长度正确。
    #[tokio::test]
    async fn response_roundtrip() {
        let (mut tx, rx) = duplex(4096);
        write_response(
            &mut tx,
            200,
            "OK",
            &[("Content-Type", "application/octet-stream")],
            b"hello",
        )
        .await
        .unwrap();
        drop(tx);

        let mut reader = BufReader::new(rx);
        let head = ResponseHead::read(&mut reader).await.unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.header("content-type"), Some("application/octet-stream"));
        assert_eq!(head.content_length(), Some(5));
        let mut body = Vec::new();
        reader.read_to_end(&mut body).await.unwrap();
        assert_eq!(body, b"hello");
    }

    /// 非法请求行 → 报错。
    #[tokio::test]
    async fn malformed_request_errors() {
        let data = b"NOT-A-REQUEST\r\n\r\n";
        let mut reader = BufReader::new(&data[..]);
        assert!(Request::read(&mut reader).await.is_err());
    }

    /// 头过大 → 报错（长度限制）。
    #[tokio::test]
    async fn oversized_headers_error() {
        let mut data = b"GET / HTTP/1.1\r\n".to_vec();
        for i in 0..3000 {
            data.extend_from_slice(format!("X-{i}: aaaaaaaaaaaaaaaaaaaa\r\n").as_bytes());
        }
        data.extend_from_slice(b"\r\n");
        let mut reader = BufReader::new(&data[..]);
        assert!(Request::read(&mut reader).await.is_err());
    }
}
