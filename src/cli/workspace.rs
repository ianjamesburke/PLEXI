use super::open::read_secret_from_stdin;
use super::print_tip;
use super::reply::OutputMode;
use serde_json::Value;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn workspace_init() -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };
    log::info!("workspace_init:cli: cwd={}", cwd.display());
    if let Err(error) = crate::host::anchor::check_workspace_init_root(&cwd) {
        log::warn!("workspace_init:cli: {error}");
        eprintln!("error: {error}");
        eprintln!("  cd into a project directory first.");
        return 1;
    }
    let channel_dir = crate::config::workspace_channel_dir();
    match crate::workspace::secrets::init_workspace(&cwd, &channel_dir) {
        Ok(cfg) => {
            log::info!(
                "workspace_init:cli: initialized workspace_id={} at {} channel_dir={}",
                cfg.id,
                cwd.display(),
                channel_dir
            );
            println!("Initialized workspace at {}", cwd.display());
            println!("  workspace id: {}", cfg.id);
            println!("  channel dir:  {channel_dir}/");
            println!("  router:       {channel_dir}/secrets.toml (fallback = true)");
            println!(
                "  plugins:         {channel_dir}/plugins.toml (declare plugin dependencies here)"
            );
            println!("  commands:     {channel_dir}/commands.toml (run commands with: plexi run)");
            print_tip(&format!("declare plugin dependencies in {channel_dir}/plugins.toml, then run `plexi plugin install`."));
            0
        }
        Err(e) => {
            log::warn!("workspace_init:cli: init_workspace failed: {e}");
            eprintln!("error: workspace init failed: {e}");
            1
        }
    }
}

pub fn workspace_clean_cli(dry_run: bool, mode: OutputMode) -> i32 {
    log::info!("workspace_clean:cli: dry_run={dry_run}");
    let content = match super::request(
        serde_json::json!({
            "type": "workspace_clean_slots",
            "dry_run": dry_run,
        }),
        "workspace-clean-response",
        "workspace clean",
        mode,
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    let value = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => value,
        Err(e) => {
            eprintln!("error: invalid workspace clean response: {e}");
            return 1;
        }
    };
    let paths = value
        .get("paths")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if paths.is_empty() {
        println!("No stale pane slot files found.");
        return 0;
    }
    for path in paths {
        if let Some(path) = path.as_str() {
            println!("{path}");
        }
    }
    0
}

// ── plexi secret subcommands (workspace-aware, issue #322) ───────────────────

#[cfg(test)]
thread_local! {
    static TEST_PANE_CONTEXT_ROOT: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
    static TEST_SCOPE_REPLY: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
struct TestScopeReplyGuard(Option<Value>);

#[cfg(test)]
impl Drop for TestScopeReplyGuard {
    fn drop(&mut self) {
        TEST_SCOPE_REPLY.set(self.0.take());
    }
}

#[cfg(test)]
fn set_test_scope_reply(reply: Value) -> TestScopeReplyGuard {
    TestScopeReplyGuard(TEST_SCOPE_REPLY.replace(Some(reply)))
}

#[cfg(test)]
pub(crate) struct TestPaneContextGuard(Option<std::path::PathBuf>);

#[cfg(test)]
impl Drop for TestPaneContextGuard {
    fn drop(&mut self) {
        let prev = self.0.take();
        TEST_PANE_CONTEXT_ROOT.with(|cell| *cell.borrow_mut() = prev);
    }
}

#[cfg(test)]
pub(crate) fn set_test_pane_context_root(path: &std::path::Path) -> TestPaneContextGuard {
    let prev = TEST_PANE_CONTEXT_ROOT.with(|cell| cell.borrow_mut().replace(path.to_path_buf()));
    TestPaneContextGuard(prev)
}

/// The calling pane's live context authority, asked of the host from the
/// kernel-verified socket peer. Outer `None`: the host could not be asked.
/// `Some(None)`: the caller is outside every pane (a human at a plain terminal).
#[cfg(not(test))]
fn host_caller_context() -> Option<Option<std::path::PathBuf>> {
    static CACHE: std::sync::OnceLock<Option<Option<std::path::PathBuf>>> =
        std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let socket = super::resolve_command_socket()?;
            let reply = crate::rpc::request_json(
                &socket,
                serde_json::json!({"type": "caller_context"}),
                std::time::Duration::from_secs(3),
            )
            .ok()?;
            match super::reply::HostReply::decode(Ok(reply.to_string())) {
                super::reply::HostReply::Payload(_) => {}
                super::reply::HostReply::Refused(refusal) => {
                    log::info!(
                        "secret scope: host refused caller context: {}",
                        refusal.coded_reason()
                    );
                    return None;
                }
                super::reply::HostReply::Transport(_) => return None,
            }
            if !super::reply::acknowledged(&reply) {
                return None;
            }
            Some(
                reply
                    .get("context_root")
                    .and_then(|v| v.as_str())
                    .map(std::path::PathBuf::from),
            )
        })
        .clone()
}

pub(crate) fn active_pane_context_root() -> Option<std::path::PathBuf> {
    #[cfg(test)]
    let raw = TEST_PANE_CONTEXT_ROOT.with(|cell| cell.borrow().clone())?;
    // The host owns the live router and the verified peer identity. The
    // environment is only a fallback when no host answers: it goes stale after
    // `context set-root` and a child can unset it.
    #[cfg(not(test))]
    let raw = match host_caller_context() {
        Some(root) => root?,
        None => std::env::var("PLEXI_CONTEXT_ROOT")
            .ok()
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)?,
    };
    let raw_canon = crate::platform::path::canonical_or_self(&raw);
    let ws_root = crate::app::registry::resolve_workspace_root(&raw_canon)
        .map(|p| crate::platform::path::canonical_or_self(&p));
    Some(match ws_root {
        Some(ws) if raw_canon.starts_with(&ws) && raw_canon != ws => raw_canon,
        Some(ws) => ws,
        None => raw_canon,
    })
}

