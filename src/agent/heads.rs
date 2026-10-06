//! Minimal Agents API.
//!
//! One host service, used by the CLI and by the host MCP endpoint. Agent
//! definitions live only under `<workspace>/.plexi/agents/`. `AGENT.md` is
//! guidance and grants nothing. Authority is a `GrantRecord` in the permission
//! monitor: an allow is a tool-scoped grant (`args_unbound`), an ask is the
//! ordinary pending-approval path, and a tool outside that set is a denial.
//! Delegation copies a subset of the parent run's grants onto a temporary
//! child. A child that asks for a tool the parent does not hold is refused
//! and audited. Reporting metadata (`reports_to`) grants no rights.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::broker::gate::{Admission, AdmitRequest, PermissionMonitor};
use crate::broker::{
    ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, TargetType,
};
use crate::plexi_ai::ledger::{self, LedgerRow};

const PACKAGE: &str = "agents";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ToolGrant {
    pub tool: String,
    pub decision: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct HeadCard {
    id: String,
    display_name: String,
    description: String,
    /// Presentation only. A reporting line does not confer a grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reports_to: Option<String>,
    #[serde(default)]
    temporary: bool,
    grants: Vec<ToolGrant>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RunRecord {
    id: String,
    head_id: String,
    actor_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_run: Option<String>,
    admission_id: String,
    /// Monotonic claim version. A repeated admission id keeps this value.
    version: u64,
    state: String,
    active: bool,
    client_ref: String,
    kind: String,
    input_tokens: u32,
    output_tokens: u32,
    grants: Vec<ToolGrant>,
}

struct TokenRec {
    run_id: String,
    actor_id: String,
    workspace: PathBuf,
}

fn tokens() -> &'static Mutex<HashMap<String, TokenRec>> {
    static TOKENS: OnceLock<Mutex<HashMap<String, TokenRec>>> = OnceLock::new();
    TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn token_by_run() -> &'static Mutex<HashMap<String, String>> {
    static BY_RUN: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    BY_RUN.get_or_init(|| Mutex::new(HashMap::new()))
}

fn journal_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub struct RunAuth {
    pub run_id: String,
    pub actor_id: String,
    pub workspace: PathBuf,
}

pub fn authenticate_run_token(token: &str) -> Option<RunAuth> {
    let map = tokens().lock().unwrap_or_else(|e| e.into_inner());
    map.get(token).map(|rec| RunAuth {
        run_id: rec.run_id.clone(),
        actor_id: rec.actor_id.clone(),
        workspace: rec.workspace.clone(),
    })
}

fn remember_token(workspace: &Path, run_id: &str, actor_id: &str) -> String {
    let mut by_run = token_by_run().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = by_run.get(run_id) {
        return existing.clone();
    }
    let token = format!("run_{}", uuid::Uuid::new_v4());
    by_run.insert(run_id.to_string(), token.clone());
    drop(by_run);
    tokens().lock().unwrap_or_else(|e| e.into_inner()).insert(
        token.clone(),
        TokenRec {
            run_id: run_id.to_string(),
            actor_id: actor_id.to_string(),
            workspace: workspace.to_path_buf(),
        },
    );
    token
}

fn actor_of(head_id: &str) -> String {
    format!("agent:{head_id}")
}

fn agents_root(workspace: &Path) -> PathBuf {
    crate::agent::workspace_agents_dir(workspace)
}

fn canonical_workspace(raw: &Path) -> Result<PathBuf, String> {
    let path = raw
        .canonicalize()
        .map_err(|error| format!("workspace {} is not available: {error}", raw.display()))?;
    if !path.is_dir() {
        return Err(format!("workspace {} is not a directory", path.display()));
    }
    Ok(path)
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn parse_grant_spec(spec: &str) -> Result<ToolGrant, String> {
    let (tool, decision) = spec
        .split_once('=')
        .ok_or_else(|| format!("grant '{spec}' must be tool=allow|ask|deny"))?;
    let tool = tool.trim();
    let decision = decision.trim();
    if tool.is_empty() || tool.contains('/') || tool.contains(' ') {
        return Err(format!("grant tool '{tool}' is not a tool name"));
    }
    if !matches!(decision, "allow" | "ask" | "deny") {
        return Err(format!(
            "grant decision '{decision}' must be allow, ask, or deny"
        ));
    }
    Ok(ToolGrant {
        tool: tool.to_string(),
        decision: decision.to_string(),
    })
}

fn decision_rank(decision: &str) -> u8 {
    match decision {
        "deny" => 0,
        "ask" => 1,
        "allow" => 2,
        _ => 3,
    }
}

fn covers(parent: &[ToolGrant], requested: &ToolGrant) -> bool {
    parent.iter().any(|grant| {
        grant.tool == requested.tool
            && decision_rank(&requested.decision) <= decision_rank(&grant.decision)
    })
}

fn monitor() -> std::sync::Arc<PermissionMonitor> {
    PermissionMonitor::for_profile(&crate::config::config_dir())
}

fn record_scope_allow(workspace: &Path, actor_id: &str, tool: &str) {
    let binding = crate::broker::ExactBinding {
        actor_type: ActorType::Agent,
        actor_id: actor_id.to_string(),
        actor_scope: ActorScope::Workspace,
        trust_origin: "host".to_string(),
        workspace_root: workspace.to_path_buf(),
        target_type: TargetType::HostTool,
        target_id: tool.to_string(),
        resource_scope: crate::broker::ResourceScope::Workspace,
        resource_id: None,
        args_fingerprint: "scope".to_string(),
        session_id: None,
        package_id: PACKAGE.to_string(),
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
        &format!("grant_{}", uuid::Uuid::new_v4()),
    );
    record.args_unbound = true;
    let gate = monitor();
    gate.store().record(record);
    gate.store().save();
    log::info!("agents_api: scope allow actor={actor_id} tool={tool}");
}

fn write_definition(workspace: &Path, card: &HeadCard) -> Result<(), String> {
    let dir = agents_root(workspace).join(&card.id);
    fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let prompt = format!(
        "# {}\n\nGuidance only. This file grants no tools and no scopes.\nReporting line: {}.\n",
        card.display_name,
        card.reports_to.as_deref().unwrap_or("none"),
    );
    fs::write(dir.join("AGENT.md"), prompt).map_err(|error| format!("write AGENT.md: {error}"))?;
    let tools = card
        .grants
        .iter()
        .map(|grant| format!("  \"{}\",", grant.tool))
        .collect::<Vec<_>>()
        .join("\n");
    let description = card.description.replace('"', "'");
    let settings = format!(
        "[agent]\n\
         id = \"{}\"\n\
         display_name = \"{}\"\n\
         default_tier = \"low\"\n\
         description = \"{description}\"\n\
         \n\
         [permissions]\n\
         default_posture = \"deny\"\n\
         \n\
         [tools]\n\
         enabled = [\n\
         {tools}\n\
         ]\n",
        card.id,
        card.display_name.replace('"', "'"),
    );
    fs::write(dir.join("settings.toml"), settings)
        .map_err(|error| format!("write settings.toml: {error}"))?;
    let body = serde_json::to_string_pretty(card).map_err(|error| error.to_string())?;
    fs::write(dir.join("head.json"), body).map_err(|error| format!("write head.json: {error}"))?;
    Ok(())
}

fn read_head(dir: &Path) -> Result<HeadCard, String> {
    let raw = fs::read_to_string(dir.join("head.json"))
        .map_err(|error| format!("read {}: {error}", dir.display()))?;
    serde_json::from_str(&raw).map_err(|error| format!("head.json in {}: {error}", dir.display()))
}

fn list_head_cards(workspace: &Path, include_temporary: bool) -> Result<Vec<HeadCard>, String> {
    let root = agents_root(workspace);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("read {}: {error}", root.display())),
    };
    let mut cards = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("read agent entry: {error}"))?;
        let path = entry.path();
        if !path.is_dir() || !path.join("head.json").is_file() {
            continue;
        }
        let card = read_head(&path)?;
        if card.temporary && !include_temporary {
            continue;
        }
        cards.push(card);
    }
    cards.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cards)
}

