use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::broker::caller::PaneCaller;
use crate::broker::gate::{
    NeedsYouDetails, NeedsYouFile, NeedsYouKind, NeedsYouQuestion, NeedsYouResolution,
    PermissionMonitor,
};
use crate::broker::pane_authority::LivePaneAuthority;
use crate::broker::principal::Principal;
use crate::rpc::{write_json_response, ReplyRegistry};

use super::PlexiApp;

pub(crate) const TOOL: &str = "AskUserQuestion";
pub(crate) const HOOK_TIMEOUT_SECS: u64 = 1800;
// Leave time for the command to return before Claude's own hook timeout.
const CARD_TTL_SECS: i64 = HOOK_TIMEOUT_SECS as i64 - 10;
type Answers = BTreeMap<String, String>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CallKey {
    owner: String,
    session: String,
    tool_use: String,
}

impl CallKey {
    fn operation(&self) -> String {
        format!("claude-question:{}", json!([self.session, self.tool_use]))
    }
}

struct Call {
    authority: Arc<LivePaneAuthority>,
    input: Value,
    questions: Vec<NeedsYouQuestion>,
    expires: i64,
    ask: Option<Ask>,
}

struct Ask {
    id: String,
    // Only this connection receives the answer. Once released, no retry can
    // claim it while we wait for Claude to report the effective tool result.
    waiter: Option<String>,
}

#[derive(Default)]
pub(super) struct ClaudeQuestions {
    calls: HashMap<CallKey, Call>,
}

fn read_questions(input: &Value) -> Option<Vec<NeedsYouQuestion>> {
    let questions: Vec<NeedsYouQuestion> =
        serde_json::from_value(input.get("questions")?.clone()).ok()?;
    let mut texts = HashSet::new();
    if !(1..=4).contains(&questions.len())
        || questions.iter().any(|question| {
            let mut labels = HashSet::new();
            question.question.trim().is_empty()
                || !texts.insert(&question.question)
                || !(2..=4).contains(&question.options.len())
                || question
                    .options
                    .iter()
                    .any(|option| option.label.trim().is_empty() || !labels.insert(&option.label))
        })
    {
        return None;
    }
    Some(questions)
}

fn read_answers(value: &Value, questions: &[NeedsYouQuestion]) -> Option<Answers> {
    let answers: Answers = serde_json::from_value(value.clone()).ok()?;
    (answers.len() == questions.len()
        && questions.iter().all(|q| {
            answers
                .get(&q.question)
                .is_some_and(|answer| !answer.trim().is_empty())
        }))
    .then_some(answers)
}

fn fallback() -> Value {
    json!({"ok":true})
}

impl Call {
    fn finish(self, monitor: &PermissionMonitor, reason: &str, answers: Option<Answers>) {
        let Some(ask) = self.ask else { return };
        let effective_answer = answers.map(|answers| json!(answers).to_string());
        if let Err(error) = monitor.finish_claude_question(&ask.id, reason, effective_answer) {
            log::error!("claude_question: could not settle id={}: {error}", ask.id);
        }
        if let Some(waiter) = ask.waiter {
            if ReplyRegistry::connected(&waiter) == Some(false) {
                ReplyRegistry::remove(&waiter);
            } else {
                write_json_response(&waiter, fallback());
            }
        }
        log::info!("claude_question: id={} settled reason={reason}", ask.id);
    }

    fn send_answer(&mut self, monitor: &PermissionMonitor) -> bool {
        let Some(ask) = &mut self.ask else {
            return true;
        };
        let Some(waiter) = &ask.waiter else {
            return true;
        };
        let Some(row) = monitor.host_records().question(&ask.id) else {
            return false;
        };
        if row.resolution.is_none() {
            return true;
        }
        let Some(answers) = row
            .answer
            .as_deref()
            .filter(|_| row.resolution == Some(NeedsYouResolution::Approved))
            .and_then(|answer| serde_json::from_str::<Value>(answer).ok())
            .and_then(|answer| read_answers(&answer, &self.questions))
        else {
            return false;
        };
        let mut updated_input = self.input.clone();
        updated_input["answers"] = json!(answers);
        let written = write_json_response(
            waiter,
            json!({
                "ok": true,
                "decision": {
                    "hookSpecificOutput": {
                        "hookEventName": "PermissionRequest",
                        "decision": {"behavior": "allow", "updatedInput": updated_input}
                    }
                }
            }),
        );
        ask.waiter = None;
        log::info!(
            "claude_question: id={} answer sent={written} awaiting PostToolUse",
            ask.id
        );
        written
    }
}

