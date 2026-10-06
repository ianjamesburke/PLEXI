//! One conversation and one model loop per agent head.
//!
//! A head is the durable id from the agents API. Assistant panes, `assistant
//! send --head`, and the command view are views onto that conversation.
//! Parent-to-child delegation still only narrows grants. A lead has no tool
//! that reads or writes another lead's conversation, and no lead-to-lead
//! message tool. Every model call and every tool call is admitted by the
//! permission monitor before it runs.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;

use serde_json::{json, Value};

use crate::broker::gate::{Admission, AdmitRequest, PermissionMonitor};
use crate::broker::{ActorScope, ActorType, TargetType};

const PACKAGE: &str = "agents";
const MAX_TOOL_ROUNDS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LeadMessage {
    pub role: String,
    pub text: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Clone)]
struct ModelReply {
    text: String,
    tool_calls: Vec<ToolCall>,
}

struct Job {
    workspace: PathBuf,
    head: String,
    text: String,
    request_id: String,
    response_file: String,
    task_id: Option<String>,
}

struct Flight {
    heads: HashSet<String>,
    waiters: VecDeque<Job>,
}

fn flight() -> &'static Mutex<Flight> {
    static FLIGHT: OnceLock<Mutex<Flight>> = OnceLock::new();
    FLIGHT.get_or_init(|| {
        Mutex::new(Flight {
            heads: HashSet::new(),
            waiters: VecDeque::new(),
        })
    })
}

fn opens() -> &'static Mutex<Vec<(String, u64)>> {
    static OPENS: OnceLock<Mutex<Vec<(String, u64)>>> = OnceLock::new();
    OPENS.get_or_init(|| Mutex::new(Vec::new()))
}

fn agents_dir(workspace: &Path) -> PathBuf {
    crate::agent::workspace_agents_dir(workspace)
}

fn conversation_path(workspace: &Path, head: &str) -> PathBuf {
    agents_dir(workspace).join(head).join("conversation.jsonl")
}

fn actor_of(head: &str) -> String {
    format!("agent:{head}")
}

fn now() -> String {
    crate::host::event_log::now_timestamp()
}

fn monitor() -> std::sync::Arc<PermissionMonitor> {
    PermissionMonitor::for_profile(&crate::config::config_dir())
}

/// Ask the host to open an Assistant pane bound to `head` in `context_id`.
pub fn request_open_pane(head: String, context_id: u64) {
    log::info!("lead: open pane requested head={head} context={context_id}");
    opens()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((head, context_id));
}

pub fn take_open_panes() -> Vec<(String, u64)> {
    std::mem::take(&mut *opens().lock().unwrap_or_else(|e| e.into_inner()))
}

pub fn head_busy(head: &str) -> bool {
    flight()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .heads
        .contains(head)
}

fn canonical_workspace(workspace: &Path) -> PathBuf {
    crate::platform::path::canonical_or_self(workspace)
}

pub fn load_conversation(workspace: &Path, head: &str) -> Vec<LeadMessage> {
    let workspace = canonical_workspace(workspace);
    let path = conversation_path(&workspace, head);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::error!("lead: read conversation {}: {error}", path.display());
            return Vec::new();
        }
    };
    let mut messages = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<LeadMessage>(line) {
            Ok(message) => messages.push(message),
            Err(error) => log::error!(
                "lead: skip bad conversation line {} in {}: {error}",
                index + 1,
                path.display()
            ),
        }
    }
    messages
}

fn append_message(workspace: &Path, head: &str, role: &str, text: &str) -> Result<(), String> {
    let path = conversation_path(workspace, head);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let message = LeadMessage {
        role: role.to_string(),
        text: text.to_string(),
        created_at: now(),
    };
    let line = serde_json::to_string(&message).map_err(|error| error.to_string())?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    use std::io::Write;
    writeln!(file, "{line}").map_err(|error| format!("write {}: {error}", path.display()))?;
    log::info!("lead: transcript head={head} role={role}");
    Ok(())
}

fn head_exists(workspace: &Path, head: &str) -> bool {
    agents_dir(workspace).join(head).join("head.json").is_file()
}

