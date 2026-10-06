//! In-memory command view.
//!
//! Leads, runs, and the pending queue live in this process. Needs-you rows
//! stay on [`crate::broker::gate::PermissionMonitor`]. The JSON from
//! [`projection`] is the seam an agents-API run store can fill later; this
//! module does not write a profile file.
//!
//! Steer tools (`command.send`, `command.enqueue`, `command.pause`,
//! `command.cancel`) are host tools. Admission uses actor `agent:command-view`,
//! instance 0, and context 0. The resource is the lead or run id.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};

use crate::broker::gate::{self, NeedsYouFile, NeedsYouKind, NeedsYouRecord};
use crate::broker::{
    ActorScope, ActorType, Decision, ExactBinding, GrantDuration, GrantRecord, GrantSource,
    ResourceScope, TargetType,
};

pub const STEER_ACTOR: &str = "agent:command-view";
pub const PUBLISHER: &str = "plexi.host.command";
pub const STREAM: &str = "command.view";

const STEER_TOOLS: &[&str] = &[
    "command.send",
    "command.enqueue",
    "command.pause",
    "command.cancel",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Running,
    WaitingOnYou,
    Idle,
    Done,
}

fn status_str(status: Status) -> &'static str {
    match status {
        Status::Running => "running",
        Status::WaitingOnYou => "waiting on you",
        Status::Idle => "idle",
        Status::Done => "done",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Task {
    id: String,
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Run {
    id: String,
    status: Status,
    last_output: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Lead {
    id: String,
    last_output: String,
    queue: Vec<Task>,
    runs: Vec<Run>,
}

/// Process-local board. Tests build their own; the host uses [`shared`].
#[derive(Clone, Debug, Default)]
pub struct CommandBoard {
    leads: BTreeMap<String, Lead>,
    revision: u64,
    next_run: u64,
    next_task: u64,
    summary: String,
}

impl CommandBoard {
    fn bump(&mut self, summary: impl Into<String>) {
        self.revision += 1;
        self.summary = summary.into();
    }

    fn ensure(&mut self, id: &str) -> &mut Lead {
        self.leads.entry(id.to_string()).or_insert_with(|| Lead {
            id: id.to_string(),
            last_output: String::new(),
            queue: Vec::new(),
            runs: Vec::new(),
        })
    }

    fn fresh_run_id(&mut self) -> String {
        self.next_run += 1;
        format!("run-{}", self.next_run)
    }

    fn fresh_task_id(&mut self) -> String {
        self.next_task += 1;
        format!("task-{}", self.next_task)
    }

    /// Record a message as the lead's last output and a running run.
    pub fn send(&mut self, lead: &str, text: &str) -> Result<String, String> {
        if lead.is_empty() || text.is_empty() {
            return Err("lead and text are required".into());
        }
        let existing = self.leads.get(lead).and_then(|row| {
            row.runs
                .iter()
                .find(|run| run.status == Status::Running || run.status == Status::Idle)
                .map(|run| run.id.clone())
        });
        let run_id = if let Some(id) = existing {
            let lead_row = self.ensure(lead);
            lead_row.last_output = text.to_string();
            if let Some(run) = lead_row.runs.iter_mut().find(|run| run.id == id) {
                run.status = Status::Running;
                run.last_output = text.to_string();
            }
            id
        } else {
            let id = self.fresh_run_id();
            let lead_row = self.ensure(lead);
            lead_row.last_output = text.to_string();
            lead_row.runs.push(Run {
                id: id.clone(),
                status: Status::Running,
                last_output: text.to_string(),
            });
            id
        };
        self.bump(format!("lead {lead} running"));
        Ok(run_id)
    }

    pub fn enqueue(&mut self, lead: &str, text: &str) -> Result<String, String> {
        if lead.is_empty() || text.is_empty() {
            return Err("lead and text are required".into());
        }
        let id = self.fresh_task_id();
        let lead_row = self.ensure(lead);
        lead_row.queue.push(Task {
            id: id.clone(),
            text: text.to_string(),
        });
        self.bump(format!("lead {lead} queued"));
        Ok(id)
    }

    pub fn pause(&mut self, run_id: &str) -> Result<(), String> {
        self.set_run(run_id, Status::Idle)?;
        self.bump(format!("run {run_id} idle"));
        Ok(())
    }

    pub fn cancel(&mut self, run_id: &str) -> Result<(), String> {
        self.set_run(run_id, Status::Done)?;
        self.bump(format!("run {run_id} done"));
        Ok(())
    }

    /// Mark a run waiting and return the needs-you row the host should file.
    pub fn block(
        &mut self,
        lead: &str,
        run_id: &str,
        summary: &str,
    ) -> Result<NeedsYouFile, String> {
        if lead.is_empty() || run_id.is_empty() || summary.is_empty() {
            return Err("lead, run, and summary are required".into());
        }
        let lead_row = self.ensure(lead);
        if let Some(run) = lead_row.runs.iter_mut().find(|run| run.id == run_id) {
            if run.status == Status::Done {
                return Err(format!("run {run_id} is done"));
            }
            run.status = Status::WaitingOnYou;
        } else {
            lead_row.runs.push(Run {
                id: run_id.to_string(),
                status: Status::WaitingOnYou,
                last_output: lead_row.last_output.clone(),
            });
        }
        self.bump(format!("run {run_id} waiting on you"));
        Ok(NeedsYouFile {
            kind: NeedsYouKind::BlockedRun,
            actor: lead.to_string(),
            resource: run_id.to_string(),
            summary: summary.to_string(),
            expires_at: None,
            run_tag: Some(run_id.to_string()),
        })
    }

    /// Approve resumes a waiting run. Deny finishes it.
    pub fn apply_resolution(&mut self, run_tag: Option<&str>, approve: bool) -> bool {
        let Some(run_id) = run_tag.filter(|tag| !tag.is_empty()) else {
            return false;
        };
        let next = if approve {
            Status::Running
        } else {
            Status::Done
        };
        let Some(lead_id) = self.leads.iter().find_map(|(id, lead)| {
            lead.runs
                .iter()
                .any(|run| run.id == run_id && run.status == Status::WaitingOnYou)
                .then(|| id.clone())
        }) else {
            return false;
        };
        if let Some(run) = self
            .leads
            .get_mut(&lead_id)
            .and_then(|lead| lead.runs.iter_mut().find(|run| run.id == run_id))
        {
            run.status = next;
            self.bump(format!("run {run_id} {}", status_str(next)));
            true
        } else {
            false
        }
    }

    fn set_run(&mut self, run_id: &str, status: Status) -> Result<(), String> {
        let Some(lead_id) = self.leads.iter().find_map(|(id, lead)| {
            lead.runs
                .iter()
                .any(|run| run.id == run_id)
                .then(|| id.clone())
        }) else {
            return Err(format!("unknown run {run_id}"));
        };
        let Some(run) = self
            .leads
            .get_mut(&lead_id)
            .and_then(|lead| lead.runs.iter_mut().find(|run| run.id == run_id))
        else {
            return Err(format!("unknown run {run_id}"));
        };
        if run.status == Status::Done {
            return Err(format!("run {run_id} is done"));
        }
        run.status = status;
        Ok(())
    }

    pub fn project(&self, needs: &[NeedsYouRecord]) -> Value {
        let leads: Vec<Value> = self
            .leads
            .values()
            .map(|lead| {
                let status = lead_status(lead);
                let needs_you: Vec<Value> = needs
                    .iter()
                    .filter(|item| item.resolution.is_none() && item_for_lead(item, lead))
                    .map(|item| {
                        json!({
                            "id": item.id,
                            "kind": item.kind.as_str(),
                            "summary": item.summary,
                            "run_tag": item.run_tag,
                        })
                    })
                    .collect();
                json!({
                    "id": lead.id,
                    "name": lead.id,
                    "status": status_str(status),
                    "last_output": lead.last_output,
                    "queue": lead.queue.iter().map(|task| json!({
                        "id": task.id,
                        "text": task.text,
                        "status": "pending",
                    })).collect::<Vec<_>>(),
                    "runs": lead.runs.iter().map(|run| json!({
                        "id": run.id,
                        "status": status_str(run.status),
                        "last_output": run.last_output,
                    })).collect::<Vec<_>>(),
                    "needs_you": needs_you,
                })
            })
            .collect();
        json!({
            "ok": true,
            "revision": self.revision,
            "leads": leads,
        })
    }

    pub fn lines(&self, needs: &[NeedsYouRecord]) -> String {
        let value = self.project(needs);
        let mut lines = Vec::new();
        for lead in value["leads"].as_array().into_iter().flatten() {
            lines.push(format!(
                "{} {} | {}",
                lead["id"].as_str().unwrap_or(""),
                lead["status"].as_str().unwrap_or(""),
                lead["last_output"].as_str().unwrap_or("")
            ));
            for task in lead["queue"].as_array().into_iter().flatten() {
                lines.push(format!(
                    "  queue {} {}",
                    task["id"].as_str().unwrap_or(""),
                    task["text"].as_str().unwrap_or("")
                ));
            }
            for item in lead["needs_you"].as_array().into_iter().flatten() {
                lines.push(format!(
                    "  needs you {} {}",
                    item["id"].as_str().unwrap_or(""),
                    item["summary"].as_str().unwrap_or("")
                ));
            }
        }
        if lines.is_empty() {
            "No leads yet.".to_string()
        } else {
            lines.join("\n")
        }
    }
}

fn lead_status(lead: &Lead) -> Status {
    if lead
        .runs
        .iter()
        .any(|run| run.status == Status::WaitingOnYou)
    {
        return Status::WaitingOnYou;
    }
    if lead.runs.iter().any(|run| run.status == Status::Running) {
        return Status::Running;
    }
    if !lead.runs.is_empty() && lead.runs.iter().all(|run| run.status == Status::Done) {
        return Status::Done;
    }
    Status::Idle
}

fn item_for_lead(item: &NeedsYouRecord, lead: &Lead) -> bool {
    item.actor == lead.id
        || item.resource == lead.id
        || item
            .run_tag
            .as_ref()
            .is_some_and(|tag| lead.runs.iter().any(|run| &run.id == tag))
        || lead.runs.iter().any(|run| item.resource == run.id)
}

/// Canonical steer arguments. Grants and later calls share this JSON.
pub fn steer_input(resource: &str, text: Option<&str>) -> String {
    let mut body = json!({ "resource": resource });
    if let Some(text) = text {
        body["text"] = json!(text);
    }
    body.to_string()
}

pub fn admit_steer(
    monitor: &gate::PermissionMonitor,
    workspace: &Path,
    tool: &str,
    input_json: &str,
) -> gate::Admission {
    monitor.admit(gate::AdmitRequest {
        call_id: &format!("cv-{}", uuid::Uuid::new_v4()),
        tool,
        input_json,
        actor_type: ActorType::Agent,
        actor_id: STEER_ACTOR,
        actor_scope: ActorScope::User,
        trust_origin: "host",
        workspace_root: workspace,
        context_id: 0,
        package_id: "",
        instance_id: 0,
        target_type: TargetType::HostTool,
    })
}

pub fn grant_steer(
    monitor: &gate::PermissionMonitor,
    workspace: &Path,
    tool: &str,
    input_json: &str,
) -> Result<String, String> {
    if !STEER_TOOLS.contains(&tool) {
        return Err(format!("unknown steer tool {tool}"));
    }
    let fingerprint = gate::fingerprint_args(input_json)?;
    let (resource_scope, resource_id) = gate::resource_of(tool, input_json);
    if resource_scope != ResourceScope::Workspace || resource_id.is_none() {
        return Err("steer resource is required".into());
    }
    let grant_id = format!("grant_{}", uuid::Uuid::new_v4());
    let binding = ExactBinding {
        actor_type: ActorType::Agent,
        actor_id: STEER_ACTOR.to_string(),
        actor_scope: ActorScope::User,
        trust_origin: "host".to_string(),
        workspace_root: workspace.to_path_buf(),
        target_type: TargetType::HostTool,
        target_id: tool.to_string(),
        resource_scope,
        resource_id: resource_id.clone(),
        args_fingerprint: fingerprint,
        session_id: None,
        package_id: String::new(),
        instance_id: Some(0),
        context_id: Some(0),
        call_id: String::new(),
        operation_id: String::new(),
    };
    let mut record = GrantRecord::from_binding(
        &binding,
        Decision::Allow,
        GrantDuration::Always,
        GrantSource::User,
        &grant_id,
    );
    record.expires_at = Some(crate::platform::clock::now_secs() as i64 + 30 * 24 * 60 * 60);
    log::info!(
        "command_view: granted {tool} resource={}",
        resource_id.unwrap_or_default()
    );
    monitor.store().record(record);
    monitor.store().save();
    Ok(grant_id)
}

fn shared() -> &'static Mutex<CommandBoard> {
    static BOARD: OnceLock<Mutex<CommandBoard>> = OnceLock::new();
    BOARD.get_or_init(|| Mutex::new(CommandBoard::default()))
}

fn wake() -> &'static (Mutex<u64>, Condvar) {
    static WAKE: OnceLock<(Mutex<u64>, Condvar)> = OnceLock::new();
    WAKE.get_or_init(|| (Mutex::new(0), Condvar::new()))
}

fn lock_board() -> std::sync::MutexGuard<'static, CommandBoard> {
    shared().lock().unwrap_or_else(|err| err.into_inner())
}

