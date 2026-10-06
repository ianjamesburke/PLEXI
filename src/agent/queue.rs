//! Durable work for a head that has no pane open.
//!
//! Tasks live in `<workspace>/.plexi/agents/queue.json`. The host pumps the
//! file from `App::logic`: a queued task starts a lead turn, and a `running`
//! task whose thread is gone becomes `outcome_unknown` instead of vanishing.
//! Cancel is a flag the turn reads before each tool, so a stopped run does
//! not admit another call.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Task {
    id: String,
    head: String,
    text: String,
    state: String,
    #[serde(default)]
    cancel: bool,
    #[serde(default)]
    run_id: String,
    #[serde(default)]
    error: String,
    created_at: String,
    updated_at: String,
}

fn store() -> &'static Mutex<()> {
    static STORE: OnceLock<Mutex<()>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(()))
}

fn starting() -> &'static Mutex<std::collections::HashSet<String>> {
    static STARTING: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    STARTING.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

fn queue_path(workspace: &Path) -> PathBuf {
    crate::agent::workspace_agents_dir(workspace).join("queue.json")
}

fn now() -> String {
    crate::host::event_log::now_timestamp()
}

fn with_tasks<T>(
    workspace: &Path,
    write: bool,
    body: impl FnOnce(&mut Vec<Task>) -> T,
) -> Result<T, String> {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    let _guard = store().lock().unwrap_or_else(|error| error.into_inner());
    let lock = lock_queue(&workspace)?;
    let path = queue_path(&workspace);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let mut tasks = read_tasks(&path)?;
    let value = body(&mut tasks);
    if write {
        write_tasks(&path, &tasks)?;
    }
    drop(lock);
    Ok(value)
}

struct QueueLock {
    _file: fs::File,
}

fn lock_queue(workspace: &Path) -> Result<QueueLock, String> {
    let path = crate::agent::workspace_agents_dir(workspace).join("queue.lock");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY: `file` is an open descriptor and LOCK_EX is a valid flock op.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return Err(format!(
                "lock {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(QueueLock { _file: file })
}

impl Drop for QueueLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // SAFETY: `self._file` is the descriptor flock locked in `lock_queue`.
            unsafe { libc::flock(self._file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

fn read_tasks(path: &Path) -> Result<Vec<Task>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn write_tasks(path: &Path, tasks: &[Task]) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(tasks).map_err(|error| error.to_string())?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file =
            fs::File::create(&tmp).map_err(|error| format!("create {}: {error}", tmp.display()))?;
        file.write_all(raw.as_bytes())
            .map_err(|error| format!("write {}: {error}", tmp.display()))?;
        file.sync_all()
            .map_err(|error| format!("sync {}: {error}", tmp.display()))?;
    }
    fs::rename(&tmp, path).map_err(|error| format!("rename {}: {error}", path.display()))?;
    Ok(())
}

fn task_json(task: &Task) -> Value {
    json!({
        "id": task.id,
        "head": task.head,
        "text": task.text,
        "state": task.state,
        "cancel": task.cancel,
        "run_id": task.run_id,
        "error": task.error,
    })
}

pub fn snapshot(workspace: &Path) -> Vec<Value> {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    with_tasks(&workspace, false, |tasks| {
        tasks.iter().map(task_json).collect()
    })
    .unwrap_or_else(|error| {
        log::error!("queue: snapshot failed: {error}");
        Vec::new()
    })
}

pub fn enqueue(workspace: &Path, head: &str, text: &str) -> Value {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    if text.trim().is_empty() {
        return json!({"ok": false, "error_code": "invalid_argument", "error": "text is required"});
    }
    if !crate::agent::workspace_agents_dir(&workspace)
        .join(head)
        .join("head.json")
        .is_file()
    {
        return json!({"ok": false, "error_code": "head_not_found", "error": format!("no head '{head}'")});
    }
    let id = format!("task_{}", uuid::Uuid::new_v4());
    let stamp = now();
    let task = Task {
        id: id.clone(),
        head: head.to_string(),
        text: text.to_string(),
        state: "queued".to_string(),
        cancel: false,
        run_id: String::new(),
        error: String::new(),
        created_at: stamp.clone(),
        updated_at: stamp,
    };
    match with_tasks(&workspace, true, |tasks| {
        tasks.push(task.clone());
    }) {
        Ok(()) => {
            log::info!("queue: assigned head={head} task={id}");
            json!({"ok": true, "task": task_json(&task)})
        }
        Err(error) => json!({"ok": false, "error_code": "io_error", "error": error}),
    }
}

pub fn request_cancel(workspace: &Path, task_id: &str) -> Value {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    match with_tasks(&workspace, true, |tasks| {
        let Some(task) = tasks.iter_mut().find(|task| task.id == task_id) else {
            return json!({"ok": false, "error_code": "task_not_found", "error": format!("no task '{task_id}'")});
        };
        if matches!(
            task.state.as_str(),
            "succeeded"
                | "failed"
                | "cancelled"
                | "outcome_unknown"
                | "permission_required"
                | "permission_denied"
        ) {
            return json!({"ok": true, "task": task_json(task), "already_terminal": true});
        }
        task.cancel = true;
        task.updated_at = now();
        if task.state == "queued" {
            task.state = "cancelled".to_string();
        }
        log::info!("queue: cancel task={task_id} state={}", task.state);
        json!({"ok": true, "task": task_json(task)})
    }) {
        Ok(value) => value,
        Err(error) => json!({"ok": false, "error_code": "io_error", "error": error}),
    }
}

fn cancelled_path(workspace: &Path) -> PathBuf {
    crate::agent::workspace_agents_dir(workspace).join("cancelled-runs.json")
}

fn with_cancelled<T>(
    workspace: &Path,
    write: bool,
    body: impl FnOnce(&mut Vec<String>) -> T,
) -> Result<T, String> {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    let _guard = store().lock().unwrap_or_else(|error| error.into_inner());
    let lock = lock_queue(&workspace)?;
    let path = cancelled_path(&workspace);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let mut ids = if path.exists() {
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if raw.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&raw)
                .map_err(|error| format!("parse {}: {error}", path.display()))?
        }
    } else {
        Vec::new()
    };
    let value = body(&mut ids);
    if write {
        let raw = serde_json::to_string_pretty(&ids).map_err(|error| error.to_string())?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut file = fs::File::create(&tmp)
                .map_err(|error| format!("create {}: {error}", tmp.display()))?;
            file.write_all(raw.as_bytes())
                .map_err(|error| format!("write {}: {error}", tmp.display()))?;
            file.sync_all()
                .map_err(|error| format!("sync {}: {error}", tmp.display()))?;
        }
        fs::rename(&tmp, &path).map_err(|error| format!("rename {}: {error}", path.display()))?;
    }
    drop(lock);
    Ok(value)
}

