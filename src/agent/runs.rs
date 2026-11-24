//! Minimal Agents API.
//!
//! One host service, used by the CLI and by the host MCP endpoint. Agent
//! authority lives under `<workspace>/.plexi/agents/`. `AGENT.md` is
//! guidance and grants nothing. Authority is a `GrantRecord` in the permission
//! monitor: an allow is a tool-scoped grant (`args_unbound`), an ask is the
//! ordinary pending-approval path, and a tool outside that set is a denial.
//! Delegation copies a subset of the parent run's grants onto a temporary
//! child. A child that asks for a tool the parent does not hold is refused
//! and audited. Reporting metadata (`reports_to`) grants no rights.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::broker::gate::{
    Admission, AdmitRequest, NeedsYouDetails, NeedsYouFile, NeedsYouKind, PermissionMonitor,
};
use crate::broker::permit::{self, Finish, Invoke, PermitSubject, Reissue, RunBinding, RunPermit, StartPermit};
use crate::broker::principal::Principal;
use crate::broker::{
    ActorScope, ActorType, Decision, GrantDuration, GrantKind, GrantRecord, GrantSource, TargetType,
};
use crate::plexi_ai::ledger::{self, LedgerRow};

const PACKAGE: &str = "agents";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ToolGrant {
    pub tool: String,
    /// The one decision type: an unknown value fails the load (stint 1285).
    pub decision: Decision,
    #[serde(default)]
    pub kind: GrantKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AgentAuthorityRecord {
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
    #[serde(alias = "head_id")]
    agent_id: String,
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
    /// Host-key MAC naming this run's owner (stint 1716). Written in the
    /// same line as the run. Absent only on runs that predate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_seal: Option<String>,
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

/// The bearer for a run, stable while this host lives. Only a sink holding
/// a reissue or start permit reaches it.
fn run_token(workspace: &Path, run_id: &str, actor_id: &str) -> String {
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

fn permit_refused(error: impl std::fmt::Display) -> Value {
    log::info!("agents_api: permit refused: {error}");
    json!({"ok": false, "error_code": "permission_denied", "error": error.to_string()})
}

/// The runs in `workspace` that predate sealed owners, captured once before
/// the first sealed run is written there.
fn legacy_run_ids(gate: &PermissionMonitor, workspace: &Path) -> Result<BTreeSet<String>, Value> {
    gate.legacy_runs(workspace, || {
        Ok(load_runs(workspace)?
            .into_iter()
            .filter(|run| run.owner_seal.is_none())
            .map(|run| run.id)
            .collect())
    })
    .map_err(permit_refused)
}

fn binding_of<'a>(workspace: &'a Path, run: &'a RunRecord) -> RunBinding<'a> {
    RunBinding {
        workspace,
        run_id: &run.id,
        actor_id: &run.actor_id,
        admission_id: &run.admission_id,
    }
}

/// The broker's permit for `Op` on this run, for this subject.
fn permit_for<Op>(
    workspace: &Path,
    run: &RunRecord,
    subject: &PermitSubject,
) -> Result<RunPermit<Op>, Value> {
    let gate = monitor()?;
    let legacy = legacy_run_ids(&gate, workspace)?;
    permit::mint_run(
        &gate,
        subject,
        binding_of(workspace, run),
        run.owner_seal.as_deref(),
        &legacy,
    )
    .map_err(permit_refused)
}

/// Test only: this run's in-memory bearer is gone, as after a host restart.
#[cfg(test)]
fn forget_run_for_test(run_id: &str) {
    if let Some(token) = token_by_run()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(run_id)
    {
        tokens().lock().unwrap_or_else(|e| e.into_inner()).remove(&token);
    }
}

fn actor_of(agent_id: &str) -> String {
    format!("agent:{agent_id}")
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

/// A grant names exactly one tool: no wildcard, path or spaces. Applies to
/// specs and to every card loaded from disk.
fn parse_grant_tool(tool: &str) -> Result<&str, String> {
    if tool.is_empty() || tool == "*" || tool.contains('/') || tool.contains(' ') {
        return Err(format!("grant tool '{tool}' is not a tool name"));
    }
    Ok(tool)
}

fn parse_grant_spec(spec: &str) -> Result<ToolGrant, String> {
    let (tool, raw_decision) = spec
        .split_once('=')
        .ok_or_else(|| format!("grant '{spec}' must be tool=allow[:hard|:soft]|ask|deny"))?;
    let tool = parse_grant_tool(tool.trim())?;
    let raw_decision = raw_decision.trim();
    let (decision, kind) = match raw_decision {
        "allow" | "allow:hard" => (Decision::Allow, GrantKind::Hard),
        "allow:soft" => (Decision::Allow, GrantKind::Soft),
        "ask" => (Decision::Ask, GrantKind::Hard),
        "deny" => (Decision::Deny, GrantKind::Hard),
        _ => {
            return Err(format!(
                "grant decision '{raw_decision}' must be allow, allow:hard, allow:soft, ask, or deny"
            ));
        }
    };
    Ok(ToolGrant {
        tool: tool.to_string(),
        decision,
        kind,
        expires_at: None,
    })
}

/// The parent ceiling row that covers `requested`, if any: the same tool
/// and at least as permissive. There is no wildcard tool.
fn covering<'a>(parent: &'a [CeilingRow], requested: &ToolGrant) -> Option<&'a CeilingRow> {
    parent.iter().find(|row| {
        row.tool == requested.tool
            && requested.decision.permissiveness() <= row.decision.permissiveness()
    })
}

/// One sealed grant-store row a caller holds, as a delegation ceiling
/// (stint 1285): a caller can delegate what it holds, for no longer than it
/// holds it, and never a one-use grant (a copy would be a second use).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CeilingRow {
    pub(crate) tool: String,
    pub(crate) decision: Decision,
    pub(crate) duration: GrantDuration,
    /// When the delegator's row stops working; a delegated row stops then
    /// too (Codex 3290 r1).
    pub(crate) expires_at: Option<i64>,
    /// The run the delegator's row is bound to; a delegated row is bound to
    /// it too, so finishing that run revokes both.
    pub(crate) bound_run: Option<String>,
    /// The grant-store rows behind this ceiling row: one for a held row,
    /// both sides for an intersection. Host-supplied rows have none.
    pub(crate) grant_ids: Vec<String>,
}

/// The earlier of two expiries, `None` meaning never.
fn earlier(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn same_grant_specs(a: &[ToolGrant], b: &[ToolGrant]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.tool == y.tool && x.decision == y.decision && x.kind == y.kind)
}

/// The profile monitor, or the API refusal a non-host process returns
/// (stint 1291).
fn monitor() -> Result<std::sync::Arc<PermissionMonitor>, Value> {
    PermissionMonitor::for_profile(&crate::config::config_dir()).map_err(|error| {
        log::info!("agents_api: refused outside the host process: {error}");
        json!({"ok": false, "error_code": "not_host", "error": error.to_string()})
    })
}

/// `monitor()` or return early: the refusal for API calls, else `$fallback`.
macro_rules! host_gate {
    () => {
        match monitor() {
            Ok(gate) => gate,
            Err(refusal) => return refusal,
        }
    };
    ($fallback:expr) => {
        match monitor() {
            Ok(gate) => gate,
            Err(_) => return $fallback,
        }
    };
}

/// Record a delegated allow. The row's source is `Delegated`, never the
/// person's, and its duration is the delegator's (stint 1285).
fn record_scope_allow(
    workspace: &Path,
    actor_id: &str,
    tool: &str,
    kind: GrantKind,
    expires_at: Option<i64>,
    duration: GrantDuration,
    bound_run: Option<&str>,
    justified_by: &[String],
) {
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
    let Ok(gate) = monitor() else {
        return;
    };
    let now = gate.now_secs();
    let grant_id = format!("grant_{}", uuid::Uuid::new_v4());
    let mut record = GrantRecord::from_binding(
        &binding,
        Decision::Allow,
        duration,
        GrantSource::Delegated,
        &grant_id,
    );
    record.created_at = now;
    record.args_unbound = true;
    if duration == GrantDuration::Session {
        record.session_id = Some(gate.session_id().to_string());
    }
    record.kind = kind;
    record.expires_at = expires_at;
    record.bound_operation_id = bound_run.map(str::to_string);
    record.justified_by = justified_by.to_vec();
    {
        let mut store = gate.store();
        store.records_mut().retain(|existing| {
            !(existing.actor_type == ActorType::Agent
                && existing.actor_id == actor_id
                && existing.workspace_root.as_deref() == Some(workspace)
                && existing.target_id == tool
                && existing.bound_operation_id.is_none())
        });
        store.record(record);
        store.save();
    }
    log::info!(
        "agents_api: scope allow actor={actor_id} tool={tool} kind={} duration={duration:?} source=delegated expires_at={expires_at:?}",
        kind.as_str()
    );
}

/// Sync `agent.json` and any active run in `runs.jsonl` when a soft grant expires.
/// Writes only the authority record and the run journal; the user-owned
/// definition files (`AGENT.md`, `settings.toml`) are never touched.
pub fn expire_soft_grant_in_workspace(
    workspace: &Path,
    actor_id: &str,
    tool: &str,
) -> Result<(), String> {
    let agent_id = actor_id.strip_prefix("agent:").unwrap_or(actor_id);
    let agent_dir = agents_root(workspace).join(agent_id);
    if crate::agent::authority_file_migrating_legacy(&agent_dir).is_some() {
        let mut card = read_agent(&agent_dir)?;
        if expire_soft_allow(&mut card.grants, tool) {
            write_authority(&agent_dir, &card)?;
            log::info!(
                "agents_api: expired soft grant agent={} tool={tool} -> ask (agent.json only)",
                card.id
            );
        }
    }
    let _guard = journal_lock().lock().unwrap_or_else(|e| e.into_inner());
    for mut run in load_runs(workspace)? {
        if !run.active || (run.actor_id != actor_id && run.agent_id != agent_id) {
            continue;
        }
        if expire_soft_allow(&mut run.grants, tool) {
            append_run_record(workspace, &run)
                .map_err(|error| format!("expire soft grant on run {}: {error}", run.id))?;
        }
    }
    Ok(())
}

/// Flip every soft `allow` grant for `tool` to `ask`; true when any changed.
fn expire_soft_allow(grants: &mut [ToolGrant], tool: &str) -> bool {
    let mut changed = false;
    for grant in grants {
        if grant.tool == tool && grant.kind == GrantKind::Soft && grant.decision == Decision::Allow {
            grant.decision = Decision::Ask;
            grant.expires_at = None;
            changed = true;
        }
    }
    changed
}

/// Create a new agent definition: `AGENT.md`, `settings.toml` and `agent.json`.
/// Creation only. `create_dir` refuses an existing directory, so a user's
/// definition files are never replaced; later authority changes go through
/// `write_authority`.
fn create_definition(workspace: &Path, card: &AgentAuthorityRecord) -> Result<(), String> {
    let root = agents_root(workspace);
    fs::create_dir_all(&root).map_err(|error| format!("create {}: {error}", root.display()))?;
    let dir = root.join(&card.id);
    fs::create_dir(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
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
    write_authority(&dir, card)
}

/// Write the authority record `agent.json` in an existing agent directory.
fn write_authority(dir: &Path, card: &AgentAuthorityRecord) -> Result<(), String> {
    let path = dir.join("agent.json");
    let result = serde_json::to_string_pretty(card)
        .map_err(|error| format!("serialize authority for {}: {error}", card.id))
        .and_then(|body| {
            fs::write(&path, body).map_err(|error| format!("write {}: {error}", path.display()))
        });
    if let Err(error) = &result {
        log::error!(
            "agents_api: write_authority failed agent={}: {error}",
            card.id
        );
    }
    result
}

fn read_agent(dir: &Path) -> Result<AgentAuthorityRecord, String> {
    let path = crate::agent::authority_file_migrating_legacy(dir)
        .ok_or_else(|| format!("no agent.json in {}", dir.display()))?;
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let record: AgentAuthorityRecord = serde_json::from_str(&raw)
        .map_err(|error| format!("agent.json in {}: {error}", dir.display()))?;
    if let Some(grant) = record.grants.iter().find(|grant| parse_grant_tool(&grant.tool).is_err()) {
        return Err(format!(
            "agent.json in {}: grant tool '{}' is not a tool name",
            dir.display(),
            grant.tool
        ));
    }
    Ok(record)
}

fn list_agent_authorities(workspace: &Path, include_temporary: bool) -> Result<Vec<AgentAuthorityRecord>, String> {
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
        if !path.is_dir() || crate::agent::authority_file_migrating_legacy(&path).is_none() {
            continue;
        }
        let card = read_agent(&path)?;
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

struct RefusalContext<'a> {
    workspace: &'a Path,
    actor: &'a str,
    tool: &'a str,
    reason: &'a str,
    parent_run: Option<&'a str>,
    run_id: Option<&'a str>,
    caller_pane: Option<u64>,
}

fn deny_with_request(ctx: RefusalContext<'_>) -> Value {
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    let gate = host_gate!();
    gate.note_denial(ctx.actor, &call_id, ctx.tool, "agents-api", "deny");
    let effective_parent_run = ctx.parent_run.or(ctx.run_id).map(str::to_string);
    let effective_run_id = ctx.run_id.or(ctx.parent_run).map(str::to_string);
    let needs_you_id = gate
        .file_needs_you_with_details(
            NeedsYouFile {
                kind: NeedsYouKind::PermissionRequest,
                actor: ctx.actor.to_string(),
                resource: ctx.tool.to_string(),
                summary: ctx.reason.to_string(),
                expires_at: None,
                run_tag: effective_parent_run.clone(),
            },
            Some(NeedsYouDetails {
                pane_id: ctx.caller_pane,
                tool: Some(ctx.tool.to_string()),
                parent_run: effective_parent_run.clone(),
                run_id: effective_run_id,
                workspace: Some(ctx.workspace.display().to_string()),
                correlation_id: effective_parent_run,
                ..Default::default()
            }),
        )
        .ok();
    log::info!(
        "agents_api: denied actor={} tool={} reason={} needs_you={needs_you_id:?}",
        ctx.actor,
        ctx.tool,
        ctx.reason
    );
    json!({
        "ok": false,
        "error_code": "permission_denied",
        "error": ctx.reason,
        "tool": ctx.tool,
        "actor_id": ctx.actor,
        "call_id": call_id,
        "needs_you_id": needs_you_id,
    })
}

fn subset_denied(
    workspace: &Path,
    actor: &str,
    parent: &[CeilingRow],
    requested: &[ToolGrant],
    parent_run: Option<&str>,
    caller_pane: Option<u64>,
) -> Option<Value> {
    for grant in requested {
        // Only an allow carries authority: `ask` still reaches the person on
        // every call and `deny` narrows, so neither needs a parent row.
        if grant.decision == Decision::Allow && covering(parent, grant).is_none() {
            return Some(deny_with_request(RefusalContext {
                workspace,
                actor,
                tool: &grant.tool,
                reason: &format!(
                    "delegation refused: '{grant_decision}' on '{tool}' is not a subset of the parent grants",
                    grant_decision = grant.decision.as_str(),
                    tool = grant.tool,
                ),
                parent_run,
                run_id: parent_run,
                caller_pane,
            }));
        }
    }
    None
}

struct CreateAgent<'a> {
    workspace: &'a Path,
    name: &'a str,
    display_name: &'a str,
    description: &'a str,
    grants: &'a [ToolGrant],
    temporary: bool,
    reports_to: Option<&'a str>,
    /// The caller's delegation ceiling. Every create has one.
    parent_grants: &'a [CeilingRow],
    denial_actor: &'a str,
    parent_run: Option<&'a str>,
    caller_pane: Option<u64>,
}

fn create_agent(req: CreateAgent<'_>) -> Value {
    if !valid_name(req.name) {
        return json!({"ok": false, "error_code": "invalid_name", "error": "agent name must be a lowercase slug"});
    }
    let gate = host_gate!();
    gate.expire_soft_grants();
    if let Some(refused) = subset_denied(
        req.workspace,
        req.denial_actor,
        req.parent_grants,
        req.grants,
        req.parent_run,
        req.caller_pane,
    ) {
        return refused;
    }
    let dir = agents_root(req.workspace).join(req.name);
    if crate::agent::authority_file_migrating_legacy(&dir).is_some() {
        return json!({"ok": false, "error_code": "agent_exists", "error": format!("agent '{}' already exists", req.name)});
    }
    // A definition without an authority record (a seeded `default`, a
    // `plexi agent create` folder) is adopted: only `agent.json` is written.
    let adopt = dir.exists();
    if let Some(issue) = crate::agent::unrecognised_agent_dir(&dir) {
        return json!({
            "ok": false, "error_code": "agent_unrecognised",
            "error": format!("agent directory {} is unrecognised: {}", dir.display(), issue.issue.describe()),
        });
    }
    let now = gate.now_secs();
    let soft_days = crate::config::PlexiConfig::load_with_workspace(Some(req.workspace))
        .soft_grant_days() as i64;
    let resolved_grants: Vec<ToolGrant> = req
        .grants
        .iter()
        .map(|g| {
            let expires_at = if g.decision == Decision::Allow && g.kind == GrantKind::Soft {
                Some(now.saturating_add(soft_days.saturating_mul(86_400)))
            } else {
                None
            };
            ToolGrant {
                tool: g.tool.clone(),
                decision: g.decision,
                kind: g.kind,
                expires_at,
            }
        })
        .collect();
    let card = AgentAuthorityRecord {
        id: req.name.to_string(),
        display_name: if req.display_name.is_empty() {
            req.name.to_string()
        } else {
            req.display_name.to_string()
        },
        description: req.description.to_string(),
        reports_to: req.reports_to.map(str::to_string),
        temporary: req.temporary,
        grants: resolved_grants,
    };
    let written = if adopt {
        write_authority(&dir, &card)
    } else {
        create_definition(req.workspace, &card)
    };
    if let Err(error) = written {
        log::error!("agents_api: create agent {} failed: {error}", req.name);
        return json!({"ok": false, "error_code": "io_error", "error": error});
    }
    let actor = actor_of(req.name);
    for grant in &card.grants {
        if grant.decision != Decision::Allow {
            continue;
        }
        // No longer than the caller holds it (stint 1285): its duration,
        // its expiry and its run binding carry to the delegated row.
        let parent = covering(req.parent_grants, grant);
        let duration = parent.map_or(GrantDuration::Session, |row| row.duration);
        record_scope_allow(
            req.workspace,
            &actor,
            &grant.tool,
            grant.kind,
            earlier(grant.expires_at, parent.and_then(|row| row.expires_at)),
            duration,
            parent.and_then(|row| row.bound_run.as_deref()),
            parent.map_or(&[][..], |row| &row.grant_ids),
        );
    }
    log::info!(
        "agents_api: created agent {} actor={actor} adopted_definition={adopt} temporary={} grants={} workspace={}",
        card.id,
        card.temporary,
        card.grants.len(),
        req.workspace.display(),
    );
    json!({"ok": true, "agent": card})
}

struct IssueRun<'a> {
    workspace: &'a Path,
    /// Who is asking. A retry of a charged admission is admitted for it.
    subject: &'a PermitSubject,
    /// The broker's permit for a new run; `None` when only a retry can apply.
    start: Option<&'a StartPermit>,
    agent: &'a AgentAuthorityRecord,
    parent_run: Option<&'a str>,
    admission_id: &'a str,
    client_ref: &'a str,
    kind: &'a str,
    input_tokens: u32,
    output_tokens: u32,
}

