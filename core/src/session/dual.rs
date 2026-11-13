//! 独立设备控制连接与传输码候选竞争；认证完成前不作废另一入口。
use super::*;
use crate::{
    contacts::{Contacts, Pairing},
    identity::Identity,
    signal::device::{DeviceControl, DeviceMessage},
};
use std::time::Duration;

fn device_error(error: impl std::fmt::Display) -> SessionError {
    SessionError::Authentication(format!("device rendezvous failed: {error}"))
}

/// 本机发送时保留传输码入口，并发布已配对设备的 ID 呼叫入口。
/// 调用者应在整个载荷发送期间持有 Contacts 的发送租约。
pub async fn sender_dual_session(
    config: &Config,
    code: &str,
    opts: &SessionOptions,
    identity: &Identity,
    contacts: &Contacts,
    on_ready: impl FnOnce() + Send,
) -> Result<TxSession, SessionError> {
    opts.validate_for(opts.transport.unwrap_or(TransportKind::Quic))
        .map_err(SessionError::Options)?;
    let mut code_door = Box::pin(sender_session_with_ready(config, code, opts, on_ready));
    let control = DeviceControl::connect(config, opts.family, identity, true);
    tokio::pin!(control);
    let mut code_error = None;
    let control = tokio::select! {
        result = &mut code_door => match result {
            Ok(session) => return Ok(session),
            Err(error) => {
                code_error = Some(error);
                control.await.map_err(device_error)?
            }
        },
        result = &mut control => match result {
            Ok(control) => control,
            Err(_) => {
                crate::log_warn!("Device ID entry unavailable; transfer code remains available");
                return code_door.await;
            }
        }
    };
    let id_door = listen_devices(config, opts, identity, contacts, control);
    tokio::pin!(id_door);
    let mut candidate = if let Some(error) = code_error {
        crate::log_warn!("Transfer code candidate failed; device ID entry remains available");
        id_door.await.map_err(|_| error)?
    } else {
        tokio::select! {
            result = &mut code_door => match result {
                Ok(session) => return Ok(session),
                Err(error) => {
                    crate::log_warn!("Transfer code candidate failed; device ID entry remains available");
                    id_door.await.map_err(|_| error)?
                }
            },
            result = &mut id_door => match result {
                Ok(candidate) => candidate,
                Err(_) => {
                    crate::log_warn!("Device control disconnected; transfer code remains available");
                    return code_door.await;
                }
            }
        }
    };
    // 先撤销落败码入口，再允许获胜设备提交；确认阶段也有独立上限。
    drop(code_door);
    let session = tokio::time::timeout(
        Duration::from_secs(25),
        candidate.auth.finish(
            &mut candidate.ws,
            candidate.channel,
            &candidate.key,
            contacts,
        ),
    )
    .await
    .map_err(|_| device_error("device confirmation timed out"))??;
    Ok(TxSession {
        conn: session.conn,
        token: session.token,
    })
}

struct DeviceCandidate {
    auth: super::device::AuthenticatedCandidate,
    ws: WsStream,
    key: [u8; 32],
    channel: u16,
    _control: DeviceControl,
}

async fn listen_devices(
    config: &Config,
    opts: &SessionOptions,
    identity: &Identity,
    contacts: &Contacts,
    mut control: DeviceControl,
) -> Result<DeviceCandidate, SessionError> {
    loop {
        let event = control.next_event().await.map_err(device_error)?;
        let DeviceMessage::Incoming {
            caller_id,
            call_id,
            channel_id,
        } = event
        else {
            continue;
        };
        let store = contacts.clone();
        let Some(peer) = tokio::task::spawn_blocking(move || store.find(caller_id))
            .await
            .map_err(device_error)?
            .map_err(device_error)?
        else {
            control.decline(&call_id).await.map_err(device_error)?;
            continue;
        };
        control.accept(&call_id).await.map_err(device_error)?;
        let attempt = async {
            let mut ws = open(config, opts.family).await?;
            let observed = client::join(&mut ws, channel_id).await?;
            let password = super::device::pairing_password(&peer);
            let key =
                receiver_first_handshake(&mut ws, channel_id, &password, Role::Sender).await?;
            let candidate = super::device::authenticate_device_candidate(
                &mut ws,
                channel_id,
                &key,
                observed,
                Role::Sender,
                opts,
                identity,
                contacts,
                Some(&peer),
            )
            .await?;
            Ok::<_, SessionError>((candidate, ws, key))
        };
        match tokio::time::timeout(Duration::from_secs(20), attempt).await {
            Ok(Ok((candidate, ws, key))) => {
                // 该 future 仅在完整认证成功后退出；调度器选定此分支后再提交。
                return Ok(DeviceCandidate {
                    auth: candidate,
                    ws,
                    key,
                    channel: channel_id,
                    _control: control,
                });
            }
            _ => {
                control.decline(&call_id).await.map_err(device_error)?;
                crate::log_warn!(
                    "Device candidate authentication failed; transfer code remains available"
                );
            }
        }
    }
}

