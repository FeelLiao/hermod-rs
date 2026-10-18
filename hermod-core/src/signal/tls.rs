//! TLS 证书 pinning（M2.2）：rustls 自定义 `ServerCertVerifier`。
//!
//! 对应指南 M2.2 与 Go `PubKeyFingerprint`（config.go:222）：pin 的是服务器证书的
//! **SPKI 指纹**（SHA-256(SubjectPublicKeyInfo)），而不是整个证书的指纹。
//! 原因是证书可以续签（Go `RenewServerCert`），只要密钥对不变，SPKI 指纹就不变，
//! pinning 不会因为续签而失效。
//!
//! 实现要点：
//! - `spki_fingerprint`：用 `der` crate 解析 X.509 DER，抽出 `subjectPublicKeyInfo`
//!   元素并规范化重编码（DER 本身是规范编码，等于 Go `MarshalPKIXPublicKey` 输出）。
//! - 指纹比较用 `subtle::ConstantTimeEq`（**常量时间**），防止通过比较耗时推断指纹。
//! - 签名验证委托给 rustls 的 `verify_tls12/13_signature`（用默认 crypto provider）。

use der::asn1::AnyRef;
use der::{Decode, Encode, Reader, Tag, TagNumber};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// 证书解析错误。
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("failed to parse server certificate DER: {0}")]
    Parse(#[from] der::Error),
    #[error("server certificate has no subjectPublicKeyInfo")]
    NoSpki,
    #[error("connect to server failed: {0}")]
    Connect(#[from] tokio_tungstenite::tungstenite::Error),
}

/// 计算服务器证书的 SPKI 指纹：SHA-256(SubjectPublicKeyInfo DER)。
/// 对应 Go `PubKeyFingerprint`（config.go:222）。
pub fn spki_fingerprint(cert_der: &CertificateDer<'_>) -> Result<[u8; 32], TlsError> {
    let spki_der = extract_spki(cert_der)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(spki_der));
    Ok(out)
}

/// 从 DER 证书里抽出 `subjectPublicKeyInfo` 的规范 DER 字节。
///
/// X.509 结构（DER）：
/// ```text
/// Certificate ::= SEQUENCE { tbsCertificate TBSCertificate, ... }
/// TBSCertificate ::= SEQUENCE {
///     [0] EXPLICIT version OPTIONAL,   // 通常有；有则跳过
///     serialNumber, signature, issuer, validity, subject,
///     subjectPublicKeyInfo, ... }
/// ```
/// 所以去掉可选的 version 后，第 6 个元素（下标 5）就是 SPKI。
fn extract_spki(cert_der: &[u8]) -> Result<Vec<u8>, TlsError> {
    // 外层 Any = Certificate（SEQUENCE）
    let cert = AnyRef::from_der(cert_der)?;

    // 外层 Certificate：第一个元素是 tbsCertificate，读完后消费剩余元素
    // （der 的 sequence() 要求闭包消费完整个内容，否则报 TrailingData）
    let tbs = cert.sequence(|outer| -> Result<AnyRef, der::Error> {
        let tbs = AnyRef::decode(outer)?;
        while !outer.is_finished() {
            let _ = AnyRef::decode(outer)?;
        }
        Ok(tbs)
    })?;

    // 在 tbsCertificate 的内容里数到 subjectPublicKeyInfo，并消费剩余元素
    let spki: Option<AnyRef> = tbs.sequence(|content| -> Result<Option<AnyRef>, der::Error> {
        // 第一个元素可能是 [0] EXPLICIT version
        let has_version = Tag::peek(content)?
            == Tag::ContextSpecific {
                constructed: true,
                number: TagNumber(0),
            };
        let target = if has_version { 6 } else { 5 };
        let mut found = None;
        let mut i = 0;
        while !content.is_finished() {
            let element = AnyRef::decode(content)?;
            if i == target {
                found = Some(element);
            }
            i += 1;
        }
        Ok(found)
    })?;

    let spki = spki.ok_or(TlsError::NoSpki)?;
    // 规范化重编码成完整 DER（DER 即规范编码，与 Go MarshalPKIXPublicKey 输出一致）
    spki.to_der().map_err(|_| TlsError::NoSpki)
}

