//! CLI 设备认证：显式配对和设备候选使用独立的加密控制消息；载荷仍为 v3。
//!
//! 每端常规发送 7 个 blob（含 CPace 公钥），单边 pending 恢复增加 1 个，
//! 均不超过信令频道的每端 10 blob 上限。active/pending 同记录可恢复；
//! active/无记录或冲突记录必须双方删除后重配，不能静默轮换长期秘密。

use super::*;
use crate::contacts::{Contacts, Pairing};
use crate::identity::{self, Identity};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

const DOMAIN: &[u8] = b"hermod-device-session-v1";

fn auth_error(message: impl Into<String>) -> SessionError {
    SessionError::Authentication(message.into())
}

fn derive(k: &[u8; 32], channel: u16, label: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    Hkdf::<Sha256>::new(Some(&channel.to_be_bytes()), k)
        .expand(label, &mut out)
        .expect("32 bytes is within HKDF-SHA256 output limit");
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 已保存配对的 CPace 密码；高熵秘密仅以 hex 形式传给 CPace，不记录日志。
pub fn pairing_password(pairing: &Pairing) -> String {
    hex(pairing.secret())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    version: u8,
    mode: String,
    role: String,
    id: String,
    public_key: Vec<u8>,
    nonce: [u8; 32],
    endpoint: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingStatus {
    pairing_id: Option<String>,
    proof: Option<[u8; 32]>,
    pending: bool,
}

async fn send<T: Serialize>(
    ws: &mut WsStream,
    channel: u16,
    key: &[u8; 32],
    phase: u8,
    value: &T,
) -> Result<(), SessionError> {
    let plain = serde_json::to_vec(value)
        .map_err(|_| auth_error("invalid local authentication message"))?;
    let aad = [b'D', b'A', 1, phase, (channel >> 8) as u8, channel as u8];
    client::send_blob(ws, channel, &aead::seal(key, &aad, &plain)?).await?;
    Ok(())
}

async fn recv<T: serde::de::DeserializeOwned>(
    ws: &mut WsStream,
    channel: u16,
    key: &[u8; 32],
    phase: u8,
) -> Result<T, SessionError> {
    let blob = client::recv_blob(ws).await?;
    if blob.len() > 128 * 1024 {
        return Err(auth_error("authentication message exceeds limit"));
    }
    let aad = [b'D', b'A', 1, phase, (channel >> 8) as u8, channel as u8];
    let plain = aead::open(key, &aad, &blob)?;
    serde_json::from_slice(&plain).map_err(|_| auth_error("invalid peer authentication message"))
}

async fn exchange<T: Serialize + serde::de::DeserializeOwned>(
    ws: &mut WsStream,
    channel: u16,
    key: &[u8; 32],
    role: Role,
    phase: u8,
    value: &T,
) -> Result<T, SessionError> {
    match role {
        Role::Sender => {
            send(ws, channel, key, phase, value).await?;
            recv(ws, channel, key, phase).await
        }
        Role::Receiver => {
            let peer = recv(ws, channel, key, phase).await?;
            send(ws, channel, key, phase, value).await?;
            Ok(peer)
        }
    }
}

fn canonical(
    channel: u16,
    exporter: &[u8; 32],
    tx: &Hello,
    rx: &Hello,
    tx_status: &PairingStatus,
    rx_status: &PairingStatus,
) -> Result<Vec<u8>, SessionError> {
    // 结构体字段顺序固定，数组明确限定发送方/接收方顺序；不得使用 JSON 对象 map。
    serde_json::to_vec(&(
        "hermod-device-transcript",
        1u8,
        channel,
        exporter,
        tx,
        rx,
        tx_status,
        rx_status,
    ))
    .map_err(|_| auth_error("cannot encode authentication transcript"))
}

/// 已完成双向设备认证的候选；丢弃候选不会取消另一路传输码。
///
/// 调度器必须先选定唯一获胜候选，再调用 `finish`；认证阶段不写配对记录。
pub struct AuthenticatedCandidate {
    transport: AnyTransport,
    role: Role,
    peer_endpoint: Vec<u8>,
    psk: [u8; 32],
    token: [u8; 16],
    offer: Option<crate::transfer::TransferOffer>,
    peer_multi_stream: bool,
    peer_id: identity::DeviceId,
    peer_key: [u8; 33],
    pairing_id: String,
    new_secret: Option<[u8; 32]>,
    recovery_secret: Option<[u8; 32]>,
    needs_recovery: bool,
}

impl std::fmt::Debug for AuthenticatedCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthenticatedCandidate")
            .finish_non_exhaustive()
    }
}

