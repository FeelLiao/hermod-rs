//! UDP 反射端点（对应 Go `udp_reflect.go`）：帮 CGNAT 后的端发现自己的
//! 外部 UDP 地址。
//!
//! ## 安全：两阶段无状态 cookie，完全防反射放大
//!
//! ```text
//! 阶段 1（探测请求）：   [0x10]                (1 字节)
//! 阶段 1 应答：         [0x10][cookie(8)]     (9 字节)
//! 阶段 2（验证请求）：   [0x10][cookie(8)]     (9 字节)
//! 阶段 2 应答：         [family][IP][port]    (7-19 字节)
//! ```
//!
//! cookie 是 `HMAC-SHA256(密钥, 源 IP)[:8]`，绑定源 IP。伪造源 IP 的攻击者
//! 拿不到 cookie，永远到不了阶段 2——外部地址**从不**发给未验证的源。
//! HMAC 密钥按 UTC 日（24h）轮换；旧密钥保留 5 分钟宽限以容忍在途探测。

use hmac::{Hmac, KeyInit, Mac};
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use super::ratelimit::RateLimiter;

type HmacSha256 = Hmac<sha2::Sha256>;

const REFLECT_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PACKET: usize = 512;
const REFLECT_RATE: f64 = 10.0;
const REFLECT_BURST: f64 = 5.0;
const COOKIE_SIZE: usize = 8;
const KEY_GRACE: Duration = Duration::from_secs(5 * 60);
const KEY_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// cookie 模式探测/应答共用的首字节。
const MAGIC: u8 = 0x10;

/// 运行中的反射端点；`abort()` 收尾。
pub struct ReflectorHandle {
    task: JoinHandle<()>,
    local_addr: SocketAddr,
}

impl ReflectorHandle {
    /// 反射器实际绑定的地址（port=0 启动时由系统分配）。
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn abort(self) {
        self.task.abort();
    }
}

struct Keys {
    key: [u8; 32],
    old_key: Option<[u8; 32]>,
    key_time: Instant,
}

/// 在 `addr` 上启动 UDP 反射；失败返回 Err（调用方降级为仅 TCP，与 Go 一致）。
/// 必须在 tokio 运行时内调用（socket 交给异步驱动）。
pub fn start(addr: SocketAddr) -> std::io::Result<ReflectorHandle> {
    let socket = tokio::net::UdpSocket::from_std(std::net::UdpSocket::bind(addr)?)?;
    let local_addr = socket.local_addr()?;
    crate::log_info!(addr = %local_addr, "UDP reflection enabled for CGNAT address discovery");
    let keys = Keys {
        key: fresh_key(),
        old_key: None,
        key_time: Instant::now(),
    };
    let limiter = RateLimiter::new(REFLECT_RATE, REFLECT_BURST);
    let task = tokio::spawn(serve(socket, limiter, Mutex::new(keys)));
    Ok(ReflectorHandle { task, local_addr })
}

fn fresh_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand_core::Rng::fill_bytes(&mut rand_core::UnwrapErr(getrandom::SysRng), &mut key);
    key
}

fn ip_octets(ip: IpAddr) -> [u8; 16] {
    // HMAC 输入：IPv4 用 4 字节、IPv6 用 16 字节（对齐 Go 的 net.IP 写法）。
    let mut out = [0u8; 16];
    match ip {
        IpAddr::V4(ip) => out[..4].copy_from_slice(&ip.octets()),
        IpAddr::V6(ip) => out.copy_from_slice(&ip.octets()),
    }
    out
}

fn compute_cookie(key: &[u8; 32], ip: IpAddr) -> [u8; COOKIE_SIZE] {
    let mut mac =
        <HmacSha256 as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(ip_octets(ip).as_slice());
    let tag = mac.finalize().into_bytes();
    let mut cookie = [0u8; COOKIE_SIZE];
    cookie.copy_from_slice(&tag[..COOKIE_SIZE]);
    cookie
}

fn verify_cookie(keys: &Keys, ip: IpAddr, cookie: &[u8]) -> bool {
    if cookie.len() != COOKIE_SIZE {
        return false;
    }
    if verify_one(&keys.key, ip, cookie) {
        return true;
    }
    // 宽限期内旧密钥仍有效（容忍轮换瞬间的在途探测）。
    if let Some(old) = keys.old_key
        && keys.key_time.elapsed() <= KEY_GRACE
    {
        return verify_one(&old, ip, cookie);
    }
    false
}

fn verify_one(key: &[u8; 32], ip: IpAddr, cookie: &[u8]) -> bool {
    let mut mac =
        <HmacSha256 as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(ip_octets(ip).as_slice());
    let tag = mac.finalize().into_bytes();
    bool::from(tag.as_slice().ct_eq(cookie))
}

