//! CLI 小工具。

use std::net::{SocketAddr, SocketAddrV6};

/// 解析 `--listen`：接受 `[::]:0`、`:0`（等价 `[::]:0`）、`[::1]:1234`。
///
/// 只接受 IPv6（DEVELOPMENT.md D7）；IPv4 地址直接报错。
pub fn parse_listen(s: &str) -> Result<SocketAddrV6, String> {
    let candidate = if let Some(rest) = s.strip_prefix(':') {
        format!("[::]{rest}")
    } else {
        s.to_string()
    };
    match candidate.parse::<SocketAddr>() {
        Ok(SocketAddr::V6(a)) => Ok(a),
        Ok(SocketAddr::V4(_)) => Err(format!("--listen 只接受 IPv6 地址（D7）: {s}")),
        Err(_) => Err(format!("无法解析 --listen 地址: {s}")),
    }
}

/// 把任意 error 装箱，便于 handler 统一返回 `Box<dyn Error>`。
pub fn boxed<E: std::error::Error + 'static>(e: E) -> Box<dyn std::error::Error> {
    Box::new(e)
}
