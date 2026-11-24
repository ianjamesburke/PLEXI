//! CLI consumers of the existing brokered event connection.
use super::ui_mailbox::UiMailbox;
use crate::broker::ActorType;
use crate::host::event_subscriptions::{HostSubscribeReply, HostSubscribeRequest};
use crate::host::pane_lifecycle::{PUBLISHER, STREAM};
use crate::host::plugin_timeline::{self, lifecycle_matches, PluginEventRecord};
use crate::platform::ipc::IpcStream;
use crate::protocol::{HostRequest, PayloadMode, TriggerMode};
use serde_json::{json, Value};
use std::io::{BufReader, Lines, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
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
    /// The exact key the subscription was registered under; cleanup clears
    /// that key and no other.
    subscriber_type: ActorType,
    subscriber: String,
    wake: UiMailbox<crate::app::socket_caller::PaneIpcMessage>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Err(error) = self.socket.shutdown(std::net::Shutdown::Both) {
            if error.kind() != std::io::ErrorKind::NotConnected {
                log::debug!("events: connection shutdown: {error}");
            }
        }
        match plugin_timeline::global().lock() {
            Ok(mut timeline) => {
                timeline.clear_subscriber(self.subscriber_type, &self.subscriber);
                timeline.event_ready().notify_all();
            }
            Err(error) => log::error!("events: subscription cleanup failed: {error}"),
        }
        if self
            .wake
            .send(crate::app::socket_caller::PaneIpcMessage::host(HostRequest::Wake))
            .is_err()
        {
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

fn event_line(event: &PluginEventRecord, subscription: &str) -> Value {
    let mut line = json!(event);
    line["type"] = json!("event");
    line["subscription_id"] = json!(subscription);
    line
}

fn wait_match(
    records: &[PluginEventRecord],
    snapshot: Result<Option<PluginEventRecord>, String>,
    predicate: &str,
) -> Result<Option<PluginEventRecord>, String> {
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
    wake: &UiMailbox<crate::app::socket_caller::PaneIpcMessage>,
    caller: crate::broker::caller::SocketCaller,
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
    // The delivery key is final before the registration is enqueued, so a
    // connection that drops before reading the reply clears exactly what
    // was registered (Codex 3290 r2).
    let subscriber_type = match &caller {
        crate::broker::caller::SocketCaller::External(_) => ActorType::External,
        _ => ActorType::Agent,
    };
    let connection = Connection {
        socket: clone,
        cancelled: Arc::new(AtomicBool::new(false)),
        subscriber_type,
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
        match plugin_timeline::global().lock() {
            Ok(timeline) => timeline.event_ready().notify_all(),
            Err(error) => log::error!("events: disconnect notification failed: {error}"),
        }
        if disconnect_wake
            .send(crate::app::socket_caller::PaneIpcMessage::host(HostRequest::Wake))
            .is_err()
        {
            log::debug!("events: host stopped before disconnect wake");
        }
    });
    let lifecycle = !matches!(consumer, Consumer::Events);
    let (reply, receiver) = mpsc::sync_channel(1);
    let request = HostSubscribeRequest {
        publisher_plugin_id: if lifecycle {
            PUBLISHER.into()
        } else {
            val["plugin_id"].as_str().unwrap_or_default().into()
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
        // Never from the wire: the admitted caller names the pane.
        from_pane_id: None,
        subscriber_override: Some(connection.subscriber.clone()),
        subscriber_type_override: Some(connection.subscriber_type),
        // The host resolves the stable broker actor from the verified caller.
        broker_actor_override: None,
        workspace_root_override: None,
        // Resolved on the host thread from the verified peer.
        subscriber_authority: crate::host::event_subscriptions::SubscriberAuthority::External,
        context_id_override: None,
        socket_caller: Some(caller),
        cancelled: Some(Arc::clone(&connection.cancelled)),
        reply,
    };
    let plugin_id = request.publisher_plugin_id.clone();
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
    if (subscriber_type, subscriber_id.as_str())
        != (connection.subscriber_type, connection.subscriber.as_str())
    {
        log::error!(
            "events: {subscription_id} registered as {subscriber_type:?} '{subscriber_id}', not the connection's key"
        );
        error(&mut socket, "subscription registered under an unexpected key");
        return;
    }
    log::info!("events: registered {subscription_id} plugin={plugin_id} lifecycle={lifecycle}");
    let timeline = plugin_timeline::global();
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
            json!({"type":"subscribed", "subscription_id":subscription_id, "plugin_id":plugin_id}),
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
                    "plugin_id":delivery.plugin_id, "event":delivery.event, "event_id":delivery.event_id,
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

    /// Codex 3290 r2/r3: an outside process's subscription is registered
    /// under the key its connection bound before the request was enqueued.
    /// The client goes away after registration, with a delivery already
    /// queued, and the connection ends before it consumes the reply: it
    /// clears exactly that key, so no subscription and no delivery remain.
    /// The interleaving is fixed: the disconnect reader's wake is the
    /// barrier, the worker is joined before the reply is offered, and the
    /// reply finding no receiver proves it was never consumed.
    #[test]
    fn an_outside_subscription_cancelled_before_its_reply_leaves_nothing() {
        use std::io::BufRead as _;
        const CONTEXT: u64 = 1_285_000;
        let ctx = egui::Context::default();
        let wake = Arc::new(crate::app::ui_mailbox::EguiWake::new(ctx));
        let (subscribe, subscribe_rx) =
            UiMailbox::<HostSubscribeRequest>::channel(wake.clone(), "event_subscribe");
        let (host, host_rx) =
            UiMailbox::<crate::app::socket_caller::PaneIpcMessage>::channel(wake, "pane_ipc");
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        let lines = BufReader::new(server.try_clone().unwrap()).lines();
        let caller = crate::broker::caller::SocketCaller::External(
            crate::broker::caller::PeerWitness::capture(std::process::id()),
        );
        let frame = json!({"type": "events_subscribe", "plugin_id": "probe-1285",
            "event_names": ["tick"]});
        let worker = std::thread::spawn(move || {
            handle_events_subscribe(server, lines, frame, &subscribe, &host, caller)
        });
        let request = subscribe_rx.recv().unwrap();
        assert_eq!(
            request.subscriber_type_override,
            Some(ActorType::External),
            "the key is bound before enqueue"
        );
        let key = request.subscriber_override.clone().unwrap();
        // Registered as the host registers an outside caller, and one event
        // delivered to it before the client is gone.
        {
            let global = plugin_timeline::global();
            let mut timeline = global.lock().unwrap();
            timeline
                .declare_streams(
                    CONTEXT,
                    "probe-1285",
                    vec![crate::protocol::EventStreamDecl {
                        name: "tick".into(),
                        schema: json!({"type": "object"}),
                        description: None,
                    }],
                )
                .unwrap();
            timeline.add_subscription(plugin_timeline::SubscriptionRecord {
                subscription_id: format!("sub-{key}"),
                subscriber_type: ActorType::External,
                subscriber_id: key.clone(),
                plugin_id: "probe-1285".into(),
                event_names: vec!["tick".into()],
                payload_mode: PayloadMode::Full,
                trigger_mode: TriggerMode::Conversation,
                resource_id: None,
                duration: crate::broker::GrantDuration::Session,
                subscriber_context_id: CONTEXT,
                created_at: "test".into(),
                admitted: None,
            });
            timeline
                .record_event(
                    CONTEXT,
                    "probe-1285",
                    1,
                    plugin_timeline::EmittedEvent {
                        event: "tick".into(),
                        actor: crate::protocol::PluginEventActor::System,
                        actor_id: None,
                        caused_by: None,
                        summary: "tick".into(),
                        resource_id: "r".into(),
                        resource_scope: None,
                        revision_after: "1".into(),
                        payload: Some(json!({})),
                        state_ref: None,
                        revision_before: None,
                        rollback_token: None,
                        changed_resources: vec![],
                        suggested_trigger: None,
                    },
                )
                .unwrap();
            assert!(
                timeline.has_deliveries_for(ActorType::External, &key),
                "fixture: a delivery is queued for the subscriber"
            );
        }
        drop(client);
        // Barrier: the disconnect reader saw the client go and woke the host.
        match host_rx.recv().unwrap() {
            crate::app::socket_caller::PaneIpcMessage::Request { request, .. } => {
                assert!(matches!(*request, HostRequest::Wake))
            }
            other => panic!("expected the disconnect wake, got {other:?}"),
        }
        worker.join().unwrap();
        assert!(
            request
                .reply
                .send(HostSubscribeReply::Ok {
                    subscription_id: format!("sub-{key}"),
                    subscriber_type: ActorType::External,
                    subscriber_id: key.clone(),
                })
                .is_err(),
            "the connection consumed its reply"
        );
        let global = plugin_timeline::global();
        let timeline = global.lock().unwrap();
        assert!(
            !timeline.subscriptions().iter().any(|sub| sub.subscriber_id == key),
            "a subscription outlived its connection"
        );
        assert!(
            !timeline.has_deliveries_for(ActorType::External, &key),
            "a delivery outlived its connection"
        );
    }

    #[test]
    fn pane_consumer_queued_match_survives_close_before_snapshot() {
        use crate::host::pane_lifecycle::{PaneLifecycleEvent, Provenance, Source};
        let mut timeline = plugin_timeline::PluginTimeline::default();
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