/// Enforce that `target_dir` falls within the calling pane's context root.
///
/// Subdirectory contexts are isolated units: a pane in context A has no
/// authority over context B, including when B is nested inside A's directory.
pub(crate) fn enforce_pane_context_scope(
    target_dir: &std::path::Path,
    op: &str,
) -> Result<(), String> {
    let Some(ctx_root) = active_pane_context_root() else {
        return Ok(());
    };
    log::info!(
        "secrets: {op} checking pane context scope for {}",
        target_dir.display()
    );
    let socket = super::resolve_secret_socket();
    let unconfirmed = |error: &str| {
        log::warn!("secrets: {op} caller context at {socket:?}: {error}");
        format!(
            "permission_denied: host could not confirm this pane's context scope for {}",
            target_dir.display()
        )
    };
    #[cfg(not(test))]
    let reply = request_workspace_scope(&socket, target_dir).map_err(|e| unconfirmed(&e))?;
    #[cfg(test)]
    let reply = TEST_SCOPE_REPLY
        .with_borrow(Clone::clone)
        .unwrap_or_else(|| {
            let error =
                crate::app::context_scope::check_context_scope(ctx_root.clone(), target_dir).err();
            serde_json::json!({"ok":true, "workspace_error":error})
        });
    if !confirmed_workspace_scope(&reply) {
        return Err(unconfirmed("missing or invalid scope confirmation"));
    }
    if let Some(reason) = reply["workspace_error"].as_str() {
        let target_canon = crate::platform::path::canonical_or_self(target_dir);
        let actor = std::env::var("PLEXI_PANE_ID")
            .ok()
            .filter(|id| !id.is_empty())
            .map(|id| format!("pane:{id}"))
            .unwrap_or_else(|| "pane:context".to_string());
        let call_id = format!("{op}-{}", uuid::Uuid::new_v4());
        let resource = target_canon.display().to_string();
        audit_folder_denial(&actor, &resource, &call_id);
        log::info!(
            "secrets: {op} refused target={} outside pane context root {}",
            target_canon.display(),
            ctx_root.display()
        );
        return Err(reason.to_owned());
    }
    Ok(())
}

/// Keep secret lookup inside the pane's context while reusing the registry's
/// channel and home-directory exclusions. Never reinterpret a parent profile
/// as this pane's workspace (the H8 request walked from ~/temp into $HOME).
pub(crate) fn secret_workspace_root(
    cwd: &std::path::Path,
    context: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let cwd = crate::platform::path::canonical_or_self(cwd);
    let root = crate::app::registry::resolve_workspace_root(&cwd)?;
    let root = crate::platform::path::canonical_or_self(&root);
    if let Some(context) = context {
        let context = crate::platform::path::canonical_or_self(context);
        if !cwd.starts_with(&context) || !root.starts_with(&context) {
            return None;
        }
    }
    Some(root)
}

fn secret_not_set(name: &str) -> String {
    let directory = crate::config::workspace_channel_dir();
    let binary = directory
        .strip_prefix(".plexi-")
        .map(|channel| format!("plexi-{channel}"))
        .unwrap_or_else(|| "plexi".into());
    format!("{name} not set; run `{binary} secret set {name}`")
}

fn request_workspace_scope(socket: &Path, cwd: &Path) -> Result<Value, String> {
    // Relative paths belong to the caller's cwd. The host may have a different one.
    let cwd = std::path::absolute(cwd).map_err(|e| e.to_string())?;
    let cwd = cwd.to_str().ok_or("workspace path is not UTF-8")?;
    let request = serde_json::json!({"type":"caller_context", "workspace_cwd":cwd});
    crate::rpc::request_json(socket, request, Duration::from_secs(3)).map_err(|e| e.to_string())
}

fn confirmed_workspace_scope(reply: &Value) -> bool {
    super::reply::acknowledged(reply)
        && matches!(
            reply.get("workspace_error"),
            Some(Value::Null | Value::String(_))
        )
}

/// The host checks the cwd against its verified pane (including nested folder
/// secrets). Only workspace metadata is read in the CLI.
fn workspace_caller_context(cwd: &Path, op: &str) -> Result<Value, i32> {
    let socket = super::resolve_secret_socket();
    let reply = request_workspace_scope(&socket, cwd).map_err(|error| {
        log::warn!("secret {op}: caller context at {socket:?}: {error}");
        no_secret_reply(&format!("secret {op}"))
    })?;
    if !confirmed_workspace_scope(&reply) {
        eprintln!("error: secret {op}: host could not confirm the caller's workspace scope");
        return Err(1);
    }
    Ok(reply)
}

fn require_workspace(cwd: &Path, scope: &Value) -> Result<PathBuf, String> {
    let context = scope["context_root"].as_str().map(Path::new);
    if let Some(error) = scope["workspace_error"].as_str() {
        return Err(error.into());
    }
    let directory = crate::config::workspace_channel_dir();
    let root = secret_workspace_root(cwd, context).ok_or_else(|| {
        format!(
            "no {directory}/ workspace found at or above {}.\nRun `plexi workspace init` first.",
            cwd.display()
        )
    })?;
    if context.is_some_and(|context| root != context) {
        return Err(format!(
            "permission_denied: pane has no authority over context '{}'",
            root.display()
        ));
    }
    crate::workspace::secrets::WorkspaceConfig::load(&root)?
        .ok_or_else(|| format!("{directory}/ exists at {} but workspace.toml is missing.\nRun `plexi workspace init` to create it.", root.display()))?;
    Ok(root)
}

/// Send the workspace path only. The host resolves its id and authorizes the operation.
fn secret_request(op: &str, name: &str, global: bool) -> Result<Value, i32> {
    let workspace = if global {
        None
    } else {
        Some(match std::env::current_dir() {
            Ok(cwd) => require_workspace(&cwd, &workspace_caller_context(&cwd, op)?),
            Err(error) => Err(format!("cwd: {error}")),
        })
    };
    secret_request_for_workspace(op, name, workspace)
}

fn secret_request_for_workspace(
    op: &str,
    name: &str,
    workspace: Option<Result<PathBuf, String>>,
) -> Result<Value, i32> {
    let global = workspace.is_none();
    let root = match workspace {
        None => None,
        Some(Ok(root)) => Some(root),
        Some(Err(error)) if op == "get" && error.starts_with("permission_denied") => {
            eprintln!("error: {error}");
            println!("permission_denied");
            return Err(1);
        }
        Some(Err(_)) if op == "get" => None,
        Some(Err(error)) => {
            if op == "set" {
                log::warn!("secret_set:cli: no workspace found: {error}");
                eprintln!("error: no .plexi/ workspace found in this directory tree.");
                eprintln!("  → plexi workspace init        (initialize one here, then retry)");
                eprintln!("  → plexi secret set --global {name}   (set globally — requires explicit flag)");
            } else {
                eprintln!("error: {error}");
            }
            return Err(1);
        }
    };
    log::info!("secret_{op}:cli: asking host name={name} global={global} workspace={root:?}");
    Ok(
        serde_json::json!({"type":format!("secret_{op}"), "name":name, "global":global, "workspace_root":root}),
    )
}

const WAIT_NOTICE_DELAY: Duration = Duration::from_millis(700);

fn no_secret_reply(label: &str) -> i32 {
    eprintln!(
        "error: {label}: the Plexi host did not answer at {}; start Plexi and try again.",
        super::resolve_secret_socket().display()
    );
    1
}

