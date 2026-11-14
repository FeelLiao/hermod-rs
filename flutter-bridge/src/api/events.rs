//! Ordered, latest-value transfer observations. Cancelling an observation never
//! cancels its transfer. Dart must call `unsubscribe` from its stream onCancel.
use crate::{
    app::{Engine, StateEvent},
    frb_generated::StreamSink,
    gui_api::{GuiError, TransferSnapshot},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::sync::{Notify, watch};

struct Observer {
    cancel: Notify,
    acknowledged: watch::Sender<Option<u64>>,
}
impl Observer {
    fn new() -> Self {
        Self {
            cancel: Notify::new(),
            acknowledged: watch::channel(None).0,
        }
    }
}

#[flutter_rust_bridge::frb(non_opaque)]
#[derive(Clone)]
pub struct GuiEvent {
    pub sequence: u64,
    pub status: Option<TransferSnapshot>,
}
impl From<StateEvent> for GuiEvent {
    fn from(event: StateEvent) -> Self {
        Self {
            sequence: event.sequence,
            status: event.status.map(|snapshot| {
                let (phase, status) = Engine::snapshot_status(snapshot);
                TransferSnapshot::from_parts(phase, status)
            }),
        }
    }
}

fn subscriptions() -> &'static Mutex<HashMap<String, Arc<Observer>>> {
    static SUBSCRIPTIONS: OnceLock<Mutex<HashMap<String, Arc<Observer>>>> = OnceLock::new();
    SUBSCRIPTIONS.get_or_init(Default::default)
}

/// Subscribe with a unique observer ID. The first event is the current snapshot,
/// including an idle `None`; intermediate progress may coalesce. Sequences are
/// engine-scoped and strictly increase for each delivered update. Completed
/// state remains available until the next transfer or engine shutdown.
pub fn subscribe(subscription_id: String, sink: StreamSink<GuiEvent>) -> Result<(), GuiError> {
    if subscription_id.is_empty() || subscription_id.len() > 128 {
        return Err(GuiError::from_message("Invalid subscription id".into()));
    }
    let receiver = super::controller()?
        .subscribe_status()
        .ok_or_else(|| GuiError::from_message("GUI engine is shut down".into()))?;
    let cancel = Arc::new(Observer::new());
    {
        let mut subscriptions = subscriptions().lock().unwrap();
        if subscriptions.contains_key(&subscription_id) {
            return Err(GuiError::from_message(
                "Subscription id already exists".into(),
            ));
        }
        // Explicit observer admission budget, independently of transfer work.
        if subscriptions.len() >= 16 {
            return Err(GuiError::from_message(
                "Too many status subscriptions".into(),
            ));
        }
        subscriptions.insert(subscription_id.clone(), cancel.clone());
    }
    crate::app::runtime().spawn(async move {
        observe(receiver, &cancel, |event| sink.add(event).is_ok()).await;
        let mut subscriptions = subscriptions().lock().unwrap();
        if subscriptions
            .get(&subscription_id)
            .is_some_and(|entry| Arc::ptr_eq(entry, &cancel))
        {
            subscriptions.remove(&subscription_id);
        }
    });
    Ok(())
}

/// Idempotently stop only this observer, including while its transfer is idle.
/// FRB 2.13 has no sink-close future, so Dart's onCancel must call this API.
pub fn unsubscribe(subscription_id: String) {
    if let Some(cancel) = subscriptions().lock().unwrap().remove(&subscription_id) {
        cancel.cancel.notify_one();
    }
}

/// Acknowledge only after processing an event. At most one event per observer
/// is posted into FRB/Dart; all intervening worker updates coalesce in watch.
pub fn acknowledge(subscription_id: String, sequence: u64) {
    if let Some(observer) = subscriptions().lock().unwrap().get(&subscription_id) {
        observer.acknowledged.send_replace(Some(sequence));
    }
}