fn publish(revision: u64, summary: &str, payload: Value) {
    let (lock, cond) = wake();
    *lock.lock().unwrap_or_else(|err| err.into_inner()) = revision;
    cond.notify_all();
    log::info!("command_view: revision={revision} {summary}");
    let timeline = crate::host::app_timeline::global();
    let Ok(mut timeline) = timeline.lock() else {
        log::warn!("command_view: timeline lock failed");
        return;
    };
    if let Err(error) = timeline.record_command_view(revision, summary, payload) {
        log::warn!("command_view: event record failed: {error}");
    }
}

fn emit_if_changed(before: u64, monitor: &gate::PermissionMonitor) {
    let (revision, summary, payload) = {
        let board = lock_board();
        if board.revision == before {
            return;
        }
        let needs = monitor.open_needs_you();
        (board.revision, board.summary.clone(), board.project(&needs))
    };
    publish(revision, &summary, payload);
}

/// One `plexi command-view` call.
pub struct CommandRequest<'a> {
    pub op: &'a str,
    pub lead: &'a str,
    pub run: &'a str,
    pub text: &'a str,
    pub summary: &'a str,
    pub tool: &'a str,
    pub id: &'a str,
    pub approve: bool,
    pub workspace: &'a str,
}

/// Host entry for `plexi command-view`. Steer ops admit before they mutate.
pub fn handle(request: &CommandRequest<'_>) -> Value {
    let monitor = gate::PermissionMonitor::for_profile(&crate::config::config_dir());
    let before = lock_board().revision;
    let result = dispatch(&monitor, request);
    if request.op != "resolve" && result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        emit_if_changed(before, &monitor);
    }
    result
}