fn receive_secret<T>(
    label: &str,
    payload: Value,
    decode: impl FnOnce(Result<String, crate::rpc::PollError>) -> T,
) -> Result<T, i32> {
    #[cfg(test)]
    if let Some(reply) = secret_set_tests::BUNDLE_RESPONSE.take() {
        return Ok(decode(Ok(reply(&payload))));
    }
    let (token, stream) = super::send_request_connected(payload, "secret-response", label)
        .map_err(|_| no_secret_reply(label))?;
    let timeout = crate::workspace::secrets::APPROVAL_TIMEOUT + Duration::from_secs(10);
    let reply = std::thread::scope(|scope| {
        let (done, timer) = std::sync::mpsc::channel::<()>();
        scope.spawn(move || {
            if timer.recv_timeout(WAIT_NOTICE_DELAY)
                == Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            {
                eprintln!("Waiting for approval in the Plexi window (Needs you)...");
            }
        });
        let body = crate::rpc::read_secret_reply(&token, stream, timeout);
        drop(done);
        body
    });
    Ok(decode(reply))
}

pub(crate) enum GlobalSecret {
    Value(zeroize::Zeroizing<String>),
    NotSet,
    Unavailable(String),
}

pub(crate) fn global_secret_lookup(name: &str) -> GlobalSecret {
    use super::reply::HostReply;
    use zeroize::Zeroizing;

    #[derive(serde::Deserialize)]
    #[serde(tag = "status", rename_all = "snake_case")]
    enum Payload {
        Value { value: String },
        NotFound,
    }

    let reply = secret_request("get", name, true)
        .and_then(|payload| receive_secret("secret get", payload, HostReply::decode));
    let outcome = match reply {
        Ok(HostReply::Payload(body)) => match serde_json::from_str(&Zeroizing::new(body)) {
            Ok(Payload::Value { value }) => GlobalSecret::Value(Zeroizing::new(value)),
            Ok(Payload::NotFound) => GlobalSecret::NotSet,
            Err(_) => GlobalSecret::Unavailable("invalid secret reply".into()),
        },
        Ok(HostReply::Refused(refusal)) if refusal.fields["status"] == "denied" => {
            let reason = match refusal.fields["reason"].as_str() {
                Some("expired") => "expired",
                Some("revoked") => "revoked",
                _ => "denied",
            };
            GlobalSecret::Unavailable(reason.into())
        }
        Ok(HostReply::Refused(refusal)) => GlobalSecret::Unavailable(refusal.coded_reason()),
        Ok(HostReply::Transport(error)) => GlobalSecret::Unavailable(error.to_string()),
        Err(_) => GlobalSecret::Unavailable("host unavailable".into()),
    };
    let kind = match &outcome {
        GlobalSecret::Value(_) => "value",
        GlobalSecret::NotSet => "not_found",
        GlobalSecret::Unavailable(_) => "unavailable",
    };
    log::info!("secret_get:cli: name={name} global=true outcome={kind}");
    outcome
}

fn send_secret(op: &str, name: &str, payload: Value) -> Result<Value, i32> {
    use super::reply::HostReply;
    let label = format!("secret {op}");
    let reply = receive_secret(&label, payload, HostReply::decode)?;
    let unconfirmed = || {
        if matches!(op, "get" | "read") {
            eprintln!("No value was received from Plexi");
        } else {
            eprintln!("Plexi did not confirm the outcome");
        }
        1
    };
    match reply {
        HostReply::Transport(_) => Err(unconfirmed()),
        HostReply::Refused(refusal) if refusal.fields["status"] == "denied" => {
            match refusal.fields["reason"].as_str() {
                Some("denied") => eprintln!("Request denied"),
                Some("expired") => eprintln!("Approval expired"),
                Some("revoked") => eprintln!("Caller revoked"),
                _ => return Err(refusal.report(OutputMode::Text)),
            }
            Err(1)
        }
        reply => {
            let body = reply.payload_or_report(&label, OutputMode::Text)?;
            let json = serde_json::from_str(&body).map_err(|_| unconfirmed())?;
            secret_result(op, name, json)
        }
    }
}

fn secret_result(op: &str, name: &str, reply: Value) -> Result<Value, i32> {
    match (reply["status"].as_str(), op) {
        (Some("value"), "get" | "read") if reply["value"].is_string() => Ok(reply),
        (Some("ok"), "set" | "delete" | "rm") => Ok(reply),
        (Some("not_found"), "read") => {
            eprintln!("error: secret '{name}' not found");
            Err(1)
        }
        (Some("not_found"), _) => {
            eprintln!("error: {}", secret_not_set(name));
            Err(1)
        }
        _ => {
            eprintln!("error: unexpected secret {op} response");
            Err(1)
        }
    }
}

pub fn workspace_secret_set(
    friendly: &str,
    from_env: bool,
    global: bool,
    alias: Option<&str>,
) -> i32 {
    if global && alias.is_some() {
        eprintln!("error: --alias cannot be combined with --global.");
        return 1;
    }
    let mut request = match secret_request("set", friendly, global) {
        Ok(request) => request,
        Err(code) => return code,
    };
    request["name"] = alias.unwrap_or(friendly).into();
    request["route_name"] = alias.map(|_| friendly).into();
    request["value"] = match read_secret_value(friendly, from_env) {
        Ok(value) => value.as_str().into(),
        Err(code) => return code,
    };
    let reply = match send_secret("set", friendly, request) {
        Ok(reply) => reply,
        Err(code) => return code,
    };
    let channel_dir = crate::config::workspace_channel_dir();
    if global {
        println!("Stored '{friendly}' globally (plexi:user:{friendly})");
        print_tip(&format!(
            "reference '{friendly}' in {channel_dir}/commands.toml (or {}) under `[secrets] required = [\"{friendly}\"]`.",
            super::global_commands_path().display()
        ));
    } else {
        let Some(root) = reply["target"]["Workspace"]["root"].as_str() else {
            eprintln!("error: secret set reply is missing the workspace root");
            return 1;
        };
        let id = reply["target"]["Workspace"]["id"]
            .as_str()
            .unwrap_or_default();
        eprintln!("Stored '{friendly}' for workspace {root} ({id})");
        print_tip(&format!("Route written to {channel_dir}/secrets.toml — use in {channel_dir}/commands.toml: [secrets] required = [\"{friendly}\"]"));
        print_tip(&format!("'{friendly}' is not added to terminal environments. Agent panes never receive workspace secrets; a pane reads it with plexi secret get after you click Allow."));
        match crate::workspace::secrets::WorkspaceSecrets::load(Path::new(root)) {
            Ok(Some(config)) if !config.terminal.env.inject.is_empty() => eprintln!(
                "Still injected into human terminal panes by {channel_dir}/secrets.toml: {}. Turn injection off in Secrets (Cmd+Shift+S).",
                config.terminal.env.inject.join(", ")
            ),
            Err(error) => eprintln!("warning: could not read terminal injection settings: {error}"),
            _ => {}
        }
    }
    0
}

