//! Host-command tests for `plexi assistant send` (`SubmitAssistantTurn`).
//!
//! The command's JSON reply is the file the CLI prints for `--json`. A stub
//! broker stands in for the model provider.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::model::{PendingPermission, PermissionChoice, Turn, TurnRole};
use super::AssistantApp;
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse};
use crate::plexi_ai::turn_loop::TurnDelta;
use crate::protocol::AppRequest;
use crate::testing::HostHarness;

struct ReplyBroker {
    reply: String,
    error: Option<String>,
    calls: AtomicUsize,
}

impl AiBroker for ReplyBroker {
    fn dispatch(
        &self,
        _request: AiBrokerRequest,
        on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> AiBrokerResponse {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = &self.error {
            return AiBrokerResponse::err(error.clone());
        }
        on_delta(TurnDelta::Text(&self.reply));
        AiBrokerResponse::ok(self.reply.clone(), 1, 1)
    }
}

struct Gate {
    ready: Mutex<bool>,
    cv: Condvar,
}

struct GatedBroker {
    gate: Arc<Gate>,
    calls: AtomicUsize,
}

impl AiBroker for GatedBroker {
    fn dispatch(
        &self,
        request: AiBrokerRequest,
        on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> AiBrokerResponse {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            let mut ready = self.gate.ready.lock().unwrap();
            while !*ready {
                ready = self.gate.cv.wait(ready).unwrap();
            }
            drop(ready);
            on_delta(TurnDelta::Text("DESKTOP_REPLY"));
            return AiBrokerResponse::ok("DESKTOP_REPLY".to_string(), 1, 1);
        }
        let last_user = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.clone())
            .unwrap_or_default();
        let reply = format!("PHONE:{last_user}");
        on_delta(TurnDelta::Text(&reply));
        AiBrokerResponse::ok(reply, 1, 1)
    }
}

/// Asks for a non-read-only host tool so the real permission sheet opens.
/// A denial becomes the turn error. An approval returns a reply the desktop
/// transcript must not receive.
struct AskingBroker;

impl AiBroker for AskingBroker {
    fn dispatch(
        &self,
        request: AiBrokerRequest,
        on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> AiBrokerResponse {
        if let Some(dispatcher) = &request.tool_dispatcher {
            let result = dispatcher.dispatch_call(
                "call-1".to_string(),
                "host.files.write",
                r#"{"path":"x","content":"y"}"#.to_string(),
            );
            if let Some(error) = result.error {
                if error.contains("denied") {
                    return AiBrokerResponse::err(error);
                }
            }
        }
        let reply = "approved-reply";
        on_delta(TurnDelta::Text(reply));
        AiBrokerResponse::ok(reply.to_string(), 1, 1)
    }
}

fn response_path() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("assistant-send.json");
    (dir, path)
}

fn submit(h: &HostHarness, path: &Path, request_id: &str, text: &str) {
    submit_to(h, path, request_id, text, Some("phone-session"), false);
}

fn submit_to(
    h: &HostHarness,
    path: &Path,
    request_id: &str,
    text: &str,
    conversation_id: Option<&str>,
    join_desktop: bool,
) {
    h.inject_ipc(AppRequest::SubmitAssistantTurn {
        text: text.to_string(),
        request_id: request_id.to_string(),
        response_file: path.to_string_lossy().into_owned(),
        pane_id: None,
        context_id: None,
        conversation_id: conversation_id.map(str::to_string),
        join_desktop,
        status_for: None,
    });
}

fn status_for(h: &HostHarness, path: &Path, request_id: &str, turn_id: &str) {
    h.inject_ipc(AppRequest::SubmitAssistantTurn {
        text: String::new(),
        request_id: request_id.to_string(),
        response_file: path.to_string_lossy().into_owned(),
        pane_id: None,
        context_id: None,
        conversation_id: None,
        join_desktop: false,
        status_for: Some(turn_id.to_string()),
    });
}

