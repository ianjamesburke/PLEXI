//! CLI consumers of the existing brokered event connection.
use super::ui_mailbox::UiMailbox;
use crate::broker::ActorType;
use crate::host::app_timeline::{self, AppEventRecord, lifecycle_matches};
use crate::host::event_subscriptions::{HostSubscribeReply, HostSubscribeRequest};
use crate::host::pane_lifecycle::{PUBLISHER, STREAM};
use crate::protocol::{AppRequest, PayloadMode, TriggerMode};
use serde_json::{Value, json};
use std::io::{BufReader, Lines, Write};
use crate::platform::ipc::IpcStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

enum Consumer {
    Events,
    Follow {
        pane: Option<u64>,
    },
    Wait {
        pane: u64,
        predicate: String,
        deadline: Instant,
    },
}

impl Consumer {
    fn parse(val: &Value) -> Result<Self, String> {
        match val["type"].as_str() {
            Some("pane_lifecycle_wait") => {
                let pane = val["pane_id"].as_u64().ok_or("pane_id is required")?;
                let predicate = val["until"].as_str().unwrap_or_default();
                if !matches!(predicate, "idle" | "blocked" | "exited") {
                    return Err("--until must be idle, blocked, or exited".into());
                }
                let seconds = val["timeout"].as_f64().unwrap_or(300.0);
                let timeout = Duration::try_from_secs_f64(seconds)
                    .ok()
                    .filter(|timeout| !timeout.is_zero())
                    .ok_or("timeout must be a finite positive number")?;
                let deadline = Instant::now()
                    .checked_add(timeout)
                    .ok_or("timeout is too large")?;
                Ok(Self::Wait {
                    pane,
                    predicate: predicate.into(),
                    deadline,
                })
            }
            Some("pane_lifecycle_follow") => {
                if !val["pane_id"].is_null() && !val["pane_id"].is_u64() {
                    return Err("pane_id must be an unsigned integer".into());
                }
                Ok(Self::Follow {
                    pane: val["pane_id"].as_u64(),
                })
            }
            _ => Ok(Self::Events),
        }
    }

    fn pane(&self) -> Option<u64> {
        match self {
            Self::Events => None,
            Self::Follow { pane } => *pane,
            Self::Wait { pane, .. } => Some(*pane),
        }
    }

    fn expired(&self) -> bool {
        matches!(self, Self::Wait { deadline, .. } if Instant::now() >= *deadline)
    }
}

/// Owns all early-return cleanup, including a disconnected consent request.
struct Connection {
    socket: IpcStream,
    cancelled: Arc<AtomicBool>,
    subscriber: String,
    wake: UiMailbox<AppRequest>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Err(error) = self.socket.shutdown(std::net::Shutdown::Both) {
            if error.kind() != std::io::ErrorKind::NotConnected {
                log::debug!("events: connection shutdown: {error}");
            }
        }
        match app_timeline::global().lock() {
            Ok(mut timeline) => {
                timeline.clear_subscriber(ActorType::Agent, &self.subscriber);
                timeline.event_ready().notify_all();
            }
            Err(error) => log::error!("events: subscription cleanup failed: {error}"),
        }
        if self.wake.send(AppRequest::Wake).is_err() {
            log::debug!("events: host stopped before cancellation wake");
        }
        log::info!("events: connection {} released", self.subscriber);
    }
}

fn send(socket: &mut IpcStream, value: Value) -> bool {
    if let Err(error) = writeln!(socket, "{value}").and_then(|()| socket.flush()) {
        log::info!("events: writing connection reply failed: {error}");
        false
    } else {
        true
    }
}

fn error(socket: &mut IpcStream, message: impl AsRef<str>) {
    send(socket, json!({"type":"error", "message":message.as_ref()}));
}

fn event_line(event: &AppEventRecord, subscription: &str) -> Value {
    let mut line = json!(event);
    line["type"] = json!("event");
    line["subscription_id"] = json!(subscription);
    line
}

fn wait_match(
    records: &[AppEventRecord],
    snapshot: Result<Option<AppEventRecord>, String>,
    predicate: &str,
) -> Result<Option<AppEventRecord>, String> {
    match records
        .iter()
        .find(|event| lifecycle_matches(event, predicate))
    {
        Some(event) => Ok(Some(event.clone())),
        None => snapshot,
    }
}