/// `plexi secret list` — list friendly names. Inside a workspace, includes
/// workspace namespace plus user-scope. Outside a workspace, gracefully falls
/// back to user-scope. Names only, never values.
pub fn workspace_secret_list(
    global: bool,
    context: Option<&super::ContextTarget>,
    mode: OutputMode,
) -> i32 {
    log::info!("secret_list:cli: global={global} json={}", mode.is_json());
    if let Some(context) = context {
        return explicit_context_secret_list(context, mode);
    }
    let result = std::env::current_dir()
        .map_err(|error| format!("current directory: {error}"))
        .and_then(|cwd| {
            write_secret_list(&cwd, global, mode.is_json(), &mut std::io::stdout().lock())
        });
    match result {
        Ok(()) => 0,
        Err(error) => {
            log::warn!("secret_list: failed: {error}");
            eprintln!("error: {error}");
            1
        }
    }
}

fn explicit_context_secret_list(context: &super::ContextTarget, mode: OutputMode) -> i32 {
    let reply = match super::request(
        serde_json::json!({"type": "pane_secret_list", "context": context.as_str()}),
        "secret-list-context",
        "context secret inventory",
        mode,
    ) {
        Ok(reply) => reply,
        Err(code) => return code,
    };
    let result = serde_json::from_str::<serde_json::Value>(&reply)
        .map_err(|e| e.to_string())
        .and_then(|body| {
            let inventory = serde_json::from_value(body).map_err(|e| e.to_string())?;
            write_secret_inventory(&inventory, mode.is_json(), &mut std::io::stdout().lock())
        });
    match result {
        Ok(()) => 0,
        Err(error) => {
            log::warn!("secret_list: {error}");
            eprintln!("error: {error}");
            1
        }
    }
}

fn write_secret_list(
    cwd: &std::path::Path,
    global: bool,
    json: bool,
    output: &mut impl std::io::Write,
) -> Result<(), String> {
    let inventory = crate::workspace::secrets::manager::secret_inventory_report(cwd, global)?;
    write_secret_inventory(&inventory, json, output)
}