fn dispatch(monitor: &gate::PermissionMonitor, request: &CommandRequest<'_>) -> Value {
    let CommandRequest {
        op,
        lead,
        run,
        text,
        summary,
        tool,
        id,
        approve,
        workspace,
    } = *request;
    match op {
        "list" => projection(monitor),
        "send" | "enqueue" | "pause" | "cancel" => {
            let (tool_name, resource, message) = match op {
                "send" => ("command.send", lead, Some(text)),
                "enqueue" => ("command.enqueue", lead, Some(text)),
                "pause" => ("command.pause", run, None),
                "cancel" => ("command.cancel", run, None),
                _ => unreachable!("op matched above"),
            };
            let workspace = match workspace_path(workspace) {
                Ok(path) => path,
                Err(error) => return error_body(&error),
            };
            let input = steer_input(resource, message.filter(|value| !value.is_empty()));
            if message.is_some_and(str::is_empty) {
                return error_body("text is required");
            }
            if resource.is_empty() {
                return error_body("lead or run is required");
            }
            match admit_steer(monitor, &workspace, tool_name, &input) {
                gate::Admission::Proceed { .. } => {}
                gate::Admission::Required { pending_request_id } => {
                    return json!({
                        "ok": false,
                        "error": "permission_required",
                        "error_code": "permission_required",
                        "pending_request_id": pending_request_id,
                    });
                }
                gate::Admission::Denied { code } => {
                    return json!({
                        "ok": false,
                        "error": code,
                        "error_code": code,
                    });
                }
            }
            let mutated = {
                let mut board = lock_board();
                match op {
                    "send" => board
                        .send(lead, message.unwrap_or(""))
                        .map(|run_id| json!({"run_id": run_id})),
                    "enqueue" => board
                        .enqueue(lead, message.unwrap_or(""))
                        .map(|task_id| json!({"task_id": task_id})),
                    "pause" => board.pause(run).map(|()| json!({})),
                    "cancel" => board.cancel(run).map(|()| json!({})),
                    _ => unreachable!("op matched above"),
                }
            };
            match mutated {
                Ok(extra) => {
                    let mut body = projection(monitor);
                    if let (Some(obj), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
                        for (key, value) in extra {
                            obj.insert(key.clone(), value.clone());
                        }
                    }
                    body
                }
                Err(error) => error_body(&error),
            }
        }
        "block" => {
            let filed = {
                let mut board = lock_board();
                board.block(lead, run, summary)
            };
            match filed {
                Ok(file) => match monitor.file_needs_you(file) {
                    Ok(needs_id) => {
                        let mut body = projection(monitor);
                        body["needs_you_id"] = json!(needs_id);
                        body
                    }
                    Err(error) => error_body(&error),
                },
                Err(error) => error_body(&error),
            }
        }
        "resolve" => match settle_needs_you(monitor, id, approve) {
            Ok(receipt) => {
                let mut body = projection(monitor);
                body["id"] = json!(receipt.id);
                body["resolution"] = json!(receipt.resolution.as_str());
                body["already"] = json!(receipt.already);
                body["ok"] = json!(!receipt.already);
                body
            }
            Err(error) => error_body(&error),
        },
        "allow" => {
            let workspace = match workspace_path(workspace) {
                Ok(path) => path,
                Err(error) => return error_body(&error),
            };
            let (resource, text) = match tool {
                "command.send" | "command.enqueue" => (lead, Some(text)),
                "command.pause" | "command.cancel" => (run, None),
                _ => {
                    return error_body(
                        "tool must be command.send, command.enqueue, command.pause, or command.cancel",
                    );
                }
            };
            if resource.is_empty() || text.is_some_and(str::is_empty) {
                return error_body("allow needs the same lead, run, and text as the steer");
            }
            let input = steer_input(resource, text.filter(|value| !value.is_empty()));
            match grant_steer(monitor, &workspace, tool, &input) {
                Ok(grant_id) => json!({"ok": true, "grant_id": grant_id, "tool": tool}),
                Err(error) => error_body(&error),
            }
        }
        _ => error_body("unknown command-view operation"),
    }
}