impl PlexiApp {
    pub(super) fn claude_question_hook(
        &mut self,
        monitor: &PermissionMonitor,
        payload: &Value,
        pane: Option<&PaneCaller>,
        response: &str,
    ) -> Option<Value> {
        let Some(pane) = pane.filter(|pane| !pane.authority.is_revoked()) else {
            return Some(json!({"ok":false,"error_code":"permission_denied"}));
        };
        let owner = pane.authority.principal();
        let Some(session) = payload["session_id"].as_str().filter(|id| !id.is_empty()) else {
            return Some(fallback());
        };
        let event = payload["hook_event_name"].as_str().unwrap_or_default();
        let calls = &mut self.claude_questions.calls;
        if matches!(event, "Stop" | "StopFailure" | "SessionEnd") {
            for (_, call) in
                calls.extract_if(|key, _| key.owner == owner.actor_id() && key.session == session)
            {
                call.finish(monitor, event, None);
            }
            return Some(fallback());
        }
        if payload["tool_name"] != TOOL {
            return Some(fallback());
        }
        let input = &payload["tool_input"];
        let expires = monitor.now_secs() + CARD_TTL_SECS;
        let principal = Principal::Pane(owner.clone());
        let records = monitor.records_for(&principal);
        if event == "PermissionRequest" {
            let mut matches = calls.iter_mut().filter(|(key, call)| {
                key.owner == owner.actor_id()
                    && key.session == session
                    && &call.input == input
                    && call.expires > monitor.now_secs()
            });
            let Some((key, call)) = matches.next() else {
                return Some(fallback());
            };
            if matches.next().is_some() || call.ask.is_some() {
                return Some(fallback());
            }
            let operation = key.operation();
            if records.run(&operation).is_some() {
                return Some(fallback());
            }
            let filed = super::plugin_call::file_needs_you_ask(
                monitor,
                &records,
                NeedsYouFile {
                    kind: NeedsYouKind::Question,
                    actor: records.filing_owner(),
                    resource: format!("pane:{}", pane.visible_pane),
                    summary: call.questions[0].question.clone(),
                    expires_at: Some(expires),
                    run_tag: Some(operation.clone()),
                },
                NeedsYouDetails {
                    claude_questions: call.questions.clone(),
                    correlation_id: Some(operation),
                    session_id: Some(session.to_owned()),
                    context_id: Some(pane.context_id),
                    pane_id: Some(pane.visible_pane),
                    ..Default::default()
                },
            );
            match filed {
                Ok(body) => {
                    let Some(id) = body["id"].as_str() else {
                        return Some(fallback());
                    };
                    call.expires = expires;
                    call.ask = Some(Ask {
                        id: id.to_owned(),
                        waiter: Some(response.to_owned()),
                    });
                    self.ctx.request_repaint();
                    log::info!(
                        "claude_question: id={id} waiting for pane={}",
                        pane.terminal_pane
                    );
                    return None;
                }
                Err(error) => {
                    log::error!("claude_question: could not file question: {error}");
                    return Some(fallback());
                }
            }
        }

        let Some(tool_use) = payload["tool_use_id"].as_str().filter(|id| !id.is_empty()) else {
            return Some(fallback());
        };
        let key = CallKey {
            owner: owner.actor_id().to_owned(),
            session: session.to_owned(),
            tool_use: tool_use.to_owned(),
        };
        match event {
            "PreToolUse" => {
                if let Some(questions) = read_questions(input) {
                    calls.entry(key).or_insert_with(|| Call {
                        authority: pane.authority.clone(),
                        input: input.clone(),
                        questions,
                        expires,
                        ask: None,
                    });
                }
            }
            "PostToolUse" | "PostToolUseFailure" => {
                if let Some(call) = calls.remove(&key) {
                    let answers = (event == "PostToolUse")
                        .then(|| {
                            read_answers(&payload["tool_response"]["answers"], &call.questions)
                        })
                        .flatten();
                    call.finish(monitor, event, answers);
                }
            }
            _ => {}
        }
        Some(fallback())
    }

    pub(super) fn service_claude_questions(
        &mut self,
        monitor: &PermissionMonitor,
        ctx: &egui::Context,
    ) {
        let now = monitor.now_secs();
        let mut calls = std::mem::take(&mut self.claude_questions.calls);
        let mut retired = Vec::new();
        for (key, call) in &mut calls {
            let owner = call.authority.principal();
            let disconnected = call
                .ask
                .as_ref()
                .and_then(|ask| ask.waiter.as_deref())
                .is_some_and(|token| match ReplyRegistry::connected(token) {
                    Some(connected) => !connected,
                    None => !cfg!(test),
                });
            let reason = if self.pane_principal(owner.number()).as_ref() != Some(&owner) {
                "pane_closed"
            } else if call.expires <= now {
                "hook_deadline"
            } else if disconnected {
                "hook_disconnected"
            } else if !call.send_answer(monitor) {
                "wait_ended"
            } else {
                continue;
            };
            retired.push((key.clone(), reason));
        }
        for (key, reason) in retired {
            if let Some(call) = calls.remove(&key) {
                call.finish(monitor, reason, None);
            }
        }
        if calls.values().any(|call| call.ask.is_some()) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.claude_questions.calls = calls;
    }
}