impl AuthenticatedCandidate {
    /// 唯一获胜候选才持久化配对、交换确认并建立数据面连接。
    pub async fn finish(
        mut self,
        ws: &mut WsStream,
        channel: u16,
        k: &[u8; 32],
        contacts: &Contacts,
    ) -> Result<RxSession, SessionError> {
        let auth_key = derive(k, channel, b"hermod-device-aead-v1");
        if self.needs_recovery {
            let peer_secret: Option<[u8; 32]> =
                exchange(ws, channel, &auth_key, self.role, 7, &self.recovery_secret).await?;
            if self.new_secret.is_some() {
                self.new_secret =
                    Some(peer_secret.ok_or_else(|| auth_error("missing pending recovery secret"))?);
            }
        }
        if let Some(secret) = self.new_secret {
            let contacts = contacts.clone();
            let peer_id = self.peer_id;
            let peer_key = self.peer_key;
            let pairing_id = self.pairing_id.clone();
            // 文件锁、权限检查和落盘必须留在阻塞线程，不能占用信令运行时。
            tokio::task::spawn_blocking(move || {
                contacts.stage_pairing(peer_id, peer_key, "", &pairing_id, secret)
            })
            .await
            .map_err(|_| auth_error("pending pairing storage task failed"))?
            .map_err(|_| auth_error("cannot persist pending pairing"))?;
        }
        let ack: String = exchange(ws, channel, &auth_key, self.role, 5, &self.pairing_id).await?;
        if ack != self.pairing_id {
            return Err(auth_error("pending pairing acknowledgement mismatch"));
        }
        let stored_contacts = contacts.clone();
        let peer_id = self.peer_id;
        let pairing_id = self.pairing_id.clone();
        tokio::task::spawn_blocking(move || stored_contacts.confirm_pairing(peer_id, &pairing_id))
            .await
            .map_err(|_| auth_error("pairing confirmation storage task failed"))?
            .map_err(|_| auth_error("cannot confirm pairing"))?;
        let confirmed: String =
            exchange(ws, channel, &auth_key, self.role, 6, &self.pairing_id).await?;
        if confirmed != self.pairing_id {
            return Err(auth_error("confirmed pairing acknowledgement mismatch"));
        }
        Ok(RxSession {
            conn: self
                .transport
                .establish(self.role, &self.psk, &self.peer_endpoint)
                .await?,
            token: self.token,
            offer: self.offer,
            peer_multi_stream: self.peer_multi_stream,
        })
    }
}