async fn observe(
    mut receiver: watch::Receiver<StateEvent>,
    observer: &Observer,
    mut send: impl FnMut(GuiEvent) -> bool,
) {
    let mut ack = observer.acknowledged.subscribe();
    let initial = receiver.borrow_and_update().clone();
    let mut in_flight = initial.sequence;
    if !send(initial.into()) {
        return;
    }
    loop {
        // Wait for the exact in-flight acknowledgement, not a future/stale ID.
        // changed() also lets shutdown interrupt an unacknowledged Dart event.
        while *ack.borrow_and_update() != Some(in_flight) {
            tokio::select! {
                biased;
                _ = observer.cancel.notified() => return,
                changed = ack.changed() => { if changed.is_err() { return; } },
                changed = receiver.changed() => { if changed.is_err() { return; } },
            }
        }
        let latest = receiver.borrow_and_update().clone();
        if latest.sequence > in_flight {
            in_flight = latest.sequence;
            if !send(latest.into()) {
                break;
            }
            continue;
        }
        tokio::select! {
            biased;
            _ = observer.cancel.notified() => break,
            changed = receiver.changed() => {
                if changed.is_err() { break; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::worker::{Snapshot, TransferPhase};

    #[tokio::test]
    async fn initial_snapshot_then_monotonic_coalesced_terminal() {
        let (sender, receiver) = watch::channel(StateEvent {
            sequence: 0,
            status: None,
        });
        let cancel = Observer::new();
        let mut events = Vec::new();
        observe(receiver, &cancel, |event| {
            let initial = event.status.is_none();
            cancel.acknowledged.send_replace(Some(event.sequence));
            events.push(event);
            if initial {
                // Simulate a consumer that cannot process any intermediate
                // progress. Memory remains one latest value, not 10k entries.
                for sequence in 1..=10_000 {
                    let mut status = Snapshot {
                        bytes: sequence,
                        ..Snapshot::default()
                    };
                    if sequence == 10_000 {
                        status.phase = TransferPhase::Completed;
                        status.completed = Some(std::time::Instant::now());
                    }
                    sender.send_replace(StateEvent {
                        sequence,
                        status: Some(status),
                    });
                }
                true
            } else {
                false
            }
        })
        .await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[1].sequence, 10_000);
        assert!(events[1].status.as_ref().unwrap().done);
        // A new observer restores the terminal snapshot without any new publish.
        let mut recovered = None;
        observe(sender.subscribe(), &cancel, |event| {
            recovered = Some(event);
            false
        })
        .await;
        assert!(recovered.unwrap().status.unwrap().done);
    }

    #[tokio::test]
    async fn unacknowledged_consumer_has_only_one_in_flight() {
        let (sender, receiver) = watch::channel(StateEvent {
            sequence: 0,
            status: None,
        });
        let observer = Arc::new(Observer::new());
        let task_observer = observer.clone();
        let (delivered, mut events) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            observe(receiver, &task_observer, |event| {
                delivered.send(event.sequence).is_ok()
            })
            .await;
        });
        assert_eq!(events.recv().await, Some(0));
        for sequence in 1..=10_000 {
            sender.send_replace(StateEvent {
                sequence,
                status: Some(Snapshot::default()),
            });
        }
        // A scheduling barrier, not a timing-dependent sleep.
        tokio::task::yield_now().await;
        assert!(events.try_recv().is_err());
        observer.acknowledged.send_replace(Some(0));
        assert_eq!(events.recv().await, Some(10_000));
        observer.cancel.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn idle_observer_cancel_and_shutdown_exit_without_task_cancellation() {
        let (sender, receiver) = watch::channel(StateEvent {
            sequence: 0,
            status: None,
        });
        let cancel = Observer::new();
        cancel.cancel.notify_one();
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            observe(receiver, &cancel, |_| true),
        )
        .await
        .unwrap();
        // Observer cancellation did not close the engine publisher.
        assert!(
            sender
                .send(StateEvent {
                    sequence: 1,
                    status: Some(Snapshot::default())
                })
                .is_err()
        );
        let receiver = sender.subscribe();
        drop(sender);
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            observe(receiver, &Observer::new(), |_| true),
        )
        .await
        .unwrap();
    }
}