async fn serve(socket: UdpSocket, limiter: RateLimiter, keys: Mutex<Keys>) {
    let mut buf = [0u8; MAX_PACKET];
    loop {
        // 每日轮换密钥（旧密钥进宽限期）。
        {
            let mut keys = keys.lock().unwrap_or_else(|p| p.into_inner());
            if keys.key_time.elapsed() > KEY_LIFETIME {
                keys.old_key = Some(keys.key);
                keys.key = fresh_key();
                keys.key_time = Instant::now();
            }
        }
        let received = match tokio::time::timeout(REFLECT_TIMEOUT, socket.recv_from(&mut buf)).await
        {
            Err(_elapsed) => continue,
            Ok(Err(_error)) => continue,
            Ok(Ok((0, _))) => continue,
            Ok(Ok((n, from))) => (n, from),
        };
        let (n, from) = received;
        if buf[0] != MAGIC {
            // 不匹配协议态的一律静默丢弃——不能被当成任意载荷的反射放大器。
            continue;
        }
        if n == 1 {
            // 阶段 1：cookie 请求（限流：9 字节应答是可控放大）。
            if !limiter.allow(from) {
                crate::log_warn!(remote = %from, "rate limit applied to UDP reflection probe");
                continue;
            }
            let cookie = compute_cookie(
                &keys.lock().unwrap_or_else(|p| p.into_inner()).key,
                from.ip(),
            );
            let mut resp = [0u8; 1 + COOKIE_SIZE];
            resp[0] = MAGIC;
            resp[1..].copy_from_slice(&cookie);
            let _ = socket.send_to(&resp, from).await;
        } else if n == 1 + COOKIE_SIZE
            && verify_cookie(
                &keys.lock().unwrap_or_else(|p| p.into_inner()),
                from.ip(),
                &buf[1..n],
            )
        {
            // 阶段 2：cookie 验证通过——外部地址只发给验证过的源，不限流。
            if let Some(resp) = encode_external_address(from) {
                let _ = socket.send_to(&resp, from).await;
            }
        }
    }
}

/// 外部地址的二进制编码：`[4][IPv4(4)][port BE u16]` 或 `[6][IPv6(16)][port BE u16]`。
pub fn encode_external_address(addr: SocketAddr) -> Option<Vec<u8>> {
    let mut out = match addr.ip() {
        IpAddr::V4(ip) => {
            let mut out = vec![4u8];
            out.extend_from_slice(&ip.octets());
            out
        }
        IpAddr::V6(ip) => {
            let mut out = vec![6u8];
            out.extend_from_slice(&ip.octets());
            out
        }
    };
    out.extend_from_slice(&addr.port().to_be_bytes());
    Some(out)
}

/// 解码反射应答里的外部地址（客户端用）。
pub fn decode_external_address(data: &[u8]) -> Result<SocketAddr, String> {
    match data.first() {
        None => Err("external address data too short".into()),
        Some(4) => {
            if data.len() < 7 {
                return Err(format!(
                    "IPv4 external address too short: {} bytes",
                    data.len()
                ));
            }
            let ip = std::net::Ipv4Addr::new(data[1], data[2], data[3], data[4]);
            let port = u16::from_be_bytes([data[5], data[6]]);
            Ok(SocketAddr::new(IpAddr::V4(ip.into()), port))
        }
        Some(6) => {
            if data.len() < 19 {
                return Err(format!(
                    "IPv6 external address too short: {} bytes",
                    data.len()
                ));
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&data[1..17]);
            let port = u16::from_be_bytes([data[17], data[18]]);
            Ok(SocketAddr::new(
                IpAddr::V6(std::net::Ipv6Addr::from(octets)),
                port,
            ))
        }
        Some(other) => Err(format!(
            "unknown address family in external address: {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cookie_handshake_reveals_external_address() {
        let handle = start("127.0.0.1:0".parse().unwrap()).unwrap();
        let bound = handle.local_addr();
        let probe = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();

        // 阶段 1：拿 cookie
        probe.send_to(&[MAGIC], bound).await.unwrap();
        let mut buf = [0u8; MAX_PACKET];
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), probe.recv_from(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n, 1 + COOKIE_SIZE);
        assert_eq!(buf[0], MAGIC);

        // 阶段 2：回显 cookie 换外部地址
        probe.send_to(&buf[..n], bound).await.unwrap();
        let (n2, _) = tokio::time::timeout(Duration::from_secs(2), probe.recv_from(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let external = decode_external_address(&buf[..n2]).unwrap();
        assert_eq!(external.ip(), probe.local_addr().unwrap().ip());
        handle.abort();
    }

    #[test]
    fn forged_cookie_is_rejected() {
        let keys = Keys {
            key: fresh_key(),
            old_key: None,
            key_time: Instant::now(),
        };
        let ip: IpAddr = "198.51.100.1".parse().unwrap();
        // 正确 cookie 通过
        let cookie = compute_cookie(&keys.key, ip);
        assert!(verify_cookie(&keys, ip, &cookie));
        // 伪造（全零 / 别的 IP 的 cookie）拒绝
        assert!(!verify_cookie(&keys, ip, &[0u8; COOKIE_SIZE]));
        let other_ip: IpAddr = "198.51.100.2".parse().unwrap();
        let other_cookie = compute_cookie(&keys.key, other_ip);
        assert!(!verify_cookie(&keys, ip, &other_cookie));
        // 长度不对直接拒绝
        assert!(!verify_cookie(&keys, ip, &[0u8; 4]));
    }

    #[test]
    fn external_address_roundtrip() {
        for addr in [
            "198.51.100.7:4376".parse().unwrap(),
            "[2001:db8::1]:4376".parse().unwrap(),
        ] {
            let addr: SocketAddr = addr;
            let encoded = encode_external_address(addr).unwrap();
            assert_eq!(decode_external_address(&encoded).unwrap(), addr);
        }
        assert!(decode_external_address(&[4]).is_err()); // IPv4 截断
        assert!(decode_external_address(&[6; 1]).is_err()); // IPv6 截断
        assert!(decode_external_address(&[9; 7]).is_err()); // 未知地址族
        assert!(decode_external_address(&[]).is_err()); // 空
    }
}