/// Stop a live run. The model loop reads this before it admits the next tool.
pub fn request_cancel_run(workspace: &Path, run_id: &str) -> Value {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    if run_id.is_empty() {
        return json!({"ok": false, "error_code": "invalid_argument", "error": "run id is required"});
    }
    let shown = crate::agent::heads::handle_request(
        "show_run",
        &json!({"workspace": workspace, "id": run_id}),
    );
    if shown.get("ok").and_then(|value| value.as_bool()) != Some(true) {
        return shown;
    }
    let stored = with_cancelled(&workspace, true, |ids| {
        if !ids.iter().any(|id| id == run_id) {
            ids.push(run_id.to_string());
        }
    });
    if let Err(error) = stored {
        return json!({"ok": false, "error_code": "io_error", "error": error});
    }
    let _ = with_tasks(&workspace, true, |tasks| {
        for task in tasks.iter_mut() {
            if task.run_id == run_id && task.state == "running" {
                task.cancel = true;
                task.updated_at = now();
            }
        }
    });
    log::info!("queue: cancel run={run_id}");
    json!({"ok": true, "run_id": run_id, "state": "cancelled"})
}

pub fn run_cancel_requested(workspace: &Path, run_id: &str) -> bool {
    if run_id.is_empty() {
        return false;
    }
    let workspace = crate::platform::path::canonical_or_self(workspace);
    with_cancelled(&workspace, false, |ids| ids.iter().any(|id| id == run_id)).unwrap_or(false)
}

pub fn attach_run(workspace: &Path, task_id: &str, run_id: &str) {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    let result = with_tasks(&workspace, true, |tasks| {
        if let Some(task) = tasks.iter_mut().find(|task| task.id == task_id) {
            task.run_id = run_id.to_string();
            task.updated_at = now();
        }
    });
    if let Err(error) = result {
        log::error!("queue: attach run {run_id} to {task_id} failed: {error}");
    }
}

pub fn cancel_requested(workspace: &Path, task_id: &str) -> bool {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    with_tasks(&workspace, false, |tasks| {
        tasks.iter().any(|task| task.id == task_id && task.cancel)
    })
    .unwrap_or(false)
}