fn display_name(workspace: &Path, head: &str) -> String {
    let path = agents_dir(workspace).join(head).join("head.json");
    let Ok(raw) = fs::read_to_string(&path) else {
        return head.to_string();
    };
    serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|value| {
            value
                .get("display_name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| head.to_string())
}

/// Read `target`'s conversation as `actor_head`. A different lead is refused
/// and audited. The operator (no actor head) may read; that is the human view.
pub fn read_conversation_as(workspace: &Path, actor_head: Option<&str>, target: &str) -> Value {
    let workspace = canonical_workspace(workspace);
    if let Some(actor) = actor_head {
        if actor != target {
            let call_id = format!("call_{}", uuid::Uuid::new_v4());
            monitor().note_denial(
                &actor_of(actor),
                &call_id,
                "leads.conversation.read",
                "lead-isolation",
                "deny",
            );
            log::info!("lead: {actor} refused read of {target}");
            return json!({
                "ok": false,
                "error_code": "permission_denied",
                "error": "a lead cannot read another lead's conversation",
                "tool": "leads.conversation.read",
            });
        }
    }
    if !head_exists(&workspace, target) {
        return json!({"ok": false, "error_code": "head_not_found", "error": format!("no head '{target}'")});
    }
    json!({
        "ok": true,
        "head": target,
        "messages": load_conversation(&workspace, target),
    })
}

pub fn projection(workspace: &Path) -> Value {
    let workspace = canonical_workspace(workspace);
    let heads = crate::agent::heads::handle_request(
        "list_heads",
        &json!({"workspace": workspace, "all": true}),
    );
    let runs = crate::agent::heads::handle_request("list_runs", &json!({"workspace": &workspace}));
    let pending = serde_json::to_value(monitor().list_pending()).unwrap_or(json!([]));
    let mut rows = Vec::new();
    if let Some(list) = heads.get("heads").and_then(|v| v.as_array()) {
        for head in list {
            let id = head.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let messages = load_conversation(&workspace, id);
            let last = messages
                .last()
                .map(|message| json!({"role": message.role, "text": message.text}));
            rows.push(json!({
                "id": id,
                "display_name": head.get("display_name").cloned().unwrap_or(json!(id)),
                "last": last,
                "busy": head_busy(id),
            }));
        }
    }
    log::info!("lead: projection heads={}", rows.len());
    json!({
        "ok": true,
        "heads": rows,
        "runs": runs.get("runs").cloned().unwrap_or(json!([])),
        "needs_you": pending,
        "tasks": crate::agent::queue::snapshot(&workspace),
    })
}

pub fn pane_lines(workspace: &Path) -> Vec<String> {
    let workspace = canonical_workspace(workspace);
    let body = projection(&workspace);
    let mut lines = Vec::new();
    if let Some(heads) = body.get("heads").and_then(|v| v.as_array()) {
        for head in heads {
            let name = head
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("lead");
            let last = head
                .get("last")
                .and_then(|v| v.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let busy = head.get("busy").and_then(|v| v.as_bool()).unwrap_or(false);
            let state = if busy { "running" } else { "idle" };
            lines.push(format!("lead {name} {state}"));
            if !last.is_empty() {
                lines.push(format!("  {last}"));
            }
        }
    }
    if let Some(pending) = body.get("needs_you").and_then(|v| v.as_array()) {
        for item in pending {
            let id = item
                .get("pending_request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tool = item
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("approval");
            lines.push(format!("waiting {id} {tool}"));
        }
    }
    if let Some(tasks) = body.get("tasks").and_then(|value| value.as_array()) {
        for task in tasks {
            let id = task
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let head = task
                .get("head")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let state = task
                .get("state")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            lines.push(format!("queue {id} {head} {state}"));
        }
    }
    if let Some(runs) = body.get("runs").and_then(|value| value.as_array()) {
        for run in runs {
            if run.get("active").and_then(|value| value.as_bool()) != Some(true) {
                continue;
            }
            let id = run.get("id").and_then(|value| value.as_str()).unwrap_or("");
            let head = run
                .get("head_id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            lines.push(format!("run {id} {head} running"));
        }
    }
    if lines.is_empty() {
        lines.push("no leads".to_string());
    }
    lines
}

/// Admit `assistant.turn` before a missing head can be created. Ask and deny
/// leave no head directory. A grant still does not create the head.
fn refuse_missing_head(workspace: &Path, head: &str, text: &str) -> Value {
    let actor = actor_of(head);
    let turn_input = json!({"head": head, "text": text}).to_string();
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    match admit(workspace, &actor, "assistant.turn", &turn_input, &call_id) {
        Admission::Required { pending_request_id } => {
            log::info!("lead: send refused before create head={head} pending={pending_request_id}");
            crate::host::command_view::publish(workspace, "permission_required");
            json!({
                "ok": false,
                "error_code": "permission_required",
                "error": "permission_required",
                "pending_request_id": pending_request_id,
                "state": "permission_required",
                "head": head,
            })
        }
        Admission::Denied { code } => {
            log::info!("lead: send denied before create head={head} code={code}");
            json!({
                "ok": false,
                "error_code": code,
                "error": code,
                "state": code,
                "head": head,
            })
        }
        Admission::Proceed { .. } => {
            log::info!("lead: granted send found no head={head}");
            json!({
                "ok": false,
                "error_code": "head_not_found",
                "error": format!("no head '{head}'"),
                "state": "head_not_found",
                "head": head,
            })
        }
    }
}

/// Start a model turn for `head`. The worker writes `response_file` when the
/// turn reaches a terminal state. A second turn for the same head waits.
pub fn submit_turn(
    workspace: &Path,
    head: &str,
    text: &str,
    request_id: &str,
    response_file: &str,
    task_id: Option<&str>,
) -> Value {
    if text.trim().is_empty() {
        return json!({"ok": false, "error_code": "invalid_argument", "error": "text is required"});
    }
    let workspace = canonical_workspace(workspace);
    if !head_exists(&workspace, head) {
        return refuse_missing_head(&workspace, head, text);
    }
    log::info!(
        "lead: submit head={head} request_id={request_id} workspace={}",
        workspace.display()
    );
    let job = Job {
        workspace,
        head: head.to_string(),
        text: text.to_string(),
        request_id: request_id.to_string(),
        response_file: response_file.to_string(),
        task_id: task_id.map(str::to_string),
    };
    enqueue_job(job);
    json!({"ok": true, "accepted": true, "head": head, "request_id": request_id})
}

fn enqueue_job(job: Job) {
    let mut flight = flight().lock().unwrap_or_else(|e| e.into_inner());
    if !flight.heads.insert(job.head.clone()) {
        log::info!("lead: queued behind in-flight head={}", job.head);
        flight.waiters.push_back(job);
        return;
    }
    drop(flight);
    spawn_job(job);
}

fn spawn_job(job: Job) {
    let head = job.head.clone();
    let response_file = job.response_file.clone();
    let request_id = job.request_id.clone();
    let fail_head = head.clone();
    let fail_file = response_file.clone();
    let fail_request = request_id.clone();
    let fail_task = job.task_id.clone();
    let fail_workspace = job.workspace.clone();
    let built = thread::Builder::new().name(format!("plexi-lead-{head}"));
    if let Err(error) = built.spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_job(&job)));
        let body = match outcome {
            Ok(body) => body,
            Err(_) => {
                log::error!("lead: turn panicked head={head}");
                if let Some(task_id) = &job.task_id {
                    crate::agent::queue::finish_task(
                        &job.workspace,
                        task_id,
                        "failed",
                        "",
                        "lead turn panicked",
                    );
                }
                json!({
                    "request_id": request_id,
                    "state": "failed",
                    "error": "lead turn panicked",
                    "head": head,
                })
            }
        };
        crate::rpc::write_json_response(&response_file, body);
        release_head(&head);
    }) {
        log::error!("lead: failed to spawn turn thread: {error}");
        if let Some(task_id) = &fail_task {
            crate::agent::queue::finish_task(
                &fail_workspace,
                task_id,
                "failed",
                "",
                &error.to_string(),
            );
        }
        crate::rpc::write_json_response(
            &fail_file,
            json!({
                "request_id": fail_request,
                "state": "failed",
                "error": format!("failed to spawn lead turn: {error}"),
                "head": fail_head,
            }),
        );
        release_head(&fail_head);
    }
}

fn release_head(head: &str) {
    let mut flight = flight().lock().unwrap_or_else(|e| e.into_inner());
    flight.heads.remove(head);
    let next = flight
        .waiters
        .iter()
        .position(|job| job.head == head)
        .and_then(|index| flight.waiters.remove(index));
    if let Some(job) = next {
        flight.heads.insert(head.to_string());
        drop(flight);
        spawn_job(job);
    }
}

fn run_job(job: &Job) -> Value {
    let spawned = crate::agent::heads::handle_request(
        "spawn_run",
        &json!({
            "workspace": job.workspace,
            "head": job.head,
            "kind": "output",
            "client_ref": "assistant",
            "admission": job.request_id,
            "journal_only": true,
        }),
    );
    let run_id = spawned
        .get("run")
        .and_then(|run| run.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or("")
        .to_string();
    if spawned.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        log::error!("lead: spawn_run failed head={} body={spawned}", job.head);
        if let Some(task_id) = &job.task_id {
            let error = spawned
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("spawn_run failed");
            crate::agent::queue::finish_task(&job.workspace, task_id, "failed", "", error);
        }
        return json!({
            "request_id": job.request_id,
            "state": "failed",
            "error": spawned.get("error").cloned().unwrap_or(json!("spawn_run failed")),
            "error_code": spawned.get("error_code").cloned().unwrap_or(json!("spawn_failed")),
            "head": job.head,
        });
    }
    if let Some(task_id) = job.task_id.as_deref() {
        if !run_id.is_empty() {
            crate::agent::queue::attach_run(&job.workspace, task_id, &run_id);
        }
    }
    let done = run_model_turn(
        &job.workspace,
        &job.head,
        &job.text,
        job.task_id.as_deref(),
        Some(run_id.as_str()).filter(|id| !id.is_empty()),
        &mut http_complete,
    );
    if let Some(task_id) = &job.task_id {
        let error = done.error.as_deref().unwrap_or("");
        crate::agent::queue::finish_task(&job.workspace, task_id, done.state, &run_id, error);
    }
    if !run_id.is_empty() {
        let _ = crate::agent::heads::handle_request(
            "finish_run",
            &json!({"workspace": job.workspace, "id": run_id}),
        );
    }
    log::info!(
        "lead: turn finished head={} run={run_id} state={}",
        job.head,
        done.state
    );
    crate::host::command_view::publish(&job.workspace, "turn finished");
    json!({
        "request_id": job.request_id,
        "state": done.state,
        "reply": done.reply,
        "error": done.error,
        "head": job.head,
        "run_id": run_id,
    })
}

struct TurnDone {
    state: &'static str,
    reply: String,
    error: Option<String>,
}

/// Run one prompt through the permission gate and the model. The caller owns
/// the run record; this does not journal or finish a run.
pub fn run_prompt(workspace: &Path, head: &str, text: &str) -> Value {
    let done = run_model_turn(workspace, head, text, None, None, &mut http_complete);
    log::info!("lead: prompt finished head={head} state={}", done.state);
    json!({"state": done.state, "reply": done.reply, "error": done.error})
}

fn run_model_turn(
    workspace: &Path,
    head: &str,
    text: &str,
    task_id: Option<&str>,
    run_id: Option<&str>,
    model: &mut dyn FnMut(&[Value]) -> Result<ModelReply, String>,
) -> TurnDone {
    let workspace_buf = canonical_workspace(workspace);
    let workspace = workspace_buf.as_path();
    let actor = actor_of(head);
    let turn_input = json!({"head": head, "text": text}).to_string();
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    match admit(workspace, &actor, "assistant.turn", &turn_input, &call_id) {
        Admission::Proceed {
            grant_id,
            fingerprint,
            resource_id,
            ..
        } => {
            if let Err(error) = monitor().note_use(
                &actor,
                &call_id,
                &grant_id,
                &fingerprint,
                resource_id.as_deref().unwrap_or(""),
                "assistant.turn",
            ) {
                log::error!("lead: audit use failed head={head}: {error}");
                return fail("permission_denied", error);
            }
        }
        Admission::Required { pending_request_id } => {
            let message = format!("permission_required {pending_request_id}");
            let _ = append_message(workspace, head, "error", &message);
            log::info!(
                "lead: model call needs permission head={head} pending={pending_request_id}"
            );
            return TurnDone {
                state: "permission_required",
                reply: String::new(),
                error: Some(message),
            };
        }
        Admission::Denied { code } => {
            let _ = append_message(workspace, head, "error", code);
            log::info!("lead: model call denied head={head} code={code}");
            return fail(code, code.to_string());
        }
    }

    if let Err(error) = append_message(workspace, head, "user", text) {
        return fail("failed", error);
    }
    let label = display_name(workspace, head);
    let mut messages = vec![json!({
        "role": "system",
        "content": format!(
            "You are lead {label}. You have your own conversation. You cannot read or write another lead's conversation. You cannot message another lead. Delegation only narrows authority."
        ),
    })];
    for message in load_conversation(workspace, head) {
        let role = match message.role.as_str() {
            "user" => "user",
            "assistant" => "assistant",
            "tool" => "tool",
            _ => continue,
        };
        messages.push(json!({"role": role, "content": message.text}));
    }

    for round in 0..MAX_TOOL_ROUNDS {
        if stop_if_cancelled(workspace, head, task_id, run_id) {
            return TurnDone {
                state: "cancelled",
                reply: String::new(),
                error: Some("cancelled".to_string()),
            };
        }
        let reply = match model(&messages) {
            Ok(reply) => reply,
            Err(error) => {
                let _ = append_message(workspace, head, "error", &error);
                log::error!("lead: model failed head={head}: {error}");
                return fail("failed", error);
            }
        };
        if reply.tool_calls.is_empty() {
            let text = if reply.text.is_empty() {
                "(empty reply)".to_string()
            } else {
                reply.text
            };
            let _ = append_message(workspace, head, "assistant", &text);
            return TurnDone {
                state: "succeeded",
                reply: text,
                error: None,
            };
        }
        if !reply.text.is_empty() {
            let _ = append_message(workspace, head, "assistant", &reply.text);
            messages.push(json!({"role": "assistant", "content": reply.text}));
        }
        for call in reply.tool_calls {
            if stop_if_cancelled(workspace, head, task_id, run_id) {
                return TurnDone {
                    state: "cancelled",
                    reply: String::new(),
                    error: Some("cancelled".to_string()),
                };
            }
            let result = execute_tool(workspace, head, &actor, &call);
            let _ = append_message(
                workspace,
                head,
                "tool",
                &format!("{}: {}", call.name, result.text),
            );
            messages.push(json!({
                "role": "tool",
                "name": call.name,
                "content": result.text,
            }));
            if result.stop {
                return TurnDone {
                    state: result.state,
                    reply: result.text.clone(),
                    error: if result.state == "succeeded" {
                        None
                    } else {
                        Some(result.text)
                    },
                };
            }
        }
        log::info!("lead: tool round {} finished head={head}", round + 1);
    }
    fail("failed", "too many tool rounds".to_string())
}

struct ToolResult {
    text: String,
    state: &'static str,
    stop: bool,
}

fn execute_tool(workspace: &Path, head: &str, actor: &str, call: &ToolCall) -> ToolResult {
    if call.name == "leads.message" || call.name.contains("ask_question") {
        let call_id = format!("call_{}", uuid::Uuid::new_v4());
        monitor().note_denial(actor, &call_id, &call.name, "lead-isolation", "deny");
        log::info!("lead: refused lead-to-lead tool {} head={head}", call.name);
        return ToolResult {
            text: "permission_denied: leads cannot message each other".to_string(),
            state: "permission_denied",
            stop: true,
        };
    }
    let call_id = if call.id.is_empty() {
        format!("call_{}", uuid::Uuid::new_v4())
    } else {
        call.id.clone()
    };
    let input = if call.arguments.trim().is_empty() {
        "{}".to_string()
    } else {
        call.arguments.clone()
    };
    match admit(workspace, actor, &call.name, &input, &call_id) {
        Admission::Required { pending_request_id } => {
            log::info!(
                "lead: tool {} needs permission head={head} pending={pending_request_id}",
                call.name
            );
            ToolResult {
                text: format!("permission_required {pending_request_id}"),
                state: "permission_required",
                stop: true,
            }
        }
        Admission::Denied { code } => {
            log::info!("lead: tool {} denied head={head} code={code}", call.name);
            ToolResult {
                text: code.to_string(),
                state: "permission_denied",
                stop: false,
            }
        }
        Admission::Proceed {
            grant_id,
            fingerprint,
            resource_id,
            ..
        } => {
            if let Err(error) = monitor().note_use(
                actor,
                &call_id,
                &grant_id,
                &fingerprint,
                resource_id.as_deref().unwrap_or(""),
                &call.name,
            ) {
                log::error!(
                    "lead: audit use failed tool={} head={head}: {error}",
                    call.name
                );
                return ToolResult {
                    text: error,
                    state: "permission_denied",
                    stop: true,
                };
            }
            match run_admitted_tool(workspace, head, actor, call, &input) {
                Ok(text) => ToolResult {
                    text,
                    state: "succeeded",
                    stop: false,
                },
                Err(error) => ToolResult {
                    text: error,
                    state: "permission_denied",
                    stop: false,
                },
            }
        }
    }
}

fn run_admitted_tool(
    workspace: &Path,
    head: &str,
    actor: &str,
    call: &ToolCall,
    input: &str,
) -> Result<String, String> {
    let args: Value = serde_json::from_str(input).unwrap_or_else(|_| json!({}));
    match call.name.as_str() {
        "leads.conversation.read" => {
            let target = args.get("head").and_then(|v| v.as_str()).unwrap_or("");
            if target != head {
                let call_id = format!("call_{}", uuid::Uuid::new_v4());
                monitor().note_denial(
                    actor,
                    &call_id,
                    "leads.conversation.read",
                    "lead-isolation",
                    "deny",
                );
                log::info!("lead: {head} blocked from reading {target}");
                return Err("a lead cannot read another lead's conversation".to_string());
            }
            let messages = load_conversation(workspace, head);
            Ok(serde_json::to_string(&messages).unwrap_or_else(|_| "[]".to_string()))
        }
        "host.files.write" => {
            let file = args
                .get("file")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "host.files.write requires file".to_string())?;
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            write_workspace_file(workspace, file, content)?;
            log::info!("lead: wrote file {file} head={head}");
            Ok(format!("wrote {file}"))
        }
        "lead.step" => {
            let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
            thread::sleep(std::time::Duration::from_millis(200));
            log::info!("lead: step {n} head={head}");
            Ok(format!("step {n}"))
        }
        other => Err(format!("unknown lead tool {other}")),
    }
}