fn runs_path(workspace: &Path) -> PathBuf {
    agents_root(workspace).join("runs.jsonl")
}

fn load_runs(workspace: &Path) -> Result<Vec<RunRecord>, String> {
    let path = runs_path(workspace);
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("open {}: {error}", path.display())),
    };
    let mut folded: HashMap<String, RunRecord> = HashMap::new();
    for (index, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|error| format!("read {}: {error}", path.display()))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<RunRecord>(line) {
            Ok(record) => {
                folded.insert(record.id.clone(), record);
            }
            Err(error) => {
                log::error!(
                    "agents_api: skip bad run line {} in {}: {error}",
                    index + 1,
                    path.display()
                );
            }
        }
    }
    let mut runs: Vec<RunRecord> = folded.into_values().collect();
    runs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(runs)
}

fn append_run_record(workspace: &Path, record: &RunRecord) -> Result<(), String> {
    let path = runs_path(workspace);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let line = serde_json::to_string(record).map_err(|error| error.to_string())?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    writeln!(file, "{line}").map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok(())
}

fn deny(actor: &str, tool: &str, reason: &str) -> Value {
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    monitor().note_denial(actor, &call_id, tool, "agents-api", "deny");
    log::info!("agents_api: denied actor={actor} tool={tool} reason={reason}");
    json!({
        "ok": false,
        "error_code": "permission_denied",
        "error": reason,
        "tool": tool,
        "actor_id": actor,
        "call_id": call_id,
    })
}

fn subset_denied(actor: &str, parent: &[ToolGrant], requested: &[ToolGrant]) -> Option<Value> {
    for grant in requested {
        if !covers(parent, grant) {
            return Some(deny(
                actor,
                &grant.tool,
                &format!(
                    "delegation refused: '{grant_decision}' on '{tool}' is not a subset of the parent grants",
                    grant_decision = grant.decision,
                    tool = grant.tool,
                ),
            ));
        }
    }
    None
}

struct CreateHead<'a> {
    workspace: &'a Path,
    name: &'a str,
    display_name: &'a str,
    description: &'a str,
    grants: &'a [ToolGrant],
    temporary: bool,
    reports_to: Option<&'a str>,
    parent_grants: Option<&'a [ToolGrant]>,
    denial_actor: &'a str,
}

