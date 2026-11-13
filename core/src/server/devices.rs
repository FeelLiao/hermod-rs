//! 易失设备注册与呼叫路由。身份认证不等于联系人授权；联系人由终端判断。

use super::{Server, WsStream};
use crate::{
    identity::{self, DeviceId},
    signal::device::{self, DeviceMessage as Msg, REGISTER_DOMAIN, VERSION},
};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message as Frame;

const MAX_CONNECTIONS: usize = 4096;
const MAX_CALLS: usize = 1024;
const MAX_CALLS_PER_CONNECTION: usize = 8;
const CALL_TTL: Duration = Duration::from_secs(30);
const IO_TIMEOUT: Duration = Duration::from_secs(10);

type Outbox = mpsc::Sender<Msg>;
struct Connection {
    id: DeviceId,
    outbox: Outbox,
    stop: tokio_util::sync::CancellationToken,
}
struct Call {
    caller: u64,
    target: u64,
    expires: Instant,
}
#[derive(Default)]
struct State {
    connections: HashMap<u64, Connection>,
    listeners: HashMap<DeviceId, u64>,
    calls: HashMap<String, Call>,
}

/// 设备服务状态；不写磁盘、不读取联系人、不处理传输秘密。
#[derive(Default)]
pub(super) struct Devices {
    state: Mutex<State>,
}

impl State {
    fn notify(&self, conn: u64, message: Msg) {
        if let Some(connection) = self.connections.get(&conn) {
            // 满队列立即关闭接收端，使慢消费者不能积压无界内存。
            if connection.outbox.try_send(message).is_err() {
                connection.stop.cancel();
            }
        }
    }

    fn remove_connection(&mut self, conn: u64) {
        if let Some(connection) = self.connections.remove(&conn)
            && self.listeners.get(&connection.id) == Some(&conn)
        {
            self.listeners.remove(&connection.id);
        }
        let ids: Vec<_> = self
            .calls
            .iter()
            .filter(|(_, call)| call.caller == conn || call.target == conn)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(call) = self.calls.remove(&id) {
                let peer = if call.caller == conn {
                    call.target
                } else {
                    call.caller
                };
                self.notify(peer, Msg::Cancelled { call_id: id });
            }
        }
    }

    fn expire(&mut self) {
        let ids: Vec<_> = self
            .calls
            .iter()
            .filter(|(_, call)| call.expires <= Instant::now())
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(call) = self.calls.remove(&id) {
                self.notify(
                    call.caller,
                    Msg::Cancelled {
                        call_id: id.clone(),
                    },
                );
                self.notify(call.target, Msg::Cancelled { call_id: id });
            }
        }
    }
}