fn workspace_path(workspace: &str) -> Result<std::path::PathBuf, String> {
    if workspace.is_empty() {
        return Err("workspace is required".into());
    }
    Ok(crate::platform::path::canonical_or_self(Path::new(
        workspace,
    )))
}

fn error_body(error: &str) -> Value {
    json!({"ok": false, "error": error})
}

pub fn projection(monitor: &gate::PermissionMonitor) -> Value {
    let needs = monitor.open_needs_you();
    lock_board().project(&needs)
}

/// Resolve one needs-you item and, when it names a waiting run, unblock it.
pub fn settle_needs_you(
    monitor: &gate::PermissionMonitor,
    id: &str,
    approve: bool,
) -> Result<gate::NeedsYouReceipt, String> {
    let before_rev = lock_board().revision;
    let before = monitor
        .open_needs_you()
        .into_iter()
        .find(|row| row.id == id);
    let receipt = monitor.resolve_needs_you(id, approve)?;
    if !receipt.already {
        if let Some(row) = before {
            let mut board = lock_board();
            let _ = board.apply_resolution(row.run_tag.as_deref(), approve);
        }
    }
    emit_if_changed(before_rev, monitor);
    Ok(receipt)
}

pub fn pane_text() -> String {
    let monitor = gate::PermissionMonitor::for_profile(&crate::config::config_dir());
    let needs = monitor.open_needs_you();
    lock_board().lines(&needs)
}