pub fn finish_task(workspace: &Path, task_id: &str, state: &str, run_id: &str, error: &str) {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    let result = with_tasks(&workspace, true, |tasks| {
        if let Some(task) = tasks.iter_mut().find(|task| task.id == task_id) {
            if task.state == "cancelled" && state != "cancelled" {
                return;
            }
            task.state = state.to_string();
            if !run_id.is_empty() {
                task.run_id = run_id.to_string();
            }
            task.error = error.to_string();
            task.updated_at = now();
            log::info!("queue: task={task_id} state={state}");
        }
    });
    if let Err(error) = result {
        log::error!("queue: finish {task_id} failed: {error}");
    }
}

/// Mark crashed `running` tasks and start queued ones. A task this process
/// just handed to a thread is in the starting set until `submit_turn` records
/// it as in flight, so it is not declared `outcome_unknown`.
fn mark_orphans(workspace: &Path) -> Result<(), String> {
    with_tasks(workspace, true, |tasks| {
        let starting_ids = starting()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        for task in tasks.iter_mut() {
            if task.state == "running"
                && !crate::agent::leads::head_busy(&task.head)
                && !starting_ids.contains(&task.id)
            {
                task.state = "outcome_unknown".to_string();
                task.error = "the host restarted while this task was running".to_string();
                task.updated_at = now();
                log::info!("queue: outcome_unknown task={} head={}", task.id, task.head);
            }
        }
    })
}

fn pending_id_of(error: &str) -> Option<&str> {
    error.split_whitespace().find(|token| token.starts_with("req_"))
}

/// A click does not rerun the turn by itself. Put an approved task back on
/// the queue so the same assignment continues under the grant.
fn release_approved(workspace: &Path) -> Result<(), String> {
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
    with_tasks(workspace, true, |tasks| {
        for task in tasks.iter_mut() {
            if task.state != "permission_required" || task.cancel {
                continue;
            }
            let Some(pending) = pending_id_of(&task.error) else {
                continue;
            };
            if !monitor.approval_granted(pending) {
                continue;
            }
            log::info!(
                "queue: approval released task={} head={} pending={pending}",
                task.id, task.head
            );
            task.state = "queued".to_string();
            task.error.clear();
            task.updated_at = now();
        }
    })
}

pub fn pump(workspace: &Path) {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    if !queue_path(&workspace).exists() {
        return;
    }
    if let Err(error) = mark_orphans(&workspace) {
        log::error!("queue: recover failed: {error}");
        return;
    }
    if let Err(error) = release_approved(&workspace) {
        log::error!("queue: release approved failed: {error}");
        return;
    }
    let ready = match with_tasks(&workspace, true, |tasks| {
        let mut ready = Vec::new();
        for task in tasks.iter_mut() {
            if task.state != "queued" || task.cancel {
                continue;
            }
            if crate::agent::leads::head_busy(&task.head) {
                continue;
            }
            if ready
                .iter()
                .any(|(_, head, _): &(String, String, String)| head == &task.head)
            {
                continue;
            }
            task.state = "running".to_string();
            task.updated_at = now();
            ready.push((task.id.clone(), task.head.clone(), task.text.clone()));
        }
        ready
    }) {
        Ok(ready) => ready,
        Err(error) => {
            log::error!("queue: start failed: {error}");
            return;
        }
    };
    for (id, head, text) in ready {
        starting()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.clone());
        let response = crate::rpc::response_file("lead-queue", "json");
        let accepted =
            crate::agent::leads::submit_turn(&workspace, &head, &text, &id, &response, Some(&id));
        starting()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
        if accepted.get("ok").and_then(|value| value.as_bool()) != Some(true) {
            let error = accepted
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("lead turn was not accepted");
            finish_task(&workspace, &id, "failed", "", error);
        } else {
            log::info!("queue: started task={id} head={head}");
        }
    }
}

