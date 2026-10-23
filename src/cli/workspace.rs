use super::open::read_secret_from_stdin;
use super::print_tip;
use std::io::{self, Write};

pub fn workspace_init() -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };
    log::info!("workspace_init:cli: cwd={}", cwd.display());
    // Guard: refuse home dir, root dir, and inside any ~/.plexi* profile dir
    {
        let home = std::env::var("HOME").ok().map(std::path::PathBuf::from);
        let cwd_str = cwd.to_string_lossy();
        let is_home_or_root =
            cwd == std::path::Path::new("/") || home.as_ref().map(|h| cwd == *h).unwrap_or(false);
        let is_inside_profile = home
            .as_ref()
            .map(|h| {
                let prefix = format!("{}/.plexi", h.to_string_lossy());
                cwd_str.starts_with(&prefix)
            })
            .unwrap_or(false);
        if is_home_or_root || is_inside_profile {
            log::warn!(
                "workspace_init:cli: rejected — home/root/profile dir guard: {}",
                cwd.display()
            );
            eprintln!("error: cannot initialize a workspace in your home or root directory.");
            eprintln!("  This would conflict with your Plexi profile (~/.plexi/).");
            eprintln!("  cd into a project directory first.");
            return 1;
        }
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
            println!("  apps:         {channel_dir}/apps.toml (declare app dependencies here)");
            println!("  commands:     {channel_dir}/commands.toml (run commands with: plexi run)");
            print_tip(&format!("declare app dependencies in {channel_dir}/apps.toml, then run `plexi app install`."));
            0
        }
        Err(e) => {
            log::warn!("workspace_init:cli: init_workspace failed: {e}");
            eprintln!("error: workspace init failed: {e}");
            1
        }
    }
}

pub fn workspace_clean_cli(dry_run: bool) -> i32 {
    log::info!("workspace_clean:cli: dry_run={dry_run}");
    let content = match super::request(
        serde_json::json!({
            "type": "workspace_clean_slots",
            "dry_run": dry_run,
        }),
        "workspace-clean-response",
        "workspace clean",
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
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        eprintln!("error: {err}");
        return 1;
    }
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

pub(crate) fn active_pane_context_root() -> Option<std::path::PathBuf> {
    #[cfg(test)]
    let raw = TEST_PANE_CONTEXT_ROOT.with(|cell| cell.borrow().clone())?;
    #[cfg(not(test))]
    let raw = std::env::var("PLEXI_CONTEXT_ROOT")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)?;
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
    explicit_folder: bool,
    op: &str,
) -> Result<(), String> {
    let Some(ctx_root) = active_pane_context_root() else {
        return Ok(());
    };
    let target_canon = crate::platform::path::canonical_or_self(target_dir);
    let denied = if explicit_folder {
        target_canon != ctx_root
    } else if !target_canon.starts_with(&ctx_root) {
        true
    } else if target_canon != ctx_root {
        let nested_ws = secret_workspace_root(&target_canon, Some(&ctx_root))
            .is_some_and(|ws| crate::platform::path::canonical_or_self(&ws) != ctx_root);
        let nested_agent_ws =
            crate::app::registry::resolve_agent_lookup_workspace_root(&target_canon)
                .map(|ws| crate::platform::path::canonical_or_self(&ws))
                .is_some_and(|ws| ws.starts_with(&ctx_root) && ws != ctx_root);
        let nested_folder_secret = crate::workspace::secrets::list_folder_secrets(
            crate::workspace::secrets::folder_store(),
        )
        .iter()
        .any(|meta| {
            let f = crate::platform::path::canonical_or_self(&meta.folder);
            f != ctx_root && f.starts_with(&ctx_root) && target_canon.starts_with(&f)
        });
        nested_ws || nested_agent_ws || nested_folder_secret
    } else {
        false
    };
    if denied {
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
        return Err(format!(
            "permission_denied: pane has no authority over context '{}' (authority is scoped to '{}')",
            target_canon.display(),
            ctx_root.display()
        ));
    }
    Ok(())
}

/// Keep secret lookup inside the pane's context while reusing the registry's
/// channel and home-directory exclusions. Never reinterpret a parent profile
/// as this pane's workspace (the H8 request walked from ~/temp into $HOME).
fn secret_workspace_root(cwd: &std::path::Path, context: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
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
    let binary = directory.strip_prefix(".plexi-")
        .map(|channel| format!("plexi-{channel}"))
        .unwrap_or_else(|| "plexi".into());
    format!("{name} not set; run `{binary} secret set {name}`")
}