fn write_secret_inventory(
    inventory: &crate::workspace::secrets::manager::SecretInventory,
    json: bool,
    output: &mut impl std::io::Write,
) -> Result<(), String> {
    use crate::workspace::secrets::manager::{SecretTarget, SkippedSecrets};
    let entries = &inventory.entries;
    if json {
        let rendered = serde_json::to_string_pretty(inventory).map_err(|e| e.to_string())?;
        return writeln!(output, "{rendered}").map_err(|e| e.to_string());
    }
    if entries.is_empty() {
        writeln!(output, "No secrets stored.").map_err(|e| e.to_string())?;
    }
    if let Some(SecretTarget::Workspace { id, root }) = entries.iter().find_map(|entry| {
        matches!(entry.target, SecretTarget::Workspace { .. }).then_some(&entry.target)
    }) {
        let injected = crate::workspace::secrets::WorkspaceSecrets::load(root)?
            .map(|router| router.terminal.env.inject)
            .unwrap_or_default();
        writeln!(output, "Workspace ({id}):").map_err(|e| e.to_string())?;
        for entry in entries {
            if matches!(entry.target, SecretTarget::Workspace { .. }) {
                let annotation = if injected.contains(&entry.name) {
                    "  (injected into human terminal panes)"
                } else {
                    ""
                };
                writeln!(output, "  {}{annotation}", entry.name).map_err(|e| e.to_string())?;
            }
        }
        writeln!(output).map_err(|e| e.to_string())?;
    }
    if entries
        .iter()
        .any(|entry| entry.target == SecretTarget::Global)
    {
        writeln!(output, "User scope:").map_err(|e| e.to_string())?;
        for entry in entries {
            if entry.target == SecretTarget::Global {
                writeln!(output, "  {}", entry.name).map_err(|e| e.to_string())?;
            }
        }
    }
    let mut folders = entries
        .iter()
        .filter_map(|entry| match &entry.target {
            SecretTarget::Folder(folder) => Some((&entry.name, folder)),
            _ => None,
        })
        .peekable();
    if folders.peek().is_some() {
        let backend = crate::workspace::secrets::backend_label();
        eprintln!("folder secrets backend: {backend}");
        writeln!(output, "Folder scope:").map_err(|e| e.to_string())?;
        for (name, folder) in folders {
            writeln!(output, "  {name}  {}", folder.display()).map_err(|e| e.to_string())?;
        }
        log::info!("folder_secrets: listed inventory backend={backend}");
    }
    if let Some(note) = &inventory.grant_details_unavailable {
        writeln!(output, "{note}").map_err(|e| e.to_string())?;
    }
    for skipped in &inventory.skipped {
        match skipped {
            SkippedSecrets::OutsidePaneContext {
                path,
                pane_context_root,
                pane_context_name,
                ..
            } => {
                writeln!(output, "Skipped workspace secrets for {}: this pane's authority is scoped to {} (context {:?}). Use --context <name|id|root>, --global, or cd into that context.", path.display(), pane_context_root.display(), pane_context_name.as_deref().unwrap_or("unknown")).map_err(|e| e.to_string())?;
            }
            SkippedSecrets::ExperimentDisabled {
                count, experiment, ..
            } => {
                writeln!(output, "{count} folder secret(s) hidden: {experiment} experiment is off (plexi experiments enable {experiment})").map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn request_secret_once(op: &str, name: &str, global: bool) -> Result<serde_json::Value, i32> {
    send_secret(op, name, secret_request(op, name, global)?)
}

pub fn workspace_secret_get(friendly: &str, global: bool) -> i32 {
    match request_secret_once("get", friendly, global) {
        Ok(reply) => {
            println!("{}", reply["value"].as_str().expect("validated get reply"));
            0
        }
        Err(code) => code,
    }
}

pub fn workspace_secret_delete(friendly: &str, global: bool) -> i32 {
    match request_secret_once("delete", friendly, global) {
        Ok(reply) => {
            if global {
                println!("Deleted global secret '{friendly}'");
            } else if let Some(id) = reply["target"]["Workspace"]["id"].as_str() {
                println!("Deleted '{friendly}' from workspace {id}");
            } else {
                eprintln!("error: secret delete reply is missing the workspace id");
                return 1;
            }
            0
        }
        Err(code) => code,
    }
}

fn read_secret_value(name: &str, from_env: bool) -> Result<zeroize::Zeroizing<String>, i32> {
    let value = if from_env {
        match std::env::var(name) {
            Ok(value) => value,
            Err(_) => {
                log::warn!("folder_secrets: env var {name} is not set");
                eprintln!("error: env var {name} is not set");
                return Err(1);
            }
        }
    } else {
        eprint!("Enter value for {name}: ");
        let _ = io::stderr().flush();
        match read_secret_from_stdin() {
            Ok(value) => value,
            Err(err) => {
                log::warn!("folder_secrets: stdin read failed: {err}");
                eprintln!("\nerror: failed to read secret: {err}");
                return Err(1);
            }
        }
    };
    if value.is_empty() {
        eprintln!("error: empty value, nothing stored");
        return Err(1);
    }
    Ok(zeroize::Zeroizing::new(value))
}

/// The CLI refuses, logs, and writes no audit row: the sealed audit belongs
/// to the host process (stint 1291).
fn audit_folder_denial(actor: &str, resource: &str, call_id: &str) {
    log::info!(
        "folder_secrets: refused actor={actor} resource={resource} call_id={call_id}; not audited, the CLI process does not write the sealed audit"
    );
}

fn actor_from_flags(
    agent: Option<&str>,
    app: Option<&str>,
) -> Result<crate::workspace::secrets::SecretActor, i32> {
    match (agent, app) {
        (Some(id), None) => Ok(crate::workspace::secrets::SecretActor::agent(id)),
        (None, Some(id)) => Ok(crate::workspace::secrets::SecretActor::app(id)),
        _ => {
            eprintln!("error: pass --agent <id> or --plugin <id>");
            Err(1)
        }
    }
}

/// Relative flags belong to the caller's cwd. The host canonicalizes the
/// absolute path before checking the caller's authority.
fn secret_folder_path(path: &Path) -> Result<String, i32> {
    let absolute = std::path::absolute(path).map_err(|error| {
        eprintln!("error: folder {}: {error}", path.display());
        1
    })?;
    absolute.into_os_string().into_string().map_err(|_| {
        eprintln!("error: folder path must be valid UTF-8");
        1
    })
}

fn folder_feature_available() -> bool {
    let feature = crate::release::ReleaseFeature::FolderSecrets;
    if crate::release::feature_enabled(feature) {
        return true;
    }
    eprintln!("{}", crate::release::feature_unavailable_message(feature));
    false
}

/// `plexi secret set NAME --folder <path>`.
pub fn folder_secret_set(
    name: &str,
    from_env: bool,
    global: bool,
    alias: Option<&str>,
    folder: &std::path::Path,
) -> i32 {
    if !folder_feature_available() {
        return 1;
    }
    if global {
        eprintln!("error: --folder cannot be combined with --global");
        return 1;
    }
    if alias.is_some() {
        eprintln!("error: --folder cannot be combined with --alias");
        return 1;
    }
    let folder = match secret_folder_path(folder) {
        Ok(path) => path,
        Err(code) => return code,
    };
    if !crate::workspace::secrets::manager::valid_env_name(name) {
        eprintln!("error: secret name '{name}' is not an environment variable name");
        return 1;
    }
    let value = match read_secret_value(name, from_env) {
        Ok(value) => value,
        Err(code) => return code,
    };
    let request = serde_json::json!({"type":"folder_secret_set", "name":name, "folder":folder, "value":value.as_str()});
    match send_secret("set", name, request) {
        Ok(reply) => {
            let (Some(path), Some(backend)) = (
                reply["target"]["Folder"].as_str(),
                reply["backend"].as_str(),
            ) else {
                eprintln!("error: secret set reply is missing the folder or backend");
                return 1;
            };
            println!("Stored '{name}' for {path} ({backend})");
            0
        }
        Err(code) => code,
    }
}

/// `plexi secret rm NAME --folder <path>`.
pub fn folder_secret_rm(name: &str, folder: &Path) -> i32 {
    if !folder_feature_available() {
        return 1;
    }
    let folder = match secret_folder_path(folder) {
        Ok(path) => path,
        Err(code) => return code,
    };
    let request = serde_json::json!({"type":"folder_secret_rm", "name":name, "folder":folder});
    match send_secret("rm", name, request) {
        Ok(_) => {
            println!("Removed '{name}'");
            0
        }
        Err(code) => code,
    }
}

/// `plexi secret grant NAME --agent <id>`.
///
/// The CLI holds no authority to allow a read, so it refuses every valid
/// request with `permission_denied` without discovering a folder or opening a
/// secret store. The host admits a read on its own pending request in Needs you.
pub fn folder_secret_grant(
    name: &str,
    agent: Option<&str>,
    app: Option<&str>,
    folder: Option<&std::path::Path>,
) -> i32 {
    if !folder_feature_available() {
        return 1;
    }
    let actor = match actor_from_flags(agent, app) {
        Ok(actor) => actor,
        Err(code) => return code,
    };
    let resource = match folder {
        Some(folder) => format!("{name}@{}", folder.display()),
        None => format!("{name}@<folder not supplied>"),
    };
    let call_id = format!("secret-grant-{}", uuid::Uuid::new_v4());
    audit_folder_denial(&actor.id, &resource, &call_id);
    println!("permission_denied");
    1
}

/// Read one folder account through the host's command socket.
pub fn folder_secret_read(name: &str, folder: Option<&Path>) -> i32 {
    if !folder_feature_available() {
        return 1;
    }
    let folder = match folder.map(secret_folder_path).transpose() {
        Ok(folder) => folder,
        Err(code) => return code,
    };
    let cwd = match secret_folder_path(Path::new(".")) {
        Ok(path) => path,
        Err(code) => return code,
    };
    let request =
        serde_json::json!({"type":"folder_secret_read", "name":name, "folder":folder, "cwd":cwd});
    match send_secret("read", name, request) {
        Ok(reply) => {
            println!("{}", reply["value"].as_str().expect("validated read reply"));
            0
        }
        Err(code) => code,
    }
}

pub(super) fn request_bundle(
    label: &str,
    request: Value,
) -> Result<crate::protocol::ApprovedBundle, i32> {
    use crate::protocol::SecretBundleReply as Reply;
    let body = receive_secret(label, request, |reply| reply.map(zeroize::Zeroizing::new))?
        .map_err(|_| {
            eprintln!(
                "No environment was received from Plexi for {label}. Check the host connection."
            );
            1
        })?;
    match serde_json::from_str(&body) {
        Ok(Reply::Values(bundle)) => Ok(bundle),
        Ok(Reply::Missing { names }) => {
            for name in names {
                eprintln!("Missing required secret '{name}'. Set it with plexi secret set {name}.");
            }
            Err(1)
        }
        Ok(Reply::Denied { reason }) => {
            eprintln!(
                "{label}: request denied ({}). Review the request in Plexi.",
                reason.as_deref().unwrap_or("permission denied")
            );
            Err(1)
        }
        Ok(Reply::Error { error }) => {
            eprintln!("{label}: {error}");
            Err(1)
        }
        Err(_) => {
            eprintln!(
                "No environment was received from Plexi for {label}. Check the host connection."
            );
            Err(1)
        }
    }
}

/// Run the program with the environment released on this invocation's connection.
pub fn folder_secret_exec(cwd: &std::path::Path, command: &[String]) -> i32 {
    if !folder_feature_available() {
        return 1;
    }
    if command.is_empty() {
        eprintln!("error: secret exec needs a command");
        return 1;
    }
    let cwd = match secret_folder_path(cwd) {
        Ok(folder) => std::path::PathBuf::from(folder),
        Err(code) => return code,
    };
    let bundle = match request_bundle(
        "secret exec",
        serde_json::json!({"type":"folder_secret_exec","cwd":cwd}),
    ) {
        Ok(bundle) => bundle,
        Err(code) => return code,
    };
    let pane_env = crate::host::shell::build_env_with_secrets(
        Some(&bundle.cwd),
        crate::host::shell::SecretPolicy::NONE,
    );
    log::info!(
        "folder_secrets: exec cwd={} program={}",
        bundle.cwd.display(),
        command[0]
    );
    #[expect(
        clippy::disallowed_methods,
        reason = "foreground: the command owns the terminal and its exit code is passed through"
    )]
    let mut child = std::process::Command::new(&command[0]);
    child.args(&command[1..]).current_dir(&bundle.cwd);
    child.envs(pane_env);
    for entry in &bundle.values {
        child.env(&entry.name, &entry.value.0);
    }
    match child.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("error: failed to spawn {}: {err}", command[0]);
            1
        }
    }
}