pub fn pane_state() -> Value {
    let monitor = gate::PermissionMonitor::for_profile(&crate::config::config_dir());
    projection(&monitor)
}

/// Stream `command.view` events until the client disconnects.
pub fn serve_follow(mut socket: crate::platform::ipc::IpcStream) {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = Arc::clone(&stop);
    let mut reader = match socket.try_clone() {
        Ok(clone) => clone,
        Err(error) => {
            log::warn!("command_view: follow clone failed: {error}");
            return;
        }
    };
    std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) | Err(_) => {
                    stop_reader.store(true, Ordering::Release);
                    wake().1.notify_all();
                    break;
                }
                Ok(_) => {}
            }
        }
    });
    let hello = json!({"type":"subscribed","event":STREAM,"app_id":PUBLISHER});
    if writeln_line(&mut socket, &hello).is_err() {
        return;
    }
    let mut seen = u64::MAX;
    while !stop.load(Ordering::Acquire) {
        let (revision, line) = {
            let monitor = gate::PermissionMonitor::for_profile(&crate::config::config_dir());
            let needs = monitor.open_needs_you();
            let board = lock_board();
            (board.revision, board.project(&needs))
        };
        if revision != seen {
            seen = revision;
            let mut event = line;
            event["type"] = json!("event");
            event["event"] = json!(STREAM);
            if writeln_line(&mut socket, &event).is_err() {
                return;
            }
        }
        let (lock, cond) = wake();
        let guard = lock.lock().unwrap_or_else(|err| err.into_inner());
        let _ = cond.wait_timeout(guard, Duration::from_secs(15));
    }
    log::info!("command_view: follow closed");
}