fn write_workspace_file(workspace: &Path, file: &str, content: &str) -> Result<(), String> {
    if file.is_empty() || file.starts_with('/') || file.starts_with('\\') || file.contains("..") {
        return Err("file must stay inside the workspace".to_string());
    }
    let path = workspace.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::write(&path, content).map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok(())
}

fn admit(workspace: &Path, actor: &str, tool: &str, input_json: &str, call_id: &str) -> Admission {
    monitor().admit(AdmitRequest {
        call_id,
        tool,
        input_json,
        actor_type: ActorType::Agent,
        actor_id: actor,
        actor_scope: ActorScope::Workspace,
        trust_origin: "host",
        workspace_root: workspace,
        context_id: 0,
        package_id: PACKAGE,
        instance_id: 0,
        target_type: TargetType::HostTool,
    })
}

fn stop_if_cancelled(
    workspace: &Path,
    head: &str,
    task_id: Option<&str>,
    run_id: Option<&str>,
) -> bool {
    let task_hit = task_id.is_some_and(|id| crate::agent::queue::cancel_requested(workspace, id));
    let run_hit = run_id.is_some_and(|id| crate::agent::queue::run_cancel_requested(workspace, id));
    if !task_hit && !run_hit {
        return false;
    }
    log::info!(
        "lead: cancelled before the next tool head={head} task={} run={}",
        task_id.unwrap_or(""),
        run_id.unwrap_or("")
    );
    true
}