// ── plexi plugin subcommands ─────────────────────────────────────────────────────

#[cfg(test)]
pub(super) mod secret_set_tests {
    use super::folder_secret_grant;
    use std::fs;
    use tempfile::TempDir;

    type BundleResponse = Box<dyn FnOnce(&serde_json::Value) -> String>;
    thread_local! {
        pub(crate) static BUNDLE_RESPONSE: std::cell::Cell<Option<BundleResponse>> =
            const { std::cell::Cell::new(None) };
    }

    pub(crate) fn with_stored_openrouter_key(check: impl FnOnce()) {
        with_stored_account("OPENROUTER_API_KEY", check);
    }

    pub(crate) fn with_stored_account(name: &str, check: impl FnOnce()) {
        use crate::workspace::secrets::{keychain_user_name, system_store};
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let account = keychain_user_name(name);
        system_store().set(&account, "fixture").unwrap();
        check();
        system_store().delete(&account).unwrap();
    }

    #[test]
    fn global_lookup_uses_one_host_reply_and_no_store_calls() {
        use super::{global_secret_lookup, GlobalSecret};
        use crate::workspace::secrets::take_test_store_calls;
        use serde_json::json;

        for (body, expected) in [
            (
                r#"{"status":"value","value":"fixture"}"#,
                Ok(Some("fixture")),
            ),
            (r#"{"status":"not_found"}"#, Ok(None)),
            (r#"{"status":"denied"}"#, Err("denied")),
            (r#"{"status":"denied","reason":"expired"}"#, Err("expired")),
            (r#"{"status":"denied","reason":"revoked"}"#, Err("revoked")),
            (
                r#"{"ok":false,"error_code":"auth_failed"}"#,
                Err("auth_failed"),
            ),
            (r#"{"status":"value"}"#, Err("invalid secret reply")),
            ("", Err("could not read response file: empty host reply")),
        ] {
            BUNDLE_RESPONSE.set(Some(Box::new(move |wire| {
                assert_eq!(
                    wire,
                    &json!({"type":"secret_get","name":"KEY","global":true,"workspace_root":null})
                );
                body.into()
            })));
            take_test_store_calls();
            let lookup = global_secret_lookup("KEY");
            assert!(BUNDLE_RESPONSE.take().is_none());
            assert_eq!(take_test_store_calls(), 0);
            let actual = match &lookup {
                GlobalSecret::Value(value) => Ok(Some(value.as_str())),
                GlobalSecret::NotSet => Ok(None),
                GlobalSecret::Unavailable(reason) => Err(reason.as_str()),
            };
            assert_eq!(actual, expected);
        }
    }

    #[cfg(unix)]
    #[test]
    fn bundle_child_uses_canonical_reply_cwd() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().join("profile"));
        let _enabled =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let approved = dir.path().canonicalize().unwrap().join("approved");
        let other = dir.path().join("other");
        fs::create_dir(&approved).unwrap();
        fs::create_dir(&other).unwrap();
        let commands = crate::cli::ensure_commands_file(Some(&approved));
        fs::write(
            commands,
            "[commands]\nbuild = { run = 'pwd', secrets = ['TEST'] }",
        )
        .unwrap();
        let snippet = "pwd -P > \"$1\"";
        for kind in ["folder_secret_exec", "run_secrets"] {
            let output = dir.path().join(format!("{kind}.cwd"));
            let args = [output.to_str().unwrap().to_owned()];
            let link = dir.path().join(kind);
            symlink(&approved, &link).unwrap();
            let other = other.clone();
            BUNDLE_RESPONSE.set(Some(Box::new(move |wire| {
                assert_eq!(wire["type"], kind);
                let cwd = std::path::Path::new(wire["cwd"].as_str().unwrap());
                let canonical = cwd.canonicalize().unwrap();
                fs::remove_file(cwd).unwrap();
                symlink(other, cwd).unwrap();
                serde_json::json!({
                    "status": "values", "cwd": canonical, "values": [],
                    "run": snippet, "steps": [], "on_load": []
                })
                .to_string()
            })));
            let code = if kind == "folder_secret_exec" {
                let command = [
                    "/bin/sh".into(),
                    "-c".into(),
                    snippet.into(),
                    "--".into(),
                    args[0].clone(),
                ];
                super::folder_secret_exec(&link, &command)
            } else {
                crate::cli::run::run_command_for_root(
                    Some(&approved),
                    &link,
                    "build",
                    &args,
                    super::OutputMode::Text,
                )
                .exit_code
            };
            assert_eq!(code, 0, "{kind}");
            let observed = fs::read_to_string(&output).unwrap();
            assert_eq!(
                std::path::Path::new(observed.trim_end()),
                approved,
                "{kind}"
            );
        }
    }

    #[test]
    fn secret_commands_require_a_final_reply() {
        let pending =
            serde_json::json!({"status":"permission_required","pending_request_id":"old"});
        for op in ["get", "set", "delete", "read", "rm"] {
            assert_eq!(
                super::secret_result(op, "KEY", pending.clone()),
                Err(1),
                "{op}"
            );
        }
    }

    /// Stint 1635: the CLI refuses a grant without touching any store, with
    /// or without `--folder`, and writes no allow or audit.
    #[test]
    fn cli_secret_grant_refuses_without_touching_the_store() {
        let _enabled =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let folder = dir.path().join("B");
        fs::create_dir(&folder).unwrap();
        // A stored binding gives folder discovery something to find.
        crate::workspace::secrets::set_folder_secret(
            crate::workspace::secrets::folder_store(),
            "FOLDER_B_SECRET_1635",
            &folder,
            "test-only",
        )
        .unwrap();
        for (agent, app, folder) in [
            (Some("me"), None, Some(folder.as_path())),
            (Some("me"), None, None),
            (None, Some("mail"), Some(folder.as_path())),
            (None, Some("mail"), None),
        ] {
            crate::workspace::secrets::take_test_store_calls();
            let code = folder_secret_grant("FOLDER_B_SECRET_1635", agent, app, folder);
            assert_eq!(code, 1);
            assert_eq!(
                crate::workspace::secrets::take_test_store_calls(),
                0,
                "a grant refusal must not open the store (folder={folder:?})"
            );
        }
        assert!(
            !profile.join("grants.toml").exists(),
            "the cli must not record a folder-secret allow"
        );
        // Stint 1291: the CLI process does not write the sealed audit; the
        // refusal is logged only.
        assert!(
            !profile.join("permission-audit.jsonl").exists(),
            "the cli must not write the sealed audit"
        );
        crate::workspace::secrets::remove_folder_secret(
            crate::workspace::secrets::folder_store(),
            "FOLDER_B_SECRET_1635",
            &folder,
        )
        .unwrap();
    }

    use crate::host::anchor::check_workspace_init_root;

    #[test]
    fn root_dir_is_rejected_by_init_guard() {
        assert!(check_workspace_init_root(std::path::Path::new("/")).is_err());
    }

    #[test]
    fn home_dir_is_rejected_by_init_guard() {
        let home = std::env::var("HOME").unwrap();
        assert!(check_workspace_init_root(std::path::Path::new(&home)).is_err());
    }

    #[test]
    fn plexi_profile_dir_is_rejected_by_init_guard() {
        let profile = crate::config::profile_home().unwrap().join(".plexi-alpha");
        assert!(check_workspace_init_root(&profile).is_err());
    }

    #[test]
    fn project_subdir_is_allowed_by_init_guard() {
        let tmp = tempfile::tempdir().unwrap();
        // A temp dir under /private/var/... is not home/root/profile
        check_workspace_init_root(tmp.path()).unwrap();
    }

    #[test]
    fn walk_up_finds_nearest_plexi_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        fs::create_dir_all(workspace.path().join(&channel_dir)).unwrap();
        let deep = workspace.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep).unwrap();

        let found = crate::app::registry::resolve_workspace_root(&deep);
        assert!(found.is_some(), "should find .plexi ancestor");
        let found = found.unwrap().canonicalize().unwrap();
        let expected = workspace.path().canonicalize().unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn no_plexi_dir_in_tree_returns_none() {
        let bare: TempDir = tempfile::tempdir().unwrap();
        let inner = bare.path().join("x").join("y");
        fs::create_dir_all(&inner).unwrap();
        assert!(crate::app::registry::resolve_workspace_root(&inner).is_none());
    }

    #[test]
    fn walk_up_stops_at_home() {
        // Simulate a path that extends above home but has .plexi above home.
        // resolve_workspace_root never walks above HOME, so .plexi above home
        // must NOT be found.
        //
        // We cannot safely create dirs above HOME, so we assert indirectly:
        // a bare temp dir with no .plexi returns None, proving the walk stops.
        let bare: TempDir = tempfile::tempdir().unwrap();
        assert!(crate::app::registry::resolve_workspace_root(bare.path()).is_none());
    }
}
#[cfg(test)]
mod workspace_init_tests {
    /// Calls the internal init_workspace logic and then the channel-dir creation
    /// on a temp dir, asserting both `.plexi/` and the channel dir are present.
    #[test]
    fn workspace_init_creates_channel_dir_with_all_files() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();

        let channel_dir = crate::config::workspace_channel_dir();
        // Run init_workspace (creates channel dir with workspace.toml + secrets.toml)
        crate::workspace::secrets::init_workspace(&cwd, &channel_dir)
            .expect("init_workspace should succeed in a temp dir");

        assert!(
            cwd.join(&channel_dir).is_dir(),
            "{channel_dir}/ dir must exist after workspace init"
        );
        assert!(
            cwd.join(&channel_dir).join("workspace.toml").is_file(),
            "{channel_dir}/workspace.toml must exist after init"
        );
        assert!(
            cwd.join(&channel_dir).join("secrets.toml").is_file(),
            "{channel_dir}/secrets.toml must exist after init"
        );
    }

    /// After init, resolve_workspace_root must still find the workspace.
    #[test]
    fn workspace_remains_resolvable_after_init() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();

        let channel_dir = crate::config::workspace_channel_dir();
        crate::workspace::secrets::init_workspace(&cwd, &channel_dir)
            .expect("init_workspace should succeed");

        let found = crate::app::registry::resolve_workspace_root(&cwd);
        assert!(
            found.is_some(),
            "resolve_workspace_root should find the workspace after init"
        );
        assert!(
            cwd.join(&channel_dir).is_dir(),
            "channel directory should exist after init"
        );
    }
}