/// The step that frees an agent held by a stuck run (stint 1502): the
/// dashboard cancel finishes the run on the host.
fn clear_run_step(run_id: &str) -> String {
    format!("stop it with `plexi agents-dashboard cancel {run_id}`")
}

fn issue_run(req: IssueRun<'_>) -> Value {
    let IssueRun {
        workspace,
        subject,
        start,
        agent,
        parent_run,
        admission_id,
        client_ref,
        kind,
        input_tokens,
        output_tokens,
    } = req;
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
        .find(|run| run.agent_id == agent.id && run.admission_id == admission_id)
    {
        // A retry is only the same success if the first attempt was charged
        // (stint 1441): a run with no ledger row never returns ok.
        match ledger::has_agent_run_row(&existing.id) {
            Ok(true) => {}
            Ok(false) => {
                log::error!(
                    "agents_api: admission {admission_id} found run {} with no ledger row; refusing the retry",
                    existing.id
                );
                return json!({
                    "ok": false,
                    "error_code": "ledger_missing",
                    "error": format!(
                        "run {} for admission {admission_id} has no ledger row, so this admission cannot be retried; {}, then spawn with a new admission id",
                        existing.id,
                        clear_run_step(&existing.id),
                    ),
                    "run": existing,
                });
            }
            Err(error) => {
                log::error!(
                    "agents_api: ledger unreadable for retry of {}: {error}",
                    existing.id
                );
                return json!({"ok": false, "error_code": "ledger_error", "error": error});
            }
        }
        let permit: RunPermit<Reissue> = match permit_for(workspace, existing, subject) {
            Ok(permit) => permit,
            Err(refused) => return refused,
        };
        let token = run_token(permit.workspace(), permit.run_id(), &existing.actor_id);
        log::info!(
            "agents_api: admission {} returned existing run {} for agent {}",
            admission_id,
            existing.id,
            agent.id,
        );
        return json!({
            "ok": true,
            "run": existing,
            "run_token": token,
            "idempotent": true,
        });
    }
    if let Some(active) = runs.iter().find(|run| run.agent_id == agent.id && run.active) {
        log::info!(
            "agents_api: assignment_conflict agent={} active_run={} admission={admission_id}",
            agent.id,
            active.id,
        );
        return json!({
            "ok": false,
            "error_code": "assignment_conflict",
            "error": format!(
                "agent {} already has active run {}; if that run is no longer working, {}",
                agent.id,
                active.id,
                clear_run_step(&active.id),
            ),
            "active_run": active.id,
            "admission_id": admission_id,
        });
    }
    let Some(start) = start else {
        return permit_refused(format!("no start permit for agent {}", agent.id));
    };
    if let Err(error) = start.check(workspace, &agent.id, admission_id) {
        return permit_refused(error);
    }
    let version = runs
        .iter()
        .filter(|run| run.agent_id == agent.id)
        .map(|run| run.version)
        .max()
        .unwrap_or(0)
        + 1;
    let mut record = RunRecord {
        id: format!("run_{}", uuid::Uuid::new_v4()),
        agent_id: agent.id.clone(),
        actor_id: actor_of(&agent.id),
        parent_run: parent_run.map(str::to_string),
        admission_id: admission_id.to_string(),
        version,
        state: "running".to_string(),
        active: true,
        client_ref: client_ref.to_string(),
        kind: kind.to_string(),
        input_tokens,
        output_tokens,
        grants: agent.grants.clone(),
        owner_seal: None,
    };
    // Sealed before anything is charged: a run that cannot name its owner
    // is never started.
    let gate = host_gate!();
    let seal = match permit::seal_run_owner(&gate, start, binding_of(workspace, &record)) {
        Ok(seal) => seal,
        Err(error) => {
            log::error!("agents_api: could not seal owner of {}: {error}", record.id);
            return json!({"ok": false, "error_code": "owner_seal_failed", "error": error});
        }
    };
    record.owner_seal = Some(seal);
    log::info!("agents_api: run={} owner={}", record.id, start.owner_id());
    // The ledger row is written before the run record (stint 1441): a ledger
    // that refuses the row leaves no run behind, so nothing is active, and a
    // retry cannot find a run that was never charged.
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
        return json!({"ok": false, "error_code": "ledger_error", "error": error});
    }
    if let Err(error) = append_run_record(workspace, &record) {
        // Charged but not started: the ledger row stands (fail closed) and the
        // run is not recorded, so a retry starts a new run.
        log::error!(
            "agents_api: append run failed after its ledger row for {}: {error}",
            record.id
        );
        return json!({"ok": false, "error_code": "io_error", "error": error});
    }
    let token = run_token(workspace, &record.id, &record.actor_id);
    log::info!(
        "agents_api: spawned run {} agent={} client_ref={} kind={} tokens={}/{} version={}",
        record.id,
        record.agent_id,
        record.client_ref,
        record.kind,
        record.input_tokens,
        record.output_tokens,
        record.version,
    );
    drop(guard);
    let body = json!({
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
    crate::host::command_view::publish(&workspace_buf, "run updated");
    body
}

fn find_agent(workspace: &Path, id: &str) -> Result<AgentAuthorityRecord, Value> {
    let dir = agents_root(workspace).join(id);
    let definition = match crate::cli::agent::resolve_agent_definition_in(
        id, &crate::config::config_dir(), Some(workspace),
    ) {
        Ok((definition, _)) => definition,
        Err(error) => {
            return Err(match crate::agent::unrecognised_agent_dir(&dir) {
                Some(issue) => json!({
                    "ok": false, "error_code": "agent_unrecognised",
                    "error": format!("agent directory {} is unrecognised: {}", dir.display(), issue.issue.describe()),
                }),
                None => json!({"ok": false, "error_code": "agent_not_found", "error": if dir.exists() { error } else { format!("no agent '{id}'") }}),
            });
        }
    };
    if crate::agent::authority_file_migrating_legacy(&dir).is_some() {
        let authority = read_agent(&dir)
            .map_err(|error| json!({"ok": false, "error_code": "io_error", "error": error}))?;
        return Ok(authority);
    }
    log::info!("agents_api: resolved identity {} with no local authority", definition.id);
    Ok(AgentAuthorityRecord {
        id: definition.id,
        display_name: definition.display_name,
        description: definition.description,
        reports_to: None,
        temporary: false,
        grants: Vec::new(),
    })
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

/// The sealed grant-store rows `principal` holds in `workspace`, as a
/// delegation ceiling (stint 1285): live (not consumed, not expired, this
/// session's session rows), never one-use, never read from a card file.
/// A pane principal also holds rows bound to its pane instance. A row bound
/// to a run counts only for `run`, and for a pane only if that pane asked.
fn sealed_ceiling(
    workspace: &Path,
    principal: &Principal,
    run: Option<&str>,
) -> Vec<CeilingRow> {
    grant_rows(workspace, principal, run, false)
}

/// What a run of `principal` could execute (stint 1716): the delegable
/// rows plus its unused one-use rows. A launch is checked against this, so
/// a one-use row is never executed by a run its starter could not start.
fn executable_rows(workspace: &Path, principal: &Principal) -> Vec<CeilingRow> {
    grant_rows(workspace, principal, None, true)
}

fn grant_rows(
    workspace: &Path,
    principal: &Principal,
    run: Option<&str>,
    once: bool,
) -> Vec<CeilingRow> {
    let gate = host_gate!(Vec::new());
    let session_id = gate.session_id().to_string();
    let now = gate.now_secs();
    let actor_type = principal.actor_type();
    let actor_id = principal.actor_id();
    let pane_instance = actor_id
        .strip_prefix("pane:")
        .and_then(|pane| pane.parse::<u64>().ok())
        .filter(|pane| *pane != 0);
    let store = gate.store();
    store
        .records()
        .iter()
        .filter(|rec| {
            if !store.ancestry_live(rec, now) {
                return false;
            }
            if rec.consumed
                || (!once && rec.duration == GrantDuration::Once)
                || rec.target_id.is_empty()
                || !rec.duration_admissible()
            {
                return false;
            }
            if rec.expires_at.is_some_and(|exp| now >= exp)
                || rec.workspace_root.as_deref() != Some(workspace)
            {
                return false;
            }
            if rec.duration == GrantDuration::Session
                && rec.session_id.as_deref() != Some(session_id.as_str())
            {
                return false;
            }
            if let Some(bound) = rec.bound_run() {
                if Some(bound) != run {
                    return false;
                }
                if pane_instance.is_some() {
                    return rec.run_requester_pane == pane_instance;
                }
            }
            let own = rec.actor_type == actor_type && rec.actor_id == actor_id;
            let pane_bound = pane_instance.is_some_and(|pane| {
                rec.bound_operation_id.as_deref() == Some(actor_id)
                    || (rec.resource_scope == crate::broker::ResourceScope::Pane
                        && rec.resource_id.as_deref() == Some(actor_id))
                    || (rec.instance_id == Some(pane) && rec.actor_type != ActorType::Agent)
            });
            own || pane_bound
        })
        .map(|rec| CeilingRow {
            tool: rec.target_id.clone(),
            decision: rec.decision,
            duration: rec.duration,
            expires_at: rec.expires_at,
            bound_run: rec.bound_run().map(str::to_string),
            grant_ids: vec![rec.grant_id.clone()],
        })
        .collect()
}

/// The rows both ceilings allow: same tool, the less permissive decision,
/// the shorter duration.
fn meet_ceilings(a: &[CeilingRow], b: &[CeilingRow]) -> Vec<CeilingRow> {
    a.iter()
        .filter_map(|left| {
            let right = b.iter().find(|right| right.tool == left.tool)?;
            Some(CeilingRow {
                tool: left.tool.clone(),
                decision: if left.decision.permissiveness() <= right.decision.permissiveness() {
                    left.decision
                } else {
                    right.decision
                },
                duration: if left.duration == GrantDuration::Session
                    || right.duration == GrantDuration::Session
                {
                    GrantDuration::Session
                } else {
                    left.duration
                },
                expires_at: earlier(left.expires_at, right.expires_at),
                bound_run: left.bound_run.clone().or_else(|| right.bound_run.clone()),
                grant_ids: left
                    .grant_ids
                    .iter()
                    .chain(&right.grant_ids)
                    .cloned()
                    .collect(),
            })
        })
        .collect()
}

/// Who calls the agents API, as the host established it (stint 1285). No
/// identity is ever read from the payload.
#[derive(Debug, Clone)]
pub(crate) enum AgentsCaller {
    /// A socket caller with a principal: a pane, or an outside process.
    Principal {
        principal: Principal,
        pane: Option<u64>,
        /// The team member the caller's pane is bound to.
        agent: Option<String>,
        /// The live context root the caller is scoped to; `None` for an
        /// outside process.
        context_root: Option<PathBuf>,
        /// The host found the request outside the caller's context.
        context_denied: bool,
        /// The caller's live pane incarnation, when it has one.
        authority: Option<std::sync::Arc<crate::broker::pane_authority::LivePaneAuthority>>,
    },
    /// Host code acting under its own run principal (team setup, the
    /// dashboard). It brings the ceiling it was admitted with.
    Host { ceiling: Vec<CeilingRow> },
}

impl AgentsCaller {
    /// The host found the request outside the caller's context.
    pub(crate) fn context_denied(&self) -> bool {
        matches!(self, Self::Principal { context_denied: true, .. })
    }

    fn actor(&self) -> String {
        match self {
            Self::Principal { principal, .. } => principal.actor_id().to_string(),
            Self::Host { .. } => "host".to_string(),
        }
    }

    /// This caller as the broker's permit subject.
    fn subject(&self) -> PermitSubject {
        match self {
            Self::Principal {
                principal,
                authority,
                ..
            } => PermitSubject::principal(principal.actor_id(), authority.as_ref()),
            Self::Host { .. } => PermitSubject::Host,
        }
    }

    pub(crate) fn pane(&self) -> Option<u64> {
        match self {
            Self::Principal { pane, .. } => *pane,
            Self::Host { .. } => None,
        }
    }

    /// What this caller can delegate in `workspace`: its own sealed rows,
    /// or, for a bound team member, its agent's sealed rows. `run` admits
    /// the rows a desktop approval bound to that run.
    pub(crate) fn ceiling(&self, workspace: &Path, run: Option<&str>) -> Vec<CeilingRow> {
        match self {
            Self::Principal {
                agent: Some(agent), ..
            } => sealed_ceiling(workspace, &Principal::agent(actor_of(agent)), run),
            Self::Principal { principal, .. } => sealed_ceiling(workspace, principal, run),
            Self::Host { ceiling } => ceiling.clone(),
        }
    }
}

pub(crate) fn handle_request(op: &str, payload: &Value, caller: &AgentsCaller) -> Value {
    let Some(workspace_raw) = payload.get("workspace").and_then(|v| v.as_str()) else {
        return json!({"ok": false, "error_code": "invalid_argument", "error": "workspace is required"});
    };
    let workspace = match canonical_workspace(Path::new(workspace_raw)) {
        Ok(path) => path,
        Err(error) => return json!({"ok": false, "error_code": "invalid_argument", "error": error}),
    };
    host_gate!().expire_soft_grants();
    let caller_pane = caller.pane();
    // Every pane is scoped to its context root, a pane the person opened
    // included; an outside process to no context (stint 1285).
    let (context_root, context_denied) = match caller {
        AgentsCaller::Principal {
            context_root,
            context_denied,
            ..
        } => (
            context_root.clone(),
            *context_denied || context_root.as_ref().is_some_and(|root| root != &workspace),
        ),
        AgentsCaller::Host { .. } => (None, false),
    };
    if context_denied {
        let actor = caller.actor();
        let call_id = format!("agents-api-{op}-{}", uuid::Uuid::new_v4());
        let resource = workspace.display().to_string();
        host_gate!().note_denial(&actor, &call_id, &resource, op, "context_scope_violation");
        log::info!(
            "agents_api: op={op} denied caller={actor} workspace={} outside caller context root {context_root:?}",
            workspace.display()
        );
        return json!({
            "ok": false,
            "error_code": "permission_denied",
            "error": format!(
                "admission refused for {actor}: requested '{}' is outside allowed scope '{}'. Run from a pane in the target folder.",
                workspace.display(),
                context_root.as_ref().map_or_else(|| "unknown".to_string(), |root| root.display().to_string())
            ),
        });
    }
    log::info!("agents_api: op={op} workspace={}", workspace.display());
    // Pre-owner runs are listed before this workspace's first sealed run.
    if matches!(op, "spawn_run" | "delegate") {
        if let Err(refused) = legacy_run_ids(&*host_gate!(), &workspace) {
            return refused;
        }
    }
    match op {
        "create_agent" => {
            let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let grants = match payload.get("grants").map(grants_from_value).transpose() {
                Ok(Some(grants)) => grants,
                Ok(None) => Vec::new(),
                Err(error) => {
                    return json!({"ok": false, "error_code": "invalid_argument", "error": error})
                }
            };
            // The ceiling is what the caller holds in the grant store, never
            // a card file and never "everything" for any caller (stint 1285).
            let ceiling = caller.ceiling(&workspace, None);
            let denial_actor = caller.actor();
            log::info!(
                "agents_api: create_agent caller={denial_actor} pane={caller_pane:?} ceiling_rows={}",
                ceiling.len()
            );
            let created = create_agent(CreateAgent {
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
                parent_grants: &ceiling,
                denial_actor: &denial_actor,
                parent_run: None,
                caller_pane,
            });
            if created.get("ok").and_then(|value| value.as_bool()) == Some(true) {
                crate::host::command_view::publish(&workspace, "agent created");
            }
            created
        }
        "list_agents" => {
            let include_temporary = payload
                .get("all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match list_agent_authorities(&workspace, include_temporary) {
                Ok(agents) => {
                    log::info!("agents_api: listed {} agent(s)", agents.len());
                    json!({"ok": true, "agents": agents})
                }
                Err(error) => json!({"ok": false, "error_code": "io_error", "error": error}),
            }
        }
        "list_grants" => list_grants(&workspace),
        "spawn_run" => {
            let agent_id = payload
                .get("agent")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let agent = match find_agent(&workspace, agent_id) {
                Ok(agent) => agent,
                Err(error) => return error,
            };
            let admission = payload
                .get("admission")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("adm_{}", uuid::Uuid::new_v4()));
            // A retry of a charged admission is answered as alpha answered
            // it: by the existing run (its owner rule applies in issue_run),
            // before any fresh-start check (Codex 3290 r4).
            let retry = match load_runs(&workspace) {
                Ok(runs) => runs
                    .iter()
                    .any(|run| run.agent_id == agent.id && run.admission_id == admission),
                Err(error) => return json!({"ok": false, "error_code": "io_error", "error": error}),
            };
            // A fresh run acts with the agent's sealed rows: a socket caller
            // starts one only when it holds every allow the agent holds, else
            // it files a request (stint 1285).
            let subject = caller.subject();
            let start = if retry {
                None
            } else {
                let agent_rows: Vec<ToolGrant> =
                    executable_rows(&workspace, &Principal::agent(actor_of(&agent.id)))
                .into_iter()
                .filter(|row| row.decision == Decision::Allow)
                .map(|row| ToolGrant {
                    tool: row.tool,
                    decision: Decision::Allow,
                    kind: GrantKind::Hard,
                    expires_at: None,
                })
                .collect();
                let held = caller.ceiling(&workspace, None);
                if matches!(caller, AgentsCaller::Principal { .. }) {
                    if let Some(refused) = subset_denied(
                        &workspace,
                        &caller.actor(),
                        &held,
                        &agent_rows,
                        None,
                        caller_pane,
                    ) {
                        return refused;
                    }
                }
                match permit::mint_start(
                    &subject,
                    &workspace,
                    &agent.id,
                    &admission,
                    &allowed_tools(&agent_rows),
                    &held_allows(&held),
                ) {
                    Ok(start) => Some(start),
                    Err(error) => return permit_refused(error),
                }
            };
            issue_run(IssueRun {
                workspace: &workspace,
                subject: &subject,
                start: start.as_ref(),
                agent: &agent,
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
        "finish_run" => {
            let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let run = match find_run(&workspace, id) {
                Ok(run) => run,
                Err(error) => return error,
            };
            match permit_for::<Finish>(&workspace, &run, &caller.subject()) {
                Ok(permit) => finish_run(&permit),
                Err(refused) => refused,
            }
        }
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
            caller,
        ),
        "call" => {
            let run_id = payload.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
            let run = match find_run(&workspace, run_id) {
                Ok(run) => run,
                Err(error) => return error,
            };
            match permit_for::<Invoke>(&workspace, &run, &caller.subject()) {
                Ok(permit) => invoke(
                    &permit,
                    payload.get("tool").and_then(|v| v.as_str()).unwrap_or(""),
                    payload
                        .get("input_json")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}"),
                    caller_pane,
                ),
                Err(refused) => refused,
            }
        }
        other => {
            json!({"ok": false, "error_code": "invalid_argument", "error": format!("unknown agents op {other}")})
        }
    }
}

fn list_grants(workspace: &Path) -> Value {
    let gate = host_gate!();
    gate.expire_soft_grants();
    let store = gate.store();
    let mut rows: Vec<Value> = store
        .records()
        .iter()
        .filter(|rec| {
            rec.actor_type == ActorType::Agent
                && !rec.consumed
                && (rec.duration != GrantDuration::Session
                    || rec.session_id.as_deref() == Some(gate.session_id()))
                && (rec.workspace_root.as_deref() == Some(workspace)
                    || rec.workspace_root.is_none())
        })
        .map(|rec| {
            let agent = rec.actor_id.strip_prefix("agent:").unwrap_or(&rec.actor_id);
            json!({
                "grant_id": rec.grant_id,
                "actor_id": rec.actor_id,
                "agent": agent,
                "tool": rec.target_id,
                "decision": rec.decision.as_str(),
                "kind": rec.kind.as_str(),
                "created_at": rec.created_at,
                "expires_at": rec.expires_at,
                "run_id": rec.bound_operation_id,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        let ah = a.get("actor_id").and_then(|v| v.as_str()).unwrap_or("");
        let bh = b.get("actor_id").and_then(|v| v.as_str()).unwrap_or("");
        let at = a.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        let bt = b.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        ah.cmp(bh).then_with(|| at.cmp(bt))
    });
    log::info!("agents_api: listed {} grant(s)", rows.len());
    json!({"ok": true, "grants": rows})
}

fn finish_run(permit: &RunPermit<Finish>) -> Value {
    let workspace = permit.workspace();
    let id = permit.run_id();
    let finished = {
        let _guard = journal_lock().lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = permit.check() {
            return permit_refused(error);
        }
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
        finished
    };
    host_gate!().revoke_run_grants(id);
    log::info!("agents_api: finished run {id}");
    json!({"ok": true, "run": finished})
}

fn delegate(
    workspace: &Path,
    parent_run: &str,
    name: &str,
    grants: Vec<ToolGrant>,
    caller: &AgentsCaller,
) -> Value {
    let caller_pane = caller.pane();
    let parent = match find_run(workspace, parent_run) {
        Ok(run) => run,
        Err(error) => return error,
    };
    if !parent.active {
        return json!({"ok": false, "error_code": "permission_denied", "error": "parent run is not active"});
    }
    // A child gets at most what both the parent run's agent and the caller
    // hold in the grant store; never the run file's list (stint 1285).
    let ceiling = meet_ceilings(
        &sealed_ceiling(
            workspace,
            &Principal::agent(parent.actor_id.clone()),
            Some(parent_run),
        ),
        &caller.ceiling(workspace, Some(parent_run)),
    );
    if let Some(refused) = subset_denied(
        workspace,
        &parent.actor_id,
        &ceiling,
        &grants,
        Some(parent_run),
        caller_pane,
    ) {
        return refused;
    }
    let parent_agent = match find_agent(workspace, &parent.agent_id) {
        Ok(agent) => agent,
        Err(error) => return error,
    };
    let created = create_agent(CreateAgent {
        workspace,
        name,
        display_name: name,
        description: "Temporary child. reports_to is not authority.",
        grants: &grants,
        temporary: true,
        reports_to: Some(&parent_agent.id),
        parent_grants: &ceiling,
        denial_actor: &parent.actor_id,
        parent_run: Some(parent_run),
        caller_pane,
    });
    if created.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        if created.get("error_code").and_then(|v| v.as_str()) == Some("agent_exists") {
            let existing = match find_agent(workspace, name) {
                Ok(agent) => agent,
                Err(error) => return error,
            };
            if !same_grant_specs(&existing.grants, &grants)
                || existing.reports_to.as_deref() != Some(parent_agent.id.as_str())
            {
                return deny_with_request(RefusalContext {
                    workspace,
                    actor: &parent.actor_id,
                    tool: name,
                    reason: "existing child grants differ; refusing to widen",
                    parent_run: Some(parent_run),
                    run_id: Some(parent_run),
                    caller_pane,
                });
            }
        } else {
            return created;
        }
    }
    let child = match find_agent(workspace, name) {
        Ok(agent) => agent,
        Err(error) => return error,
    };
    let admission = format!("delegate:{parent_run}:{name}");
    // The child run belongs to whoever delegated it.
    let subject = caller.subject();
    let start = match permit::mint_start(
        &subject,
        workspace,
        &child.id,
        &admission,
        &allowed_tools(&grants),
        &held_allows(&ceiling),
    ) {
        Ok(start) => start,
        Err(error) => return permit_refused(error),
    };
    let spawned = issue_run(IssueRun {
        workspace,
        subject: &subject,
        start: Some(&start),
        agent: &child,
        parent_run: Some(parent_run),
        admission_id: &admission,
        client_ref: &parent.client_ref,
        kind: &parent.kind,
        input_tokens: 0,
        output_tokens: 0,
    });
    if spawned.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        log::info!(
            "agents_api: delegated child={name} parent_run={parent_run} grants={}",
            grants.len()
        );
    }
    spawned
}

/// The tools `grants` allow.
fn allowed_tools(grants: &[ToolGrant]) -> BTreeSet<String> {
    grants
        .iter()
        .filter(|grant| grant.decision == Decision::Allow)
        .map(|grant| grant.tool.clone())
        .collect()
}

/// The tools a ceiling allows.
fn held_allows(rows: &[CeilingRow]) -> BTreeSet<String> {
    rows.iter()
        .filter(|row| row.decision == Decision::Allow)
        .map(|row| row.tool.clone())
        .collect()
}

fn invoke(
    permit: &RunPermit<Invoke>,
    tool: &str,
    input_json: &str,
    caller_pane: Option<u64>,
) -> Value {
    let workspace = permit.workspace();
    let run_id = permit.run_id();
    let run = match find_run(workspace, run_id) {
        Ok(run) => run,
        Err(error) => return error,
    };
    if !run.active {
        return deny_with_request(RefusalContext {
            workspace,
            actor: &run.actor_id,
            tool,
            reason: "run is not active",
            parent_run: run.parent_run.as_deref(),
            run_id: Some(&run.id),
            caller_pane,
        });
    }
    // The run file only narrows. A tool it does not list runs when a desktop
    // approval sealed a row for this run; the gate below still decides.
    let listed = run.grants.iter().find(|grant| grant.tool == tool);
    if listed.is_none() && !host_gate!().run_holds_approval(workspace, &run.actor_id, &run.id, tool) {
        return deny_with_request(RefusalContext {
            workspace,
            actor: &run.actor_id,
            tool,
            reason: "tool is outside the run's granted set",
            parent_run: run.parent_run.as_deref(),
            run_id: Some(&run.id),
            caller_pane,
        });
    }
    if listed.is_some_and(|grant| grant.decision == Decision::Deny) {
        return deny_with_request(RefusalContext {
            workspace,
            actor: &run.actor_id,
            tool,
            reason: "tool is denied for this run",
            parent_run: run.parent_run.as_deref(),
            run_id: Some(&run.id),
            caller_pane,
        });
    }
    let call_id = format!("call_{}", uuid::Uuid::new_v4());
    let admission = host_gate!().admit(AdmitRequest {
        call_id: &call_id,
        tool,
        input_json,
        principal: &crate::broker::principal::Principal::agent(&run.actor_id),
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
            let effective_parent = run.parent_run.as_deref().unwrap_or(&run.id);
            let needs_you_id = host_gate!()
                .file_needs_you_with_details(
                    NeedsYouFile {
                        kind: NeedsYouKind::PermissionRequest,
                        actor: run.actor_id.clone(),
                        resource: tool.to_string(),
                        summary: format!("tool '{tool}' was denied by the permission gate"),
                        expires_at: None,
                        run_tag: Some(effective_parent.to_string()),
                    },
                    Some(NeedsYouDetails {
                        pane_id: caller_pane,
                        tool: Some(tool.to_string()),
                        parent_run: Some(effective_parent.to_string()),
                        run_id: Some(run.id.clone()),
                        workspace: Some(workspace.display().to_string()),
                        correlation_id: Some(effective_parent.to_string()),
                        ..Default::default()
                    }),
                )
                .ok();
            json!({
                "ok": false,
                "error_code": code,
                "error": code,
                "tool": tool,
                "actor_id": run.actor_id,
                "call_id": call_id,
                "needs_you_id": needs_you_id,
            })
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
            if host_gate!()
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
                return deny_with_request(RefusalContext {
                    workspace,
                    actor: &run.actor_id,
                    tool,
                    reason: "grant was revoked before the call",
                    parent_run: run.parent_run.as_deref(),
                    run_id: Some(&run.id),
                    caller_pane,
                });
            }
            if let Err(error) = permit.check() {
                return permit_refused(error);
            }
            let body = execute_tool(workspace, &run, tool, input_json);
            host_gate!().consume_once(&grant_id, Some("agents-api"));
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
        "agents.list" => match list_agent_authorities(workspace, false) {
            Ok(agents) => json!({"ok": true, "agents": agents, "actor_id": run.actor_id}),
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
            create_agent(CreateAgent {
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
                parent_grants: &sealed_ceiling(
                    workspace,
                    &Principal::agent(run.actor_id.clone()),
                    Some(&run.id),
                ),
                denial_actor: &run.actor_id,
                parent_run: Some(&run.id),
                caller_pane: None,
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

pub fn projection(workspace: &Path) -> Value {
    let workspace = crate::platform::path::canonical_or_self(workspace);
    let host = AgentsCaller::Host { ceiling: Vec::new() };
    let agents = handle_request(
        "list_agents",
        &json!({"workspace": workspace, "all": true}),
        &host,
    );
    let runs = handle_request("list_runs", &json!({"workspace": &workspace}), &host);
    let pending = serde_json::to_value(host_gate!().list_pending_in_workspace(&workspace))
        .unwrap_or(json!([]));
    json!({
        "ok": true,
        "agents": agents.get("agents").cloned().unwrap_or(json!([])),
        "runs": runs.get("runs").cloned().unwrap_or(json!([])),
        "needs_you": pending,
    })
}

pub fn pane_lines(workspace: &Path) -> Vec<String> {
    let body = projection(workspace);
    let mut lines = Vec::new();
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
    if let Some(runs) = body.get("runs").and_then(|value| value.as_array()) {
        for run in runs {
            if run.get("active").and_then(|value| value.as_bool()) != Some(true) {
                continue;
            }
            let id = run.get("id").and_then(|value| value.as_str()).unwrap_or("");
            let agent = run
                .get("agent_id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            lines.push(format!("run {id} {agent} running"));
        }
    }
    if lines.is_empty() {
        lines.push("no active runs".to_string());
    }
    lines
}

pub struct McpAuth {
    pub workspace: PathBuf,
    pub run_id: Option<String>,
    pub actor_id: Option<String>,
    pub pane_id: u64,
    pub context_id: u64,
    /// The calling pane's ceiling, from its bearer's authority binding
    /// (stint 1632). `None` for a run token or an unrestricted pane.
    pub posture: Option<crate::broker::PermissionPosture>,
}

/// Host MCP front end. A run token calls as that run. A pane credential is
/// admitted as itself through the same monitor; an ungranted call becomes a
/// pending approval rather than a second permission system.
pub fn mcp_call(auth: &McpAuth, tool: &str, input_json: &str) -> Result<String, String> {
    let caller_pane = (auth.pane_id != 0).then_some(auth.pane_id);
    let value = if let Some(run_id) = &auth.run_id {
        if tool == "agents.delegate" {
            let input: Value = serde_json::from_str(input_json).unwrap_or_else(|_| json!({}));
            let parent = input
                .get("parent_run")
                .and_then(|v| v.as_str())
                .unwrap_or(run_id);
            if parent != run_id {
                deny_with_request(RefusalContext {
                    workspace: &auth.workspace,
                    actor: auth.actor_id.as_deref().unwrap_or("agent"),
                    tool,
                    reason: "run token does not own parent_run",
                    parent_run: Some(run_id),
                    run_id: Some(run_id),
                    caller_pane,
                })
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
                // The run delegates as its own agent principal.
                let caller = AgentsCaller::Principal {
                    principal: Principal::agent(auth.actor_id.clone().unwrap_or_default()),
                    pane: caller_pane,
                    agent: None,
                    context_root: None,
                    context_denied: false,
                    authority: None,
                };
                delegate(
                    &auth.workspace,
                    run_id,
                    input.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    grants,
                    &caller,
                )
            }
        } else {
            let subject = PermitSubject::RunBearer {
                run_id: run_id.clone(),
            };
            match find_run(&auth.workspace, run_id)
                .and_then(|run| permit_for::<Invoke>(&auth.workspace, &run, &subject))
            {
                Ok(permit) => invoke(&permit, tool, input_json, caller_pane),
                Err(refused) => refused,
            }
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
    // Admitted under the pane's ceiling, with the broker's grant precedence.
    let admission = host_gate!().admit_with_posture(
        AdmitRequest {
            call_id: &call_id,
            tool,
            input_json,
            principal: &crate::broker::principal::Principal::agent(&actor),
            actor_scope: ActorScope::User,
            trust_origin: "host",
            workspace_root: &auth.workspace,
            context_id: auth.context_id,
            package_id: PACKAGE,
            instance_id: auth.pane_id,
            target_type: TargetType::HostTool,
        },
        auth.posture.as_ref(),
    );
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
                workspace,
            }
        }

        fn ws(&self) -> &Path {
            self.workspace.path()
        }
    }

    /// Round 3 item 4: a pane's `agents.*` MCP call is admitted under the
    /// pane's ceiling. An explicit deny refuses it outright; the same call
    /// without a ceiling becomes a pending approval.
    #[test]
    fn pane_agents_call_is_admitted_under_its_ceiling() {
        let fix = Fixture::new();
        let auth = |posture| McpAuth {
            workspace: fix.ws().to_path_buf(),
            run_id: None,
            actor_id: None,
            pane_id: 42_016_320,
            context_id: 1,
            posture,
        };
        let unrestricted = mcp_call(&auth(None), "agents.list", "{}").unwrap_err();
        let unrestricted: Value = serde_json::from_str(&unrestricted).unwrap();
        assert_eq!(
            unrestricted["error_code"], "permission_required",
            "{unrestricted}"
        );
        let denied = mcp_call(
            &auth(Some(crate::broker::PermissionPosture {
                default_posture: crate::broker::Decision::Ask,
                allow: Vec::new(),
                ask: Vec::new(),
                deny: vec!["agents.list".to_string()],
            })),
            "agents.list",
            "{}",
        )
        .unwrap_err();
        let denied: Value = serde_json::from_str(&denied).unwrap();
        assert_eq!(denied["error_code"], "permission_denied", "{denied}");
    }

    /// Host code holding every tool these fixtures use. A real caller's
    /// ceiling is its sealed rows; the refusal cases below build that.
    fn test_host() -> AgentsCaller {
        AgentsCaller::Host {
            ceiling: ["agents.ping", "agents.review", "agents.admin", "assistant.turn"]
                .iter()
                .map(|tool| CeilingRow {
                    tool: tool.to_string(),
                    decision: Decision::Allow,
                    duration: GrantDuration::Always,
                    expires_at: None,
                    bound_run: None,
                    grant_ids: Vec::new(),
                })
                .collect(),
        }
    }

    /// A pane caller, as the host classifies one.
    fn pane_caller(pane: u64, agent: Option<&str>) -> AgentsCaller {
        AgentsCaller::Principal {
            principal: Principal::agent(format!("pane:{pane}")),
            pane: Some(pane),
            agent: agent.map(str::to_string),
            context_root: None,
            context_denied: false,
            authority: None,
        }
    }

    fn grants(specs: &[&str]) -> Vec<ToolGrant> {
        specs
            .iter()
            .map(|spec| parse_grant_spec(spec).unwrap())
            .collect()
    }

    fn create(fix: &Fixture, name: &str, specs: &[&str]) -> Value {
        create_agent(CreateAgent {
            workspace: fix.ws(),
            name,
            display_name: name,
            description: "test",
            grants: &grants(specs),
            temporary: false,
            reports_to: None,
            // A test fixture's own ceiling: exactly what it asks for.
            parent_grants: &grants(specs)
                .iter()
                .map(|grant| CeilingRow {
                    tool: grant.tool.clone(),
                    decision: grant.decision,
                    duration: GrantDuration::Always,
                    expires_at: None,
                    bound_run: None,
                    grant_ids: Vec::new(),
                })
                .collect::<Vec<_>>(),
            denial_actor: "fixture",
            parent_run: None,
            caller_pane: None,
        })
    }

    #[test]
    fn agent_lands_under_neutral_agents_dir_and_lists() {
        let fix = Fixture::new();
        let created = create(&fix, "worker", &["agents.ping=allow", "agents.review=ask"]);
        assert_eq!(created["ok"], true, "{created}");
        let dir = fix.ws().join(".plexi").join("agents").join("worker");
        assert!(dir.join("AGENT.md").is_file());
        assert!(dir.join("settings.toml").is_file());
        assert!(!fix.ws().join(".plexi-alpha").join("agents").exists());
        let listed = handle_request("list_agents", &json!({"workspace": fix.ws()}), &test_host());
        assert_eq!(listed["agents"][0]["id"], "worker");
        assert_eq!(listed["agents"][0]["grants"][0]["tool"], "agents.ping");
    }

    #[test]
    fn run_identity_uses_workspace_then_user_registry_precedence() {
        let fix = Fixture::new();
        let profile_agent = fix._profile.path().join("agents/worker");
        fs::create_dir_all(&profile_agent).unwrap();
        fs::write(profile_agent.join("AGENT.md"), "User persona").unwrap();
        fs::write(profile_agent.join("settings.toml"), "[agent]\nid = \"worker\"\ndisplay_name = \"User\"\ndefault_tier = \"low\"\n[permissions]\ndefault_posture = \"deny\"\n").unwrap();
        assert_eq!(find_agent(fix.ws(), "worker").unwrap().display_name, "User");
        create(&fix, "worker", &["agents.ping=allow"]);
        let workspace_agent = find_agent(fix.ws(), "worker").unwrap();
        assert_eq!(workspace_agent.display_name, "worker");
        assert_eq!(workspace_agent.grants[0].tool, "agents.ping");
        fs::remove_file(agents_root(fix.ws()).join("worker/agent.json")).unwrap();
        assert_eq!(find_agent(fix.ws(), "worker").unwrap().display_name, "worker");
    }

    #[test]
    fn definition_without_authority_can_spawn_but_cannot_call_tools() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        fs::remove_file(agents_root(fix.ws()).join("worker/agent.json")).unwrap();
        let spawned = handle_request("spawn_run", &json!({"workspace": fix.ws(), "agent": "worker"}), &test_host());
        assert_eq!(spawned["ok"], true, "{spawned}");
        let call = handle_request("call", &json!({"workspace": fix.ws(), "run_id": spawned["run"]["id"], "tool": "agents.ping", "input_json": "{}"}), &test_host());
        assert_eq!(call["error_code"], "permission_denied", "{call}");
    }

    #[test]
    fn spawn_claims_once_and_writes_tagged_ledger_row() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let payload = json!({
            "workspace": fix.ws(),
            "agent": "worker",
            "admission": "adm-1",
            "client_ref": "acme",
            "kind": "output",
            "input_tokens": 11,
            "output_tokens": 4,
        });
        let spawned = handle_request("spawn_run", &payload, &test_host());
        assert_eq!(spawned["ok"], true, "{spawned}");
        assert_eq!(spawned["ledger"]["client_ref"], "acme");
        assert_eq!(spawned["ledger"]["kind"], "output");
        assert_eq!(spawned["ledger"]["input_tokens"], 11);
        assert_eq!(spawned["ledger"]["output_tokens"], 4);
        let again = handle_request("spawn_run", &payload, &test_host());
        assert_eq!(again["idempotent"], true);
        assert_eq!(again["run"]["id"], spawned["run"]["id"]);
        let conflict = handle_request(
            "spawn_run",
            &json!({
                "workspace": fix.ws(),
                "agent": "worker",
                "admission": "adm-2",
            }), &test_host());
        assert_eq!(conflict["error_code"], "assignment_conflict");
        let ledger =
            fs::read_to_string(crate::config::config_dir().join("ai-ledger.jsonl")).unwrap();
        assert_eq!(ledger.lines().count(), 1, "{ledger}");
        assert!(ledger.contains("\"run_id\""));
        assert!(ledger.contains("acme"));
        assert!(ledger.contains("\"input_tokens\":11"));
    }

    /// Stint 1441: a ledger that refuses the row (an old-name ledger, here)
    /// leaves no active run behind, so the next spawn is not blocked by
    /// `assignment_conflict`.
    #[test]
    fn failed_ledger_append_leaves_no_active_run() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let retired = crate::config::config_dir().join("agent-ledger.jsonl");
        fs::write(&retired, b"").unwrap();

        let first = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-1"}), &test_host());
        assert_eq!(first["error_code"], "ledger_error", "{first}");
        assert!(
            load_runs(fix.ws()).unwrap().iter().all(|run| !run.active),
            "no active run is left behind"
        );

        fs::remove_file(&retired).unwrap();
        let next = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-2"}), &test_host());
        assert_eq!(next["ok"], true, "the next spawn is not blocked: {next}");
    }

    /// Stint 1441: a retry with the same admission after a failed ledger
    /// append never returns ok without a ledger row.
    #[test]
    fn retry_after_failed_ledger_append_is_not_ok_without_a_row() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let retired = crate::config::config_dir().join("agent-ledger.jsonl");
        fs::write(&retired, b"").unwrap();
        let payload = json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-1"});

        let first = handle_request("spawn_run", &payload, &test_host());
        assert_eq!(first["error_code"], "ledger_error", "{first}");
        let retry = handle_request("spawn_run", &payload, &test_host());
        assert_ne!(
            retry["ok"], true,
            "retry must not succeed without a ledger row: {retry}"
        );
        assert!(
            !crate::config::config_dir().join("ai-ledger.jsonl").exists(),
            "no ledger row was written"
        );
    }

    /// Stint 1441: a run journalled by an older build without its ledger row
    /// is not returned as an idempotent ok.
    #[test]
    fn idempotent_retry_refuses_a_run_with_no_ledger_row() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let agent = find_agent(fix.ws(), "worker").unwrap();
        let orphan = RunRecord {
            id: "run_orphan".to_string(),
            agent_id: agent.id.clone(),
            actor_id: actor_of(&agent.id),
            parent_run: None,
            admission_id: "adm-1".to_string(),
            version: 1,
            state: "running".to_string(),
            active: true,
            client_ref: String::new(),
            kind: "output".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            grants: agent.grants.clone(),
            owner_seal: None,
        };
        append_run_record(fix.ws(), &orphan).unwrap();

        let retry = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-1"}), &test_host());
        assert_ne!(retry["ok"], true, "{retry}");
        assert_eq!(retry["error_code"], "ledger_missing", "{retry}");
    }

    /// Stint 1502 part 1: an agent stuck on an orphan active run (journalled
    /// with no ledger row) is told exactly how to clear it, and that step works.
    #[test]
    fn orphan_active_run_errors_name_the_command_that_clears_it() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let agent = find_agent(fix.ws(), "worker").unwrap();
        append_run_record(
            fix.ws(),
            &RunRecord {
                id: "run_orphan".to_string(),
                agent_id: agent.id.clone(),
                actor_id: actor_of(&agent.id),
                parent_run: None,
                admission_id: "adm-1".to_string(),
                version: 1,
                state: "running".to_string(),
                active: true,
                client_ref: String::new(),
                kind: "output".to_string(),
                input_tokens: 0,
                output_tokens: 0,
                grants: agent.grants.clone(),
                owner_seal: None,
            },
        )
        .unwrap();
        let clear = "plexi agents-dashboard cancel run_orphan";

        let same = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-1"}), &test_host());
        assert_eq!(same["error_code"], "ledger_missing", "{same}");
        let message = same["error"].as_str().unwrap();
        assert!(message.contains(clear), "{message}");
        assert!(message.contains("new admission"), "{message}");

        let other = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-2"}), &test_host());
        assert_eq!(other["error_code"], "assignment_conflict", "{other}");
        let message = other["error"].as_str().unwrap();
        assert!(message.contains(clear), "{message}");

        // The named command runs finish_run on the host.
        let finished = handle_request(
            "finish_run",
            &json!({"workspace": fix.ws(), "id": "run_orphan"}), &test_host());
        assert_eq!(finished["ok"], true, "{finished}");
        let next = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-2"}), &test_host());
        assert_eq!(next["ok"], true, "the cleared agent spawns again: {next}");
    }

    #[test]
    fn delegation_refuses_a_grant_the_parent_lacks_and_enforces_the_rest() {
        let fix = Fixture::new();
        create(
            &fix,
            "worker",
            &[
                "agents.ping=allow",
                "agents.review=ask",
                "agents.list=allow",
                "agents.create=allow",
            ],
        );
        let spawned = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-1"}), &test_host());
        let parent = spawned["run"]["id"].as_str().unwrap();
        let widened = handle_request(
            "delegate",
            &json!({
                "workspace": fix.ws(),
                "parent_run": parent,
                "name": "rogue",
                "grants": ["agents.admin=allow"],
            }), &test_host());
        assert_eq!(widened["error_code"], "permission_denied", "{widened}");
        assert!(!fix
            .ws()
            .join(".plexi")
            .join("agents")
            .join("rogue")
            .exists());
        let audit = monitor().unwrap().audit_records();
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
            }), &test_host());
        assert_eq!(child["ok"], true, "{child}");
        let child_run = child["run"]["id"].as_str().unwrap();
        let card = read_agent(&fix.ws().join(".plexi").join("agents").join("scout")).unwrap();
        assert!(card.temporary);
        assert_eq!(card.reports_to.as_deref(), Some("worker"));
        assert!(!card.grants.iter().any(|grant| grant.tool == "agents.admin"));

        let ping = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": child_run,
                "tool": "agents.ping",
                "input_json": "{}",
            }), &test_host());
        assert_eq!(ping["ok"], true, "{ping}");
        assert_eq!(ping["actor_id"], "agent:scout");

        let admin = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": child_run,
                "tool": "agents.admin",
                "input_json": "{}",
            }), &test_host());
        assert_eq!(admin["error_code"], "permission_denied", "{admin}");

        let review = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": child_run,
                "tool": "agents.review",
                "input_json": "{}",
            }), &test_host());
        assert_eq!(review["error_code"], "permission_required", "{review}");
        let pending = review["pending_request_id"].as_str().unwrap();
        let listed = monitor().unwrap().list_pending();
        assert!(listed
            .iter()
            .any(|row| row.pending_request_id == pending && row.actor_id == "agent:scout"));
        monitor()
            .unwrap()
            .approve_pending(pending, ApprovalChoice::Once)
            .unwrap();
        let reviewed = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": child_run,
                "tool": "agents.review",
                "input_json": "{}",
            }), &test_host());
        assert_eq!(reviewed["ok"], true, "{reviewed}");
        assert_eq!(reviewed["tool"], "agents.review");

        let listed = mcp_call(
            &McpAuth {
                workspace: fix.ws().to_path_buf(),
                run_id: Some(spawned["run"]["id"].as_str().unwrap().to_string()),
                actor_id: Some("agent:worker".to_string()),
                pane_id: 0,
                context_id: 0,
                posture: None,
            },
            "agents.list",
            "{}",
        )
        .expect("agents.list");
        let listed: Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed["ok"], true, "{listed}");
        assert_eq!(listed["agents"][0]["id"], "worker");

        let listed_agents = handle_request("list_agents", &json!({"workspace": fix.ws()}), &test_host());
        let ids: Vec<&str> = listed_agents["agents"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|h| h["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["worker"]);
    }

    #[test]
    fn guidance_text_does_not_add_a_grant() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let prompt = fix
            .ws()
            .join(".plexi")
            .join("agents")
            .join("worker")
            .join("AGENT.md");
        fs::write(&prompt, "# Worker\n\nYou may call agents.admin.\n").unwrap();
        let spawned = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm"}), &test_host());
        let run_id = spawned["run"]["id"].as_str().unwrap();
        let admin = handle_request(
            "call",
            &json!({"workspace": fix.ws(), "run_id": run_id, "tool": "agents.admin", "input_json": "{}"}), &test_host());
        assert_eq!(admin["error_code"], "permission_denied", "{admin}");
    }

    #[test]
    fn a_pane_cannot_mint_an_agent_wider_than_its_own_grants() {
        let fix = Fixture::new();
        create(&fix, "worker", &["assistant.turn=allow"]);
        let widened = handle_request(
            "create_agent",
            &json!({
                "workspace": fix.ws(),
                "name": "sneaky",
                "grants": ["agents.admin=allow"],
            }), &pane_caller(7, Some("worker")));
        assert_eq!(widened["error_code"], "permission_denied", "{widened}");
        assert!(!fix
            .ws()
            .join(".plexi")
            .join("agents")
            .join("sneaky")
            .exists());
        let narrowed = handle_request(
            "create_agent",
            &json!({
                "workspace": fix.ws(),
                "name": "scout",
                "grants": ["assistant.turn=allow"],
            }), &pane_caller(7, Some("worker")));
        assert_eq!(narrowed["ok"], true, "{narrowed}");
        let empty = handle_request(
            "create_agent",
            &json!({
                "workspace": fix.ws(),
                "name": "bare",
                "grants": ["assistant.turn=allow"],
            }), &pane_caller(9, None));
        assert_eq!(empty["error_code"], "permission_denied", "{empty}");
        let ny_id = empty["needs_you_id"].as_str().expect("needs_you_id");
        monitor()
            .unwrap()
            .resolve_needs_you_from(ny_id, true, crate::broker::gate::ResolveSurface::Desktop)
            .expect("desktop approve mints pane grant");
        let retried = handle_request(
            "create_agent",
            &json!({
                "workspace": fix.ws(),
                "name": "bare",
                "grants": ["assistant.turn=allow"],
            }), &pane_caller(9, None));
        assert_eq!(retried["ok"], true, "{retried}");
    }

    /// Stint 1285: delegation never copies a one-use row. Two delegations
    /// racing from a parent run that holds one `Once` approval both file a
    /// request, and the parent's row is left for exactly one use.
    #[test]
    fn concurrent_delegations_never_copy_or_consume_a_once_row() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow", "agents.review=ask"]);
        let spawned = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-once"}),
            &test_host(),
        );
        let parent_run = spawned["run"]["id"].as_str().unwrap().to_string();
        let call = || {
            handle_request(
                "call",
                &json!({"workspace": fix.ws(), "run_id": parent_run, "tool": "agents.review",
                    "input_json": "{}"}),
                &test_host(),
            )
        };
        let asked = call();
        assert_eq!(asked["error_code"], "permission_required", "{asked}");
        monitor()
            .unwrap()
            .approve_pending(asked["pending_request_id"].as_str().unwrap(), ApprovalChoice::Once)
            .unwrap();
        let live_once_rows = || {
            monitor()
                .unwrap()
                .store()
                .records()
                .iter()
                .filter(|rec| {
                    rec.target_id == "agents.review"
                        && rec.duration == GrantDuration::Once
                        && !rec.consumed
                })
                .count()
        };
        assert_eq!(live_once_rows(), 1);

        let profile = fix._profile.path().to_path_buf();
        let workspace = fix.ws().to_path_buf();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let racers: Vec<_> = ["scout-a", "scout-b"]
            .into_iter()
            .map(|name| {
                let (profile, workspace, parent_run, barrier) =
                    (profile.clone(), workspace.clone(), parent_run.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let _guard = crate::config::set_test_profile_dir(profile);
                    barrier.wait();
                    handle_request(
                        "delegate",
                        &json!({"workspace": workspace, "parent_run": parent_run, "name": name,
                            "grants": ["agents.review=allow"]}),
                        &test_host(),
                    )
                })
            })
            .collect();
        for racer in racers {
            let reply = racer.join().unwrap();
            assert_eq!(reply["ok"], false, "{reply}");
            assert!(reply["needs_you_id"].is_string(), "a delegation must file a request: {reply}");
        }
        assert!(find_agent(fix.ws(), "scout-a").is_err() && find_agent(fix.ws(), "scout-b").is_err());
        assert_eq!(live_once_rows(), 1, "a delegation consumed or copied the one-use row");

        let used = call();
        assert_eq!(used["ok"], true, "{used}");
        let again = call();
        assert_eq!(again["error_code"], "permission_required", "used twice: {again}");
    }

    #[test]
    fn delegate_refusal_audits_and_files_needs_you_and_desktop_allow_grants_for_that_run_only() {
        use crate::broker::gate::ResolveSurface;

        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let spawned = handle_request(
            "spawn_run",
            &json!({
                "workspace": fix.ws(),
                "agent": "worker",
                "admission": "adm-run-1",
            }), &test_host());
        assert_eq!(spawned["ok"], true, "{spawned}");
        let parent_run = spawned["run"]["id"].as_str().unwrap().to_string();

        let refused = handle_request(
            "delegate",
            &json!({
                "workspace": fix.ws(),
                "parent_run": parent_run,
                "name": "scout",
                "grants": ["agents.admin=allow"],
            }), &pane_caller(42, None));
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error_code"], "permission_denied", "{refused}");
        let needs_you_id = refused["needs_you_id"]
            .as_str()
            .expect("refusal must return needs_you_id")
            .to_string();

        // 1. A permission-audit.jsonl row is written.
        let audit_path = crate::config::config_dir().join("permission-audit.jsonl");
        let audit_raw = fs::read_to_string(&audit_path).expect("permission-audit.jsonl must exist");
        assert!(
            audit_raw.contains("agents.admin") && audit_raw.contains("agent:worker"),
            "expected deny audit row in permission-audit.jsonl, got:\n{audit_raw}"
        );
        let audit_rows = monitor().unwrap().audit_records();
        assert!(
            audit_rows.iter().any(|row| {
                row.kind == "deny"
                    && row.resource_id == "agents.admin"
                    && row.actor == "agent:worker"
            }),
            "expected deny AuditFact in monitor().unwrap().audit_records(), got: {audit_rows:?}"
        );

        // 2. The refusal appears in `plexi needs-you list --json` (`list_needs_you`).
        let items = monitor().unwrap().list_needs_you();
        let entry = items
            .iter()
            .find(|item| item.id == needs_you_id)
            .expect("refusal must appear in needs-you list");
        assert_eq!(entry.kind, NeedsYouKind::PermissionRequest);
        assert_eq!(entry.actor, "agent:worker");
        assert_eq!(entry.resource, "agents.admin");
        let entry_json = serde_json::to_value(entry).unwrap();
        assert_eq!(entry_json["kind"], "permission_request");
        assert_eq!(entry_json["tool"], "agents.admin");
        assert_eq!(entry_json["parent_run"], parent_run);
        assert_eq!(entry_json["pane_id"], 42);

        // 3. Click-only approval rule from 0788: socket cannot approve.
        assert!(monitor()
            .unwrap()
            .resolve_needs_you_from(&needs_you_id, true, ResolveSurface::Socket)
            .is_err());

        // 4. Resolving with Allow from Desktop grants `agents.admin` for `parent_run` only,
        //    leaving `agent.json` untouched.
        monitor()
            .unwrap()
            .resolve_needs_you_from(&needs_you_id, true, ResolveSurface::Desktop)
            .expect("desktop Allow must resolve PermissionRequest");

        let worker_card =
            read_agent(&fix.ws().join(".plexi").join("agents").join("worker")).unwrap();
        assert!(
            !worker_card.grants.iter().any(|g| g.tool == "agents.admin"),
            "agent.json must NOT be permanently widened: {worker_card:?}"
        );

        // Now `parent_run` can call `agents.admin` and delegate `agents.admin=allow` during this run.
        let direct_call = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": parent_run,
                "tool": "agents.admin",
                "input_json": "{}",
            }), &test_host());
        assert_eq!(direct_call["ok"], true, "{direct_call}");

        let delegated = handle_request(
            "delegate",
            &json!({
                "workspace": fix.ws(),
                "parent_run": parent_run,
                "name": "scout",
                "grants": ["agents.admin=allow"],
            }), &pane_caller(42, None));
        assert_eq!(delegated["ok"], true, "{delegated}");

        // Finish `parent_run` and start a new run of `worker`: the new run must NOT hold `agents.admin`.
        let finished = handle_request(
            "finish_run",
            &json!({
                "workspace": fix.ws(),
                "id": parent_run,
            }), &test_host());
        assert_eq!(finished["ok"], true, "{finished}");

        let spawned_next = handle_request(
            "spawn_run",
            &json!({
                "workspace": fix.ws(),
                "agent": "worker",
                "admission": "adm-run-2",
            }), &test_host());
        assert_eq!(spawned_next["ok"], true, "{spawned_next}");
        let next_run = spawned_next["run"]["id"].as_str().unwrap();
        let next_call = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": next_run,
                "tool": "agents.admin",
                "input_json": "{}",
            }), &test_host());
        assert_eq!(
            next_call["error_code"], "permission_denied",
            "subsequent run must not inherit run-only grant: {next_call}"
        );
    }

    /// Stint 1713: expiring a soft grant flips only the grant in `agent.json`;
    /// the user's hand-edited `AGENT.md` and `settings.toml` survive byte for byte.
    #[test]
    fn soft_grant_expiry_keeps_hand_edited_definition_files() {
        let fix = Fixture::new();
        let t0: i64 = 1_700_000_000;
        monitor().unwrap().set_now_for_test(t0);
        let created = handle_request(
            "create_agent",
            &json!({
                "workspace": fix.ws(),
                "name": "worker",
                "grants": ["agents.review=allow:soft"],
            }),
            &test_host(),
        );
        assert_eq!(created["ok"], true, "{created}");
        let dir = agents_root(fix.ws()).join("worker");
        let persona = "# Worker\n\nYou are a terse reviewer. Quote \"exact\" lines.\n";
        let settings = "# my comment, keep me\n[agent]\nid = \"worker\"\ndisplay_name = \"Worker \\\"W\\\"\"\ndefault_tier = \"high\"\ndescription = \"says \\\"hi\\\"\"\n\n[permissions]\ndefault_posture = \"ask\"\n\n[notes]\nowner = \"me\"\n";
        fs::write(dir.join("AGENT.md"), persona).unwrap();
        fs::write(dir.join("settings.toml"), settings).unwrap();

        monitor().unwrap().set_now_for_test(t0 + 400 * 86_400);
        assert!(monitor().unwrap().expire_soft_grants() >= 1);

        assert_eq!(fs::read_to_string(dir.join("AGENT.md")).unwrap(), persona);
        assert_eq!(
            fs::read_to_string(dir.join("settings.toml")).unwrap(),
            settings
        );
        let record = read_agent(&dir).unwrap();
        let review = record
            .grants
            .iter()
            .find(|g| g.tool == "agents.review")
            .unwrap();
        assert_eq!(review.decision, Decision::Ask);
        assert!(review.expires_at.is_none());
    }

    /// A definition with no authority record (the seeded workspace
    /// `default`) is adopted by create: `agent.json` is written and the
    /// user's definition files stay byte for byte. A second create refuses.
    #[test]
    fn create_adopts_existing_definition_writing_only_authority() {
        let fix = Fixture::new();
        let dir = crate::agent::ensure_workspace_default_agent(fix.ws()).unwrap();
        let settings =
            "# mine\n".to_string() + &fs::read_to_string(dir.join("settings.toml")).unwrap();
        fs::write(dir.join("settings.toml"), &settings).unwrap();
        let persona = fs::read(dir.join("persona.md")).unwrap();
        let result = create(&fix, "default", &["agents.ping=allow"]);
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(
            fs::read_to_string(dir.join("settings.toml")).unwrap(),
            settings
        );
        assert_eq!(fs::read(dir.join("persona.md")).unwrap(), persona);
        assert!(!dir.join("AGENT.md").exists());
        assert_eq!(read_agent(&dir).unwrap().grants[0].tool, "agents.ping");
        let again = create(&fix, "default", &[]);
        assert_eq!(again["error_code"], "agent_exists", "{again}");
        assert_eq!(
            fs::read_to_string(dir.join("settings.toml")).unwrap(),
            settings
        );
    }

    /// A folder that is not a usable definition is reported, never filled in.
    #[test]
    fn create_refuses_unrecognised_directory() {
        let fix = Fixture::new();
        let dir = agents_root(fix.ws()).join("worker");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("AGENT.md"), "hand written\n").unwrap();
        let result = create(&fix, "worker", &["agents.ping=allow"]);
        assert_eq!(result["error_code"], "agent_unrecognised", "{result}");
        assert_eq!(
            fs::read_to_string(dir.join("AGENT.md")).unwrap(),
            "hand written\n"
        );
        assert!(!dir.join("agent.json").exists());
    }

    /// A failed authority write during expiry is returned to the caller.
    #[test]
    fn expiry_returns_authority_write_failure() {
        let fix = Fixture::new();
        assert_eq!(
            create(&fix, "worker", &["agents.review=allow:soft"])["ok"],
            true
        );
        let dir = agents_root(fix.ws()).join("worker");
        let authority = dir.join("agent.json");
        let mut perms = fs::metadata(&authority).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&authority, perms).unwrap();
        let error =
            expire_soft_grant_in_workspace(fix.ws(), "agent:worker", "agents.review").unwrap_err();
        assert!(error.contains("agent.json"), "{error}");
    }

    /// Codex 3290 r1 P1: a run belongs to whoever started it. A socket caller
    /// that did not start a run cannot call through it, finish it or have its
    /// token re-issued, and starts a run of an agent only when it holds every
    /// allow that agent holds.
    #[test]
    fn a_socket_caller_acts_only_on_runs_it_started() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let payload = json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-owned"});
        let spawned = handle_request("spawn_run", &payload, &test_host());
        assert_eq!(spawned["ok"], true, "{spawned}");
        let run_id = spawned["run"]["id"].as_str().unwrap().to_string();
        let stranger = pane_caller(42, None);

        let call = handle_request(
            "call",
            &json!({"workspace": fix.ws(), "run_id": run_id, "tool": "agents.ping",
                "input_json": "{}"}),
            &stranger,
        );
        assert_eq!(call["error_code"], "permission_denied", "{call}");
        assert!(call.get("output").is_none(), "the tool ran: {call}");
        let reissued = handle_request("spawn_run", &payload, &stranger);
        assert_eq!(reissued["ok"], false, "{reissued}");
        assert!(reissued.get("run_token").is_none(), "a token leaked: {reissued}");
        let finished = handle_request(
            "finish_run",
            &json!({"workspace": fix.ws(), "id": run_id}),
            &stranger,
        );
        assert_eq!(finished["error_code"], "permission_denied", "{finished}");
        assert_eq!(find_run(fix.ws(), &run_id).unwrap().active, true, "the run still runs");

        // A caller holding none of the agent's allows starts no run of it.
        let fresh = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-stranger"}),
            &stranger,
        );
        assert_eq!(fresh["ok"], false, "{fresh}");
        assert!(fresh["needs_you_id"].is_string(), "a request is filed: {fresh}");
        assert!(fresh.get("run_token").is_none(), "{fresh}");

        // The host, which started it, still can.
        let own = handle_request(
            "call",
            &json!({"workspace": fix.ws(), "run_id": run_id, "tool": "agents.ping",
                "input_json": "{}"}),
            &test_host(),
        );
        assert_eq!(own["ok"], true, "{own}");
    }

    /// Stint 1716: a run's owner is sealed into its own journal line, so it
    /// outlives the host, writes nothing to the grant store, and cannot be
    /// renamed, removed or copied by editing the journal.
    #[test]
    fn a_run_owner_survives_restart_and_the_journal_cannot_change_it() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=ask"]);
        let grants = crate::config::config_dir().join("grants.toml");
        let snapshot = || fs::read(&grants).ok();
        let before = snapshot();
        let owner = pane_caller(42, None);
        let stranger = pane_caller(43, None);
        let start = json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-owned"});
        let started = handle_request("spawn_run", &start, &owner);
        assert_eq!(started["ok"], true, "{started}");
        let run_id = started["run"]["id"].as_str().unwrap().to_string();
        let finish = json!({"workspace": fix.ws(), "id": run_id});

        // Restart: the bearer is gone, the owner is not.
        forget_run_for_test(&run_id);
        let foreign = handle_request("spawn_run", &start, &stranger);
        assert_eq!(foreign["error_code"], "permission_denied", "{foreign}");
        assert!(foreign.get("run_token").is_none(), "{foreign}");
        let foreign = handle_request("finish_run", &finish, &stranger);
        assert_eq!(foreign["error_code"], "permission_denied", "{foreign}");
        let foreign = handle_request(
            "call",
            &json!({"workspace": fix.ws(), "run_id": run_id, "tool": "agents.ping",
                "input_json": "{}"}),
            &stranger,
        );
        assert_eq!(foreign["error_code"], "permission_denied", "{foreign}");
        assert!(find_run(fix.ws(), &run_id).unwrap().active);
        let retried = handle_request("spawn_run", &start, &owner);
        assert_eq!(retried["ok"], true, "{retried}");
        let auth = authenticate_run_token(retried["run_token"].as_str().unwrap()).unwrap();
        assert_eq!(auth.run_id, run_id);

        // Removing the seal does not turn the run into a pre-owner run.
        let path = runs_path(fix.ws());
        let sealed = fs::read_to_string(&path).unwrap();
        let mut stripped: RunRecord = serde_json::from_str(sealed.lines().last().unwrap()).unwrap();
        stripped.owner_seal = None;
        fs::write(&path, format!("{}\n", serde_json::to_string(&stripped).unwrap())).unwrap();
        let foreign = handle_request("finish_run", &finish, &stranger);
        assert_eq!(foreign["error_code"], "permission_denied", "stripped: {foreign}");
        fs::write(&path, &sealed).unwrap();

        // The owner's finish and its replay both answer.
        let finished = handle_request("finish_run", &finish, &owner);
        assert_eq!(finished["ok"], true, "{finished}");
        let replayed = handle_request("finish_run", &finish, &owner);
        assert_eq!(replayed["ok"], true, "{replayed}");
        assert_eq!(snapshot(), before, "run start and finish wrote the grant store");
    }

    /// A run whose owner cannot be sealed is never charged or recorded, so
    /// no line can exist without its owner.
    #[test]
    fn a_seal_is_in_the_same_line_as_its_run() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=ask"]);
        let start = json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-line"});
        let started = handle_request("spawn_run", &start, &pane_caller(42, None));
        assert_eq!(started["ok"], true, "{started}");
        let journal = fs::read_to_string(runs_path(fix.ws())).unwrap();
        assert_eq!(journal.lines().count(), 1);
        let line: RunRecord = serde_json::from_str(journal.trim()).unwrap();
        assert!(line.owner_seal.is_some(), "the run line carries its owner");
    }

    /// Codex 3290 r4 L1: a charged run that predates sealed owners is
    /// retried by a pane without the agent's allow. The bearer reissues and
    /// authenticates as that run, agent and workspace.
    #[test]
    fn a_retried_pre_owner_admission_reissues_its_bearer() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let workspace = crate::platform::path::canonical_or_self(fix.ws());
        let run = RunRecord {
            id: format!("run_{}", uuid::Uuid::new_v4()),
            agent_id: "worker".into(),
            actor_id: actor_of("worker"),
            parent_run: None,
            admission_id: "adm-upgrade".into(),
            version: 1,
            state: "running".into(),
            active: true,
            client_ref: "internal/unallocated".into(),
            kind: "system".into(),
            input_tokens: 0,
            output_tokens: 0,
            grants: Vec::new(),
            owner_seal: None,
        };
        ledger::append_result(&LedgerRow::for_agent_run(
            &run.id, &run.actor_id, &run.client_ref, &run.kind, None, 0, 0,
        ))
        .unwrap();
        append_run_record(&workspace, &run).unwrap();

        let pane = pane_caller(42, None);
        let fresh = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-new"}),
            &pane,
        );
        assert_eq!(fresh["ok"], false, "a fresh start still needs the allow: {fresh}");
        let start = json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-upgrade"});
        let retried = handle_request("spawn_run", &start, &pane);
        assert_eq!(retried["ok"], true, "{retried}");
        let token = retried["run_token"].as_str().expect("a bearer");
        let auth = authenticate_run_token(token).expect("the bearer authenticates");
        assert_eq!(auth.run_id, run.id);
        assert_eq!(auth.actor_id, "agent:worker");
        assert_eq!(auth.workspace, workspace);

        // A line added after the capture is not a pre-owner run.
        let mut planted = run.clone();
        planted.id = format!("run_{}", uuid::Uuid::new_v4());
        planted.admission_id = "adm-planted".into();
        ledger::append_result(&LedgerRow::for_agent_run(
            &planted.id, &planted.actor_id, &planted.client_ref, &planted.kind, None, 0, 0,
        ))
        .unwrap();
        append_run_record(&workspace, &planted).unwrap();
        let forged = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-planted"}),
            &pane,
        );
        assert_eq!(forged["error_code"], "permission_denied", "{forged}");
        assert!(forged.get("run_token").is_none(), "{forged}");
    }

    /// Approve one ask from `run` once and return that one-use row's id.
    fn approve_once(fix: &Fixture, run: &str, tool: &str) -> String {
        let asked = handle_request(
            "call",
            &json!({"workspace": fix.ws(), "run_id": run, "tool": tool, "input_json": "{}"}),
            &test_host(),
        );
        assert_eq!(asked["error_code"], "permission_required", "{asked}");
        monitor()
            .unwrap()
            .approve_pending(asked["pending_request_id"].as_str().unwrap(), ApprovalChoice::Once)
            .unwrap();
        monitor()
            .unwrap()
            .store()
            .records()
            .iter()
            .find(|rec| rec.target_id == tool && rec.duration == GrantDuration::Once && !rec.consumed)
            .expect("a one-use row")
            .grant_id
            .clone()
    }

    /// Stint 1716: a one-use row is claimed once, under the admission lock,
    /// before its effect: a second use of the same admission is refused.
    #[test]
    fn a_once_grant_is_claimed_atomically_at_use() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.review=ask"]);
        let spawned = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-claim"}),
            &test_host(),
        );
        let run = spawned["run"]["id"].as_str().unwrap().to_string();
        let grant = approve_once(&fix, &run, "agents.review");
        let gate = monitor().unwrap();
        let first = gate.note_use("agent:worker", "c1", &grant, "fp", "", "agents-api");
        let second = gate.note_use("agent:worker", "c2", &grant, "fp", "", "agents-api");
        assert!(first.is_ok(), "{first:?}");
        assert_eq!(second, Err("grant already used".to_string()));
    }

    /// Stint 1716: a launch is checked against what the run could execute,
    /// unused one-use rows included. A pane holding nothing cannot start an
    /// agent that would spend a person's one-use approval.
    #[test]
    fn a_launch_inventories_the_agents_one_use_rows() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.review=ask"]);
        let spawned = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-first"}),
            &test_host(),
        );
        let run = spawned["run"]["id"].as_str().unwrap().to_string();
        let grant = approve_once(&fix, &run, "agents.review");
        {
            let gate = monitor().unwrap();
            let mut store = gate.store();
            let row = store
                .records_mut()
                .iter_mut()
                .find(|rec| rec.grant_id == grant)
                .unwrap();
            // An approval not bound to one run.
            row.bound_operation_id = None;
        }
        handle_request("finish_run", &json!({"workspace": fix.ws(), "id": run}), &test_host());
        assert!(sealed_ceiling(fix.ws(), &Principal::agent("agent:worker"), None).is_empty());
        let launched = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-pane"}),
            &pane_caller(42, None),
        );
        assert_eq!(launched["ok"], false, "{launched}");
        assert!(launched.get("run_token").is_none(), "{launched}");
    }

    /// Stint 1716: a delegated row records both sides of the intersection
    /// that justified it. Revoking either disables the child immediately.
    #[test]
    fn revoking_either_side_of_a_delegation_disables_the_child() {
        for revoke_parent_side in [true, false] {
            let fix = Fixture::new();
            create(&fix, "lead", &["agents.ping=allow"]);
            let spawned = handle_request(
                "spawn_run",
                &json!({"workspace": fix.ws(), "agent": "lead", "admission": "adm-lead"}),
                &test_host(),
            );
            let parent_run = spawned["run"]["id"].as_str().unwrap().to_string();
            let workspace = crate::platform::path::canonical_or_self(fix.ws());
            record_scope_allow(
                &workspace,
                "pane:42",
                "agents.ping",
                GrantKind::Hard,
                None,
                GrantDuration::Always,
                None,
                &[],
            );
            let row_of = |actor: &str| {
                monitor()
                    .unwrap()
                    .store()
                    .records()
                    .iter()
                    .find(|rec| rec.actor_id == actor && rec.target_id == "agents.ping")
                    .unwrap()
                    .grant_id
                    .clone()
            };
            let (lead_row, pane_row) = (row_of("agent:lead"), row_of("pane:42"));
            let child = handle_request(
                "delegate",
                &json!({"workspace": fix.ws(), "parent_run": parent_run, "name": "scout",
                    "grants": ["agents.ping=allow"]}),
                &pane_caller(42, None),
            );
            assert_eq!(child["ok"], true, "{child}");
            let child_run = child["run"]["id"].as_str().unwrap().to_string();
            let child_row = monitor()
                .unwrap()
                .store()
                .records()
                .iter()
                .find(|rec| rec.actor_id == "agent:scout")
                .unwrap()
                .clone();
            assert_eq!(
                child_row.justified_by.iter().collect::<BTreeSet<_>>(),
                [&lead_row, &pane_row].into_iter().collect(),
            );
            let ping = || {
                handle_request(
                    "call",
                    &json!({"workspace": fix.ws(), "run_id": child_run, "tool": "agents.ping",
                        "input_json": "{}"}),
                    &test_host(),
                )
            };
            assert_eq!(ping()["ok"], true, "{}", ping());
            let revoked = if revoke_parent_side { &lead_row } else { &pane_row };
            assert!(monitor().unwrap().revoke_grant_id(revoked).unwrap());
            let after = ping();
            assert_ne!(after["ok"], true, "revoked {revoked}: {after}");
        }
    }

    /// Stint 1716: a run permit carries the admitted pane incarnation. A
    /// pane closed between admission and the effect finishes nothing and
    /// calls nothing.
    #[test]
    fn a_pane_revoked_between_admission_and_effect_reaches_no_run_sink() {
        let fix = Fixture::new();
        create(&fix, "worker", &["agents.ping=allow"]);
        let live = crate::broker::pane_authority::LivePaneAuthority::admitted(
            crate::broker::pane_authority::PaneAuthority::for_test(
                42,
                crate::broker::pane_authority::Ceiling::Baseline,
                1,
                fix.ws().to_path_buf(),
            ),
        );
        let subject = PermitSubject::principal("pane:42", Some(&live));
        let spawned = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-live"}),
            &test_host(),
        );
        let run_id = spawned["run"]["id"].as_str().unwrap().to_string();
        // A run this pane owns.
        let workspace = crate::platform::path::canonical_or_self(fix.ws());
        let mut run = find_run(&workspace, &run_id).unwrap();
        let start = permit::mint_start(
            &subject,
            &workspace,
            &run.agent_id,
            &run.admission_id,
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        run.owner_seal =
            Some(permit::seal_run_owner(&monitor().unwrap(), &start, binding_of(&workspace, &run)).unwrap());
        append_run_record(&workspace, &run).unwrap();

        let finish: RunPermit<Finish> = permit_for(&workspace, &run, &subject).unwrap();
        let call: RunPermit<Invoke> = permit_for(&workspace, &run, &subject).unwrap();
        live.revoke();
        let finished = finish_run(&finish);
        assert_eq!(finished["error_code"], "permission_denied", "{finished}");
        assert!(find_run(&workspace, &run_id).unwrap().active, "a revoked pane finished the run");
        let called = invoke(&call, "agents.ping", "{}", Some(42));
        assert_ne!(called["ok"], true, "a revoked pane called a tool: {called}");
        assert!(permit_for::<Finish>(&workspace, &run, &subject).is_err());
    }

    /// Design 1285 section 4, condition 3: the dashboard projection of one
    /// workspace lists only the pending requests whose stored binding names
    /// that workspace.
    #[test]
    fn dashboard_projection_lists_only_its_workspaces_pending_requests() {
        use crate::broker::gate::{Admission, AdmitRequest};
        let fix = Fixture::new();
        let other = tempfile::tempdir().unwrap();
        let ask = |workspace: &Path, marker: &str| {
            let Admission::Required { pending_request_id } = monitor().unwrap().admit(AdmitRequest {
                call_id: &format!("call-{marker}"),
                tool: "agents.review",
                input_json: &json!({"marker": marker}).to_string(),
                principal: &Principal::agent("agent:worker"),
                actor_scope: ActorScope::Workspace,
                trust_origin: "host",
                workspace_root: &crate::platform::path::canonical_or_self(workspace),
                context_id: 0,
                package_id: PACKAGE,
                instance_id: 0,
                target_type: TargetType::HostTool,
            }) else {
                panic!("expected a pending request");
            };
            pending_request_id
        };
        let mine = ask(fix.ws(), "alpha-1285");
        let theirs = ask(other.path(), "bravo-1285");
        let listed = |workspace: &Path| -> Vec<String> {
            projection(workspace)["needs_you"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["pending_request_id"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(listed(fix.ws()), vec![mine.clone()]);
        assert_eq!(listed(other.path()), vec![theirs.clone()]);
        let body = projection(fix.ws()).to_string();
        assert!(!body.contains("bravo-1285"), "another workspace's summary leaked: {body}");
    }

    /// Codex 3290 r1 P1: a delegated allow stops working when the parent's
    /// does. A parent soft allow that expires takes the child's row with it,
    /// though the child asked for a hard allow.
    #[test]
    fn a_delegated_allow_expires_with_its_parents_row() {
        let fix = Fixture::new();
        fs::write(
            crate::config::config_dir().join("config.toml"),
            "[permissions]\nsoft_grant_days = 10\n",
        )
        .unwrap();
        let t0: i64 = 1_700_000_000;
        monitor().unwrap().set_now_for_test(t0);
        create(&fix, "worker", &["agents.ping=allow:soft"]);
        let spawned = handle_request(
            "spawn_run",
            &json!({"workspace": fix.ws(), "agent": "worker", "admission": "adm-expiry"}),
            &test_host(),
        );
        let parent_run = spawned["run"]["id"].as_str().unwrap().to_string();
        let delegated = handle_request(
            "delegate",
            &json!({"workspace": fix.ws(), "parent_run": parent_run, "name": "scout",
                "grants": ["agents.ping=allow"]}),
            &test_host(),
        );
        assert_eq!(delegated["ok"], true, "{delegated}");
        let child_run = delegated["run"]["id"].as_str().unwrap().to_string();
        let ping = |run: &str| {
            handle_request(
                "call",
                &json!({"workspace": fix.ws(), "run_id": run, "tool": "agents.ping",
                    "input_json": "{}"}),
                &test_host(),
            )
        };
        assert_eq!(ping(&child_run)["ok"], true);
        monitor().unwrap().set_now_for_test(t0 + 10 * 86_400 + 1);
        assert_ne!(ping(&parent_run)["ok"], true, "the parent's soft allow expired");
        let child = ping(&child_run);
        assert_ne!(child["ok"], true, "the child outlived its parent's row: {child}");
    }

    #[test]
    fn soft_grant_expires_past_soft_grant_days_and_audits_while_hard_grant_persists() {
        let fix = Fixture::new();
        // Configure `soft_grant_days = 10` in the profile `config.toml`.
        fs::write(
            crate::config::config_dir().join("config.toml"),
            "[permissions]\nsoft_grant_days = 10\n",
        )
        .unwrap();

        let t0: i64 = 1_700_000_000;
        monitor().unwrap().set_now_for_test(t0);

        let created = handle_request(
            "create_agent",
            &json!({
                "workspace": fix.ws(),
                "name": "worker",
                "grants": ["agents.ping=allow:hard", "agents.review=allow:soft"],
            }), &test_host());
        assert_eq!(created["ok"], true, "{created}");

        let grants_before = handle_request("list_grants", &json!({"workspace": fix.ws()}), &test_host());
        assert_eq!(grants_before["ok"], true, "{grants_before}");
        let list_before = grants_before["grants"].as_array().unwrap();
        let soft_row = list_before
            .iter()
            .find(|g| g["tool"] == "agents.review")
            .expect("soft grant must be listed");
        assert_eq!(soft_row["kind"], "soft");
        assert_eq!(soft_row["decision"], "allow");
        assert_eq!(soft_row["expires_at"], t0 + 10 * 86_400);
        let hard_row = list_before
            .iter()
            .find(|g| g["tool"] == "agents.ping")
            .expect("hard grant must be listed");
        assert_eq!(hard_row["kind"], "hard");
        assert_eq!(hard_row["decision"], "allow");
        assert!(hard_row["expires_at"].is_null());

        let spawned = handle_request(
            "spawn_run",
            &json!({
                "workspace": fix.ws(),
                "agent": "worker",
                "admission": "adm-soft",
            }), &test_host());
        let run_id = spawned["run"]["id"].as_str().unwrap();

        // Before expiry, both hard and soft grants allow without prompting.
        let review_ok = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": run_id,
                "tool": "agents.review",
                "input_json": "{}",
            }), &test_host());
        assert_eq!(review_ok["ok"], true, "{review_ok}");

        // Advance injected clock past 10 days (`soft_grant_days`).
        monitor().unwrap().set_now_for_test(t0 + 10 * 86_400 + 1);

        // Soft grant reverts to `ask` (`permission_required`) and writes an `expire` audit row.
        let review_after = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": run_id,
                "tool": "agents.review",
                "input_json": "{}",
            }), &test_host());
        assert_eq!(
            review_after["error_code"], "permission_required",
            "expired soft grant must revert to ask: {review_after}"
        );

        // Hard grant never expires.
        let ping_after = handle_request(
            "call",
            &json!({
                "workspace": fix.ws(),
                "run_id": run_id,
                "tool": "agents.ping",
                "input_json": "{}",
            }), &test_host());
        assert_eq!(
            ping_after["ok"], true,
            "hard grant must not expire: {ping_after}"
        );

        // Audit log contains the soft expiry row (`kind: "expire"`, `decision: "ask"`).
        let audit_path = crate::config::config_dir().join("permission-audit.jsonl");
        let audit_raw = fs::read_to_string(&audit_path).unwrap();
        assert!(
            audit_raw.contains("expire") && audit_raw.contains("agents.review"),
            "expected expire audit row in permission-audit.jsonl, got:\n{audit_raw}"
        );
        let audit_rows = monitor().unwrap().audit_records();
        assert!(
            audit_rows.iter().any(|row| {
                row.kind == "expire" && row.resource_id == "agents.review" && row.decision == "ask"
            }),
            "expected expire AuditFact in monitor().unwrap().audit_records(), got: {audit_rows:?}"
        );

        // `list_grants` and `agent.json` reflect the reverted `ask` decision.
        let grants_after = handle_request("list_grants", &json!({"workspace": fix.ws()}), &test_host());
        let soft_after = grants_after["grants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["tool"] == "agents.review")
            .unwrap();
        assert_eq!(soft_after["decision"], "ask");
        assert!(soft_after["expires_at"].is_null());
    }

    /// Stint 1036: `head.json` was renamed to `agent.json` in code but never on
    /// disk, which made every pre-rename card — `~/.plexi/agents/lead-120cadfb`
    /// is the live one — unreachable with `no agent '<id>'`.
    #[test]
    fn legacy_head_json_card_is_migrated_then_found() {
        let fix = Fixture::new();
        create(&fix, "lead-120cadfb", &["agents.ping=allow"]);
        let dir = agents_root(fix.ws()).join("lead-120cadfb");
        fs::rename(dir.join("agent.json"), dir.join("head.json")).unwrap();

        let card = find_agent(fix.ws(), "lead-120cadfb").expect("the legacy authority record resolves");
        assert_eq!(card.id, "lead-120cadfb");
        assert_eq!(card.grants[0].tool, "agents.ping");
        assert!(
            dir.join("agent.json").is_file() && !dir.join("head.json").exists(),
            "head.json is migrated in place, not read under its old name"
        );
        let listed = handle_request("list_agents", &json!({"workspace": fix.ws()}), &test_host());
        assert_eq!(listed["agents"][0]["id"], "lead-120cadfb", "{listed}");
    }

    /// Stint 1036: a directory the loader does not understand is named, not
    /// answered with the same `no agent` as a directory that is not there.
    #[test]
    fn find_agent_reports_an_unrecognised_directory_by_path() {
        let fix = Fixture::new();
        let dir = agents_root(fix.ws()).join("writer");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("AGENT.md"), "pane-agent prose\n").unwrap();

        let error = find_agent(fix.ws(), "writer").unwrap_err();
        assert_eq!(error["error_code"], "agent_unrecognised", "{error}");
        let message = error["error"].as_str().unwrap();
        assert!(message.contains("settings.toml"), "{error}");
    }

    /// Stint 1036: a name with no directory at all keeps the old answer.
    #[test]
    fn find_agent_still_reports_a_missing_directory_as_not_found() {
        let fix = Fixture::new();
        let error = find_agent(fix.ws(), "nobody").unwrap_err();
        assert_eq!(error["error_code"], "agent_not_found", "{error}");
        assert_eq!(error["error"], "no agent 'nobody'");
    }
}
