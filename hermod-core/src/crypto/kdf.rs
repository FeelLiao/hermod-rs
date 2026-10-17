//! 会话密钥派生：从 CPace 输出的共享密钥 K 派生协议后续需要的密钥。
//!
//! 目前只有一个输出：WireGuard 预共享密钥（PSK）：
//!
//! ```text
//! psk = HKDF-SHA256(ikm = K, salt = channel_id, info = "hermod-wg-psk-v1")
//! ```
//!
//! 对应设计文档 DEVELOPMENT.md §3.3（第 182 行）：
//! - 两端各自本地派生，**PSK 从不过网**；
//! - 它混入 WireGuard Noise 握手的 chaining key，拿不到传输码的一方即使偷到
//!   静态密钥也无法完成握手；
//! - 它是将来恢复"后量子 pillar"的接缝（D6/S1）：若把 K 的来源换成
//!   `CPace ‖ X25519 ‖ ML-KEM` 的混合密钥，这里的 HKDF 派生协议保持不变。

use hkdf::Hkdf;
use sha2::Sha256;

/// 从 CPace 共享密钥 K 派生 WireGuard 预共享密钥（32 字节）。
///
/// - `k`：CPaceFinish 输出的 32 字节共享密钥（`cpace::finish` 的返回值）。
/// - `channel_id`：传输频道号，作为 HKDF 的 salt（2 字节大端）。频道号进 salt
///   让不同频道的 PSK 不同（频道绑定）。
/// - info 固定为 `"hermod-wg-psk-v1"`，与 Go/设计文档一致。
///
/// HKDF-Expand 请求 32 字节输出，远小于上限（255×32 字节），所以 `expand`
/// 的 `Result` 用 `expect` 处理即可（不可达错误）。
pub fn derive_wg_psk(k: &[u8; 32], channel_id: u16) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(&channel_id.to_be_bytes()), k);
    let mut okm = [0u8; 32];
    hk.expand(b"hermod-wg-psk-v1", &mut okm)
        .expect("32-byte HKDF output cannot exceed the 255*32 byte limit");
    okm
}

#[cfg(test)]
mod tests {
    use super::*;

    // 独立基准：用 Python 的 RFC 5869 HKDF-SHA256 实现算出（/tmp/gen_vectors.py），
    // K 取自 CPace 固定向量测试的 K，把两个模块串起来验证。
    const K: [u8; 32] = [
        0x2b, 0x94, 0x2e, 0x52, 0x5c, 0xc3, 0xf0, 0xf9, 0x5a, 0xe8, 0x77, 0x20,
        0x02, 0x77, 0x65, 0x70, 0xe5, 0x55, 0xcc, 0x9c, 0xb1, 0x75, 0xc5, 0x5f,
        0x2f, 0x25, 0x31, 0x6b, 0xb0, 0xd4, 0xb4, 0x27,
    ];

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// 独立基准向量：Python HKDF 算出，与 Rust 输出逐字节一致。
    #[test]
    fn matches_independent_hkdf_reference() {
        assert_eq!(
            hex(&derive_wg_psk(&K, 42)),
            "9835dd49104e1133e443ad0fe6dfc66582601a4a64d65d765f27989130f642f1"
        );
        // 频道号不同 → salt 不同 → PSK 不同（频道绑定生效）
        assert_eq!(
            hex(&derive_wg_psk(&K, 7)),
            "d75b95a33c578857613a159902320db95fa6f0ab3a37b4e0d0627867563eec27"
        );
    }

    /// 相同输入 → 相同输出（确定性）。
    #[test]
    fn deterministic() {
        assert_eq!(derive_wg_psk(&K, 42), derive_wg_psk(&K, 42));
    }

    /// K 不同 → PSK 不同（Python 独立基准）。
    #[test]
    fn different_k_differs() {
        let other = [0x11u8; 32];
        assert_eq!(
            hex(&derive_wg_psk(&other, 42)),
            "66943532b30d4daed5352f8cd9bd1f0f213988b1160eddd121c0c4fceb7d069a"
        );
        assert_ne!(derive_wg_psk(&K, 42), derive_wg_psk(&other, 42));
    }
}