fn create_head(req: CreateHead<'_>) -> Value {
    if !valid_name(req.name) {
        return json!({"ok": false, "error_code": "invalid_name", "error": "head name must be a lowercase slug"});
    }
    if let Some(parent) = req.parent_grants {
        if let Some(refused) = subset_denied(req.denial_actor, parent, req.grants) {
            return refused;
        }
    }
    let dir = agents_root(req.workspace).join(req.name);
    if dir.join("head.json").is_file() {
        return json!({"ok": false, "error_code": "head_exists", "error": format!("head '{}' already exists", req.name)});
    }
    let card = HeadCard {
        id: req.name.to_string(),
        display_name: if req.display_name.is_empty() {
            req.name.to_string()
        } else {
            req.display_name.to_string()
        },
        description: req.description.to_string(),
        reports_to: req.reports_to.map(str::to_string),
        temporary: req.temporary,
        grants: req.grants.to_vec(),
    };
    if let Err(error) = write_definition(req.workspace, &card) {
        log::error!("agents_api: create head {} failed: {error}", req.name);
        return json!({"ok": false, "error_code": "io_error", "error": error});
    }
    let actor = actor_of(req.name);
    for grant in &card.grants {
        if grant.decision == "allow" {
            record_scope_allow(req.workspace, &actor, &grant.tool);
        }
    }
    log::info!(
        "agents_api: created head {} actor={actor} temporary={} grants={} workspace={}",
        card.id,
        card.temporary,
        card.grants.len(),
        req.workspace.display(),
    );
    json!({"ok": true, "head": card})
}

struct IssueRun<'a> {
    workspace: &'a Path,
    head: &'a HeadCard,
    parent_run: Option<&'a str>,
    admission_id: &'a str,
    client_ref: &'a str,
    kind: &'a str,
    input_tokens: u32,
    output_tokens: u32,
    /// When set, the new run also executes this prompt. `None` only journals.
    prompt: Option<&'a str>,
}

fn issue_run(req: IssueRun<'_>) -> Value {
    let IssueRun {
        workspace,
        head,
        parent_run,
        admission_id,
        client_ref,
        kind,
        input_tokens,
        output_tokens,
        prompt,
    } = req;
    let prompt = prompt.map(str::to_string);
    let head_id = head.id.clone();
    let workspace_buf = workspace.to_path_buf();
    if kind != "system" && kind != "output" {
        return json!({"ok": false, "error_code": "invalid_kind", "error": "kind must be system or output"});
    }
    let guard = journal_lock().lock().unwrap_or_else(|e| e.into_inner());
    let runs = match load_runs(workspace) {
        Ok(runs) => runs,
        Err(error) => return json!({"ok": false, "error_code": "io_error", "error": error}),
    };
    if let Some(existing) = runs
        .iter()
        .find(|run| run.head_id == head.id && run.admission_id == admission_id)
    {
        let token = remember_token(workspace, &existing.id, &existing.actor_id);
        log::info!(
            "agents_api: admission {} returned existing run {} for head {}",
            admission_id,
            existing.id,
            head.id,
        );
        return json!({
            "ok": true,
            "run": existing,
            "run_token": token,
            "idempotent": true,
        });
    }
    if let Some(active) = runs.iter().find(|run| run.head_id == head.id && run.active) {
        log::info!(
            "agents_api: assignment_conflict head={} active_run={} admission={admission_id}",
            head.id,
            active.id,
        );
        return json!({
            "ok": false,
            "error_code": "assignment_conflict",
            "error": "head already has an active run",
            "active_run": active.id,
            "admission_id": admission_id,
        });
    }
    let version = runs
        .iter()
        .filter(|run| run.head_id == head.id)
        .map(|run| run.version)
        .max()
        .unwrap_or(0)
        + 1;
    let record = RunRecord {
        id: format!("run_{}", uuid::Uuid::new_v4()),
        head_id: head.id.clone(),
        actor_id: actor_of(&head.id),
        parent_run: parent_run.map(str::to_string),
        admission_id: admission_id.to_string(),
        version,
        state: "running".to_string(),
        active: true,
        client_ref: client_ref.to_string(),
        kind: kind.to_string(),
        input_tokens,
        output_tokens,
        grants: head.grants.clone(),
    };
    if let Err(error) = append_run_record(workspace, &record) {
        log::error!("agents_api: append run failed: {error}");
        return json!({"ok": false, "error_code": "io_error", "error": error});
    }
    let row = LedgerRow::for_agent_run(
        &record.id,
        &record.actor_id,
        &record.client_ref,
        &record.kind,
        record.parent_run.as_deref(),
        record.input_tokens,
        record.output_tokens,
    );
    if let Err(error) = ledger::append_result(&row) {
        log::error!(
            "agents_api: ledger append failed for {}: {error}",
            record.id
        );
        return json!({"ok": false, "error_code": "ledger_error", "error": error, "run": record});
    }
    let token = remember_token(workspace, &record.id, &record.actor_id);
    let run_id = record.id.clone();
    log::info!(
        "agents_api: spawned run {} head={} client_ref={} kind={} tokens={}/{} version={}",
        record.id,
        record.head_id,
        record.client_ref,
        record.kind,
        record.input_tokens,
        record.output_tokens,
        record.version,
    );
    drop(guard);
    let mut body = json!({
        "ok": true,
        "run": record,
        "run_token": token,
        "idempotent": false,
        "ledger": {
            "run_id": row.run_id,
            "agent_id": row.agent_id,
            "client_ref": row.client_ref,
            "kind": row.kind,
            "input_tokens": row.input_tokens,
            "output_tokens": row.output_tokens,
            "parent_run_id": row.parent_run_id,
        },
    });
    if let Some(prompt) = prompt {
        log::info!("agents_api: spawn model turn head={head_id} run={run_id}");
        let turn = crate::agent::leads::run_prompt(&workspace_buf, &head_id, &prompt, Some(run_id.as_str()).filter(|id| !id.is_empty()));
        if let Some(obj) = body.as_object_mut() {
            if let Some(state) = turn.get("state").cloned() {
                obj.insert("state".to_string(), state);
            }
            if let Some(reply) = turn.get("reply").cloned() {
                obj.insert("reply".to_string(), reply);
            }
            if let Some(error) = turn.get("error").filter(|value| !value.is_null()) {
                obj.insert("model_error".to_string(), error.clone());
            }
        }
    }
    body
}