/// 从已经完成 CPace 的频道构造候选。
///
/// `role` 是文件角色，与 allocate/join 或 CPace 先后无关。`expected` 为 Some 时
/// 以保存的设备 ID、公钥和 pairing_id 固定对端；None 仅用于双方显式同意的码配对。
#[allow(clippy::too_many_arguments)]
pub async fn authenticate_device_candidate(
    ws: &mut WsStream,
    channel: u16,
    k: &[u8; 32],
    observed: ObservedAddrs,
    role: Role,
    opts: &SessionOptions,
    identity: &Identity,
    contacts: &Contacts,
    expected: Option<&Pairing>,
) -> Result<AuthenticatedCandidate, SessionError> {
    opts.validate_for(opts.transport.unwrap_or(TransportKind::Quic))
        .map_err(SessionError::Options)?;
    let token = derive_http_token(k, channel);
    let kind = opts.transport.unwrap_or(TransportKind::Quic);
    let mut transport = build_transport(opts, kind, &observed, sha256_32(&token))?;
    let endpoint = transport.prepare().await?;
    let endpoint = if matches!(role, Role::Sender) && opts.multi_stream_capable {
        let mut bundle = crate::transport::quic::bundle::QuicEndpoint::decode(&endpoint)
            .map_err(|e| TransportError::Quic(crate::transport::QuicError::Bundle(e)))?;
        bundle.multi_stream = true;
        bundle
            .encode()
            .map_err(|e| TransportError::Quic(crate::transport::QuicError::Bundle(e)))?
    } else {
        endpoint
    };
    let endpoint = attach_offer(&endpoint, opts.offer.as_ref())?;
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|_| auth_error("random source failed"))?;
    let mode = if expected.is_some() { "device" } else { "pair" };
    let local = Hello {
        version: 1,
        mode: mode.into(),
        role: if matches!(role, Role::Sender) {
            "tx"
        } else {
            "rx"
        }
        .into(),
        id: identity.device_id().to_string(),
        public_key: identity.public_key().to_vec(),
        nonce,
        endpoint: wrap_endpoint(kind, &endpoint),
    };
    let auth_key = derive(k, channel, b"hermod-device-aead-v1");
    let exporter = derive(k, channel, b"hermod-device-exporter-v1");
    let peer: Hello = exchange(ws, channel, &auth_key, role, 1, &local).await?;
    if peer.version != 1
        || peer.mode != mode
        || peer.role == local.role
        || !matches!(peer.role.as_str(), "tx" | "rx")
        || peer.id == local.id
    {
        return Err(auth_error("device mode or role mismatch"));
    }
    let peer_id = peer
        .id
        .parse::<identity::DeviceId>()
        .map_err(|_| auth_error("invalid peer device ID"))?;
    let peer_key: [u8; 33] = peer
        .public_key
        .as_slice()
        .try_into()
        .map_err(|_| auth_error("invalid peer public key"))?;
    if identity::DeviceId::from_public_key(&peer_key)
        .map_err(|_| auth_error("invalid peer public key"))?
        != peer_id
    {
        return Err(auth_error("peer device ID does not match public key"));
    }
    if let Some(expected) = expected
        && (expected.contact().device_id != peer_id || expected.contact().public_key != peer_key)
    {
        return Err(auth_error("peer does not match pinned device identity"));
    }
    let stored_contacts = contacts.clone();
    let existing = tokio::task::spawn_blocking(move || stored_contacts.find_any(peer_id))
        .await
        .map_err(|_| auth_error("pairing lookup storage task failed"))?
        .map_err(|_| auth_error("cannot read saved pairing"))?;
    let stored = expected.or(existing.as_ref());
    let local_status = PairingStatus {
        pairing_id: stored.map(|p| p.contact().pairing_id.clone()),
        pending: stored
            .is_some_and(|p| p.contact().state == crate::contacts::PairingState::Pending),
        proof: stored.map(|p| {
            let mut proof = Vec::from(p.secret().as_slice());
            proof.extend_from_slice(&exporter);
            sha256_32(&proof)
        }),
    };
    let peer_status: PairingStatus =
        exchange(ws, channel, &auth_key, role, 2, &local_status).await?;
    let needs_recovery = local_status.pairing_id.is_none() && peer_status.pending
        || peer_status.pairing_id.is_none() && local_status.pending;
    if !needs_recovery
        && (local_status.pairing_id != peer_status.pairing_id
            || local_status.proof != peer_status.proof)
    {
        return Err(auth_error(
            "pairing records differ; delete the pairing on both devices and pair again",
        ));
    }
    let transcript = match role {
        Role::Sender => canonical(
            channel,
            &exporter,
            &local,
            &peer,
            &local_status,
            &peer_status,
        )?,
        Role::Receiver => canonical(
            channel,
            &exporter,
            &peer,
            &local,
            &peer_status,
            &local_status,
        )?,
    };
    let signature = identity
        .sign(DOMAIN, &transcript)
        .map_err(|_| auth_error("cannot sign authentication transcript"))?;
    let peer_signature: Vec<u8> =
        exchange(ws, channel, &auth_key, role, 3, &signature.to_vec()).await?;
    if peer_signature.len() != 64 {
        return Err(auth_error("invalid peer signature length"));
    }
    identity::verify(&peer_key, DOMAIN, &transcript, &peer_signature)
        .map_err(|_| auth_error("peer transcript signature rejected"))?;
    let (peer_kind, peer_endpoint) = parse_endpoint(&peer.endpoint)?;
    if peer_kind != kind {
        return Err(SessionError::TransportMismatch {
            local: kind,
            peer: peer_kind,
        });
    }
    let offer = read_offer(peer_endpoint)?;
    let peer_multi_stream = crate::transport::quic::bundle::QuicEndpoint::decode(peer_endpoint)
        .map(|bundle| bundle.multi_stream)
        .unwrap_or(false);
    let transcript_hash = sha256_32(&transcript);
    let verified: [u8; 32] = exchange(ws, channel, &auth_key, role, 4, &transcript_hash).await?;
    if verified != transcript_hash {
        return Err(auth_error("peer did not confirm transcript verification"));
    }
    let pairing_id = local_status
        .pairing_id
        .or(peer_status.pairing_id)
        .unwrap_or_else(|| hex(&transcript_hash));
    let recovery_secret = needs_recovery
        .then(|| stored.map(|p| *p.secret()))
        .flatten();
    let new_secret = stored
        .is_none()
        .then(|| derive(k, channel, b"hermod-pairing-secret-v1"));
    Ok(AuthenticatedCandidate {
        transport,
        role,
        peer_endpoint: peer_endpoint.to_vec(),
        psk: derive_wg_psk(k, channel),
        token,
        offer,
        peer_multi_stream,
        peer_id,
        peer_key,
        pairing_id,
        new_secret,
        recovery_secret,
        needs_recovery,
    })
}

