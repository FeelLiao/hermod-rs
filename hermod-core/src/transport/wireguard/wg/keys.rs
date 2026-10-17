//! WireGuard 密钥（M3.1）：每会话临时 X25519 密钥对 + PSK 派生。
//!
//! 对应 DESIGN DEVELOPMENT.md §3.3：
//! - 每会话生成**全新**的 X25519 临时密钥对，从不复用、从不落盘；
//! - 本端公钥公布在加密 bundle 里（M2.4），对端公钥来自对端 bundle，
//!   因此对端身份的可信来源是传输码，而不是传输层本身；
//! - PSK = HKDF-SHA256(K, salt=channel_id, info="hermod-wg-psk-v1")，
//!   见 [`crate::crypto::kdf::derive_wg_psk`]（本模块不重复实现）。

use rand_core::CryptoRng;
use x25519_dalek::{PublicKey, StaticSecret};

/// WireGuard 密钥对（本会话临时）。
#[derive(Clone)]
pub struct KeyPair {
    secret: StaticSecret,
    public: PublicKey,
}

impl KeyPair {
    /// 生成新的临时密钥对。随机源由调用方注入，便于确定性测试。
    pub fn generate(rng: &mut impl CryptoRng) -> Self {
        let secret = StaticSecret::random_from_rng(rng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// 从 32 字节私钥构造（确定性测试 / 恢复用）。
    pub fn from_private_bytes(bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// 私钥字节（传给 boringtun `Tunn::new` 的 `static_private`）。
    pub fn private_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    /// 公钥字节（公布在 bundle 里）。
    pub fn public_bytes(&self) -> [u8; 32] {
        self.public.to_bytes()
    }

    /// 与对端公钥做 X25519 协商（boringtun 内部也会做；这里供测试/验证用）。
    pub fn diffie_hellman(&self, peer_public: &PublicKey) -> [u8; 32] {
        self.secret.diffie_hellman(peer_public).to_bytes()
    }
}

/// 解析对端公钥（来自加密 bundle 的 32 字节）。
pub fn parse_public_key(bytes: [u8; 32]) -> PublicKey {
    PublicKey::from(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 7748 §6.1 X25519 官方向量（独立基准，不依赖本代码）。
    const ALICE_PRIV: [u8; 32] = unhex(
        "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
    );
    const ALICE_PUB: [u8; 32] = unhex(
        "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a",
    );
    const BOB_PRIV: [u8; 32] = unhex(
        "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
    );
    const BOB_PUB: [u8; 32] = unhex(
        "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f",
    );
    const SHARED: [u8; 32] = unhex(
        "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742",
    );

    /// 公钥派生与 RFC 7748 一致（私钥→公钥）。
    #[test]
    fn public_key_derivation_matches_rfc7748() {
        assert_eq!(
            KeyPair::from_private_bytes(ALICE_PRIV).public_bytes(),
            ALICE_PUB
        );
        assert_eq!(KeyPair::from_private_bytes(BOB_PRIV).public_bytes(), BOB_PUB);
    }

    /// Diffie-Hellman 共享密钥与 RFC 7748 一致（双向一致）。
    #[test]
    fn diffie_hellman_matches_rfc7748() {
        let alice = KeyPair::from_private_bytes(ALICE_PRIV);
        let bob = KeyPair::from_private_bytes(BOB_PRIV);
        let s_alice = alice.diffie_hellman(&parse_public_key(bob.public_bytes()));
        let s_bob = bob.diffie_hellman(&parse_public_key(alice.public_bytes()));
        assert_eq!(s_alice, SHARED);
        assert_eq!(s_bob, SHARED);
    }

    /// 私钥字节往返稳定。
    #[test]
    fn private_bytes_roundtrip() {
        let kp = KeyPair::from_private_bytes(ALICE_PRIV);
        assert_eq!(kp.private_bytes(), ALICE_PRIV);
        assert_eq!(
            KeyPair::from_private_bytes(kp.private_bytes()).public_bytes(),
            kp.public_bytes()
        );
    }

    /// 两次生成得到不同密钥（临时性）。
    #[test]
    fn generate_produces_distinct_keys() {
        let mut rng = rand_core::UnwrapErr(getrandom::SysRng);
        let a = KeyPair::generate(&mut rng);
        let b = KeyPair::generate(&mut rng);
        assert_ne!(a.private_bytes(), b.private_bytes());
        assert_ne!(a.public_bytes(), b.public_bytes());
    }

    const fn unhex(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        let b = s.as_bytes();
        let mut i = 0;
        while i < 32 {
            out[i] = (hexval(b[i * 2]) << 4) | hexval(b[i * 2 + 1]);
            i += 1;
        }
        out
    }

    const fn hexval(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => 0,
        }
    }
}