pub(super) fn handle_events_subscribe(
    mut socket: IpcStream,
    remaining: Lines<BufReader<IpcStream>>,
    val: Value,
    subscribe: &UiMailbox<HostSubscribeRequest>,
    wake: &UiMailbox<AppRequest>,
    peer_ancestry: Option<&[u32]>,
) {
    let consumer = match Consumer::parse(&val) {
        Ok(consumer) => consumer,
        Err(message) => {
            error(&mut socket, message);
            return;
        }
    };
    let clone = match socket.try_clone() {
        Ok(clone) => clone,
        Err(err) => {
            error(&mut socket, format!("cloning event connection: {err}"));
            return;
        }
    };
    if let Err(err) = socket.set_write_timeout(Some(Duration::from_secs(5))) {
        error(&mut socket, format!("setting event write deadline: {err}"));
        return;
    }
    let connection = Connection {
        socket: clone,
        cancelled: Arc::new(AtomicBool::new(false)),
        subscriber: format!("cli-connection:{}", uuid::Uuid::new_v4()),
        wake: wake.clone(),
    };
    let cancelled = Arc::clone(&connection.cancelled);
    let disconnect_wake = wake.clone();
    std::thread::spawn(move || {
        for line in remaining {
            if let Err(error) = line {
                log::debug!("events: connection reader ended: {error}");
                break;
            }
        }
        cancelled.store(true, Ordering::Release);
        match app_timeline::global().lock() {
            Ok(timeline) => timeline.event_ready().notify_all(),
            Err(error) => log::error!("events: disconnect notification failed: {error}"),
        }
        if disconnect_wake.send(AppRequest::Wake).is_err() {
            log::debug!("events: host stopped before disconnect wake");
        }
    });
    let lifecycle = !matches!(consumer, Consumer::Events);
    let (reply, receiver) = mpsc::sync_channel(1);
    let request = HostSubscribeRequest {
        publisher_app_id: if lifecycle {
            PUBLISHER.into()
        } else {
            val["app_id"].as_str().unwrap_or_default().into()
        },
        event_names: if lifecycle {
            vec![STREAM.into()]
        } else {
            val["event_names"]
                .as_array()
                .map(|names| {
                    names
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        },
        payload_mode: if lifecycle {
            PayloadMode::Full
        } else {
            serde_json::from_value(val["payload_mode"].clone()).unwrap_or(PayloadMode::Full)
        },
        trigger_mode: if lifecycle {
            TriggerMode::Conversation
        } else {
            serde_json::from_value(val["trigger_mode"].clone()).unwrap_or(TriggerMode::Conversation)
        },
        resource_id: if lifecycle {
            consumer.pane().map(|id| id.to_string())
        } else {
            val["resource_id"].as_str().map(String::from)
        },
        from_pane_id: val["from_pane_id"].as_u64(),
        subscriber_override: Some(connection.subscriber.clone()),
        subscriber_type_override: None,
        // The host resolves the stable broker actor from the verified caller.
        broker_actor_override: None,
        workspace_root_override: None,
        context_id_override: None,
        peer_ancestry: peer_ancestry.map(<[u32]>::to_vec),
        cancelled: Some(Arc::clone(&connection.cancelled)),
        reply,
    };
    let app_id = request.publisher_app_id.clone();
    if subscribe.send(request).is_err() {
        error(&mut socket, "host not accepting subscriptions");
        return;
    }
    let consent_deadline = Instant::now() + Duration::from_secs(120);
    let (subscriber_type, subscriber_id, subscription_id) = loop {
        if connection.cancelled.load(Ordering::Acquire) {
            return;
        }
        if consumer.expired() {
            send(&mut socket, json!({"type":"timeout"}));
            return;
        }
        if Instant::now() >= consent_deadline {
            error(&mut socket, "subscribe consent timed out");
            return;
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(HostSubscribeReply::Ok {
                subscription_id,
                subscriber_type,
                subscriber_id,
            }) => break (subscriber_type, subscriber_id, subscription_id),
            Ok(HostSubscribeReply::Err { message }) => {
                error(&mut socket, message);
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                error(&mut socket, "host subscription service stopped");
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    log::info!("events: registered {subscription_id} app={app_id} lifecycle={lifecycle}");
    let timeline = app_timeline::global();
    let snapshot = {
        let timeline = match timeline.lock() {
            Ok(t) => t,
            Err(err) => {
                error(&mut socket, format!("timeline lock failed: {err}"));
                return;
            }
        };
        match consumer.pane() {
            Some(pane) => timeline.lifecycle_snapshot(
                &subscription_id,
                pane,
                match &consumer {
                    Consumer::Wait { predicate, .. } => Some(predicate.as_str()),
                    _ => None,
                },
            ),
            None => Ok(None),
        }
    };
    let mut snapshot = snapshot;
    if !matches!(consumer, Consumer::Wait { .. }) {
        if let Err(message) = &snapshot {
            error(&mut socket, message);
            return;
        }
    }
    if !matches!(consumer, Consumer::Wait { .. })
        && !send(
            &mut socket,
            json!({"type":"subscribed", "subscription_id":subscription_id, "app_id":app_id}),
        )
    {
        return;
    }
    loop {
        if connection.cancelled.load(Ordering::Acquire) {
            return;
        }
        let (deliveries, records) = {
            let mut timeline = match timeline.lock() {
                Ok(t) => t,
                Err(err) => {
                    error(&mut socket, format!("timeline lock failed: {err}"));
                    return;
                }
            };
            let deliveries = timeline.take_deliveries_for(subscriber_type, &subscriber_id);
            let records: Vec<_> = deliveries
                .iter()
                .filter_map(|delivery| timeline.lifecycle_record(delivery.event_id).cloned())
                .collect();
            (deliveries, records)
        };
        if let Consumer::Wait { predicate, .. } = &consumer {
            // Queue first: a transient match after registration is still a match,
            // even if the state changed again before we acquired the snapshot.
            match wait_match(
                &records,
                std::mem::replace(&mut snapshot, Ok(None)),
                predicate,
            ) {
                Ok(Some(event)) => {
                    send(&mut socket, event_line(&event, &subscription_id));
                    log::info!("pane_wait: matched {predicate} event={}", event.event_id);
                    return;
                }
                Err(message) => {
                    error(&mut socket, message);
                    return;
                }
                Ok(None) => {}
            }
            if consumer.expired() {
                send(&mut socket, json!({"type":"timeout"}));
                log::info!("pane_wait: timed out predicate={predicate}");
                return;
            }
        } else if lifecycle {
            for record in records {
                if !send(&mut socket, event_line(&record, &subscription_id)) {
                    return;
                }
            }
        } else {
            for delivery in deliveries {
                if !send(
                    &mut socket,
                    json!({"type":"event", "subscription_id":delivery.subscription_id,
                    "app_id":delivery.app_id, "event":delivery.event, "event_id":delivery.event_id,
                    "resource_id":delivery.resource_id, "trigger_mode":delivery.trigger_mode,
                    "summary":delivery.summary, "payload":delivery.payload, "state_ref":delivery.state_ref,
                    "created_at":delivery.created_at}),
                ) {
                    return;
                }
            }
        }
        // Publication and cancellation notify under this same mutex. Checking
        // the queue before sleeping makes the handover free of lost wakeups.
        let pending = match timeline.lock() {
            Ok(pending) => pending,
            Err(err) => {
                error(&mut socket, format!("timeline lock failed: {err}"));
                return;
            }
        };
        if connection.cancelled.load(Ordering::Acquire) {
            return;
        }
        if pending.has_deliveries_for(subscriber_type, &subscriber_id) {
            continue;
        }
        if let Some(pane) = consumer.pane() {
            if let Err(message) = pending.lifecycle_snapshot(&subscription_id, pane, None) {
                error(&mut socket, message);
                return;
            }
        }
        let ready = pending.event_ready();
        let result = match &consumer {
            Consumer::Wait { deadline, .. } => ready
                .wait_timeout(pending, deadline.saturating_duration_since(Instant::now()))
                .map(|(guard, _)| drop(guard))
                .map_err(|err| err.to_string()),
            _ => ready.wait(pending).map(drop).map_err(|err| err.to_string()),
        };
        if let Err(err) = result {
            error(&mut socket, format!("waiting for timeline event: {err}"));
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pane_consumer_queued_match_survives_close_before_snapshot() {
        use crate::host::pane_lifecycle::{PaneLifecycleEvent, Provenance, Source};
        let mut timeline = app_timeline::AppTimeline::default();
        let outcome = timeline
            .record_pane_lifecycle(
                1,
                7,
                &PaneLifecycleEvent::AgentIdle {
                    provenance: Provenance {
                        source: Source::HostObservation,
                        agent_label: "test".into(),
                        session_id: None,
                        raw_event: None,
                    },
                },
            )
            .unwrap();
        let event = timeline.lifecycle_record(outcome.event_id).unwrap().clone();
        let matched = wait_match(&[event], Err("pane is closed".into()), "idle")
            .unwrap()
            .unwrap();
        assert_eq!(matched.event_id, outcome.event_id);
        assert!(wait_match(&[], Err("pane is closed".into()), "idle").is_err());
    }
}