fn find_head(workspace: &Path, id: &str) -> Result<HeadCard, Value> {
    let dir = agents_root(workspace).join(id);
    if !dir.join("head.json").is_file() {
        return Err(
            json!({"ok": false, "error_code": "head_not_found", "error": format!("no head '{id}'")}),
        );
    }
    read_head(&dir).map_err(|error| json!({"ok": false, "error_code": "io_error", "error": error}))
}

fn find_run(workspace: &Path, id: &str) -> Result<RunRecord, Value> {
    let runs = load_runs(workspace)
        .map_err(|error| json!({"ok": false, "error_code": "io_error", "error": error}))?;
    runs.into_iter().find(|run| run.id == id).ok_or_else(
        || json!({"ok": false, "error_code": "run_not_found", "error": format!("no run '{id}'")}),
    )
}

fn grants_from_value(value: &Value) -> Result<Vec<ToolGrant>, String> {
    let specs = if let Some(list) = value.as_array() {
        list.iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect::<Vec<_>>()
    } else if let Some(text) = value.as_str() {
        vec![text.to_string()]
    } else {
        return Err("grants must be an array of tool=decision strings".to_string());
    };
    specs.iter().map(|spec| parse_grant_spec(spec)).collect()
}

pub fn handle_request(op: &str, payload: &Value) -> Value {
    let Some(workspace_raw) = payload.get("workspace").and_then(|v| v.as_str()) else {
        return json!({"ok": false, "error_code": "invalid_argument", "error": "workspace is required"});
    };
    let workspace = match canonical_workspace(Path::new(workspace_raw)) {
        Ok(path) => path,
        Err(error) => return json!({"ok": false, "error_code": "invalid_argument", "error": error}),
    };
    log::info!("agents_api: op={op} workspace={}", workspace.display());
    match op {
        "create_head" => {
            let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let grants = match payload.get("grants").map(grants_from_value).transpose() {
                Ok(Some(grants)) => grants,
                Ok(None) => Vec::new(),
                Err(error) => {
                    return json!({"ok": false, "error_code": "invalid_argument", "error": error})
                }
            };
            let caller_pane = payload.get("caller_pane").and_then(|v| v.as_u64());
            let caller_head = payload.get("caller_head").and_then(|v| v.as_str()).unwrap_or("");
            let (parent_owned, denial_owned) = if caller_pane.is_some() {
                if caller_head.is_empty() {
                    log::info!("agents_api: create_head pane={caller_pane:?} holds no grants");
                    (Some(Vec::new()), "pane".to_string())
                } else {
                    match find_head(&workspace, caller_head) {
                        Ok(card) => {
                            log::info!(
                                "agents_api: create_head pane={caller_pane:?} parent_head={}",
                                card.id
                            );
                            let actor = actor_of(&card.id);
                            (Some(card.grants), actor)
                        }
                        Err(error) => return error,
                    }
                }
            } else {
                (None, "operator".to_string())
            };
            create_head(CreateHead {
                workspace: &workspace,
                name,
                display_name: payload
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(name),
                description: payload
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                grants: &grants,
                temporary: payload
                    .get("temporary")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                reports_to: payload.get("reports_to").and_then(|v| v.as_str()),
                parent_grants: parent_owned.as_deref(),
                denial_actor: &denial_owned,
            })
        }
        "list_heads" => {
            let include_temporary = payload
                .get("all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match list_head_cards(&workspace, include_temporary) {
                Ok(heads) => {
                    log::info!("agents_api: listed {} head(s)", heads.len());
                    json!({"ok": true, "heads": heads})
                }
                Err(error) => json!({"ok": false, "error_code": "io_error", "error": error}),
            }
        }
        "spawn_run" => {
            let head_id = payload.get("head").and_then(|v| v.as_str()).unwrap_or("");
            let head = match find_head(&workspace, head_id) {
                Ok(head) => head,
                Err(error) => return error,
            };
            let admission = payload
                .get("admission")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("adm_{}", uuid::Uuid::new_v4()));
            let journal_only = payload
                .get("journal_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let prompt_owned = if journal_only {
                None
            } else {
                Some(
                    payload
                        .get("text")
                        .and_then(|v| v.as_str())
                        .filter(|text| !text.is_empty())
                        .unwrap_or("run")
                        .to_string(),
                )
            };
            issue_run(IssueRun {
                workspace: &workspace,
                head: &head,
                parent_run: None,
                admission_id: &admission,
                client_ref: payload
                    .get("client_ref")
                    .and_then(|v| v.as_str())
                    .unwrap_or("internal/unallocated"),
                kind: payload
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("system"),
                input_tokens: payload
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                output_tokens: payload
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                prompt: prompt_owned.as_deref(),
            })
        }
        "list_runs" => match load_runs(&workspace) {
            Ok(runs) => {
                log::info!("agents_api: listed {} run(s)", runs.len());
                json!({"ok": true, "runs": runs})
            }
            Err(error) => json!({"ok": false, "error_code": "io_error", "error": error}),
        },
        "show_run" => {
            let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match find_run(&workspace, id) {
                Ok(run) => json!({"ok": true, "run": run}),
                Err(error) => error,
            }
        }
        "finish_run" => finish_run(
            &workspace,
            payload.get("id").and_then(|v| v.as_str()).unwrap_or(""),
        ),
        "delegate" => delegate(
            &workspace,
            payload
                .get("parent_run")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            payload.get("name").and_then(|v| v.as_str()).unwrap_or(""),
            match payload.get("grants").map(grants_from_value).transpose() {
                Ok(Some(grants)) => grants,
                Ok(None) => Vec::new(),
                Err(error) => {
                    return json!({"ok": false, "error_code": "invalid_argument", "error": error})
                }
            },
        ),
        "read_conversation" => {
            let head = payload.get("head").and_then(|v| v.as_str()).unwrap_or("");
            let actor = payload.get("as_head").and_then(|v| v.as_str());
            log::info!("agents_api: read_conversation head={head} as={actor:?}");
            crate::agent::leads::read_conversation_as(&workspace, actor, head)
        }
        "call" => invoke(
            &workspace,
            payload.get("run_id").and_then(|v| v.as_str()).unwrap_or(""),
            payload.get("tool").and_then(|v| v.as_str()).unwrap_or(""),
            payload
                .get("input_json")
                .and_then(|v| v.as_str())
                .unwrap_or("{}"),
        ),
        other => {
            json!({"ok": false, "error_code": "invalid_argument", "error": format!("unknown agents op {other}")})
        }
    }
}