/// 从已保存配对主动接收；不降级为传输码或首次配对。
pub async fn receiver_device_session(
    config: &Config,
    peer: &Pairing,
    opts: &SessionOptions,
    identity: &Identity,
    contacts: &Contacts,
) -> Result<RxSession, SessionError> {
    opts.validate_for(opts.transport.unwrap_or(TransportKind::Quic))
        .map_err(SessionError::Options)?;
    let mut control = DeviceControl::connect(config, opts.family, identity, false)
        .await
        .map_err(device_error)?;
    let mut selected = None;
    // 频道号是共享的 u16 空间；分配冲突采用有界随机重试。
    for _ in 0..8 {
        let mut random = [0; 2];
        getrandom::fill(&mut random).map_err(device_error)?;
        let channel = u16::from_be_bytes(random).max(1);
        let mut ws = open(config, opts.family).await?;
        if let Ok((observed, token)) =
            crate::signal::device::allocate_channel(&mut ws, channel).await
        {
            selected = Some((channel, ws, observed, token));
            break;
        }
    }
    let (channel, mut ws, observed, token) =
        selected.ok_or_else(|| device_error("cannot allocate candidate channel"))?;
    let call = control
        .call(peer.contact().device_id, channel, token)
        .await
        .map_err(device_error)?;
    let waiting = async {
        loop {
            match control.next_event().await.map_err(device_error)? {
                DeviceMessage::Accepted { call_id } if call_id == call => break,
                DeviceMessage::Declined { call_id } | DeviceMessage::Cancelled { call_id }
                    if call_id == call =>
                {
                    return Err(device_error("peer declined or cancelled the call"));
                }
                _ => {}
            }
        }
        client::wait_ready(&mut ws).await?;
        let password = super::device::pairing_password(peer);
        let key = sender_first_handshake(&mut ws, channel, &password, Role::Receiver).await?;
        let candidate = super::device::authenticate_device_candidate(
            &mut ws,
            channel,
            &key,
            observed,
            Role::Receiver,
            opts,
            identity,
            contacts,
            Some(peer),
        )
        .await?;
        candidate.finish(&mut ws, channel, &key, contacts).await
    };
    match tokio::time::timeout(Duration::from_secs(25), waiting).await {
        Ok(result) => result,
        Err(_) => {
            let _ = control.cancel(&call).await;
            Err(device_error("device call timed out"))
        }
    }
}

async fn sender_first_handshake(
    ws: &mut WsStream,
    channel: u16,
    password: &str,
    role: Role,
) -> Result<[u8; 32], SessionError> {
    let (session, public) =
        crate::crypto::cpace::init(password, channel, role).map_err(client::ClientError::from)?;
    client::send_blob(ws, channel, &public).await?;
    let peer: [u8; 65] = client::recv_blob(ws)
        .await?
        .try_into()
        .map_err(|_| device_error("invalid CPace point"))?;
    Ok(session.finish(&peer).map_err(client::ClientError::from)?)
}

async fn receiver_first_handshake(
    ws: &mut WsStream,
    channel: u16,
    password: &str,
    role: Role,
) -> Result<[u8; 32], SessionError> {
    let (session, public) =
        crate::crypto::cpace::init(password, channel, role).map_err(client::ClientError::from)?;
    let peer: [u8; 65] = client::recv_blob(ws)
        .await?
        .try_into()
        .map_err(|_| device_error("invalid CPace point"))?;
    client::send_blob(ws, channel, &public).await?;
    Ok(session.finish(&peer).map_err(client::ClientError::from)?)
}