pub fn handle(workspace: &Path, op: &str, payload: &Value) -> Value {
    match op {
        "assign" => {
            let head = payload
                .get("head")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let text = payload
                .get("text")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let queued = enqueue(workspace, head, text);
            if queued.get("ok").and_then(|value| value.as_bool()) == Some(true) {
                pump(workspace);
            }
            queued
        }
        "cancel" => {
            let id = payload
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            request_cancel(workspace, id)
        }
        "list" | "pump" => {
            if op == "pump" {
                pump(workspace);
            }
            json!({"ok": true, "tasks": snapshot(workspace)})
        }
        other => {
            json!({"ok": false, "error_code": "invalid_argument", "error": format!("unknown queue op {other}")})
        }
    }
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
    }

    #[test]
    fn a_queued_task_is_not_dropped_and_an_orphan_run_is_unknown() {
        let fixture = Fixture::new();
        let created = crate::agent::heads::handle_request(
            "create_head",
            &json!({"workspace": fixture.ws(), "name": "lead-b", "display_name": "Lead B"}),
        );
        assert_eq!(
            created.get("ok").and_then(|value| value.as_bool()),
            Some(true),
            "{created}"
        );
        let queued = enqueue(fixture.ws(), "lead-b", "write-file");
        assert_eq!(
            queued.get("ok").and_then(|value| value.as_bool()),
            Some(true),
            "{queued}"
        );
        let queued_id = queued["task"]["id"].as_str().unwrap().to_string();
        let running = enqueue(fixture.ws(), "lead-b", "orphan");
        let running_id = running["task"]["id"].as_str().unwrap().to_string();
        with_tasks(fixture.ws(), true, |tasks| {
            tasks
                .iter_mut()
                .find(|task| task.id == running_id)
                .unwrap()
                .state = "running".to_string();
        })
        .unwrap();
        mark_orphans(fixture.ws()).unwrap();
        let rows = snapshot(fixture.ws());
        let queued_state = rows.iter().find(|row| row["id"] == queued_id).unwrap()["state"]
            .as_str()
            .unwrap();
        let running_state = rows.iter().find(|row| row["id"] == running_id).unwrap()["state"]
            .as_str()
            .unwrap();
        assert_eq!(queued_state, "queued");
        assert_eq!(running_state, "outcome_unknown");
    }

    #[test]
    fn cancel_of_a_queued_task_does_not_start_it() {
        let fixture = Fixture::new();
        let _ = crate::agent::heads::handle_request(
            "create_head",
            &json!({"workspace": fixture.ws(), "name": "lead-b"}),
        );
        let queued = enqueue(fixture.ws(), "lead-b", "long-task");
        let id = queued["task"]["id"].as_str().unwrap();
        let cancelled = request_cancel(fixture.ws(), id);
        assert_eq!(cancelled["task"]["state"], "cancelled", "{cancelled}");
        assert!(cancel_requested(fixture.ws(), id));
    }

    #[test]
    fn an_approved_click_returns_the_same_task_to_the_queue() {
        use crate::broker::gate::{Admission, AdmitRequest, ApprovalChoice, PermissionMonitor};
        use crate::broker::{ActorScope, ActorType, TargetType};

        let fixture = Fixture::new();
        let _ = crate::agent::heads::handle_request(
            "create_head",
            &json!({"workspace": fixture.ws(), "name": "lead-b"}),
        );
        let queued = enqueue(fixture.ws(), "lead-b", "write-file");
        let id = queued["task"]["id"].as_str().unwrap().to_string();
        let held = enqueue(fixture.ws(), "lead-b", "still-waiting");
        let held_id = held["task"]["id"].as_str().unwrap().to_string();
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir());
        let admission = monitor.admit(AdmitRequest {
            call_id: "call-release",
            tool: "host.files.write",
            input_json: r#"{"file":"out.txt","content":"ok"}"#,
            actor_type: ActorType::Agent,
            actor_id: "agent:lead-b",
            actor_scope: ActorScope::User,
            trust_origin: "host",
            workspace_root: fixture.ws(),
            context_id: 0,
            package_id: "plexi",
            instance_id: 0,
            target_type: TargetType::HostTool,
        });
        let pending = match admission {
            Admission::Required { pending_request_id } => pending_request_id,
            Admission::Proceed { .. } => panic!("expected a pending ask, grant matched"),
            Admission::Denied { code } => panic!("expected a pending ask, denied {code}"),
        };
        monitor
            .approve_pending(&pending, ApprovalChoice::Once)
            .unwrap();
        with_tasks(fixture.ws(), true, |tasks| {
            for task in tasks.iter_mut() {
                if task.id == id {
                    task.state = "permission_required".to_string();
                    task.error = format!("permission_required {pending}");
                } else if task.id == held_id {
                    task.state = "permission_required".to_string();
                    task.error = "permission_required req_still_waiting".to_string();
                }
            }
        })
        .unwrap();
        release_approved(fixture.ws()).unwrap();
        let rows = snapshot(fixture.ws());
        let released = rows.iter().find(|row| row["id"] == id).unwrap();
        assert_eq!(released["state"], "queued");
        assert_eq!(released["error"], "");
        let waiting = rows.iter().find(|row| row["id"] == held_id).unwrap();
        assert_eq!(waiting["state"], "permission_required");
    }
}