fn writeln_line(socket: &mut crate::platform::ipc::IpcStream, value: &Value) -> Result<(), ()> {
    use std::io::Write;
    writeln!(socket, "{value}")
        .and_then(|()| socket.flush())
        .map_err(|error| {
            log::info!("command_view: follow write failed: {error}");
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_needs(id: &str, lead: &str, run: &str) -> NeedsYouRecord {
        NeedsYouRecord {
            id: id.into(),
            kind: NeedsYouKind::BlockedRun,
            actor: lead.into(),
            resource: run.into(),
            summary: "which file".into(),
            created_at: 1,
            expires_at: None,
            run_tag: Some(run.into()),
            resolution: None,
        }
    }

    #[test]
    fn projection_lists_status_queue_and_last_output() {
        let mut board = CommandBoard::default();
        let run = board.send("lead-a", "editing notes").unwrap();
        let task = board.enqueue("lead-a", "ship the notes").unwrap();
        board.enqueue("lead-b", "wait here").unwrap();
        let view = board.project(&[]);
        let leads = view["leads"].as_array().unwrap();
        assert_eq!(leads.len(), 2);
        assert_eq!(leads[0]["status"], "running");
        assert_eq!(leads[0]["last_output"], "editing notes");
        assert_eq!(leads[0]["queue"][0]["id"], task);
        assert_eq!(leads[0]["queue"][0]["status"], "pending");
        assert_eq!(leads[0]["runs"][0]["id"], run);
        assert_eq!(leads[1]["status"], "idle");
        assert_eq!(leads[1]["last_output"], "");
    }

    #[test]
    fn pause_cancel_and_resolve_move_the_run() {
        let mut board = CommandBoard::default();
        let run = board.send("lead-a", "hello").unwrap();
        board.pause(&run).unwrap();
        assert_eq!(board.project(&[])["leads"][0]["status"], "idle");
        board.cancel(&run).unwrap();
        assert_eq!(board.project(&[])["leads"][0]["runs"][0]["status"], "done");
        assert_eq!(board.project(&[])["leads"][0]["status"], "done");

        let mut board = CommandBoard::default();
        board.block("lead-b", "run-9", "which file").unwrap();
        let needs = sample_needs("ny_1", "lead-b", "run-9");
        let view = board.project(&[needs]);
        assert_eq!(view["leads"][0]["status"], "waiting on you");
        assert_eq!(view["leads"][0]["needs_you"][0]["id"], "ny_1");
        assert!(board.apply_resolution(Some("run-9"), true));
        assert_eq!(board.project(&[])["leads"][0]["status"], "running");
        let mut board = CommandBoard::default();
        board.block("lead-b", "run-9", "which file").unwrap();
        assert!(board.apply_resolution(Some("run-9"), false));
        assert_eq!(board.project(&[])["leads"][0]["status"], "done");
    }

    #[test]
    fn steer_is_refused_until_the_exact_grant_exists() {
        let dir = tempfile::tempdir().unwrap();
        let monitor = gate::PermissionMonitor::ephemeral();
        let input = steer_input("lead-a", Some("hello"));
        assert!(matches!(
            admit_steer(&monitor, dir.path(), "command.send", &input),
            gate::Admission::Required { .. }
        ));
        grant_steer(&monitor, dir.path(), "command.send", &input).unwrap();
        assert!(matches!(
            admit_steer(&monitor, dir.path(), "command.send", &input),
            gate::Admission::Proceed { .. }
        ));
        let other = steer_input("lead-a", Some("different"));
        assert!(matches!(
            admit_steer(&monitor, dir.path(), "command.send", &other),
            gate::Admission::Required { .. }
        ));
    }
}