fn finish_run(workspace: &Path, id: &str) -> Value {
    let _guard = journal_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut runs = match load_runs(workspace) {
        Ok(runs) => runs,
        Err(error) => return json!({"ok": false, "error_code": "io_error", "error": error}),
    };
    let Some(run) = runs.iter_mut().find(|run| run.id == id) else {
        return json!({"ok": false, "error_code": "run_not_found", "error": format!("no run '{id}'")});
    };
    run.active = false;
    run.state = "finished".to_string();
    let finished = run.clone();
    if let Err(error) = append_run_record(workspace, &finished) {
        return json!({"ok": false, "error_code": "io_error", "error": error});
    }
    log::info!("agents_api: finished run {id}");
    json!({"ok": true, "run": finished})
}

fn delegate(workspace: &Path, parent_run: &str, name: &str, grants: Vec<ToolGrant>) -> Value {
    let parent = match find_run(workspace, parent_run) {
        Ok(run) => run,
        Err(error) => return error,
    };
    if !parent.active {
        return json!({"ok": false, "error_code": "permission_denied", "error": "parent run is not active"});
    }
    if let Some(refused) = subset_denied(&parent.actor_id, &parent.grants, &grants) {
        return refused;
    }
    let parent_head = match find_head(workspace, &parent.head_id) {
        Ok(head) => head,
        Err(error) => return error,
    };
    let created = create_head(CreateHead {
        workspace,
        name,
        display_name: name,
        description: "Temporary child. reports_to is not authority.",
        grants: &grants,
        temporary: true,
        reports_to: Some(&parent_head.id),
        parent_grants: Some(&parent.grants),
        denial_actor: &parent.actor_id,
    });
    if created.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        if created.get("error_code").and_then(|v| v.as_str()) == Some("head_exists") {
            let existing = match find_head(workspace, name) {
                Ok(head) => head,
                Err(error) => return error,
            };
            if existing.grants != grants
                || existing.reports_to.as_deref() != Some(parent_head.id.as_str())
            {
                return deny(
                    &parent.actor_id,
                    name,
                    "existing child grants differ; refusing to widen",
                );
            }
        } else {
            return created;
        }
    }
    let child = match find_head(workspace, name) {
        Ok(head) => head,
        Err(error) => return error,
    };
    let admission = format!("delegate:{parent_run}:{name}");
    let spawned = issue_run(IssueRun {
        workspace,
        head: &child,
        parent_run: Some(parent_run),
        admission_id: &admission,
        client_ref: &parent.client_ref,
        kind: &parent.kind,
        input_tokens: 0,
        output_tokens: 0,
        prompt: None,
    });
    if spawned.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        log::info!(
            "agents_api: delegated child={name} parent_run={parent_run} grants={}",
            grants.len()
        );
    }
    spawned
}

