//! Emit OSC 7501 reports for Plexi's own agents.
//!
//! Variant chosen: **CLI-side**. `plexi agent report/ask/done` run as child
//! processes inside the agent's pane, so a write to the controlling tty is
//! pane *output*, which is exactly what the pane's own parser (Plexi-in-Plexi)
//! and any outer terminal (Ghostty, kitty, Rex) read. The host cannot do this:
//! `BackendCommand::Write` feeds the program's stdin, not its output.
//! The agent-tree id comes from `PLEXI_AGENT_ID`, stamped at spawn by
//! `spawn_agent_worker_pane`; without it the sanitized `PLEXI_AGENT_ROLE`
//! stands alone. A process with neither is not a Plexi agent and emits nothing.
use super::parser::MAX_SEQUENCE;
use super::records::State;
use base64::Engine;
use std::io::{IsTerminal, Write};

/// `app` value on every report Plexi agents emit.
pub const APP: &str = "plexi-agent";
const SEGMENT_MAX: usize = 32;
const ID_DEPTH_MAX: usize = 8;
const ID_MAX: usize = 128;
const MSG_DECODED_MAX: usize = 2048;
const TITLE_DECODED_MAX: usize = 192;

fn sanitize_segment(raw: &str) -> Option<String> {
    let s: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(SEGMENT_MAX)
        .collect();
    (!s.is_empty()).then_some(s)
}

/// Join agent names from the root lead down to this agent into a hierarchical
/// id. Names are sanitized to `[A-Za-z0-9_.+-]{1,32}`; past 8 levels or 128
/// bytes the outermost ancestors are dropped so the agent itself stays last.
pub fn tree_id(names: &[&str]) -> Option<String> {
    let mut segments: Vec<String> = names.iter().filter_map(|n| sanitize_segment(n)).collect();
    if segments.len() > ID_DEPTH_MAX {
        segments.drain(..segments.len() - ID_DEPTH_MAX);
    }
    while segments.len() > 1 && segments.join("/").len() > ID_MAX {
        segments.remove(0);
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

fn clean_text(text: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if out.len() + c.len_utf8() > max_bytes {
            break;
        }
        out.push(c);
    }
    out
}

/// Encode one spec-valid report: standard base64 `msg`/`title`, no `:` inside
/// any value, ST (`ESC \`) terminator. `kind` is kept only for blocked.
pub fn encode_report(
    id: Option<&str>,
    state: State,
    kind: Option<&str>,
    app: &str,
    title: Option<&str>,
    msg: Option<&str>,
) -> Vec<u8> {
    let b64 = |text: &str, max: usize| {
        let clean = clean_text(text, max);
        (!clean.trim().is_empty())
            .then(|| base64::engine::general_purpose::STANDARD.encode(clean.as_bytes()))
    };
    let mut pairs = vec![format!("state={}", state.as_str())];
    if let Some(id) = id.and_then(tree_id_of_joined) {
        pairs.push(format!("id={id}"));
    }
    if state == State::Blocked {
        if let Some(kind) = kind.filter(|k| matches!(*k, "permission" | "question" | "auth")) {
            pairs.push(format!("kind={kind}"));
        }
    }
    if let Some(app) = sanitize_segment(app) {
        pairs.push(format!("app={app}"));
    }
    if let Some(title) = title.and_then(|t| b64(t, TITLE_DECODED_MAX)) {
        pairs.push(format!("title={title}"));
    }
    if let Some(msg) = msg.and_then(|m| b64(m, MSG_DECODED_MAX)) {
        pairs.push(format!("msg={msg}"));
    }
    let mut out = format!("\x1b]7501;{}", pairs.join(":")).into_bytes();
    out.extend_from_slice(b"\x1b\\");
    debug_assert!(out.len() <= MAX_SEQUENCE);
    out
}

/// Re-validate an already joined id (`a/b/c`) through the same sanitizer.
fn tree_id_of_joined(id: &str) -> Option<String> {
    let names: Vec<&str> = id.split('/').collect();
    tree_id(&names)
}

/// True when an `agent done --result` JSON payload reports a failure.
pub fn done_is_failure(result: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(result) else {
        return false;
    };
    let status_failed = ["status", "result", "outcome"].iter().any(|key| {
        value.get(key).and_then(|v| v.as_str()).is_some_and(|s| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                "fail" | "failed" | "failure" | "error"
            )
        })
    });
    status_failed || value.get("ok").and_then(|v| v.as_bool()) == Some(false)
}

/// This process's agent id and display title, from the spawn-time env.
fn identity_from_env() -> Option<(String, String)> {
    let role = std::env::var("PLEXI_AGENT_ROLE").ok();
    let id = std::env::var("PLEXI_AGENT_ID")
        .ok()
        .and_then(|v| tree_id_of_joined(&v))
        .or_else(|| role.as_deref().and_then(|r| tree_id(&[r])))?;
    let title = role
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(&id).to_owned());
    Some((id, title))
}

/// Write `bytes` to the controlling tty, falling back to stdout only when
/// stdout is itself a tty. Best effort: failures are logged, never returned.
fn write_to_tty(bytes: &[u8]) {
    let tty = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .and_then(|mut f| f.write_all(bytes).and_then(|_| f.flush()));
    match tty {
        Ok(()) => {}
        Err(e) => {
            let stdout = std::io::stdout();
            if stdout.is_terminal() {
                let mut lock = stdout.lock();
                if let Err(e2) = lock.write_all(bytes).and_then(|_| lock.flush()) {
                    log::warn!("program_status: emit failed tty={e} stdout={e2}");
                }
            } else {
                log::warn!("program_status: emit skipped, no tty: {e}");
            }
        }
    }
}

/// Emit one report for the current Plexi agent pane. No-op outside a Plexi
/// pane (`PLEXI_PANE_ID` unset) or when the process has no agent identity.
pub fn emit_agent_report(state: State, kind: Option<&str>, msg: Option<&str>) {
    if std::env::var_os("PLEXI_PANE_ID").is_none() {
        return;
    }
    let Some((id, title)) = identity_from_env() else {
        return;
    };
    let bytes = encode_report(Some(&id), state, kind, APP, Some(&title), msg);
    log::info!(
        "program_status: emit id={id} state={}",
        state.as_str()
    );
    write_to_tty(&bytes);
}