/// 自定义服务器证书验证器：校验 SPKI 指纹等于 pin 的值（常量时间比较）。
#[derive(Debug)]
pub struct PinnedVerifier {
    expected: [u8; 32],
    provider: CryptoProvider,
}

impl ServerCertVerifier for PinnedVerifier {
    /// 只做 SPKI 指纹比对；对端证书链的其它字段不关心（自签 + pinning 模型）。
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let got = spki_fingerprint(end_entity)
            .map_err(|_| Error::General("cannot parse server certificate".into()))?;
        if bool::from(got.as_slice().ct_eq(&self.expected)) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::General("server fingerprint mismatch".into()))
        }
    }

    /// TLS1.2 签名验证：委托给 rustls 内置实现（用本 provider 支持的算法集）。
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    /// TLS1.3 签名验证：同上。
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// 构建带 SPKI pinning 的 rustls `ClientConfig`（供 `client.rs` 的 WebSocket 连接用）。
/// `expected` 是 `hermod trust` 记录的服务器 SPKI 指纹。
pub fn pinned_client_config(expected: [u8; 32]) -> Arc<rustls::ClientConfig> {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    Arc::new(
        rustls::ClientConfig::builder_with_provider(Arc::new(provider.clone()))
            .with_safe_default_protocol_versions()
            .expect("TLS 1.2 / 1.3 supported by the default provider")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedVerifier { expected, provider }))
            .with_no_client_auth(),
    )
}

/// 抓取服务器证书的验证器（TOFU）：接受任意证书，并把 end-entity DER 存下来。
///
/// 仅供 `hermod trust` 首次信任时使用；**不要**用于正常连接（正常连接必须用
/// [`PinnedVerifier`]）。
#[derive(Debug)]
struct CaptureVerifier {
    cert: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    provider: CryptoProvider,
}