fn invoke(workspace: &Path, run_id: &str, tool: &str, input_json: &str) -> Value {
    let run = match find_run(workspace, run_id) {
        Ok(run) => run,
        Err(error) => return error,
    };
    if !run.active {
        return deny(&run.actor_id, tool, "run is not active");
    }
    let Some(grant) = run.grants.iter().find(|grant| grant.tool == tool) else {
        return deny(&run.actor_id, tool, "tool is outside the run's granted set");
    };
    if grant.decision == "deny" {
        return deny(&run.actor_id, tool, "tool is denied for this run");
    }
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    let admission = monitor().admit(AdmitRequest {
        call_id: &call_id,
        tool,
        input_json,
        actor_type: ActorType::Agent,
        actor_id: &run.actor_id,
        actor_scope: ActorScope::Workspace,
        trust_origin: "host",
        workspace_root: workspace,
        context_id: 0,
        package_id: PACKAGE,
        instance_id: 0,
        target_type: TargetType::HostTool,
    });
    match admission {
        Admission::Denied { code } => {
            log::info!(
                "agents_api: gate {code} actor={} tool={tool} call_id={call_id}",
                run.actor_id
            );
            json!({"ok": false, "error_code": code, "error": code, "tool": tool, "actor_id": run.actor_id, "call_id": call_id})
        }
        Admission::Required { pending_request_id } => {
            log::info!(
                "agents_api: pending approval actor={} tool={tool} pending={pending_request_id}",
                run.actor_id
            );
            json!({
                "ok": false,
                "error_code": "permission_required",
                "error": "permission_required",
                "pending_request_id": pending_request_id,
                "tool": tool,
                "actor_id": run.actor_id,
                "call_id": call_id,
            })
        }
        Admission::Proceed {
            grant_id,
            fingerprint,
            resource_id,
            ..
        } => {
            let resource = resource_id.unwrap_or_default();
            if monitor()
                .note_use(
                    &run.actor_id,
                    &call_id,
                    &grant_id,
                    &fingerprint,
                    &resource,
                    "agents-api",
                )
                .is_err()
            {
                return deny(&run.actor_id, tool, "grant was revoked before the call");
            }
            let body = execute_tool(workspace, &run, tool, input_json);
            monitor().consume_once(&grant_id, Some("agents-api"));
            log::info!(
                "agents_api: executed tool={tool} actor={} run={} call_id={call_id}",
                run.actor_id,
                run.id
            );
            body
        }
    }
}

fn execute_tool(workspace: &Path, run: &RunRecord, tool: &str, input_json: &str) -> Value {
    let input: Value = serde_json::from_str(input_json).unwrap_or_else(|_| json!({}));
    match tool {
        "agents.list" => match list_head_cards(workspace, false) {
            Ok(heads) => json!({"ok": true, "heads": heads, "actor_id": run.actor_id}),
            Err(error) => json!({"ok": false, "error_code": "io_error", "error": error}),
        },
        "agents.create" => {
            let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let grants = match input.get("grants").map(grants_from_value).transpose() {
                Ok(Some(grants)) => grants,
                Ok(None) => Vec::new(),
                Err(error) => {
                    return json!({"ok": false, "error_code": "invalid_argument", "error": error})
                }
            };
            create_head(CreateHead {
                workspace,
                name,
                display_name: input
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(name),
                description: input
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                grants: &grants,
                temporary: false,
                reports_to: None,
                parent_grants: Some(&run.grants),
                denial_actor: &run.actor_id,
            })
        }
        "agents.ping" | "agents.review" | "agents.admin" => json!({
            "ok": true,
            "tool": tool,
            "actor_id": run.actor_id,
            "run_id": run.id,
        }),
        other => {
            json!({"ok": false, "error_code": "tool_not_found", "error": format!("no agents handler for {other}")})
        }
    }
}

pub struct McpAuth {
    pub workspace: PathBuf,
    pub run_id: Option<String>,
    pub actor_id: Option<String>,
    pub pane_id: u64,
    pub context_id: u64,
}

/// Host MCP front end. A run token calls as that run. A pane credential is
/// admitted as itself through the same monitor; an ungranted call becomes a
/// pending approval rather than a second permission system.
pub fn mcp_call(auth: &McpAuth, tool: &str, input_json: &str) -> Result<String, String> {
    let value = if let Some(run_id) = &auth.run_id {
        if tool == "agents.delegate" {
            let input: Value = serde_json::from_str(input_json).unwrap_or_else(|_| json!({}));
            let parent = input
                .get("parent_run")
                .and_then(|v| v.as_str())
                .unwrap_or(run_id);
            if parent != run_id {
                deny(
                    auth.actor_id.as_deref().unwrap_or("agent"),
                    tool,
                    "run token does not own parent_run",
                )
            } else {
                let grants =
                    match input.get("grants").map(grants_from_value).transpose() {
                        Ok(Some(grants)) => grants,
                        Ok(None) => Vec::new(),
                        Err(error) => return Err(
                            json!({"ok": false, "error_code": "invalid_argument", "error": error})
                                .to_string(),
                        ),
                    };
                delegate(
                    &auth.workspace,
                    run_id,
                    input.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    grants,
                )
            }
        } else {
            invoke(&auth.workspace, run_id, tool, input_json)
        }
    } else {
        admit_pane(auth, tool, input_json)
    };
    let text = serde_json::to_string(&value).unwrap_or_else(|_| "{\"ok\":false}".to_string());
    if value.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(text)
    } else {
        Err(text)
    }
}