fn pump_until_file(h: &mut HostHarness, path: &Path) -> String {
    let started = Instant::now();
    loop {
        if let Ok(body) = std::fs::read_to_string(path) {
            if !body.is_empty() {
                return body;
            }
        }
        h.hidden_frame();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timed out waiting for assistant send JSON at {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn submit_desktop(h: &mut HostHarness, pane: u64, text: &str) {
    let assistant: &mut AssistantApp = h.assistant_mut(pane);
    assistant.model.composer = text.to_string();
    let effects = assistant.model.submit();
    assistant.execute_effects(effects);
}

#[test]
fn assistant_send_json_returns_turn_id_and_stub_reply() {
    let broker = Arc::new(ReplyBroker {
        reply: "stub-reply".to_string(),
        error: None,
        calls: AtomicUsize::new(0),
    });
    let mut h = HostHarness::new();
    h.add_assistant_pane_with_broker(0, broker.clone());
    let (_dir, path) = response_path();
    submit(&h, &path, "phone-1", "hello from the phone");
    let body = pump_until_file(&mut h, &path);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["request_id"], "phone-1");
    assert_eq!(value["state"], "succeeded");
    assert_eq!(value["reply"], "stub-reply");
    let turn_id = value["turn_id"].as_str().expect("turn id");
    assert!(turn_id.starts_with("turn-"), "{turn_id}");
    assert_eq!(broker.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn assistant_send_does_not_attribute_a_concurrent_desktop_turn() {
    let gate = Arc::new(Gate {
        ready: Mutex::new(false),
        cv: Condvar::new(),
    });
    let broker = Arc::new(GatedBroker {
        gate: gate.clone(),
        calls: AtomicUsize::new(0),
    });
    let mut h = HostHarness::new();
    let pane = h.add_assistant_pane_with_broker(0, broker.clone());
    submit_desktop(&mut h, pane, "typed on the desktop");
    let started = Instant::now();
    while broker.calls.load(Ordering::SeqCst) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "desktop turn never dispatched"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let desktop_turn_id = h
        .assistant_mut(pane)
        .model
        .active_turn_id
        .clone()
        .expect("desktop turn id");

    let (_dir, path) = response_path();
    submit(&h, &path, "phone-1", "from the phone");
    let body = pump_until_file(&mut h, &path);
    assert!(
        !*gate.ready.lock().unwrap(),
        "the phone reply must not wait for the desktop turn"
    );
    assert_eq!(
        h.assistant_mut(pane).model.active_turn_id.as_deref(),
        Some(desktop_turn_id.as_str()),
        "the desktop turn stays in flight"
    );
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["request_id"], "phone-1");
    assert_eq!(value["state"], "succeeded");
    assert_eq!(value["reply"], "PHONE:from the phone");
    assert_ne!(value["reply"], "DESKTOP_REPLY");
    assert_ne!(value["turn_id"], desktop_turn_id);
    assert!(value["turn_id"].as_str().unwrap().starts_with("turn-"));
}

#[test]
fn assistant_send_json_surfaces_provider_error() {
    let broker = Arc::new(ReplyBroker {
        reply: String::new(),
        error: Some("provider rejected the prompt".to_string()),
        calls: AtomicUsize::new(0),
    });
    let mut h = HostHarness::new();
    h.add_assistant_pane_with_broker(0, broker);
    let (_dir, path) = response_path();
    submit(&h, &path, "phone-err", "hello");
    let body = pump_until_file(&mut h, &path);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["request_id"], "phone-err");
    assert_eq!(value["state"], "failed");
    assert_eq!(value["error"], "provider rejected the prompt");
    assert!(value["turn_id"].as_str().unwrap().starts_with("turn-"));
}

#[test]
fn assistant_send_returns_pending_permission_without_hanging() {
    let broker = Arc::new(ReplyBroker {
        reply: "should-not-run".to_string(),
        error: None,
        calls: AtomicUsize::new(0),
    });
    let mut h = HostHarness::new();
    let pane = h.add_assistant_pane_with_broker(0, broker.clone());
    {
        let assistant = h.assistant_mut(pane);
        assistant.model.active_turn_id = Some("turn-desktop-blocked".to_string());
        assistant.model.pending_permission = Some(PendingPermission {
            tool: "host.files.write".to_string(),
            input_summary: "path=x".to_string(),
            selected: 0,
            source: None,
        });
    }
    let (_dir, path) = response_path();
    submit(&h, &path, "phone-wait", "please do the thing");
    let body = pump_until_file(&mut h, &path);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["request_id"], "phone-wait");
    assert_eq!(value["state"], "waiting_for_permission");
    assert_eq!(value["status"], "waiting for approval on desktop");
    assert_eq!(value["pending_request_id"], "turn-desktop-blocked");
    assert!(value.get("turn_id").is_none());
    assert_eq!(broker.calls.load(Ordering::SeqCst), 0);
    assert!(
        h.assistant_mut(pane).model.pending_permission.is_some(),
        "reading the pending sheet must not resolve it"
    );
}

#[test]
fn assistant_send_reports_when_its_own_turn_hits_a_permission_prompt() {
    let mut h = HostHarness::new();
    let pane = h.add_assistant_pane_with_broker(0, Arc::new(AskingBroker));
    let (_dir, path) = response_path();
    submit(&h, &path, "phone-ask", "write a file");
    let body = pump_until_file(&mut h, &path);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["state"], "waiting_for_permission");
    assert_eq!(value["status"], "waiting for approval on desktop");
    assert_eq!(value["request_id"], "phone-ask");
    let turn_id = value["turn_id"].as_str().unwrap().to_string();
    assert_eq!(value["pending_request_id"], turn_id);
    assert!(
        h.assistant_mut(pane).model.pending_permission.is_some(),
        "the desktop sheet stays up"
    );

    let prompt = h
        .assistant_mut(pane)
        .model
        .pending_permission
        .as_ref()
        .unwrap()
        .prompt_line();
    assert!(
        prompt.contains("requested from a phone/CLI turn"),
        "{prompt}"
    );

    let (_dir, next_path) = response_path();
    submit(&h, &next_path, "phone-next", "while the sheet is up");
    let next = pump_until_file(&mut h, &next_path);
    let next: serde_json::Value = serde_json::from_str(&next).unwrap();
    assert_eq!(next["request_id"], "phone-next");
    assert_eq!(next["state"], "waiting_for_permission");
    assert_eq!(next["pending_request_id"], turn_id);
    assert_ne!(next["pending_request_id"], "phone-next");

    h.assistant_mut(pane)
        .resolve_permission(PermissionChoice::Deny);
    let outcome = poll_until_settled(&mut h, &turn_id);
    assert_eq!(outcome["state"], "failed");
    assert!(
        outcome["error"].as_str().unwrap_or("").contains("denied"),
        "{outcome}"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    let after: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert_eq!(after["state"], "waiting_for_permission");
    assert!(after.get("reply").is_none());
    let assistant = h.assistant_mut(pane);
    assert!(
        assistant
            .model
            .turns
            .iter()
            .all(|turn| !turn.text.contains("denied by user")),
        "a phone denial must not land in the desktop transcript"
    );
    let side = assistant
        .side_transcripts
        .get("phone-session")
        .expect("phone conversation");
    assert!(
        side.iter().any(|turn| turn.text.contains("denied by user")),
        "the deny row belongs in the phone conversation: {side:?}"
    );
}

fn poll_until_settled(h: &mut HostHarness, turn_id: &str) -> serde_json::Value {
    let started = Instant::now();
    loop {
        let (_dir, path) = response_path();
        status_for(h, &path, "poll", turn_id);
        let body = pump_until_file(h, &path);
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        if value["state"] != "waiting_for_permission" {
            return value;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "permission outcome never left waiting"
        );
        h.hidden_frame();
    }
}

#[test]
fn assistant_send_status_poll_returns_the_reply_after_desktop_approval() {
    let mut h = HostHarness::new();
    let pane = h.add_assistant_pane_with_broker(0, Arc::new(AskingBroker));
    let (_dir, path) = response_path();
    submit(&h, &path, "phone-allow", "write a file");
    let body = pump_until_file(&mut h, &path);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    let turn_id = value["turn_id"].as_str().unwrap().to_string();
    assert_eq!(value["pending_request_id"], turn_id);

    h.assistant_mut(pane)
        .resolve_permission(PermissionChoice::AllowOnce);
    let outcome = poll_until_settled(&mut h, &turn_id);
    assert_eq!(outcome["state"], "succeeded");
    assert_eq!(outcome["reply"], "approved-reply");
    assert_eq!(outcome["turn_id"], turn_id);
    assert!(h
        .assistant_mut(pane)
        .model
        .turns
        .iter()
        .all(|turn| !turn.text.contains("approved-reply")));
}

struct HistoryBroker {
    seen: Mutex<Vec<Vec<String>>>,
}

impl AiBroker for HistoryBroker {
    fn dispatch(
        &self,
        request: AiBrokerRequest,
        on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> AiBrokerResponse {
        let contents: Vec<String> = request
            .messages
            .iter()
            .map(|message| message.content.clone())
            .collect();
        let last = contents.last().cloned().unwrap_or_default();
        self.seen.lock().unwrap().push(contents);
        let reply = format!("echo:{last}");
        on_delta(TurnDelta::Text(&reply));
        AiBrokerResponse::ok(reply, 1, 1)
    }
}

#[test]
fn assistant_send_keeps_phone_turns_out_of_the_desktop_transcript() {
    let broker = Arc::new(HistoryBroker {
        seen: Mutex::new(Vec::new()),
    });
    let mut h = HostHarness::new();
    let pane = h.add_assistant_pane_with_broker(0, broker.clone());
    {
        let assistant = h.assistant_mut(pane);
        assistant
            .model
            .turns
            .push(Turn::now(TurnRole::User, "chess move e4".to_string()));
        assistant
            .model
            .turns
            .push(Turn::now(TurnRole::Assistant, "played e4".to_string()));
    }

    let (_dir, path) = response_path();
    submit_to(
        &h,
        &path,
        "phone-pong",
        "reply PONG",
        Some("phone-session"),
        false,
    );
    let body = pump_until_file(&mut h, &path);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["state"], "succeeded");
    assert_eq!(value["reply"], "echo:reply PONG");
    assert_eq!(value["conversation_id"], "phone-session");
    let first = broker.seen.lock().unwrap()[0].join("\n");
    assert!(!first.contains("chess"), "{first}");
    assert!(!first.contains("e4"), "{first}");
    assert!(h
        .assistant_mut(pane)
        .model
        .turns
        .iter()
        .all(|turn| !turn.text.contains("PONG")));

    let (_dir, path) = response_path();
    submit_to(
        &h,
        &path,
        "phone-next",
        "what did I say",
        Some("phone-session"),
        false,
    );
    let body = pump_until_file(&mut h, &path);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["conversation_id"], "phone-session");
    let second = broker.seen.lock().unwrap()[1].join("\n");
    assert!(second.contains("echo:reply PONG"), "{second}");
    assert!(second.contains("what did I say"), "{second}");
    assert!(!second.contains("chess"), "{second}");
}

#[test]
fn assistant_send_desktop_opt_in_uses_the_desktop_transcript() {
    let broker = Arc::new(HistoryBroker {
        seen: Mutex::new(Vec::new()),
    });
    let mut h = HostHarness::new();
    let pane = h.add_assistant_pane_with_broker(0, broker.clone());
    {
        let assistant = h.assistant_mut(pane);
        assistant
            .model
            .turns
            .push(Turn::now(TurnRole::User, "chess move e4".to_string()));
        assistant
            .model
            .turns
            .push(Turn::now(TurnRole::Assistant, "played e4".to_string()));
    }
    let (_dir, path) = response_path();
    submit_to(&h, &path, "phone-join", "reply PONG", None, true);
    let body = pump_until_file(&mut h, &path);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["reply"], "echo:reply PONG");
    let seen = broker.seen.lock().unwrap()[0].join("\n");
    assert!(seen.contains("chess move e4"), "{seen}");
    assert!(seen.contains("reply PONG"), "{seen}");
    assert!(h
        .assistant_mut(pane)
        .model
        .turns
        .iter()
        .any(|turn| turn.text.contains("reply PONG")));
}