/// CLI 发送入口；普通码发送保持原协议，显式 `pair` 才发送长期设备身份。
#[allow(clippy::too_many_arguments)]
pub async fn sender_device_session(
    config: &Config,
    code: &str,
    opts: &SessionOptions,
    identity: &Identity,
    contacts: &Contacts,
    pair: bool,
    on_ready: impl FnOnce() + Send,
) -> Result<TxSession, SessionError> {
    if !pair {
        return super::dual::sender_dual_session(config, code, opts, identity, contacts, on_ready)
            .await;
    }
    opts.validate_for(opts.transport.unwrap_or(TransportKind::Quic))
        .map_err(SessionError::Options)?;
    let (channel, words) = wordlist::parse_transfer_code(code)?;
    let mut ws = open(config, opts.family).await?;
    let (k, observed) =
        client::sender_handshake_with_ready(&mut ws, channel, &words.join("-"), on_ready).await?;
    let candidate = authenticate_device_candidate(
        &mut ws,
        channel,
        &k,
        observed,
        Role::Sender,
        opts,
        identity,
        contacts,
        None,
    )
    .await?;
    let session = candidate.finish(&mut ws, channel, &k, contacts).await?;
    Ok(TxSession {
        conn: session.conn,
        token: session.token,
    })
}

/// CLI 显式码配对接收入口；双方确认配对持久化后才建立文件数据面。
pub async fn receiver_pairing_session(
    config: &Config,
    code: &str,
    opts: &SessionOptions,
    identity: &Identity,
    contacts: &Contacts,
) -> Result<RxSession, SessionError> {
    opts.validate_for(opts.transport.unwrap_or(TransportKind::Quic))
        .map_err(SessionError::Options)?;
    let (channel, words) = wordlist::parse_transfer_code(code)?;
    let mut ws = open(config, opts.family).await?;
    let (k, observed) = client::receiver_handshake(&mut ws, channel, &words.join("-")).await?;
    authenticate_device_candidate(
        &mut ws,
        channel,
        &k,
        observed,
        Role::Receiver,
        opts,
        identity,
        contacts,
        None,
    )
    .await?
    .finish(&mut ws, channel, &k, contacts)
    .await
}
