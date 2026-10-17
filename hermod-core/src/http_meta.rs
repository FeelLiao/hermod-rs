//! 隧道内 HTTP 元数据约定（M1.4）。
//!
//! 对应 DEVELOPMENT.md §3.4（隧道内的 TCP 与 HTTP）：
//! - 响应头 `X-Hermod-Kind: file|text|stream`
//! - `Content-Disposition: attachment; filename=...`
//! - `Content-Length`（file/text）或 `Transfer-Encoding: chunked`（stream）
//! - URL 一次性随机 token：`/f/<token>`
//!
//! 本模块只做**编解码 + 上限校验 + 抗畸形输入**，不真正收发 HTTP
//! （隧道内实际 HTTP 收发在 `tunnel/http`，见 M3）。

use crate::transfer::Kind;

/// 响应头名：载荷类型。
pub const X_HERMOD_KIND: &str = "X-Hermod-Kind";
/// 响应头名：文件名（RFC 6266）。
pub const CONTENT_DISPOSITION: &str = "Content-Disposition";
/// 响应头名：已知长度的载荷用这个。
pub const CONTENT_LENGTH: &str = "Content-Length";
/// 响应头名：流式载荷用这个。
pub const TRANSFER_ENCODING: &str = "Transfer-Encoding";
/// `Transfer-Encoding` 的取值（未知长度，边算边发）。
pub const CHUNKED: &str = "chunked";

/// 文件名长度上限（字节）。与常见文件系统上限一致，防止畸形头撑爆内存。
pub const MAX_FILENAME_LEN: usize = 255;
/// 一次性 URL token 长度（字节；128 位随机）。
pub const TOKEN_LEN: usize = 16;

/// `X-Hermod-Kind` 头取值（对应 Go 的 `transfer.Kind` 字符串）。
pub fn x_hermod_kind_value(kind: Kind) -> &'static str {
    kind.as_str()
}

/// 解析 `X-Hermod-Kind` 头值；非法值返回 `None`。
pub fn parse_x_hermod_kind(s: &str) -> Option<Kind> {
    Kind::parse(s)
}

/// 构造 `Content-Disposition: attachment; filename="..."`。
///
/// 安全校验（本模块是纯函数，安全边界在此落实）：
/// - 拒绝空文件名；
/// - 拒绝超过 [`MAX_FILENAME_LEN`] 的过长发名；
/// - 拒绝 CR/LF（防 HTTP 头注入）和双引号（防破坏 filename 语法）。
pub fn content_disposition(filename: &str) -> Result<String, MetaError> {
    if filename.is_empty() {
        return Err(MetaError::EmptyFilename);
    }
    if filename.len() > MAX_FILENAME_LEN {
        return Err(MetaError::FilenameTooLong);
    }
    if filename.chars().any(|c| matches!(c, '\r' | '\n' | '"')) {
        return Err(MetaError::InvalidFilename);
    }
    Ok(format!("attachment; filename=\"{filename}\""))
}

/// 传输帧格式：已知长度走 `Content-Length`，流式走 chunked。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// `Content-Length: <n>`（file/text）。
    ContentLength(u64),
    /// `Transfer-Encoding: chunked`（stream，长度未知）。
    Chunked,
}

/// 按载荷类型决定帧格式（DEVELOPMENT.md §3.4：stream 长度未知）。
pub fn framing(kind: Kind, size: u64) -> Framing {
    match kind {
        Kind::Stream => Framing::Chunked,
        _ => Framing::ContentLength(size),
    }
}

/// 构造一次性 URL token 路径：`/f/<token 十六进制>`（DEVELOPMENT.md §3.4）。
pub fn token_path(token: &[u8; TOKEN_LEN]) -> String {
    let mut s = String::with_capacity(3 + TOKEN_LEN * 2);
    s.push_str("/f/");
    for b in token {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// 从路径解析回 token 字节（[`token_path`] 的逆）；不是合法格式返回 `None`。
pub fn parse_token_path(path: &str) -> Option<[u8; TOKEN_LEN]> {
    let hex_part = path.strip_prefix("/f/")?;
    if hex_part.len() != TOKEN_LEN * 2 {
        return None;
    }
    let mut out = [0u8; TOKEN_LEN];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex_part[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// HTTP 元数据错误。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MetaError {
    #[error("filename is empty")]
    EmptyFilename,
    #[error("filename exceeds {MAX_FILENAME_LEN} bytes")]
    FilenameTooLong,
    #[error("filename contains CR/LF or double-quote (header injection risk)")]
    InvalidFilename,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 头值编解码往返 + 非法值。
    #[test]
    fn kind_header_roundtrip() {
        for kind in [Kind::File, Kind::Text, Kind::Stream] {
            let v = x_hermod_kind_value(kind);
            assert_eq!(parse_x_hermod_kind(v), Some(kind));
        }
        assert_eq!(parse_x_hermod_kind("banana"), None);
        assert_eq!(parse_x_hermod_kind(""), None);
    }

    /// Content-Disposition 正常构造。
    #[test]
    fn content_disposition_valid() {
        assert_eq!(
            content_disposition("report.pdf").unwrap(),
            "attachment; filename=\"report.pdf\""
        );
    }

    /// CR/LF / 引号 / 空 / 超长都必须拒绝（安全校验）。
    #[test]
    fn content_disposition_rejects_dangerous_input() {
        assert_eq!(content_disposition(""), Err(MetaError::EmptyFilename));
        assert_eq!(
            content_disposition("evil\r\nX-Inject: 1"),
            Err(MetaError::InvalidFilename)
        );
        assert_eq!(content_disposition("a\"b"), Err(MetaError::InvalidFilename));
        let long = "x".repeat(MAX_FILENAME_LEN + 1);
        assert_eq!(content_disposition(&long), Err(MetaError::FilenameTooLong));
    }

    /// 帧格式：file/text → Content-Length，stream → chunked。
    #[test]
    fn framing_by_kind() {
        assert_eq!(framing(Kind::File, 123), Framing::ContentLength(123));
        assert_eq!(framing(Kind::Text, 5), Framing::ContentLength(5));
        assert_eq!(framing(Kind::Stream, 0), Framing::Chunked);
    }

    /// URL token 路径编解码往返 + 非法输入。
    #[test]
    fn token_path_roundtrip() {
        let token = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b,
            0x0c, 0x0d, 0x0e, 0x0f,
        ];
        let path = token_path(&token);
        assert_eq!(path, "/f/000102030405060708090a0b0c0d0e0f");
        assert_eq!(parse_token_path(&path), Some(token));
        // 非法输入
        assert_eq!(parse_token_path("/f/00"), None); // 太短
        assert_eq!(parse_token_path("/x/000102030405060708090a0b0c0d0e0f"), None); // 前缀错
        assert_eq!(parse_token_path("/f/zz0102030405060708090a0b0c0d0e0f"), None); // 非十六进制
    }

    /// 抗畸形输入：对解析函数喂大量随机字符串，必须返回 Option/Result 而不是 panic（S4）。
    #[test]
    fn fuzz_never_panics() {
        // 简单 LCG：确定性伪随机，保证可复现
        let mut state: u32 = 0x1234_5678;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state
        };
        for _ in 0..20_000 {
            // 随机长度 0..=40，随机可打印字符
            let len = (next() % 41) as usize;
            let mut s = String::with_capacity(len);
            for _ in 0..len {
                s.push(char::from((next() % 95) as u8 + 0x20));
            }
            // 这些函数对任意输入都不得 panic
            let _ = parse_x_hermod_kind(&s);
            let _ = content_disposition(&s);
            let _ = parse_token_path(&s);
        }
    }
}