fn fail(state: &'static str, error: String) -> TurnDone {
    TurnDone {
        state,
        reply: String::new(),
        error: Some(error),
    }
}

fn endpoint() -> String {
    match std::env::var("PLEXI_OPENROUTER_BASE_URL") {
        Ok(base) => {
            let base = base.trim().trim_end_matches('/');
            if base.ends_with("/chat/completions") {
                base.to_string()
            } else if base.ends_with("/v1") {
                format!("{base}/chat/completions")
            } else {
                format!("{base}/v1/chat/completions")
            }
        }
        Err(_) => "https://openrouter.ai/api/v1/chat/completions".to_string(),
    }
}

fn api_key() -> Result<String, String> {
    if let Ok(key) = std::env::var("OPENROUTER_API_KEY") {
        if !key.is_empty() {
            return Ok(key);
        }
    }
    if std::env::var("PLEXI_OPENROUTER_BASE_URL").is_ok() {
        return Ok("test-key".to_string());
    }
    Err(
        "OPENROUTER_API_KEY is not set. Set PLEXI_OPENROUTER_BASE_URL to use the mock model."
            .to_string(),
    )
}

fn http_complete(messages: &[Value]) -> Result<ModelReply, String> {
    let key = api_key()?;
    let url = endpoint();
    let model = std::env::var("PLEXI_LEAD_MODEL").unwrap_or_else(|_| "mock/lead".to_string());
    let body = json!({
        "model": model,
        "messages": messages,
        "tools": lead_tools(),
    });
    log::info!(
        "lead: model request endpoint={url} messages={}",
        messages.len()
    );
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(90))
        .build();
    let response = agent
        .post(&url)
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|error| format!("model request failed: {error}"))?;
    let parsed: Value = response
        .into_json()
        .map_err(|error| format!("model response was not JSON: {error}"))?;
    parse_model_reply(&parsed)
}

