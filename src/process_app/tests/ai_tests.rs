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
use crate::app::permissions::AppPermissions;
use crate::app_protocol::{AiMessage, AppRequest, ModelTier, PlexiEvent};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse};
use std::collections::{HashSet, VecDeque};
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
    let (app, _tx) = ProcessApp::new_for_test(
        7,
        AppPermissions {
            capabilities,
            blocked: HashSet::new(),
            is_builtin: false,
            allowed_hosts: vec![],
        },
    );
    Some(app)
}

fn make_blocked_app() -> Option<ProcessApp> {
    let mut blocked = HashSet::new();
    blocked.insert(Capability::AiQuery);
    let (app, _tx) = ProcessApp::new_for_test(
        7,
        AppPermissions {
            capabilities: HashSet::new(),
            blocked,
            is_builtin: false,
            allowed_hosts: vec![],
        },
    );
    Some(app)
}

#[test]
fn denied_app_gets_capability_denied_response() {
    // App with `ai.query` permanently BLOCKED: route_command must immediately
    // queue an AiResponse with the canonical "capability denied" error
    // — synchronously, without ever invoking the broker.
    let Some(mut app) = make_blocked_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    // Inject a broker that would *panic* if called. The blocked path
    // must short-circuit before reaching dispatch.
    struct PanicBroker;
    impl AiBroker for PanicBroker {
        fn dispatch(&self, _: AiBrokerRequest) -> AiBrokerResponse {
            panic!("blocked path must never call the broker");
        }
    }
    app.ai_broker = Arc::new(PanicBroker);

    app.route_command(AppRequest::AiQuery {
        request_id: "req-denied".to_string(),
        model_tier: ModelTier::Low,
        system: "system".to_string(),
        messages: vec![AiMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        }],
        tools: vec![],
    });

    // Blocked path is synchronous — the response is on outbound_events immediately.
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
        }
        other => panic!("expected AiResponse, got {other:?}"),
    }
}

#[test]
fn withheld_app_defers_ai_query_and_queues_consent_prompt() {
    // App without `ai.query` capability but not blocked (withheld / first-run):
    // route_command must defer the query and push a consent PendingPrompt,
    // without calling the broker or emitting an AiResponse.
    let Some(mut app) = make_app(HashSet::new()) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    struct PanicBroker;
    impl AiBroker for PanicBroker {
        fn dispatch(&self, _: AiBrokerRequest) -> AiBrokerResponse {
            panic!("withheld path must never call the broker");
        }
    }
    app.ai_broker = Arc::new(PanicBroker);

    app.route_command(AppRequest::AiQuery {
        request_id: "req-withheld".to_string(),
        model_tier: ModelTier::Low,
        system: "system".to_string(),
        messages: vec![AiMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        }],
        tools: vec![],
    });

    // No immediate AiResponse — query is deferred.
    assert!(
        !app.outbound_events
            .iter()
            .any(|e| matches!(e, PlexiEvent::AiResponse { .. })),
        "withheld path must not produce immediate AiResponse"
    );
    // Query stored in deferred queue.
    assert_eq!(app.deferred_ai_queries.len(), 1);
    assert_eq!(app.deferred_ai_queries[0].request_id, "req-withheld");
    // Consent prompt queued.
    let has_ai_prompt = app.pending_prompts.iter().any(
        |p| matches!(p, PendingPrompt::Capability { capability, .. } if capability == "ai.query"),
    );
    assert!(
        has_ai_prompt,
        "withheld path must push ai.query consent prompt"
    );

    // Second query while prompt is already pending must NOT push a duplicate prompt.
    app.route_command(AppRequest::AiQuery {
        request_id: "req-withheld-2".to_string(),
        model_tier: ModelTier::Low,
        system: "system".to_string(),
        messages: vec![],
        tools: vec![],
    });
    assert_eq!(
        app.deferred_ai_queries.len(),
        2,
        "second deferred query must be stored"
    );
    let prompt_count = app.pending_prompts.iter().filter(|p| {
        matches!(p, PendingPrompt::Capability { capability, .. } if capability == "ai.query")
    }).count();
    assert_eq!(prompt_count, 1, "must not push duplicate consent prompt");
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
        response: AiBrokerResponse::ok_with_deltas("Pong.".to_string(), 12, 4, Vec::new()),
    });

    app.route_command(AppRequest::AiQuery {
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
    assert_eq!(calls[0].app_id, "test");
    assert_eq!(calls[0].model_tier, ModelTier::High);
    assert_eq!(calls[0].system, "be terse");
    assert_eq!(calls[0].messages.len(), 1);
    assert_eq!(calls[0].messages[0].content, "ping");
}
