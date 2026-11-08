//! QUIC 的临时身份、TLS 配置与证书钉扎。
//!
//! QUIC 必须有 TLS 1.3。这里让 rx（QUIC server）每次会话生成一把临时自签名
//! 证书，把证书 DER 的 SHA-256 放进 CPace 加密的 endpoint bundle；tx（QUIC
//! client）只信任 bundle 里那一枚指纹。这样：
//!
//! - 中间人没有 server 私钥，无法冒充 rx；
//! - 证书在 TLS 握手线上是明文，但指纹经过 CPace 加密通道交换，无法被替换。
//!
//! tx 的身份不依赖 TLS 客户端证书：双方在 session 层已经用 CPace 的 K 认证
//! 过对端，数据面再用一次性 HTTP token（128 位）授权。TLS 只负责加密和认证
//! server；文件访问必须出示 token，猜不中就拿不到数据。

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// TLS ALPN；两端必须一致。
pub const ALPN: &[u8] = b"hermod/1";
/// TLS SNI / 证书 SAN 名（自签名占位；真正认证靠指纹钉扎）。
pub const SERVER_NAME: &str = "hermod";

/// 本会话的临时证书 + 私钥 + 证书 DER 指纹。
pub struct Identity {
    /// 自签名证书链（单证书）。
    pub cert_chain: Vec<CertificateDer<'static>>,
    /// PKCS#8 私钥。
    pub key: PrivateKeyDer<'static>,
    /// 证书 DER 的 SHA-256（放进 bundle 供对端钉扎）。
    pub fingerprint: [u8; 32],
}

impl Identity {
    /// 生成一把 P-256 自签名证书（rcgen）。
    pub fn generate() -> Result<Self, rcgen::Error> {
        let certified = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_string()])?;
        let fingerprint: [u8; 32] = Sha256::digest(certified.cert.der().as_ref()).into();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            certified.signing_key.serialize_der(),
        ));
        Ok(Self {
            cert_chain: vec![certified.cert.der().clone()],
            key,
            fingerprint,
        })
    }

    /// 证书 DER 指纹（十六进制，日志用）。
    pub fn fingerprint_hex(&self) -> String {
        self.fingerprint
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// 统一的 rustls crypto provider（aws-lc-rs，与 QUIC initial 套件一致）。
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// client 侧 TLS 配置：只钉扎 server 证书。
pub fn client_config(peer_fingerprint: [u8; 32]) -> Result<rustls::ClientConfig, Error> {
    let provider = provider();
    let verifier = PinnedServerVerifier {
        expected: peer_fingerprint,
        provider: provider.clone(),
    };
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![ALPN.to_vec()];
    // 每会话临时证书；不需要 TLS 会话票据。
    config.resumption = rustls::client::Resumption::disabled();
    Ok(config)
}

/// server 侧 TLS 配置：出示临时自签名证书。
pub fn server_config(identity: &Identity) -> Result<rustls::ServerConfig, Error> {
    let provider = provider();
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(identity.cert_chain.clone(), identity.key.clone_key())?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    config.send_tls13_tickets = 0;
    Ok(config)
}

fn fingerprint_matches(cert: &CertificateDer<'_>, expected: &[u8; 32]) -> bool {
    let got: [u8; 32] = Sha256::digest(cert.as_ref()).into();
    bool::from(got.ct_eq(expected))
}

fn mismatch() -> Error {
    Error::General("hermod: pinned QUIC server certificate does not match the bundle".into())
}

#[derive(Debug)]
struct PinnedServerVerifier {
    expected: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if fingerprint_matches(end_entity, &self.expected) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(mismatch())
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_ephemeral_and_unique() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        assert_ne!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint_hex().len(), 64);
        assert_eq!(a.cert_chain.len(), 1);
    }

    #[test]
    fn fingerprint_pinning_rejects_wrong_cert() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        let cert = &a.cert_chain[0];
        assert!(fingerprint_matches(cert, &a.fingerprint));
        assert!(!fingerprint_matches(cert, &b.fingerprint));
    }

    #[test]
    fn tls_configs_build() {
        let server = Identity::generate().unwrap();
        let client = client_config(server.fingerprint).unwrap();
        let server_cfg = server_config(&server).unwrap();
        assert_eq!(client.alpn_protocols, vec![ALPN.to_vec()]);
        assert_eq!(server_cfg.alpn_protocols, vec![ALPN.to_vec()]);
    }
}