fn lead_tools() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "host.files.write",
                "description": "Write a file inside this lead's workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "file": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["file", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "leads.conversation.read",
                "description": "Read this lead's own conversation. Other heads are refused.",
                "parameters": {
                    "type": "object",
                    "properties": {"head": {"type": "string"}},
                    "required": ["head"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "lead.step",
                "description": "One step of a long task.",
                "parameters": {
                    "type": "object",
                    "properties": {"n": {"type": "integer"}},
                    "required": ["n"]
                }
            }
        }
    ])
}

fn parse_model_reply(body: &Value) -> Result<ModelReply, String> {
    let message = body
        .pointer("/choices/0/message")
        .ok_or_else(|| format!("model response has no choices: {body}"))?;
    let text = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            let function = call.get("function").cloned().unwrap_or(json!({}));
            tool_calls.push(ToolCall {
                id: call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: function
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                arguments: function
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}")
                    .to_string(),
            });
        }
    }
    Ok(ModelReply { text, tool_calls })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _profile: tempfile::TempDir,
        _guard: crate::config::TestProfileDirGuard,
        workspace: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let profile = tempfile::tempdir().unwrap();
            let guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
            let workspace = tempfile::tempdir().unwrap();
            fs::create_dir_all(workspace.path().join(".plexi")).unwrap();
            Self {
                _profile: profile,
                _guard: guard,
                workspace,
            }
        }

        fn ws(&self) -> &Path {
            self.workspace.path()
        }

        fn profile(&self) -> &Path {
            self._profile.path()
        }

        /// Audit rows are sealed envelopes. Callers assert on the fact, which
        /// is the JSON string inside `fact`.
        fn audit_text(&self) -> String {
            let raw = fs::read_to_string(self.profile().join("permission-audit.jsonl"))
                .unwrap_or_default();
            let mut facts = String::new();
            for line in raw.lines() {
                if let Ok(row) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(fact) = row.get("fact").and_then(|value| value.as_str()) {
                        facts.push_str(fact);
                        facts.push('\n');
                        continue;
                    }
                }
                facts.push_str(line);
                facts.push('\n');
            }
            facts
        }

        fn create(&self, name: &str, grants: &[&str]) {
            let body = crate::agent::heads::handle_request(
                "create_head",
                &json!({
                    "workspace": self.ws(),
                    "name": name,
                    "display_name": name,
                    "grants": grants,
                }),
            );
            assert_eq!(
                body.get("ok").and_then(|v| v.as_bool()),
                Some(true),
                "{body}"
            );
        }
    }

    fn text_reply(text: &str) -> ModelReply {
        ModelReply {
            text: text.to_string(),
            tool_calls: Vec::new(),
        }
    }

    fn scripted(messages: &[Value]) -> Result<ModelReply, String> {
        let blob = serde_json::to_string(messages).unwrap_or_default();
        if blob.contains("what number?") {
            if blob.contains("remember 7") {
                Ok(text_reply("7"))
            } else {
                Ok(text_reply("I do not know a number"))
            }
        } else if blob.contains("remember 7") {
            Ok(text_reply("Noted 7"))
        } else {
            Ok(text_reply("ok"))
        }
    }

    #[test]
    fn two_heads_keep_separate_transcripts() {
        let fixture = Fixture::new();
        fixture.create("lead-a", &["assistant.turn=allow"]);
        fixture.create("lead-b", &["assistant.turn=allow"]);
        let a = run_model_turn(
            fixture.ws(),
            "lead-a",
            "remember 7",
            None,
            None,
            &mut scripted,
        );
        let b = run_model_turn(
            fixture.ws(),
            "lead-b",
            "what number?",
            None,
            None,
            &mut scripted,
        );
        assert_eq!(a.state, "succeeded");
        assert_eq!(a.reply, "Noted 7");
        assert_eq!(b.state, "succeeded");
        assert_eq!(b.reply, "I do not know a number");
        let a_text = fs::read_to_string(conversation_path(fixture.ws(), "lead-a")).unwrap();
        let b_text = fs::read_to_string(conversation_path(fixture.ws(), "lead-b")).unwrap();
        assert!(a_text.contains("remember 7"));
        assert!(a_text.contains("Noted 7"));
        assert!(!b_text.contains("remember 7"));
        assert!(!b_text.contains("Noted 7"));
        assert!(b_text.contains("I do not know a number"));
        let view = projection(fixture.ws());
        let heads = view.get("heads").and_then(|v| v.as_array()).unwrap();
        assert!(heads
            .iter()
            .any(|head| head.get("id").and_then(|v| v.as_str()) == Some("lead-a")));
        assert!(heads
            .iter()
            .any(|head| head.get("id").and_then(|v| v.as_str()) == Some("lead-b")));
    }

    #[test]
    fn a_lead_cannot_read_another_leads_conversation() {
        let fixture = Fixture::new();
        fixture.create(
            "lead-a",
            &["assistant.turn=allow", "leads.conversation.read=allow"],
        );
        fixture.create("lead-b", &["assistant.turn=allow"]);
        run_model_turn(
            fixture.ws(),
            "lead-b",
            "remember 7",
            None,
            None,
            &mut scripted,
        );
        let mut model = |messages: &[Value]| {
            let blob = serde_json::to_string(messages).unwrap_or_default();
            if blob.contains("read-other") && !blob.contains("a lead cannot read another lead") {
                Ok(ModelReply {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_read".to_string(),
                        name: "leads.conversation.read".to_string(),
                        arguments: r#"{"head":"lead-b"}"#.to_string(),
                    }],
                })
            } else {
                Ok(text_reply("I could not read it"))
            }
        };
        let done = run_model_turn(
            fixture.ws(),
            "lead-a",
            "read-other lead-b",
            None,
            None,
            &mut model,
        );
        assert_eq!(done.state, "succeeded", "{:?}", done.error);
        let a_text = fs::read_to_string(conversation_path(fixture.ws(), "lead-a")).unwrap();
        assert!(a_text.contains("cannot read another lead"));
        assert!(!a_text.contains("Noted 7"));
        let direct = read_conversation_as(fixture.ws(), Some("lead-a"), "lead-b");
        assert_eq!(
            direct.get("error_code").and_then(|v| v.as_str()),
            Some("permission_denied")
        );
        let audit = fixture.audit_text();
        assert!(audit.contains("leads.conversation.read"), "{audit}");
        assert!(
            audit.contains("deny") || audit.contains("\"decision\":\"use\""),
            "{audit}"
        );
    }

    #[test]
    fn ungranted_tool_does_not_run() {
        let fixture = Fixture::new();
        fixture.create("lead-a", &["assistant.turn=allow"]);
        let mut model = |_messages: &[Value]| {
            Ok(ModelReply {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call_write".to_string(),
                    name: "host.files.write".to_string(),
                    arguments: r#"{"file":"secret.txt","content":"nope"}"#.to_string(),
                }],
            })
        };
        let done = run_model_turn(fixture.ws(), "lead-a", "write it", None, None, &mut model);
        assert_eq!(done.state, "permission_required");
        assert!(!fixture.ws().join("secret.txt").exists());
    }

    #[test]
    fn granted_write_is_audited_and_a_message_tool_is_refused() {
        let fixture = Fixture::new();
        fixture.create(
            "lead-a",
            &["assistant.turn=allow", "host.files.write=allow"],
        );
        let mut calls = 0;
        let mut model = move |_messages: &[Value]| {
            calls += 1;
            if calls == 1 {
                Ok(ModelReply {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_write".to_string(),
                        name: "host.files.write".to_string(),
                        arguments: r#"{"file":"out.txt","content":"ok"}"#.to_string(),
                    }],
                })
            } else if calls == 2 {
                Ok(ModelReply {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_msg".to_string(),
                        name: "leads.message".to_string(),
                        arguments: r#"{"to":"lead-b","text":"hi"}"#.to_string(),
                    }],
                })
            } else {
                Ok(text_reply("done"))
            }
        };
        let done = run_model_turn(
            fixture.ws(),
            "lead-a",
            "write then message",
            None,
            None,
            &mut model,
        );
        assert_eq!(done.state, "permission_denied");
        assert_eq!(
            fs::read_to_string(fixture.ws().join("out.txt")).unwrap(),
            "ok"
        );
        let audit = fixture.audit_text();
        assert!(audit.contains("\"decision\":\"use\""), "{audit}");
        assert!(audit.contains("leads.message"), "{audit}");
    }

    #[test]
    fn cancel_stops_before_the_next_tool_is_admitted() {
        let fixture = Fixture::new();
        fixture.create("lead-a", &["assistant.turn=allow", "lead.step=allow"]);
        let queued = crate::agent::queue::enqueue(fixture.ws(), "lead-a", "long-task");
        let task_id = queued["task"]["id"].as_str().unwrap().to_string();
        crate::agent::queue::finish_task(fixture.ws(), &task_id, "running", "", "");
        let mut calls = 0;
        let ws = fixture.ws().to_path_buf();
        let flag_id = task_id.clone();
        let mut model = move |_messages: &[Value]| {
            calls += 1;
            if calls == 1 {
                Ok(ModelReply {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_step_1".to_string(),
                        name: "lead.step".to_string(),
                        arguments: r#"{"n":0}"#.to_string(),
                    }],
                })
            } else {
                let _ = crate::agent::queue::request_cancel(&ws, &flag_id);
                Ok(ModelReply {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_step_2".to_string(),
                        name: "lead.step".to_string(),
                        arguments: r#"{"n":1}"#.to_string(),
                    }],
                })
            }
        };
        let done = run_model_turn(
            fixture.ws(),
            "lead-a",
            "long-task",
            Some(&task_id),
            None,
            &mut model,
        );
        assert_eq!(done.state, "cancelled", "{:?}", done.error);
        let audit = fixture.audit_text();
        let uses = audit.matches("\"operation_id\":\"lead.step\"").count();
        assert_eq!(uses, 1, "{audit}");
    }

    #[test]
    fn cancel_run_stops_before_the_next_tool_is_admitted() {
        let fixture = Fixture::new();
        fixture.create("lead-a", &["assistant.turn=allow", "lead.step=allow"]);
        let spawned = crate::agent::heads::handle_request(
            "spawn_run",
            &json!({
                "workspace": fixture.ws(),
                "head": "lead-a",
                "kind": "output",
                "admission": "adm-cancel-run",
            }),
        );
        assert_eq!(
            spawned.get("ok").and_then(|value| value.as_bool()),
            Some(true),
            "{spawned}"
        );
        let run_id = spawned["run"]["id"].as_str().unwrap().to_string();
        let mut calls = 0;
        let ws = fixture.ws().to_path_buf();
        let flag = run_id.clone();
        let mut model = move |_messages: &[Value]| {
            calls += 1;
            if calls == 1 {
                Ok(ModelReply {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_step_1".to_string(),
                        name: "lead.step".to_string(),
                        arguments: r#"{"n":0}"#.to_string(),
                    }],
                })
            } else {
                let cancelled = crate::agent::queue::request_cancel_run(&ws, &flag);
                assert_eq!(
                    cancelled.get("ok").and_then(|value| value.as_bool()),
                    Some(true),
                    "{cancelled}"
                );
                Ok(ModelReply {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_step_2".to_string(),
                        name: "lead.step".to_string(),
                        arguments: r#"{"n":1}"#.to_string(),
                    }],
                })
            }
        };
        let done = run_model_turn(
            fixture.ws(),
            "lead-a",
            "long-task",
            None,
            Some(&run_id),
            &mut model,
        );
        assert_eq!(done.state, "cancelled", "{:?}", done.error);
        let audit = fs::read_to_string(fixture.profile().join("permission-audit.jsonl"))
            .unwrap_or_default();
        assert_eq!(
            audit.matches("\"operation_id\":\"lead.step\"").count(),
            1,
            "{audit}"
        );
    }

    #[test]
    fn ungranted_send_to_a_missing_head_creates_nothing() {
        let fixture = Fixture::new();
        let body = submit_turn(
            fixture.ws(),
            "ghost",
            "hello",
            "req-ghost",
            "/tmp/unused",
            None,
        );
        assert_eq!(body["error_code"], "permission_required", "{body}");
        assert!(body["pending_request_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()));
        assert!(!fixture
            .ws()
            .join(".plexi")
            .join("agents")
            .join("ghost")
            .exists());
        let lines = pane_lines(fixture.ws());
        assert!(
            lines.iter().any(|line| line.starts_with("waiting ")),
            "{lines:?}"
        );
        crate::agent::queue::enqueue(fixture.ws(), "ghost", "later");
        assert!(!fixture
            .ws()
            .join(".plexi")
            .join("agents")
            .join("ghost")
            .exists());
    }

    #[test]
    fn pane_lines_show_last_output_and_the_queue() {
        let fixture = Fixture::new();
        fixture.create("lead-a", &["assistant.turn=allow"]);
        let done = run_model_turn(fixture.ws(), "lead-a", "hello", None, None, &mut scripted);
        assert_eq!(done.state, "succeeded");
        let queued = crate::agent::queue::enqueue(fixture.ws(), "lead-a", "later");
        assert_eq!(queued["ok"], true, "{queued}");
        let lines = pane_lines(fixture.ws());
        assert!(
            lines.iter().any(|line| line.starts_with("lead ")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|line| line.contains("ok")), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("queue ") && line.ends_with("queued")),
            "{lines:?}"
        );
    }
}
