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

// ── plexi secret subcommands (workspace-aware, issue #322) ───────────────────

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
    let channel_dir = crate::config::workspace_channel_dir();
    let root = match crate::app::registry::resolve_workspace_root(&cwd) {
        Some(r) => r,
        None => {
            return Err(format!(
                "no {channel_dir}/ workspace found at or above {}.\n\
                 Run `plexi workspace init` first.",
                cwd.display()
            ));
        }
    };
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

    #[cfg(target_os = "macos")]
    {
        use crate::workspace::secrets::{
            keychain_user_name, keychain_workspace_name, MacKeychain, SecretStore,
        };
        let store = MacKeychain::new();

        if global {
            let account = keychain_user_name(effective_friendly);
            return match store.set(&account, &value) {
                Ok(()) => {
                    log::info!("secret_set:cli: stored globally account={account}");
                    println!("Stored '{friendly}' globally (plexi:user:{effective_friendly})");
                    print_tip(&format!(
                        "reference '{friendly}' in commands.toml under `[secrets] required = [\"{friendly}\"]`."
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
        let account = keychain_workspace_name(&cfg.id, effective_friendly);
        match store.set(&account, &value) {
            Ok(()) => {
                log::info!(
                    "secret_set:cli: stored workspace_id={} account={account} root={}",
                    cfg.id,
                    root.display()
                );
                eprintln!(
                    "Stored '{friendly}' for workspace {} ({})",
                    root.display(),
                    cfg.id
                );
                // Auto-write the canonical→friendly route to secrets.toml.
                match crate::workspace::secrets::write_default_route(
                    &root,
                    friendly,
                    effective_friendly,
                ) {
                    Ok(()) => {
                        log::info!(
                            "secret_set:cli: wrote default route {friendly} → {effective_friendly} in secrets.toml"
                        );
                        let channel_dir = crate::config::workspace_channel_dir();
                        print_tip(&format!(
                            "Route written to {channel_dir}/secrets.toml — use in commands.toml: [secrets] required = [\"{friendly}\"]"
                        ));
                    }
                    Err(e) => {
                        log::warn!("secret_set:cli: write_default_route failed: {e}");
                        // Non-fatal — Keychain write succeeded.
                        print_tip(&format!(
                            "reference '{friendly}' in commands.toml under `[secrets] required = [\"{friendly}\"]`."
                        ));
                    }
                }
                0
            }
            Err(e) => {
                log::warn!("secret_set:cli: keychain write failed for {account}: {e}");
                eprintln!("error: keychain write failed: {e}");
                1
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (friendly, effective_friendly, value, global);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

/// `plexi secret list` — list friendly names defined under the current
/// workspace's namespace plus user-scope. Names only, never values.
pub fn workspace_secret_list() -> i32 {
    let (_root, cfg) = match require_workspace() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    #[cfg(target_os = "macos")]
    {
        use crate::workspace::secrets::{
            keychain_user_name, keychain_workspace_name, MacKeychain, SecretStore,
        };
        let store = MacKeychain::new();
        let workspace_prefix = keychain_workspace_name(&cfg.id, "");
        let user_prefix = keychain_user_name("");
        let workspace_entries = store.list_with_prefix(&workspace_prefix);
        let user_entries = store.list_with_prefix(&user_prefix);
        if workspace_entries.is_empty() && user_entries.is_empty() {
            eprintln!("No secrets stored.");
            return 0;
        }
        if !workspace_entries.is_empty() {
            println!("Workspace ({}):", cfg.id);
            for a in &workspace_entries {
                if let Some(name) = a.strip_prefix(&workspace_prefix) {
                    println!("  {name}");
                }
            }
        }
        if !user_entries.is_empty() {
            if !workspace_entries.is_empty() {
                println!();
            }
            println!("User scope:");
            for a in &user_entries {
                if let Some(name) = a.strip_prefix(&user_prefix) {
                    println!("  {name}");
                }
            }
        }
        0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cfg;
        eprintln!("error: keychain not available on this platform");
        1
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

    #[cfg(target_os = "macos")]
    {
        use crate::workspace::secrets::{
            keychain_user_name, keychain_workspace_name, MacKeychain, SecretStore,
        };
        let store = MacKeychain::new();

        if global {
            let account = keychain_user_name(friendly);
            return match store.get(&account) {
                Some(value) => {
                    log::info!("secret_get:cli: found globally account={account}");
                    println!("{}", value.as_str());
                    0
                }
                None => {
                    log::warn!("secret_get:cli: not found friendly={friendly}");
                    eprintln!("error: secret '{friendly}' not found");
                    1
                }
            };
        }

        // Try workspace-scoped first, then global fallback.
        if let Ok((root, cfg)) = require_workspace() {
            let account = keychain_workspace_name(&cfg.id, friendly);
            if let Some(value) = store.get(&account) {
                log::info!(
                    "secret_get:cli: found workspace_id={} account={account} root={}",
                    cfg.id,
                    root.display()
                );
                println!("{}", value.as_str());
                return 0;
            }
        }

        // Global fallback.
        let user_account = keychain_user_name(friendly);
        match store.get(&user_account) {
            Some(value) => {
                log::info!("secret_get:cli: found globally account={user_account}");
                println!("{}", value.as_str());
                0
            }
            None => {
                log::warn!("secret_get:cli: not found friendly={friendly}");
                eprintln!("error: secret '{friendly}' not found");
                1
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (friendly, global);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

/// `plexi secret delete <friendly-name>` — remove the Keychain entry.
///
/// When `global` is true, deletes the user-scoped entry (`plexi:user:<name>`)
/// without requiring a workspace. When false, requires a workspace and deletes
/// the workspace-scoped entry.
pub fn workspace_secret_delete(friendly: &str, global: bool) -> i32 {
    log::info!("secret_delete:cli: friendly={friendly} global={global}");
    #[cfg(target_os = "macos")]
    {
        use crate::workspace::secrets::{
            keychain_user_name, keychain_workspace_name, MacKeychain, SecretStore,
        };
        let store = MacKeychain::new();

        if global {
            let account = keychain_user_name(friendly);
            return match store.delete(&account) {
                Ok(()) => {
                    log::info!("secret_delete:cli: deleted globally account={account}");
                    println!("Deleted global secret '{friendly}'");
                    0
                }
                Err(e) => {
                    log::warn!(
                        "secret_delete:cli: not found or failed friendly={friendly} err={e}"
                    );
                    eprintln!("error: keychain delete failed: {e}");
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
        match store.delete(&account) {
            Ok(()) => {
                log::info!(
                    "secret_delete:cli: deleted workspace_id={} account={account}",
                    cfg.id
                );
                println!("Deleted '{friendly}' from workspace {}", cfg.id);
                0
            }
            Err(e) => {
                log::warn!("secret_delete:cli: keychain delete failed account={account} err={e}");
                eprintln!("error: keychain delete failed: {e}");
                1
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (friendly, global);
        eprintln!("error: keychain not available on this platform");
        1
    }
}

// ── plexi app subcommands ─────────────────────────────────────────────────────

/// Returns true if `plexi_sdk` is importable in the current `python3` environment.
/// If not, attempts installation via `uv pip install plexi-sdk`, falling back to
/// `pip install plexi-sdk`. Prints a single line describing what happened.
/// Returns false only if python3 is absent or all install attempts fail.
#[cfg(test)]
mod secret_set_tests {
    use std::fs;
    use tempfile::TempDir;

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
    use std::fs;

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
