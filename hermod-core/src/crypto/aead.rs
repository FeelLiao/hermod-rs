//! 认证加密（AES-256-GCM）：加密/解密会合清单（bundle）。
//!
//! 对应 Go 版 `hermod-go/internal/crypto/crypto.go` 的：
//! - `SealAAD`（第 300 行）→ [`seal`]
//! - `OpenAAD`（第 319 行）→ [`open`]
//!
//! 字节格式（与 Go 完全一致）：
//! ```text
//! seal(key, aad, plaintext) = nonce(12) ‖ 密文 ‖ tag(16)
//! ```
//! AAD（附加认证数据）参与认证但不进密文。本项目中 AAD = channel_id（频道号），
//! 把"这份 bundle 属于哪个频道"绑定进密文，防跨频道重放（DEVELOPMENT.md §3.2 step 5）。

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

/// 输出前缀长度：12 字节 nonce（AES-GCM 标准 96 位 nonce，与 Go `gcm.NonceSize()` 一致）。
const NONCE_SIZE: usize = 12;

/// 加密 plaintext，返回 `nonce(12) ‖ 密文 ‖ tag(16)`（对应 Go `SealAAD`）。
///
/// nonce 每次随机生成（AES-GCM 要求同一密钥下 nonce 唯一，随机 96 位足够安全）。
pub fn seal(key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, AeadError> {
    let mut nonce = [0u8; NONCE_SIZE];
    getrandom::fill(&mut nonce).map_err(|_| AeadError::Rng)?;
    Ok(seal_with_nonce(key, &nonce, aad, plaintext))
}

/// 解密 [`seal`] 的输出并校验 AAD；认证失败返回 [`AeadError::Auth`]（对应 Go `OpenAAD`）。
///
/// - `blob` 不足 12 字节（连 nonce 都不够）→ [`AeadError::TooShort`]
/// - 密文被篡改 / AAD 不一致 / 密钥不对 → [`AeadError::Auth`]
pub fn open(key: &[u8; 32], aad: &[u8], blob: &[u8]) -> Result<Vec<u8>, AeadError> {
    if blob.len() < NONCE_SIZE {
        return Err(AeadError::TooShort);
    }
    let mut nonce_arr = [0u8; NONCE_SIZE];
    nonce_arr.copy_from_slice(&blob[..NONCE_SIZE]);
    let nonce = Nonce::from(nonce_arr);
    let cipher = Aes256Gcm::new_from_slice(key).expect("key is always 32 bytes (type-enforced)");
    cipher
        .decrypt(&nonce, Payload { msg: &blob[NONCE_SIZE..], aad })
        .map_err(|_| AeadError::Auth)
}

/// 用指定 nonce 加密（[`seal`] 的内部实现；测试用固定 nonce 锁定字节格式）。
fn seal_with_nonce(
    key: &[u8; 32],
    nonce: &[u8; NONCE_SIZE],
    aad: &[u8],
    plaintext: &[u8],
) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("key is always 32 bytes (type-enforced)");
    let gcm_nonce = Nonce::from(*nonce);
    let ct = cipher
        .encrypt(&gcm_nonce, Payload { msg: plaintext, aad })
        .expect("GCM encryption of arbitrary plaintext cannot fail");
    let mut out = Vec::with_capacity(NONCE_SIZE + ct.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

/// AES-GCM 错误。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AeadError {
    #[error("blob too short: need at least a 12-byte nonce")]
    TooShort,
    #[error("decryption failed: authentication tag or AAD mismatch")]
    Auth,
    #[error("rng failure")]
    Rng,
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [0x42; 32]; // 固定测试密钥
    const AAD: &[u8] = &[0x00, 0x2a]; // 模拟 channel_id=42 的 AAD（2 字节大端）

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// 往返：加密 → 解密还原明文（对应 Go TestSealOpenAAD）。
    #[test]
    fn seal_open_roundtrip() {
        let pt = b"endpoint bundle payload";
        let blob = seal(&KEY, AAD, pt).unwrap();
        assert_eq!(blob.len(), 12 + pt.len() + 16); // nonce + 密文 + tag
        assert_eq!(open(&KEY, AAD, &blob).unwrap(), pt);
    }

    /// AAD 不一致必须失败（AAD 参与认证）。
    #[test]
    fn wrong_aad_fails() {
        let blob = seal(&KEY, AAD, b"data").unwrap();
        assert!(open(&KEY, b"different-aad", &blob).is_err());
    }

    /// 密文被篡改必须失败（对应 Go TestOpenAADTamperedCiphertext）。
    #[test]
    fn tampered_ciphertext_fails() {
        let mut blob = seal(&KEY, AAD, b"data").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01; // 翻转 tag 最后一字节
        assert!(open(&KEY, AAD, &blob).is_err());
    }

    /// blob 太短必须报 TooShort（对应 Go TestOpenAADTooShort）。
    #[test]
    fn too_short_fails() {
        assert_eq!(open(&KEY, AAD, &[0x01, 0x02]), Err(AeadError::TooShort));
    }

    // ------------------------------------------------------------------
    // 独立基准：NIST SP 800-38D Appendix B Case 14（AES-256）。
    // 零密钥 / 零 IV / 16 字节零明文 / 空 AAD → 已知密文+tag。
    // 该值与 NIST 官方一致，证明 aes-gcm 用法与标准吻合。
    // ------------------------------------------------------------------
    #[test]
    fn nist_known_answer() {
        let key = [0u8; 32];
        let nonce = [0u8; 12];
        let pt = [0u8; 16];
        let blob = seal_with_nonce(&key, &nonce, b"", &pt);
        assert_eq!(
            hex(&blob[12..]),
            "cea7403d4d606b6e074ec5d3baf39d18d0d1c8a799996bf0265b98b5d48ab919"
        );
    }

    /// 锁定输出格式：固定 key/nonce/aad/pt 时字节必须完全确定
    /// （nonce(12) ‖ 密文 ‖ tag(16)，与 Go SealAAD 逐字节一致）。
    #[test]
    fn deterministic_format() {
        let fixed_nonce = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c];
        let blob = seal_with_nonce(&KEY, &fixed_nonce, AAD, b"hello");
        assert_eq!(
            hex(&blob),
            "0102030405060708090a0b0cdd9f4a20220117dc310503f9201828dd5e94295ff2"
        );
        assert_eq!(blob.len(), 12 + 5 + 16);
    }
}