fn admit_pane(auth: &McpAuth, tool: &str, input_json: &str) -> Value {
    let actor = format!("mcp:pane:{}", auth.pane_id);
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    let admission = monitor().admit(AdmitRequest {
        call_id: &call_id,
        tool,
        input_json,
        actor_type: ActorType::Agent,
        actor_id: &actor,
        actor_scope: ActorScope::User,
        trust_origin: "host",
        workspace_root: &auth.workspace,
        context_id: auth.context_id,
        package_id: PACKAGE,
        instance_id: auth.pane_id,
        target_type: TargetType::HostTool,
    });
    match admission {
        Admission::Proceed { .. } => json!({
            "ok": false,
            "error_code": "permission_denied",
            "error": "pane MCP calls do not execute agent tools directly; use a run token",
        }),
        Admission::Required { pending_request_id } => json!({
            "ok": false,
            "error_code": "permission_required",
            "error": "permission_required",
            "pending_request_id": pending_request_id,
            "actor_id": actor,
            "tool": tool,
        }),
        Admission::Denied { code } => {
            json!({"ok": false, "error_code": code, "error": code, "actor_id": actor, "tool": tool})
        }
    }
}

pub fn mcp_tool_defs() -> Vec<Value> {
    ["agents.list", "agents.create", "agents.delegate", "agents.ping", "agents.review", "agents.admin"]
        .into_iter()
        .map(|name| {
            json!({
                "name": name,
                "description": "Agents API. Skills and AGENT.md grant nothing; the permission gate admits the call.",
                "inputSchema": {"type": "object", "additionalProperties": true},
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::gate::ApprovalChoice;

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
                workspace: workspace,
            }
        }

        fn ws(&self) -> &Path {
            self.workspace.path()
        }
    }

    fn grants(specs: &[&str]) -> Vec<ToolGrant> {
        specs
            .iter()
            .map(|spec| parse_grant_spec(spec).unwrap())
            .collect()
    }

    fn create(fix: &Fixture, name: &str, specs: &[&str]) -> Value {
        create_head(CreateHead {
            workspace: fix.ws(),
            name,
            display_name: name,
            description: "test",
            grants: &grants(specs),
            temporary: false,
            reports_to: None,
            parent_grants: None,
            denial_actor: "operator",
        })
    }

    #[test]
    fn head_lands_under_neutral_agents_dir_and_lists() {
        let fix = Fixture::new();
        let created = create(&fix, "lead", &["agents.ping=allow", "agents.review=ask"]);
        assert_eq!(created["ok"], true, "{created}");
        let dir = fix.ws().join(".plexi").join("agents").join("lead");
        assert!(dir.join("AGENT.md").is_file());
        assert!(dir.join("settings.toml").is_file());
        assert!(!fix.ws().join(".plexi-alpha").join("agents").exists());
        let listed = handle_request("list_heads", &json!({"workspace": fix.ws()}));
        assert_eq!(listed["heads"][0]["id"], "lead");
        assert_eq!(listed["heads"][0]["grants"][0]["tool"], "agents.ping");
    }

    #[test]
    fn spawn_claims_once_and_writes_tagged_ledger_row() {
        let fix = Fixture::new();
        create(&fix, "lead", &["agents.ping=allow"]);
        let payload = json!({
            "workspace": fix.ws(),
            "head": "lead",
            "admission": "adm-1",
            "client_ref": "acme",
            "kind": "output",
            "input_tokens": 11,
            "output_tokens": 4,
        });
        let spawned = handle_request("spawn_run", &payload);
        assert_eq!(spawned["ok"], true, "{spawned}");
        assert_eq!(spawned["ledger"]["client_ref"], "acme");
        assert_eq!(spawned["ledger"]["kind"], "output");
        assert_eq!(spawned["ledger"]["input_tokens"], 11);
        assert_eq!(spawned["ledger"]["output_tokens"], 4);
        let again = handle_request("spawn_run", &payload);
        assert_eq!(again["idempotent"], true);
        assert_eq!(again["run"]["id"], spawned["run"]["id"]);
        let conflict = handle_request(
            "spawn_run",
            &json!({
                "workspace": fix.ws(),
                "head": "lead",
                "admission": "adm-2",
            }),
        );
        assert_eq!(conflict["error_code"], "assignment_conflict");
        let ledger =
            fs::read_to_string(crate::config::config_dir().join("ai-ledger.jsonl")).unwrap();
        assert_eq!(ledger.lines().count(), 1, "{ledger}");
        assert!(ledger.contains("\"run_id\""));
        assert!(ledger.contains("acme"));
        assert!(ledger.contains("\"input_tokens\":11"));
    }

    #[test]
    fn delegation_refuses_a_grant_the_parent_lacks_and_enforces_the_rest() {
        let fix = Fixture::new();
        create(
            &fix,
            "lead",
            &[
                "agents.ping=allow",
                "agents.review=ask",
                "agents.list=allow",
                "agents.create=allow",
            ],
        );
        let spawned = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "head": "lead", "admission": "adm-1"}),
        );
        let parent = spawned["run"]["id"].as_str().unwrap();
        let widened = handle_request(
            "delegate",
            &json!({
                "workspace": fix.ws(),
                "parent_run": parent,
                "name": "rogue",
                "grants": ["agents.admin=allow"],
            }),
        );
        assert_eq!(widened["error_code"], "permission_denied", "{widened}");
        assert!(!fix
            .ws()
            .join(".plexi")
            .join("agents")
            .join("rogue")
            .exists());
        let audit = monitor().audit_records();
        assert!(
            audit
                .iter()
                .any(|row| row.decision == "deny" && row.resource_id == "agents.admin"),
            "{audit:?}"
        );

        let child = handle_request(
            "delegate",
            &json!({
                "workspace": fix.ws(),
                "parent_run": parent,
                "name": "scout",
                "grants": ["agents.ping=allow", "agents.review=ask"],
            }),
        );
        assert_eq!(child["ok"], true, "{child}");
        let child_run = child["run"]["id"].as_str().unwrap();
        let card = read_head(&fix.ws().join(".plexi").join("agents").join("scout")).unwrap();
        assert!(card.temporary);
        assert_eq!(card.reports_to.as_deref(), Some("lead"));
        assert!(!card.grants.iter().any(|grant| grant.tool == "agents.admin"));

        let ping = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": child_run,
                "tool": "agents.ping",
                "input_json": "{}",
            }),
        );
        assert_eq!(ping["ok"], true, "{ping}");
        assert_eq!(ping["actor_id"], "agent:scout");

        let admin = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": child_run,
                "tool": "agents.admin",
                "input_json": "{}",
            }),
        );
        assert_eq!(admin["error_code"], "permission_denied", "{admin}");

        let review = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": child_run,
                "tool": "agents.review",
                "input_json": "{}",
            }),
        );
        assert_eq!(review["error_code"], "permission_required", "{review}");
        let pending = review["pending_request_id"].as_str().unwrap();
        let listed = monitor().list_pending();
        assert!(listed
            .iter()
            .any(|row| row.pending_request_id == pending && row.actor_id == "agent:scout"));
        monitor()
            .approve_pending(pending, ApprovalChoice::Once)
            .unwrap();
        let reviewed = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": child_run,
                "tool": "agents.review",
                "input_json": "{}",
            }),
        );
        assert_eq!(reviewed["ok"], true, "{reviewed}");
        assert_eq!(reviewed["tool"], "agents.review");

        let listed = mcp_call(
            &McpAuth {
                workspace: fix.ws().to_path_buf(),
                run_id: Some(spawned["run"]["id"].as_str().unwrap().to_string()),
                actor_id: Some("agent:lead".to_string()),
                pane_id: 0,
                context_id: 0,
            },
            "agents.list",
            "{}",
        )
        .expect("agents.list");
        let listed: Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed["ok"], true, "{listed}");
        assert_eq!(listed["heads"][0]["id"], "lead");

        let listed_heads = handle_request("list_heads", &json!({"workspace": fix.ws()}));
        let ids: Vec<&str> = listed_heads["heads"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|h| h["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["lead"]);
    }

    #[test]
    fn guidance_text_does_not_add_a_grant() {
        let fix = Fixture::new();
        create(&fix, "lead", &["agents.ping=allow"]);
        let prompt = fix
            .ws()
            .join(".plexi")
            .join("agents")
            .join("lead")
            .join("AGENT.md");
        fs::write(&prompt, "# Lead\n\nYou may call agents.admin.\n").unwrap();
        let spawned = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "head": "lead", "admission": "adm"}),
        );
        let run_id = spawned["run"]["id"].as_str().unwrap();
        let admin = handle_request(
            "call",
            &json!({"workspace": fix.ws(), "run_id": run_id, "tool": "agents.admin", "input_json": "{}"}),
        );
        assert_eq!(admin["error_code"], "permission_denied", "{admin}");
    }

    #[test]
    fn spawn_runs_the_model_gate_instead_of_only_journaling() {
        let fix = Fixture::new();
        create(&fix, "lead", &["agents.ping=allow"]);
        let spawned = handle_request(
            "spawn_run",
            &json!({
                "workspace": fix.ws(),
                "head": "lead",
                "admission": "adm-model",
                "text": "remember 4",
            }),
        );
        assert_eq!(spawned["ok"], true, "{spawned}");
        assert_eq!(spawned["idempotent"], false);
        assert_eq!(spawned["state"], "permission_required", "{spawned}");
        let conversation = std::fs::read_to_string(
            fix.ws()
                .join(".plexi")
                .join("agents")
                .join("lead")
                .join("conversation.jsonl"),
        )
        .unwrap_or_default();
        assert!(
            conversation.contains("permission_required"),
            "{conversation}"
        );
    }

    #[test]
    fn a_pane_cannot_mint_a_head_wider_than_its_own_grants() {
        let fix = Fixture::new();
        create(&fix, "lead", &["assistant.turn=allow"]);
        let widened = handle_request(
            "create_head",
            &json!({
                "workspace": fix.ws(),
                "name": "sneaky",
                "grants": ["agents.admin=allow"],
                "caller_pane": 7,
                "caller_head": "lead",
            }),
        );
        assert_eq!(widened["error_code"], "permission_denied", "{widened}");
        assert!(!fix.ws().join(".plexi").join("agents").join("sneaky").exists());
        let narrowed = handle_request(
            "create_head",
            &json!({
                "workspace": fix.ws(),
                "name": "scout",
                "grants": ["assistant.turn=allow"],
                "caller_pane": 7,
                "caller_head": "lead",
            }),
        );
        assert_eq!(narrowed["ok"], true, "{narrowed}");
        let empty = handle_request(
            "create_head",
            &json!({
                "workspace": fix.ws(),
                "name": "bare",
                "grants": ["assistant.turn=allow"],
                "caller_pane": 9,
                "caller_head": "",
            }),
        );
        assert_eq!(empty["error_code"], "permission_denied", "{empty}");
    }
}