impl Devices {
    pub(super) async fn serve(&self, server: &Server, mut ws: WsStream, conn_id: u64) {
        let mut challenge = [0; 32];
        if getrandom::fill(&mut challenge).is_err() {
            return;
        }
        if send(
            &mut ws,
            Msg::Challenge {
                version: VERSION,
                challenge: device::hex(&challenge),
                capabilities: vec!["channel_owner_token".into()],
            },
        )
        .await
        .is_err()
        {
            return;
        }
        let registration = tokio::time::timeout(IO_TIMEOUT, receive(&mut ws)).await;
        let (id, register) = match registration {
            Ok(Ok(Msg::Register {
                version: VERSION,
                id,
                pubkey,
                sig,
                register,
            })) => {
                let Some(public) = device::unhex::<33>(&pubkey) else {
                    return;
                };
                let Some(sig) = device::unhex::<64>(&sig) else {
                    return;
                };
                if DeviceId::from_public_key(&public).ok() != Some(id)
                    || identity::verify(
                        &public,
                        REGISTER_DOMAIN,
                        &device::registration_message(&challenge, id, register),
                        &sig,
                    )
                    .is_err()
                {
                    return;
                }
                (id, register)
            }
            _ => return,
        };
        let (outbox, mut inbox) = mpsc::channel(16);
        let stop = tokio_util::sync::CancellationToken::new();
        {
            let mut state = self.state.lock().await;
            if state.connections.len() >= MAX_CONNECTIONS
                || (register && state.listeners.contains_key(&id))
            {
                drop(state);
                let _ = send(
                    &mut ws,
                    Msg::Error {
                        code: "unavailable".into(),
                    },
                )
                .await;
                return;
            }
            state.connections.insert(
                conn_id,
                Connection {
                    id,
                    outbox: outbox.clone(),
                    stop: stop.clone(),
                },
            );
            if register {
                state.listeners.insert(id, conn_id);
            }
        }
        if send(&mut ws, Msg::Registered { version: VERSION })
            .await
            .is_ok()
        {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                if outbox.capacity() == 0 {
                    break;
                }
                tokio::select! {
                    _ = stop.cancelled() => break,
                    message = inbox.recv() => {
                        let Some(message) = message else { break; };
                        if send(&mut ws, message).await.is_err() { break; }
                    }
                    message = receive(&mut ws) => {
                        let Ok(message) = message else { break; };
                        if !self.handle(server, conn_id, message).await { break; }
                    }
                    _ = tick.tick() => { self.state.lock().await.expire(); }
                }
            }
        }
        self.state.lock().await.remove_connection(conn_id);
        let _ = tokio::time::timeout(IO_TIMEOUT, ws.close(None)).await;
    }

    async fn handle(&self, server: &Server, conn: u64, message: Msg) -> bool {
        match message {
            Msg::Call {
                call_id,
                target_id,
                channel_id,
                channel_token,
            } => {
                if device::unhex::<16>(&call_id).is_none()
                    || device::unhex::<32>(&channel_token).is_none()
                {
                    return false;
                }
                // 锁序固定 devices → waiters；旧中继路径不取得 devices 锁。
                let mut state = self.state.lock().await;
                state.expire();
                let Some(caller_id) = state.connections.get(&conn).map(|c| c.id) else {
                    return false;
                };
                let target = state.listeners.get(&target_id).copied();
                let Some(target) = target.filter(|target| *target != conn) else {
                    state.notify(conn, Msg::Declined { call_id });
                    return true;
                };
                if state.calls.contains_key(&call_id)
                    || state.calls.len() >= MAX_CALLS
                    || state
                        .calls
                        .values()
                        .filter(|call| call.caller == conn || call.target == conn)
                        .count()
                        >= MAX_CALLS_PER_CONNECTION
                    || state
                        .calls
                        .values()
                        .filter(|call| call.caller == target || call.target == target)
                        .count()
                        >= MAX_CALLS_PER_CONNECTION
                {
                    state.notify(
                        conn,
                        Msg::Error {
                            code: "busy".into(),
                        },
                    );
                    return true;
                }
                // 能力只存在于当前 legacy sender lease，成功路由即消费，不能重复呼叫。
                let mut waiters = server.waiters.lock().await;
                let owner = waiters
                    .get_mut(&channel_id)
                    .and_then(|v| v.iter_mut().find(|w| w.is_sender));
                let valid = server.store.channel_exists(channel_id)
                    && server.store.channel_owner(channel_id) == owner.as_ref().map(|w| w.conn_id)
                    && owner.as_ref().is_some_and(|w| {
                        w.device_call_token.as_deref() == Some(channel_token.as_str())
                    });
                if !valid {
                    state.notify(
                        conn,
                        Msg::Error {
                            code: "channel_not_owned".into(),
                        },
                    );
                    return true;
                }
                if let Some(owner) = owner {
                    owner.device_call_token = None;
                }
                drop(waiters);
                state.calls.insert(
                    call_id.clone(),
                    Call {
                        caller: conn,
                        target,
                        expires: Instant::now() + CALL_TTL,
                    },
                );
                state.notify(
                    conn,
                    Msg::CallPending {
                        call_id: call_id.clone(),
                    },
                );
                state.notify(
                    target,
                    Msg::Incoming {
                        caller_id,
                        call_id,
                        channel_id,
                    },
                );
                true
            }
            Msg::Accept { call_id } => self.finish(conn, call_id, 0).await,
            Msg::Decline { call_id } => self.finish(conn, call_id, 1).await,
            Msg::Cancel { call_id } => self.finish(conn, call_id, 2).await,
            _ => false,
        }
    }

    async fn finish(&self, conn: u64, id: String, action: u8) -> bool {
        let mut state = self.state.lock().await;
        state.expire();
        let Some(call) = state.calls.get(&id) else {
            return true;
        };
        if (action == 2 && call.caller != conn) || (action != 2 && call.target != conn) {
            return false;
        }
        let call = state.calls.remove(&id).expect("call checked under lock");
        let message = match action {
            0 => Msg::Accepted { call_id: id },
            1 => Msg::Declined { call_id: id },
            _ => Msg::Cancelled { call_id: id },
        };
        state.notify(call.caller, message.clone());
        state.notify(call.target, message);
        true
    }
}