#[cfg(test)]
mod secret_list_tests {
    use super::write_secret_list;
    use crate::workspace::secrets::{
        self,
        manager::{mutate_secret, Mutation, SecretTarget},
    };

    /// Stint 1291: `secret list` from a CLI process still lists names, and says
    /// in one line that grant and last-used details come from the host.
    #[test]
    fn secret_list_outside_the_host_omits_grant_details_and_says_so() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let cwd = tempfile::tempdir().unwrap();
        let mut output = Vec::new();
        crate::broker::custody::as_cli_process(|| {
            write_secret_list(cwd.path(), true, false, &mut output).unwrap()
        });
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains(crate::workspace::secrets::manager::GRANT_DETAILS_FROM_HOST),
            "{output}"
        );
        assert!(!profile.path().join("permission-audit.jsonl").exists());
    }

    fn check_listing(global: bool, folders_enabled: bool, json: bool) {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let root = tempfile::tempdir().unwrap();
        let cfg =
            secrets::init_workspace(root.path(), &crate::config::workspace_channel_dir()).unwrap();
        let root_path = crate::platform::path::canonical_or_self(root.path());
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(profile.path()).unwrap();
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let names = [
            format!("GLOBAL_{suffix}"),
            format!("WORKSPACE_{suffix}"),
            format!("FOLDER_{suffix}"),
        ];
        let targets = [
            SecretTarget::Global,
            SecretTarget::Workspace {
                id: cfg.id,
                root: root_path.clone(),
            },
            SecretTarget::Folder(root_path),
        ];
        let _enabled =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        for (name, target) in names.iter().zip(&targets) {
            let store = if matches!(target, SecretTarget::Folder(_)) {
                secrets::folder_store()
            } else {
                secrets::system_store()
            };
            mutate_secret(
                store,
                &monitor,
                target,
                name,
                Mutation::Set("never-print-this-secret-canary"),
            )
            .unwrap();
        }
        secrets::write_terminal_env_inject(root.path(), &names[1], true).unwrap();
        let enabled = if folders_enabled {
            vec![crate::release::ReleaseFeature::FolderSecrets]
        } else {
            vec![]
        };
        let _experiment = crate::release::set_test_experiments(&enabled);
        let child = root.path().join("child");
        std::fs::create_dir(&child).unwrap();
        let mut output = Vec::new();
        write_secret_list(&child, global, json, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(&names[0]), "global entry missing: {output}");
        assert_eq!(
            output.contains(&names[1]),
            !global,
            "workspace visibility: {output}"
        );
        assert_eq!(
            output.contains(&names[2]),
            !global && folders_enabled,
            "folder visibility: {output}"
        );
        assert!(!output.contains("never-print-this-secret-canary"));
        if !global && !folders_enabled {
            assert!(output.contains(if json {
                "experiment_disabled"
            } else {
                "folder secret(s) hidden"
            }));
        }
        if json {
            let rows: serde_json::Value = serde_json::from_str(&output).unwrap();
            if global {
                assert!(rows["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|row| row["scope"] == "global"));
            }
        } else if !global {
            assert!(
                output.contains(&format!(
                    "{}  (injected into human terminal panes)",
                    names[1]
                )),
                "injection annotation: {output}"
            );
        }
        let _cleanup =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        for (name, target) in names.iter().zip(&targets) {
            let store = if matches!(target, SecretTarget::Folder(_)) {
                secrets::folder_store()
            } else {
                secrets::system_store()
            };
            mutate_secret(store, &monitor, target, name, Mutation::Delete).unwrap();
        }
    }

    #[test]
    fn text_global_omits_workspace_and_folders() {
        check_listing(true, true, false);
    }
    #[test]
    fn json_global_omits_workspace_and_folders() {
        check_listing(true, true, true);
    }
    #[test]
    fn text_folder_experiment_off_omits_folders() {
        check_listing(false, false, false);
    }
    #[test]
    fn json_folder_experiment_off_omits_folders() {
        check_listing(false, false, true);
    }
    #[test]
    fn text_folder_experiment_on_includes_folders_and_injection() {
        check_listing(false, true, false);
    }
    #[test]
    fn json_folder_experiment_on_includes_folders() {
        check_listing(false, true, true);
    }
}

#[cfg(test)]
mod secret_root_tests {
    use super::*;

    #[test]
    fn pane_scope_obeys_the_host_without_opening_the_store() {
        use crate::workspace::secrets::{
            folder_store, remove_folder_secret, set_folder_secret, take_test_store_calls,
        };
        let root = tempfile::tempdir().unwrap();
        let pane = set_test_pane_context_root(root.path());
        let target = root.path().join("nested");
        std::fs::create_dir(&target).unwrap();
        set_folder_secret(folder_store(), "NESTED", &target, "fixture").unwrap();
        let denied = "permission_denied: nested folder";
        let unconfirmed = format!(
            "permission_denied: host could not confirm this pane's context scope for {}",
            target.display()
        );
        for (reply, expected) in [
            (
                serde_json::json!({"ok":true, "workspace_error":null}),
                Ok(()),
            ),
            (
                serde_json::json!({"ok":true, "workspace_error":denied}),
                Err(denied.to_owned()),
            ),
            (serde_json::json!({"ok":true}), Err(unconfirmed.clone())),
            (
                serde_json::json!({"ok":false, "workspace_error":null}),
                Err(unconfirmed.clone()),
            ),
            (
                serde_json::json!({"ok":true, "workspace_error":false}),
                Err(unconfirmed),
            ),
        ] {
            let _reply = set_test_scope_reply(reply);
            take_test_store_calls();
            let outcome = enforce_pane_context_scope(&target, "secret.list");
            assert_eq!(take_test_store_calls(), 0);
            assert_eq!(outcome, expected);
        }
        remove_folder_secret(folder_store(), "NESTED", &target).unwrap();
        drop(pane);
        let _reply = set_test_scope_reply(serde_json::json!({"ok":false}));
        take_test_store_calls();
        assert_eq!(enforce_pane_context_scope(&target, "secret.list"), Ok(()));
        assert_eq!(take_test_store_calls(), 0);
    }

    #[test]
    fn secret_workspace_walk_cannot_escape_pane_context() {
        let dir = tempfile::tempdir().unwrap();
        let context = dir.path().canonicalize().unwrap().join("context");
        let cwd = context.join("child");
        std::fs::create_dir_all(&cwd).unwrap();
        let channel = crate::config::workspace_channel_dir();
        crate::workspace::secrets::init_workspace(dir.path(), &channel).unwrap();
        let scope = serde_json::json!({"context_root":context,"workspace_error":null});
        let request =
            |op| secret_request_for_workspace(op, "TOKEN", Some(require_workspace(&cwd, &scope)));
        assert!(request("get").unwrap()["workspace_root"].is_null());
        for op in ["set", "delete"] {
            assert_eq!(request(op), Err(1));
        }
        assert_eq!(secret_workspace_root(&cwd, Some(&context)), None);
        let _pane = set_test_pane_context_root(&context);
        assert!(enforce_pane_context_scope(&cwd, "secret.workspace").is_ok());
        crate::workspace::secrets::init_workspace(&context, &channel).unwrap();
        for op in ["get", "set", "delete"] {
            assert_eq!(
                request(op).unwrap()["workspace_root"],
                serde_json::json!(context)
            );
        }
        assert_eq!(
            secret_workspace_root(&cwd, Some(&context)),
            Some(context.canonicalize().unwrap())
        );
        if let Some(home) = dirs::home_dir() {
            assert_eq!(secret_workspace_root(&home, Some(&home)), None);
        }
    }
    #[test]
    fn missing_secret_message_names_the_channel_binary_once() {
        let _channel = crate::config::set_test_channel("alpha");
        assert_eq!(
            secret_not_set("FOO"),
            "FOO not set; run `plexi-alpha secret set FOO`"
        );
    }
}