impl ServerCertVerifier for CaptureVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        *self.cert.lock().expect("capture lock") = Some(end_entity.as_ref().to_vec());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// 连接服务器并抓取它 TLS 证书的 SPKI 指纹（TOFU，不校验证书）。
///
/// 供 `hermod trust` 使用：首次信任时调用，把结果存进 `config.trusted_servers`。
/// 调用方应在展示给用户确认后（或与已知指纹比对后）才落盘。
pub async fn fetch_server_spki(url: &str) -> Result<[u8; 32], TlsError> {
    let captured: Arc<std::sync::Mutex<Option<Vec<u8>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(provider.clone()))
        .with_safe_default_protocol_versions()
        .expect("TLS 1.2 / 1.3 supported by the default provider")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(CaptureVerifier {
            cert: captured.clone(),
            provider,
        }))
        .with_no_client_auth();
    let connector = tokio_tungstenite::Connector::Rustls(Arc::new(config));
    let _ = tokio_tungstenite::connect_async_tls_with_config(url, None, false, Some(connector))
        .await?;

    let der = captured
        .lock()
        .expect("capture lock")
        .take()
        .ok_or(TlsError::NoSpki)?;
    let cert = CertificateDer::from(der);
    spki_fingerprint(&cert)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::ServerName;

    // 测试夹具：openssl 生成的自签 ECDSA P-256 证书（/CN=hermod-server）的 DER hex。
    const TEST_CERT_DER_HEX: &str = concat!(
        "308201843082012ba003020102021445c4ecb29dda5fcca53b5c1e990e518dd74a5860",
        "300a06082a8648ce3d04030230183116301406035504030c0d6865726d6f642d736572",
        "766572301e170d3236313031373135303833355a170d3336313031343135303833355a",
        "30183116301406035504030c0d6865726d6f642d7365727665723059301306072a8648",
        "ce3d020106082a8648ce3d0301070342000459633ebe0f0aa77f694badd5a5b5129ce0",
        "7a9d9b0cff30e9e609344ab3f559aff93cdf4cbd90f3078f1c6b7c4083a243629dbb4",
        "02af2ef271dbd20cc4c2d05a2a3533051301d0603551d0e041604149eda2a39dace73",
        "614d28427bc086f851bef889c4301f0603551d230418301680149eda2a39dace73614d",
        "28427bc086f851bef889c4300f0603551d130101ff040530030101ff300a06082a8648",
        "ce3d040302034700304402207f8128f565396092611d28781f0dfc972fb16bef0de92",
        "77e09f41357ee6d73d30220301fbfa00851fb58e8623b1bd9be3d47f3780c44315d76",
        "6af685ca8184bd39d7",
    );
    // 独立基准：openssl x509 -pubkey | openssl pkey -pubin -outform DER | sha256sum
    const TEST_CERT_SPKI_FP: &str = "e6f42d0f550114985ae4a655e53f5566385509c38552642c53685d18bd516514";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn cert_der() -> CertificateDer<'static> {
        CertificateDer::from(unhex(TEST_CERT_DER_HEX))
    }

    /// TOFU 抓取：CaptureVerifier 会把 end-entity 证书 DER 记录下来。
    #[test]
    fn capture_verifier_records_certificate() {
        let captured: Arc<std::sync::Mutex<Option<Vec<u8>>>> =
            Arc::new(std::sync::Mutex::new(None));
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let v = CaptureVerifier {
            cert: captured.clone(),
            provider,
        };
        let cert = cert_der();
        let name = ServerName::try_from("example.com").unwrap();
        v.verify_server_cert(&cert, &[], &name, &[], UnixTime::now())
            .unwrap();

        let got = captured.lock().unwrap().clone().unwrap();
        assert_eq!(got, cert.as_ref());
        // 抓到的证书算出的 SPKI 指纹应与 fixture 一致
        assert_eq!(
            spki_fingerprint(&CertificateDer::from(got)).unwrap(),
            spki_fingerprint(&cert).unwrap()
        );
    }

    /// SPKI 指纹与 openssl 独立算出的值逐字节一致。
    #[test]
    fn spki_fingerprint_matches_openssl() {
        let fp = spki_fingerprint(&cert_der()).unwrap();
        assert_eq!(
            fp.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            TEST_CERT_SPKI_FP
        );
    }

    /// pin 对 → 验证通过。
    #[test]
    fn verifier_accepts_pinned_cert() {
        let expected = unhex(TEST_CERT_SPKI_FP);
        let verifier = PinnedVerifier {
            expected: expected.try_into().unwrap(),
            provider: rustls::crypto::aws_lc_rs::default_provider(),
        };
        let name = ServerName::try_from("hermod-server").unwrap();
        let result = verifier.verify_server_cert(
            &cert_der(),
            &[],
            &name,
            &[],
            rustls::pki_types::UnixTime::now(),
        );
        assert!(result.is_ok(), "pinned cert should verify: {result:?}");
    }

    /// pin 错（指纹不同）→ 必须拒绝。
    #[test]
    fn verifier_rejects_wrong_pin() {
        let verifier = PinnedVerifier {
            expected: [0x00; 32], // 错误的 pin
            provider: rustls::crypto::aws_lc_rs::default_provider(),
        };
        let name = ServerName::try_from("hermod-server").unwrap();
        let result = verifier.verify_server_cert(
            &cert_der(),
            &[],
            &name,
            &[],
            rustls::pki_types::UnixTime::now(),
        );
        assert!(result.is_err(), "wrong pin must be rejected");
    }

    /// 构造 ClientConfig 不报错，且默认 provider 已挂上（builder 链走通）。
    /// 更深的握手测试依赖真实 server（M2.2 验收）。
    #[test]
    fn pinned_client_config_builds() {
        let config = pinned_client_config(unhex(TEST_CERT_SPKI_FP).try_into().unwrap());
        assert!(!config.crypto_provider().cipher_suites.is_empty());
    }
}
