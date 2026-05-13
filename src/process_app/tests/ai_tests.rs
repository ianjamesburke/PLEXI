//! Routing tests for the v3.3 `ai.query` broker capability (#284).
//!
//! Two paths under test:
//!   1. App without `ai.query` capability — synchronous denial response
//!      lands on `outbound_events`. No broker dispatch occurs.
//!   2. App with `ai.query` capability — broker is invoked once and
//!      its response surfaces on `http_rx` as `PlexiEvent::AiResponse`.
//!
//! The mock broker (`CannedBroker`) records every call so the granted
//! path also confirms that the routing layer forwarded the right
//! `model_tier`, `system`, and `messages` payload to the broker.
use super::super::*;
use crate::app_protocol::{AiMessage, HostCommand, ModelTier, PlexiEvent};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Test broker: records every dispatch and returns a canned response.
struct CannedBroker {
    seen: Arc<Mutex<Vec<AiBrokerRequest>>>,
    response: AiBrokerResponse,
}

impl AiBroker for CannedBroker {
    fn dispatch(&self, request: AiBrokerRequest) -> AiBrokerResponse {
        self.seen.lock().unwrap().push(request);
        self.response.clone()
    }
}

fn make_app(capabilities: HashSet<Capability>) -> Option<ProcessApp> {
    let sh = ["/bin/sh", "/usr/bin/sh"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(PathBuf::from)?;
    let workspace_root = std::env::temp_dir();
    ProcessApp::launch(
        "test_ai",
        "Test AI",
        &sh,
        &workspace_root,
        &["-c".to_string(), "sleep 1".to_string()],
        workspace_root.clone(),
        capabilities,
        false,
        None,
    )
    .ok()
}

#[test]
fn denied_app_gets_capability_denied_response() {
    // App without `ai.query` capability: route_command must immediately
    // queue an AiResponse with the canonical "capability denied" error
    // — synchronously, without ever invoking the broker.
    let Some(mut app) = make_app(HashSet::new()) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    // Inject a broker that would *panic* if called. The denied path
    // must short-circuit before reaching dispatch.
    struct PanicBroker;
    impl AiBroker for PanicBroker {
        fn dispatch(&self, _: AiBrokerRequest) -> AiBrokerResponse {
            panic!("denied path must never call the broker");
        }
    }
    app.ai_broker = Arc::new(PanicBroker);

    app.route_command(HostCommand::AiQuery {
        request_id: "req-denied".to_string(),
        model_tier: ModelTier::Low,
        system: "system".to_string(),
        messages: vec![AiMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        }],
        tools: vec![],
    });

    // Denied path is synchronous — the response is on outbound_events
    // immediately, no thread, no http_rx wait.
    let resp = app
        .outbound_events
        .iter()
        .find(|e| matches!(e, PlexiEvent::AiResponse { .. }))
        .expect("expected AiResponse on outbound queue");
    match resp {
        PlexiEvent::AiResponse {
            request_id,
            content,
            tokens_in,
            tokens_out,
            error,
        } => {
            assert_eq!(request_id, "req-denied");
            assert!(content.is_none());
            assert_eq!(*tokens_in, 0);
            assert_eq!(*tokens_out, 0);
            let err = error.as_ref().expect("error must be set on denial");
            assert!(
                err.contains("capability denied"),
                "denial message must say `capability denied`: {err}"
            );
            assert!(
                err.contains("ai.query"),
                "denial message must name the capability: {err}"
            );
        }
        other => panic!("expected AiResponse, got {other:?}"),
    }
}

#[test]
fn granted_app_dispatches_to_broker() {
    // App WITH `ai.query` granted: route_command must spawn a worker
    // that calls broker.dispatch exactly once with the right payload,
    // and the broker's response must arrive as a PlexiEvent::AiResponse
    // on http_rx.
    let mut caps = HashSet::new();
    caps.insert(Capability::AiQuery);
    let Some(mut app) = make_app(caps) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    let seen: Arc<Mutex<Vec<AiBrokerRequest>>> = Arc::new(Mutex::new(Vec::new()));
    app.ai_broker = Arc::new(CannedBroker {
        seen: Arc::clone(&seen),
        response: AiBrokerResponse::ok("Pong.".to_string(), 12, 4),
    });

    app.route_command(HostCommand::AiQuery {
        request_id: "req-ok".to_string(),
        model_tier: ModelTier::High,
        system: "be terse".to_string(),
        messages: vec![AiMessage {
            role: "user".to_string(),
            content: "ping".to_string(),
        }],
        tools: vec![],
    });

    // Worker thread is spawned — wait briefly for response to arrive
    // on http_rx. 2s is generous; canned broker is in-memory so the
    // typical wait is microseconds.
    let event = app
        .http_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("broker response must arrive on http_rx within 2s");

    match event {
        PlexiEvent::AiResponse {
            request_id,
            content,
            tokens_in,
            tokens_out,
            error,
        } => {
            assert_eq!(request_id, "req-ok");
            assert_eq!(content.as_deref(), Some("Pong."));
            assert_eq!(tokens_in, 12);
            assert_eq!(tokens_out, 4);
            assert!(error.is_none());
        }
        other => panic!("expected AiResponse, got {other:?}"),
    }

    // Broker must have been invoked exactly once with the correct payload.
    let calls = seen.lock().unwrap();
    assert_eq!(calls.len(), 1, "broker must be called exactly once");
    assert_eq!(calls[0].app_id, "test_ai");
    assert_eq!(calls[0].model_tier, ModelTier::High);
    assert_eq!(calls[0].system, "be terse");
    assert_eq!(calls[0].messages.len(), 1);
    assert_eq!(calls[0].messages[0].content, "ping");
}