async fn send(ws: &mut WsStream, message: Msg) -> Result<(), ()> {
    let json = serde_json::to_string(&message).map_err(|_| ())?;
    tokio::time::timeout(IO_TIMEOUT, ws.send(Frame::text(json)))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

async fn receive(ws: &mut WsStream) -> Result<Msg, ()> {
    loop {
        match ws.next().await {
            Some(Ok(Frame::Text(text))) if text.len() <= device::MAX_MESSAGE_SIZE => {
                return serde_json::from_str(&text).map_err(|_| ());
            }
            Some(Ok(Frame::Ping(_) | Frame::Pong(_))) => {
                tokio::time::timeout(IO_TIMEOUT, ws.flush())
                    .await
                    .map_err(|_| ())?
                    .map_err(|_| ())?;
            }
            _ => return Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(_n: u8) -> DeviceId {
        "0123456789abcdef0123456789abcdef".parse().unwrap()
    }
    #[test]
    fn expired_calls_notify_both_and_slow_consumers_are_cancelled() {
        let mut state = State::default();
        let (tx1, mut rx1) = mpsc::channel(1);
        let (tx2, mut rx2) = mpsc::channel(1);
        let stop1 = tokio_util::sync::CancellationToken::new();
        state.connections.insert(
            1,
            Connection {
                id: id(1),
                outbox: tx1,
                stop: stop1.clone(),
            },
        );
        state.connections.insert(
            2,
            Connection {
                id: id(2),
                outbox: tx2,
                stop: tokio_util::sync::CancellationToken::new(),
            },
        );
        state.calls.insert(
            "expired".into(),
            Call {
                caller: 1,
                target: 2,
                expires: Instant::now(),
            },
        );
        state.expire();
        assert!(state.calls.is_empty());
        assert!(matches!(rx1.try_recv(), Ok(Msg::Cancelled { .. })));
        assert!(matches!(rx2.try_recv(), Ok(Msg::Cancelled { .. })));
        state.notify(1, Msg::Registered { version: VERSION });
        state.notify(1, Msg::Registered { version: VERSION });
        assert!(stop1.is_cancelled());
    }

    #[test]
    fn stale_cleanup_cannot_remove_new_lease_and_disconnect_notifies_peer() {
        let mut state = State::default();
        let device = id(1);
        let (tx, mut rx) = mpsc::channel(4);
        state.connections.insert(
            1,
            Connection {
                id: device,
                outbox: tx.clone(),
                stop: tokio_util::sync::CancellationToken::new(),
            },
        );
        state.connections.insert(
            2,
            Connection {
                id: device,
                outbox: tx,
                stop: tokio_util::sync::CancellationToken::new(),
            },
        );
        state.listeners.insert(device, 2);
        state.calls.insert(
            "call".into(),
            Call {
                caller: 1,
                target: 2,
                expires: Instant::now() + CALL_TTL,
            },
        );
        state.remove_connection(1);
        assert_eq!(state.listeners.get(&device), Some(&2));
        assert!(state.calls.is_empty());
        assert_eq!(
            rx.try_recv().unwrap(),
            Msg::Cancelled {
                call_id: "call".into()
            }
        );
    }
}