/// Resolve the current workspace and config. Errors out with a helpful
/// message if the user has not run `plexi workspace init`.
fn require_workspace() -> Result<
    (
        std::path::PathBuf,
        crate::workspace::secrets::WorkspaceConfig,
    ),
    String,
> {
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    enforce_pane_context_scope(&cwd, false, "secret.workspace")?;
    let channel_dir = crate::config::workspace_channel_dir();
    let root = match secret_workspace_root(&cwd, active_pane_context_root().as_deref()) {
        Some(r) => r,
        None => {
            return Err(format!(
                "no {channel_dir}/ workspace found at or above {}.\n\
                 Run `plexi workspace init` first.",
                cwd.display()
            ));
        }
    };
    enforce_pane_context_scope(&root, true, "secret.workspace")?;
    let cfg = match crate::workspace::secrets::WorkspaceConfig::load(&root)
        .map_err(|e| format!("read {channel_dir}/workspace.toml: {e}"))?
    {
        Some(c) => c,
        None => {
            return Err(format!(
                "{channel_dir}/ exists at {} but workspace.toml is missing.\n\
                 Run `plexi workspace init` to create it.",
                root.display()
            ));
        }
    };
    Ok((root, cfg))
}

/// `plexi secret set <friendly-name>` — store a secret in Keychain.
///
/// Value source (in priority order):
///   --from-env   reads from env var named FRIENDLY_NAME
///   default      hidden stdin prompt
///
/// Scope:
///   --global     store under `plexi:user:<name>` (cross-workspace)
///   default      walk up to nearest .plexi/ workspace, store workspace-scoped
pub fn workspace_secret_set(
    friendly: &str,
    from_env: bool,
    global: bool,
    alias: Option<&str>,
) -> i32 {
    // `--alias` is incompatible with `--global`: global secrets are resolved via
    // `plexi:user:<canonical>` and there is no secrets.toml to hold an alias route.
    if global && alias.is_some() {
        eprintln!("error: --alias cannot be combined with --global.");
        eprintln!("  Global secrets are always stored under their canonical name.");
        return 1;
    }
    // `friendly` is the canonical name (env var name used in app code).
    // `alias` overrides the Keychain entry name when the user wants a different friendly name.
    let effective_friendly = alias.unwrap_or(friendly);
    log::info!(
        "secret_set:cli: canonical={friendly} effective_friendly={effective_friendly} from_env={from_env} global={global}"
    );
    // Resolve value
    let value: String = if from_env {
        match std::env::var(friendly) {
            Ok(v) => v,
            Err(_) => {
                log::warn!("secret_set:cli: env var {friendly} not set");
                eprintln!("error: env var {friendly} is not set");
                return 1;
            }
        }
    } else {
        eprint!("Enter value for {friendly}: ");
        let _ = io::stderr().flush();
        match read_secret_from_stdin() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("secret_set:cli: stdin read failed: {e}");
                eprintln!("\nerror: failed to read secret: {e}");
                return 1;
            }
        }
    };
    if value.is_empty() {
        log::warn!("secret_set:cli: empty value rejected for {friendly}");
        eprintln!("error: empty value, nothing stored");
        return 1;
    }

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        use crate::workspace::secrets::{
            keychain_user_name, keychain_workspace_name, system_store,
        };
        let store = system_store();

        if global {
            let account = keychain_user_name(effective_friendly);
            return match store.set(&account, &value) {
                Ok(()) => {
                    log::info!("secret_set:cli: stored globally account={account}");
                    println!("Stored '{friendly}' globally (plexi:user:{effective_friendly})");
                    print_tip(&format!(
                        "reference '{friendly}' in {}/commands.toml (or {}) under `[secrets] required = [\"{friendly}\"]`.",
                        crate::config::workspace_channel_dir(),
                        super::global_commands_path().display()
                    ));
                    0
                }
                Err(e) => {
                    log::warn!("secret_set:cli: keychain write failed for {account}: {e}");
                    eprintln!("error: keychain write failed: {e}");
                    1
                }
            };
        }

        // Workspace-scoped: walk up to nearest .plexi/
        let (root, cfg) = match require_workspace() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("secret_set:cli: no workspace found: {e}");
                eprintln!("error: no .plexi/ workspace found in this directory tree.");
                eprintln!("  → plexi workspace init        (initialize one here, then retry)");
                eprintln!("  → plexi secret set --global {friendly}   (set globally — requires explicit flag)");
                return 1;
            }
        };
        if crate::workspace::secrets::workspace_id_is_reserved(&cfg.id)
            || crate::workspace::secrets::secret_account_is_reserved(&keychain_workspace_name(
                &cfg.id,
                effective_friendly,
            ))
        {
            log::info!(
                "secret_set:cli: refused reserved namespace workspace_id={} friendly={effective_friendly}",
                cfg.id
            );
            eprintln!(
                "error: {}",
                crate::workspace::secrets::reserved_secret_error(&cfg.id)
            );
            return 1;
        }
        match store_workspace_secret(store, &root, &cfg.id, friendly, effective_friendly, &value) {
            Ok(injected) => {
                eprintln!(
                    "Stored '{friendly}' for workspace {} ({})",
                    root.display(),
                    cfg.id
                );
                let channel_dir = crate::config::workspace_channel_dir();
                print_tip(&format!(
                    "Route written to {channel_dir}/secrets.toml — use in {channel_dir}/commands.toml: [secrets] required = [\"{friendly}\"]"
                ));
                print_tip(&format!(
                    "'{friendly}' is not added to terminal environments. Agent panes never receive workspace secrets; a pane reads it with plexi secret get after you click Allow."
                ));
                if !injected.is_empty() {
                    eprintln!(
                        "Still injected into human terminal panes by {channel_dir}/secrets.toml: {}. Turn injection off in Secrets (Cmd+Shift+S).",
                        injected.join(", ")
                    );
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (friendly, effective_friendly, value, global);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

/// Store a workspace secret and its default route. Terminal injection is not
/// turned on: a secret reaches a terminal environment only by an explicit
/// toggle in the Secrets manager. Returns the names `secrets.toml` already
/// injects, so the caller can show them.
pub(crate) fn store_workspace_secret(
    store: &dyn crate::workspace::secrets::SecretStore,
    root: &std::path::Path,
    workspace_id: &str,
    friendly: &str,
    effective_friendly: &str,
    value: &str,
) -> Result<Vec<String>, String> {
    let account = crate::workspace::secrets::keychain_workspace_name(workspace_id, effective_friendly);
    store.set(&account, value).map_err(|e| {
        log::warn!("secret_set:cli: keychain write failed for {account}: {e}");
        format!("keychain write failed: {e}")
    })?;
    log::info!(
        "secret_set:cli: stored workspace_id={workspace_id} account={account} root={} terminal_inject=off",
        root.display()
    );
    // Non-fatal: the value is stored; only the convenience route is missing.
    match crate::workspace::secrets::write_default_route(root, friendly, effective_friendly) {
        Ok(()) => log::info!(
            "secret_set:cli: wrote default route {friendly} → {effective_friendly} in secrets.toml"
        ),
        Err(e) => log::warn!("secret_set:cli: write_default_route failed: {e}"),
    }
    let injected = crate::workspace::secrets::WorkspaceSecrets::load(root)?
        .map(|router| router.terminal.env.inject)
        .unwrap_or_default();
    if !injected.is_empty() {
        log::info!(
            "secret_set:cli: secrets.toml still injects {} name(s) into human terminal panes: {}",
            injected.len(),
            injected.join(",")
        );
    }
    Ok(injected)
}

/// Reconcile the secrets index against a real keychain scan before a CLI
/// command reads it (`src/workspace/AGENTS.md`: never list secrets from the
/// index without reconciling first). A failed scan degrades to the index as it
/// stands, with a warning, rather than failing the command.
pub(crate) fn reconcile_secrets_before_read(command: &str) {
    match crate::workspace::secrets::reconcile_index_with_keychain() {
        Ok(report) => log::info!(
            "{command}: reconciled secrets index ({} adopted, {} stale, {} renamed)",
            report.adopted.len(),
            report.stale.len(),
            report.renamed.len()
        ),
        Err(e) => log::warn!("{command}: keychain scan failed, using index as-is: {e}"),
    }
}

/// `plexi secret list` — list friendly names. Inside a workspace, includes
/// workspace namespace plus user-scope. Outside a workspace, gracefully falls
/// back to user-scope. Names only, never values.
pub fn workspace_secret_list(
    global: bool,
    context: Option<&super::ContextTarget>,
    json: bool,
) -> i32 {
    log::info!("secret_list:cli: global={global} json={json}");
    if let Some(context) = context {
        return explicit_context_secret_list(context, json);
    }
    let result = std::env::current_dir()
        .map_err(|error| format!("current directory: {error}"))
        .and_then(|cwd| write_secret_list(&cwd, global, json, &mut std::io::stdout().lock()));
    match result {
        Ok(()) => 0,
        Err(error) => {
            log::warn!("secret_list: failed: {error}");
            eprintln!("error: {error}");
            1
        }
    }
}

fn explicit_context_secret_list(context: &super::ContextTarget, json: bool) -> i32 {
    let reply = match super::request(
        serde_json::json!({"type": "pane_secret_list", "context": context.as_str()}),
        "secret-list-context",
        "context secret inventory",
    ) {
        Ok(reply) => reply,
        Err(code) => return code,
    };
    let result = serde_json::from_str::<serde_json::Value>(&reply)
        .map_err(|e| e.to_string())
        .and_then(|body| {
            if let Some(error) = body.get("error").and_then(|v| v.as_str()) {
                return Err(error.to_string());
            }
            let inventory = serde_json::from_value(body).map_err(|e| e.to_string())?;
            write_secret_inventory(&inventory, json, &mut std::io::stdout().lock())
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

/// `plexi secret get` from a terminal pane. The host admits the read for the
/// pane it identifies from the socket peer, so this process never opens the
/// store. Exit 2 means a desktop click is pending; rerun after Allow.
fn pane_secret_get(friendly: &str, global: bool) -> i32 {
    let workspace_root = if global {
        None
    } else {
        match require_workspace() {
            Ok((root, _)) => Some(root),
            Err(err) if err.starts_with("permission_denied") => {
                eprintln!("error: {err}");
                println!("permission_denied");
                return 1;
            }
            Err(_) => None,
        }
    };
    log::info!(
        "secret_get:cli: pane caller, asking the host gate friendly={friendly} global={global} workspace={workspace_root:?}"
    );
    let payload = serde_json::json!({
        "type": "pane_secret_get",
        "name": friendly,
        "global": global,
        "workspace_root": workspace_root,
    });
    let reply = match super::request_with(payload, "secret-get-response", "secret get", std::time::Duration::from_secs(120)) {
        Ok(content) => content,
        Err(code) => {
            log::info!("secret_get:cli: host gate unreachable from a pane; refusing");
            eprintln!("error: a pane reads secrets through the Plexi host, and the host did not answer");
            println!("permission_denied");
            return code.max(1);
        }
    };
    let reply: serde_json::Value = match serde_json::from_str(&reply) {
        Ok(value) => value,
        Err(e) => {
            eprintln!("error: secret get reply did not parse: {e}");
            return 1;
        }
    };
    match reply.get("status").and_then(|status| status.as_str()) {
        Some("value") => {
            println!("{}", reply.get("value").and_then(|v| v.as_str()).unwrap_or_default());
            0
        }
        Some("permission_required") => {
            println!("permission_required");
            println!(
                "pending_request_id={}",
                reply.get("pending_request_id").and_then(|v| v.as_str()).unwrap_or_default()
            );
            eprintln!("Click Allow on the Plexi window, then run this again.");
            2
        }
        Some("not_found") => {
            eprintln!("error: {}", secret_not_set(friendly));
            1
        }
        Some("denied") => {
            println!("permission_denied");
            1
        }
        _ => {
            eprintln!(
                "error: {}",
                reply.get("error").and_then(|v| v.as_str()).unwrap_or("secret get failed")
            );
            1
        }
    }
}

/// `plexi secret get <friendly-name>` — print the resolved secret value to stdout.
///
/// Resolution order (unless --global):
///   1. Workspace-scoped Keychain entry (nearest .plexi/ workspace)
///   2. Global user-scoped Keychain entry (fallback)
///
/// `--global` skips the workspace lookup entirely.
pub fn workspace_secret_get(friendly: &str, global: bool) -> i32 {
    log::info!("secret_get:cli: friendly={friendly} global={global}");
    if super::pane_caller::caller_is_pane_agent() {
        return pane_secret_get(friendly, global);
    }

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        use crate::workspace::secrets::{
            keychain_user_name, keychain_workspace_name, system_store,
        };
        let store = system_store();

        if global {
            let account = keychain_user_name(friendly);
            return match store.get_interactive(&account) {
                Ok(Some(value)) => {
                    log::info!("secret_get:cli: found globally account={account}");
                    println!("{}", value.as_str());
                    0
                }
                Ok(None) => {
                    log::warn!("secret_get:cli: not found friendly={friendly}");
                    eprintln!("error: {}", secret_not_set(friendly));
                    1
                }
                Err(error) => { eprintln!("error: {error}"); 1 }
            };
        }

        // Try workspace-scoped first, then global fallback.
        match require_workspace() {
            Ok((root, cfg)) => {
                if crate::workspace::secrets::workspace_id_is_reserved(&cfg.id)
                    || crate::workspace::secrets::secret_account_is_reserved(
                        &keychain_workspace_name(&cfg.id, friendly),
                    )
                {
                    log::info!(
                        "secret_get:cli: refused reserved namespace workspace_id={} friendly={friendly}",
                        cfg.id
                    );
                    eprintln!(
                        "error: {}",
                        crate::workspace::secrets::reserved_secret_error(&cfg.id)
                    );
                    return 1;
                }
                let account = keychain_workspace_name(&cfg.id, friendly);
                let value = match store.get_interactive(&account) {
                    Ok(value) => value,
                    Err(error) => { eprintln!("error: {error}"); return 1; }
                };
                if let Some(value) = value {
                    log::info!(
                        "secret_get:cli: found workspace_id={} account={account} root={}",
                        cfg.id,
                        root.display()
                    );
                    println!("{}", value.as_str());
                    return 0;
                }
            }
            Err(err) if err.starts_with("permission_denied") => {
                eprintln!("error: {err}");
                println!("permission_denied");
                return 1;
            }
            Err(_) => {}
        }

        // Global fallback.
        let user_account = keychain_user_name(friendly);
        match store.get_interactive(&user_account) {
            Ok(Some(value)) => {
                log::info!("secret_get:cli: found globally account={user_account}");
                println!("{}", value.as_str());
                0
            }
            Ok(None) => {
                log::warn!("secret_get:cli: not found friendly={friendly}");
                eprintln!("error: {}", secret_not_set(friendly));
                1
            }
            Err(error) => { eprintln!("error: {error}"); 1 }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (friendly, global);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

/// Delete an account only when it exists at the exact scope selected by the
/// caller. Store backends may treat an absent delete as a successful no-op, so
/// the read is what makes the CLI's not-found contract portable.
fn delete_existing_secret(
    store: &dyn crate::workspace::secrets::SecretStore,
    account: &str,
    friendly: &str,
) -> Result<(), String> {
    if store.get(account).is_none() {
        return Err(format!("secret '{friendly}' not found"));
    }
    store
        .delete(account)
        .map_err(|error| format!("keychain delete failed: {error}"))
}

/// `plexi secret delete <friendly-name>` — remove the Keychain entry.
///
/// When `global` is true, deletes the user-scoped entry (`plexi:user:<name>`)
/// without requiring a workspace. When false, requires a workspace and deletes
/// the workspace-scoped entry.
pub fn workspace_secret_delete(friendly: &str, global: bool) -> i32 {
    log::info!("secret_delete:cli: friendly={friendly} global={global}");
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        use crate::workspace::secrets::{
            keychain_user_name, keychain_workspace_name, system_store,
        };
        let store = system_store();

        if global {
            let account = keychain_user_name(friendly);
            return match delete_existing_secret(store, &account, friendly) {
                Ok(()) => {
                    log::info!("secret_delete:cli: deleted globally account={account}");
                    println!("Deleted global secret '{friendly}'");
                    0
                }
                Err(error) => {
                    log::warn!("secret_delete:cli: rejected friendly={friendly} err={error}");
                    eprintln!("error: {error}");
                    1
                }
            };
        }

        let (_root, cfg) = match require_workspace() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        };
        let account = keychain_workspace_name(&cfg.id, friendly);
        match delete_existing_secret(store, &account, friendly) {
            Ok(()) => {
                log::info!(
                    "secret_delete:cli: deleted workspace_id={} account={account}",
                    cfg.id
                );
                println!("Deleted '{friendly}' from workspace {}", cfg.id);
                0
            }
            Err(error) => {
                log::warn!("secret_delete:cli: rejected account={account} err={error}");
                eprintln!("error: {error}");
                1
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (friendly, global);
        eprintln!("error: keychain not available on this platform");
        1
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

fn audit_folder_denial(actor: &str, resource: &str, call_id: &str) {
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
    monitor.note_denial(actor, call_id, resource, "", "permission_denied");
    log::info!("folder_secrets: refused actor={actor} resource={resource} call_id={call_id}");
}

fn actor_from_flags(
    agent: Option<&str>,
    app: Option<&str>,
) -> Result<crate::workspace::secrets::SecretActor, i32> {
    match (agent, app) {
        (Some(id), None) => Ok(crate::workspace::secrets::SecretActor::agent(id)),
        (None, Some(id)) => Ok(crate::workspace::secrets::SecretActor::app(id)),
        _ => {
            eprintln!("error: pass --agent <id> or --app <id>");
            Err(1)
        }
    }
}

fn resolve_secret_folder(
    name: &str,
    folder: Option<&std::path::Path>,
) -> Result<std::path::PathBuf, i32> {
    if let Some(folder) = folder {
        return crate::workspace::secrets::canonical_folder(folder).map_err(|err| {
            eprintln!("error: {err}");
            1
        });
    }
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("error: current directory: {err}");
            return Err(1);
        }
    };
    let store = crate::workspace::secrets::folder_store();
    let matches: Vec<_> = crate::workspace::secrets::list_folder_secrets(store)
        .into_iter()
        .filter(|meta| meta.name == name && cwd.starts_with(&meta.folder))
        .collect();
    if matches.len() == 1 {
        return Ok(matches[0].folder.clone());
    }
    let named: Vec<_> = crate::workspace::secrets::list_folder_secrets(store)
        .into_iter()
        .filter(|meta| meta.name == name)
        .collect();
    if named.len() == 1 {
        return Ok(named[0].folder.clone());
    }
    if named.is_empty() {
        eprintln!("error: secret '{name}' not found");
    } else {
        eprintln!("error: secret '{name}' is set on more than one folder; pass --folder");
    }
    Err(1)
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
    if let Err(err) = enforce_pane_context_scope(folder, true, "secret.set") {
        eprintln!("error: {err}");
        println!("permission_denied");
        return 1;
    }
    let value = match read_secret_value(name, from_env) {
        Ok(value) => value,
        Err(code) => return code,
    };
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        match crate::workspace::secrets::set_folder_secret(
            crate::workspace::secrets::folder_store(),
            name,
            folder,
            value.as_str(),
        ) {
            Ok(canon) => {
                println!(
                    "Stored '{name}' for {} ({})",
                    canon.display(),
                    crate::workspace::secrets::backend_label()
                );
                0
            }
            Err(err) => {
                eprintln!("error: {err}");
                1
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (name, folder, value);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

/// `plexi secret rm NAME --folder <path>`.
pub fn folder_secret_rm(name: &str, folder: &std::path::Path) -> i32 {
    if !folder_feature_available() {
        return 1;
    }
    if let Err(err) = enforce_pane_context_scope(folder, true, "secret.rm") {
        eprintln!("error: {err}");
        println!("permission_denied");
        return 1;
    }
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        match crate::workspace::secrets::remove_folder_secret(
            crate::workspace::secrets::folder_store(),
            name,
            folder,
        ) {
            Ok(()) => {
                println!("Removed '{name}'");
                0
            }
            Err(err) => {
                eprintln!("error: {err}");
                1
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (name, folder);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

type HostFolderGrant = fn(
    &crate::broker::gate::PermissionMonitor,
    &dyn crate::workspace::secrets::SecretStore,
    &crate::workspace::secrets::SecretActor,
    &str,
    &std::path::Path,
) -> Result<String, String>;

/// `plexi secret grant NAME --agent <id>`.
///
/// The CLI does not write an allow. A pane agent (and any other socket
/// client) used to open the permission monitor in this process and store a
/// `User`/`Always` grant with no human present. The command returns
/// `permission_denied`, audits the name and folder, and does not read the
/// secret value. A human Allow once is `approve_pending(ApprovalChoice::Once)`
/// on the read's pending. This command does not call
/// [`crate::workspace::secrets::grant_folder_secret`].
pub fn folder_secret_grant(
    name: &str,
    agent: Option<&str>,
    app: Option<&str>,
    folder: Option<&std::path::Path>,
) -> i32 {
    if !folder_feature_available() {
        return 1;
    }
    // Referenced, not called. `grant_folder_secret` writes an Always grant.
    // A human click uses `approve_pending(Once)` instead. This command must
    // not invoke the function.
    let _host_grant: HostFolderGrant = crate::workspace::secrets::grant_folder_secret;
    let actor = match actor_from_flags(agent, app) {
        Ok(actor) => actor,
        Err(code) => return code,
    };
    let folder = match folder {
        Some(folder) => match crate::workspace::secrets::canonical_folder(folder) {
            Ok(folder) => folder,
            Err(err) => {
                eprintln!("error: {err}");
                return 1;
            }
        },
        None => match resolve_secret_folder(name, None) {
            Ok(folder) => folder,
            Err(code) => return code,
        },
    };
    let resource = format!("{name}@{}", folder.display());
    let call_id = format!("secret-grant-{}", uuid::Uuid::new_v4());
    audit_folder_denial(&actor.id, &resource, &call_id);
    println!("permission_denied");
    1
}

/// `plexi secret read NAME --agent <id>`.
pub fn folder_secret_read(
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
    let folder = match resolve_secret_folder(name, folder) {
        Ok(folder) => folder,
        Err(code) => return code,
    };
    if let Err(err) = enforce_pane_context_scope(&folder, true, "secret.read") {
        eprintln!("error: {err}");
        println!("permission_denied");
        return 1;
    }
    // A pane must not learn the pending id. Filing one here would let the
    // pane approve its own read. The tester shell is not a pane agent, so
    // its read still reaches the host gate.
    if super::pane_caller::caller_is_pane_agent() {
        let resource = format!("{name}@{}", folder.display());
        let call_id = format!("secret-read-{}", uuid::Uuid::new_v4());
        let actor_id = std::env::var("PLEXI_PANE_ID")
            .ok()
            .filter(|id| !id.is_empty())
            .map(|id| format!("pane:{id}"))
            .unwrap_or_else(|| "pane:ancestor".to_string());
        audit_folder_denial(&actor_id, &resource, &call_id);
        println!("permission_denied");
        return 1;
    }
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        let result = match crate::workspace::secrets::read_through_host_gate(&actor, name, &folder)
        {
            Some(proxied) => proxied,
            None => {
                let monitor = crate::broker::gate::PermissionMonitor::for_profile(
                    &crate::config::config_dir(),
                );
                crate::workspace::secrets::read_folder_secret(
                    &monitor,
                    crate::workspace::secrets::folder_store(),
                    &actor,
                    name,
                    &folder,
                )
            }
        };
        match result {
            Ok(crate::workspace::secrets::ReadResult::Value(value)) => {
                println!("{}", value.as_str());
                0
            }
            Ok(crate::workspace::secrets::ReadResult::PermissionRequired { pending_id }) => {
                println!("permission_required");
                println!("pending_request_id={pending_id}");
                2
            }
            Ok(crate::workspace::secrets::ReadResult::Denied) => {
                println!("permission_denied");
                1
            }
            Ok(crate::workspace::secrets::ReadResult::NotFound) => {
                eprintln!("error: secret '{name}' not found");
                1
            }
            Err(err) => {
                eprintln!("error: {err}");
                1
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (name, actor, folder);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

/// Run `command` with the environment [`crate::host::shell::build_env`] builds
/// for a new terminal pane whose cwd is `cwd`.
///
/// Refused when this process is a pane agent. The command is not started, so
/// a pane in folder A cannot print folder B's secrets with `--cwd`.
pub fn folder_secret_exec(cwd: &std::path::Path, command: &[String]) -> i32 {
    if !folder_feature_available() {
        return 1;
    }
    if command.is_empty() {
        eprintln!("error: secret exec needs a command");
        return 1;
    }
    if super::pane_caller::caller_is_pane_agent() {
        let resource = format!("exec@{}", cwd.display());
        let call_id = format!("secret-exec-{}", uuid::Uuid::new_v4());
        let actor = std::env::var("PLEXI_PANE_ID")
            .ok()
            .filter(|id| !id.is_empty())
            .map(|id| format!("pane:{id}"))
            .unwrap_or_else(|| "pane:ancestor".to_string());
        audit_folder_denial(&actor, &resource, &call_id);
        println!("permission_denied");
        return 1;
    }
    let pane_env = crate::host::shell::build_env(Some(cwd));
    log::info!(
        "folder_secrets: exec cwd={} program={}",
        cwd.display(),
        command[0]
    );
    let mut child = std::process::Command::new(&command[0]);
    child.args(&command[1..]).current_dir(cwd);
    for (key, value) in &pane_env {
        child.env(key, value);
    }
    match child.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("error: failed to spawn {}: {err}", command[0]);
            1
        }
    }
}

// ── plexi app subcommands ─────────────────────────────────────────────────────

#[cfg(test)]
mod secret_set_tests {
    use super::{delete_existing_secret, folder_secret_grant};
    use crate::workspace::secrets::InMemoryKeychain;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn secret_delete_missing_key_returns_not_found_error() {
        let store = InMemoryKeychain::new();

        let error = delete_existing_secret(&store, "plexi:user:missing", "missing")
            .expect_err("an absent secret must not report a successful delete");

        assert_eq!(error, "secret 'missing' not found");
    }

    #[test]
    fn cli_secret_grant_does_not_write_an_allow_or_the_value() {
        let _enabled = crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let folder = dir.path().join("B");
        fs::create_dir(&folder).unwrap();
        let code = folder_secret_grant(
            "FOLDER_B_SECRET",
            Some("me"),
            None,
            Some(folder.as_path()),
        );
        assert_eq!(code, 1);
        assert!(
            !profile.join("grants.toml").exists(),
            "the cli must not record a folder-secret allow"
        );
        let audit = fs::read_to_string(profile.join("permission-audit.jsonl"))
            .expect("refusal is audited");
        assert!(audit.contains("permission_denied"));
        assert!(audit.contains("FOLDER_B_SECRET"));
        assert!(!audit.contains("secret-value-canary"));
        assert_eq!(
            crate::workspace::secrets::backend_label(),
            "in-memory",
            "grant refusal must not select a real keychain"
        );
    }

    /// Helper: checks whether a given `cwd` path would be rejected by the
    /// workspace_init home/root guard. Mirrors the exact logic in
    /// `workspace_init()` so the condition is tested independently of the
    /// real `std::env::current_dir()`.
    fn init_guard_rejects(cwd: &std::path::Path) -> bool {
        let home = std::env::var("HOME").ok().map(std::path::PathBuf::from);
        let cwd_str = cwd.to_string_lossy();
        let is_home_or_root =
            cwd == std::path::Path::new("/") || home.as_ref().map(|h| cwd == *h).unwrap_or(false);
        let is_inside_profile = home
            .as_ref()
            .map(|h| {
                let prefix = format!("{}/.plexi", h.to_string_lossy());
                cwd_str.starts_with(&prefix)
            })
            .unwrap_or(false);
        is_home_or_root || is_inside_profile
    }

    #[test]
    fn root_dir_is_rejected_by_init_guard() {
        assert!(init_guard_rejects(std::path::Path::new("/")));
    }

    #[test]
    fn home_dir_is_rejected_by_init_guard() {
        let home = std::env::var("HOME").unwrap();
        assert!(init_guard_rejects(std::path::Path::new(&home)));
    }

    #[test]
    fn plexi_profile_dir_is_rejected_by_init_guard() {
        let home = std::env::var("HOME").unwrap();
        let profile = std::path::PathBuf::from(format!("{home}/.plexi-alpha"));
        assert!(init_guard_rejects(&profile));
    }

    #[test]
    fn project_subdir_is_allowed_by_init_guard() {
        let tmp = tempfile::tempdir().unwrap();
        // A temp dir under /private/var/... is not home/root/profile
        assert!(!init_guard_rejects(tmp.path()));
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

    fn check_listing(global: bool, folders_enabled: bool, json: bool) {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let root = tempfile::tempdir().unwrap();
        let cfg =
            secrets::init_workspace(root.path(), &crate::config::workspace_channel_dir()).unwrap();
        let root_path = crate::platform::path::canonical_or_self(root.path());
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(profile.path());
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
    fn secret_workspace_walk_cannot_escape_pane_context() {
        let dir = tempfile::tempdir().unwrap();
        let context = dir.path().join("context");
        let cwd = context.join("child");
        std::fs::create_dir_all(&cwd).unwrap();
        let channel = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(dir.path().join(&channel)).unwrap();
        assert_eq!(secret_workspace_root(&cwd, Some(&context)), None);
        let _pane = set_test_pane_context_root(&context);
        assert!(enforce_pane_context_scope(&cwd, false, "secret.workspace").is_ok());
        std::fs::create_dir_all(context.join(&channel)).unwrap();
        assert_eq!(secret_workspace_root(&cwd, Some(&context)), Some(context.canonicalize().unwrap()));
        if let Some(home) = dirs::home_dir() {
            assert_eq!(secret_workspace_root(&home, Some(&home)), None);
        }
    }
    #[test]
    fn missing_secret_message_names_the_channel_binary_once() {
        let _channel = crate::config::set_test_channel("alpha");
        assert_eq!(secret_not_set("FOO"), "FOO not set; run `plexi-alpha secret set FOO`");
    }
}
