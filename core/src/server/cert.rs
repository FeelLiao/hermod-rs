//! 信令服务器证书：自签 ECDSA P-256，首启生成，到期前自动续期且**密钥不变**
//! （对应 Go `config.GenerateServerCert` / `RenewServerCert`——SPKI 指纹不变，
//! 客户端钉扎不失效）。
//!
//! Go 版把 PEM 存进 config.yaml；Rust 版存到配置目录的 `server/` 子目录：
//! `server-cert.pem`（`/cert` 下发）/ `server-key.der`（PKCS#8）/
//! `server-cert.json`（到期元数据）。存放位置属于本端部署细节，线上互通
//! 只看 TLS 与 `/cert` 语义。

use std::path::Path;
use std::time::Duration;

/// 证书有效期：10 年（与 Go 参考实现一致）。
const CERT_LIFETIME_DAYS: i64 = 3652;
/// 到期前多久自动续期。
pub const CERT_AUTO_RENEW_THRESHOLD: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// 已就绪的服务器证书。
pub struct ServerCert {
    /// PEM 文本（`/cert` 端点的响应体）。
    pub cert_pem: String,
    /// 证书 DER（TLS 配置与指纹计算用）。
    pub cert_der: Vec<u8>,
    /// PKCS#8 私钥 DER（TLS 配置用）。
    pub key_der: Vec<u8>,
    /// SPKI 指纹（SHA-256(SubjectPublicKeyInfo)——`hermod trust` 记的就是它）。
    pub spki_fingerprint: [u8; 32],
}

/// 错误。
#[derive(Debug, thiserror::Error)]
pub enum CertError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("certificate: {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("metadata: {0}")]
    Meta(String),
}

/// 读取（或生成/续期）服务器证书。`renew_threshold` 内到期即续期（密钥不变）。
pub fn ensure(dir: &Path, renew_threshold: Duration) -> Result<ServerCert, CertError> {
    std::fs::create_dir_all(dir)?;
    let cert_path = dir.join("server-cert.pem");
    let key_path = dir.join("server-key.der");
    let meta_path = dir.join("server-cert.json");

    // 密钥：首启生成 PKCS#8；之后原样复用（续期不换密钥 → SPKI 不变）。
    let key_der = match std::fs::read(&key_path) {
        Ok(der) => der,
        Err(_) => {
            let key = rcgen::KeyPair::generate()?;
            let der = key.serialize_der();
            std::fs::write(&key_path, &der)?;
            der
        }
    };
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
        &rustls::pki_types::PrivatePkcs8KeyDer::from(key_der),
        &rcgen::PKCS_ECDSA_P256_SHA256,
    )?;

    let needs_issue = match std::fs::read_to_string(&cert_path) {
        Err(_) => true, // 没有证书：首启，直接签发
        Ok(_pem) => match read_not_after(&meta_path) {
            // 元数据缺失/损坏视为未知到期时间——保守重签（密钥不变，SPKI 不变）
            None => true,
            Some(not_after) => {
                let left_secs =
                    not_after.unix_timestamp() - time::OffsetDateTime::now_utc().unix_timestamp();
                left_secs <= renew_threshold.as_secs() as i64
            }
        },
    };
    let cert_pem = if needs_issue {
        issue(&cert_path, &meta_path, &key_pair)?
    } else {
        std::fs::read_to_string(&cert_path).map_err(CertError::Io)?
    };

    let cert_der = pem_decode(&cert_pem).ok_or_else(|| {
        CertError::Meta("server certificate pem is not a CERTIFICATE block".into())
    })?;
    let key_der = std::fs::read(&key_path)?;
    let der = rustls::pki_types::CertificateDer::from(cert_der.clone());
    Ok(ServerCert {
        spki_fingerprint: crate::signal::tls::spki_fingerprint(&der)
            .map_err(|e| CertError::Meta(e.to_string()))?,
        cert_der,
        key_der,
        cert_pem,
    })
}

/// 用（可能已有的）密钥签发新证书并落盘 + 写到期元数据。
fn issue(
    cert_path: &Path,
    meta_path: &Path,
    key_pair: &rcgen::KeyPair,
) -> Result<String, CertError> {
    let mut params = rcgen::CertificateParams::new(vec!["hermod-server".to_string()])?;
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1); // 时钟偏差容限
    params.not_after = now + time::Duration::days(CERT_LIFETIME_DAYS);
    let cert = params.self_signed(key_pair)?;
    let pem = pem_encode(cert.der().as_ref(), "CERTIFICATE");
    std::fs::write(cert_path, &pem)?;
    let not_after_unix = params.not_after.unix_timestamp();
    std::fs::write(
        meta_path,
        format!("{{\"not_after_unix\":{not_after_unix}}}"),
    )?;
    Ok(pem)
}

fn read_not_after(meta_path: &Path) -> Option<time::OffsetDateTime> {
    let text = std::fs::read_to_string(meta_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let unix = value.get("not_after_unix")?.as_i64()?;
    time::OffsetDateTime::from_unix_timestamp(unix).ok()
}

/// 手写 PEM 编码（base64ct 按行 64 字符；不依赖 rcgen 的 pem feature）。
fn pem_encode(der: &[u8], tag: &str) -> String {
    use base64ct::Encoding as _;
    let b64 = base64ct::Base64::encode_string(der);
    let mut out = format!("-----BEGIN {tag}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push('\n');
    }
    out.push_str(&format!("-----END {tag}-----\n"));
    out
}

/// 解析第一个 PEM 块为 DER（`/cert` 自读 / 测试用）。
fn pem_decode(pem: &str) -> Option<Vec<u8>> {
    use base64ct::Encoding as _;
    let body: String = pem
        .lines()
        .skip_while(|line| !line.starts_with("-----BEGIN"))
        .skip(1)
        .take_while(|line| !line.starts_with("-----END"))
        .collect();
    base64ct::Base64::decode_vec(&body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hermod-cert-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn first_run_issues_then_reuses() {
        let dir = temp_dir("issue");
        let first = ensure(&dir, CERT_AUTO_RENEW_THRESHOLD).unwrap();
        assert!(first.cert_pem.contains("-----BEGIN CERTIFICATE-----"));
        // 再次读取：同一张证书（SPKI 相同），不重签
        let second = ensure(&dir, CERT_AUTO_RENEW_THRESHOLD).unwrap();
        assert_eq!(first.cert_pem, second.cert_pem);
        assert_eq!(first.spki_fingerprint, second.spki_fingerprint);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn near_expiry_renews_with_same_key() {
        let dir = temp_dir("renew");
        let first = ensure(&dir, CERT_AUTO_RENEW_THRESHOLD).unwrap();
        // 把元数据改成“已过期”→ 触发续期
        let meta = dir.join("server-cert.json");
        std::fs::write(&meta, "{{\"not_after_unix\":1000000000}}").unwrap();
        let renewed = ensure(&dir, CERT_AUTO_RENEW_THRESHOLD).unwrap();
        assert_ne!(first.cert_pem, renewed.cert_pem, "到期前必须重签新证书");
        // 密钥不变 → SPKI 指纹不变（客户端钉扎不失效）
        assert_eq!(first.spki_fingerprint, renewed.spki_fingerprint);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spki_fingerprint_roundtrips_through_pem() {
        // /cert 下发的 PEM → 客户端算的 SPKI 指纹必须等于服务端自报的。
        let dir = temp_dir("spki");
        let cert = ensure(&dir, CERT_AUTO_RENEW_THRESHOLD).unwrap();
        let der = pem_decode(&cert.cert_pem).unwrap();
        let fp =
            crate::signal::tls::spki_fingerprint(&rustls::pki_types::CertificateDer::from(der))
                .unwrap();
        assert_eq!(fp, cert.spki_fingerprint);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pem_roundtrip_matches_der() {
        let dir = temp_dir("pem");
        let cert = ensure(&dir, CERT_AUTO_RENEW_THRESHOLD).unwrap();
        assert_eq!(pem_decode(&cert.cert_pem).unwrap(), cert.cert_der);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
