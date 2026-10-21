use serde::Deserialize;
use std::collections::HashMap;

pub(super) const APP_ID: &str = "plexi-run";
pub(super) fn commands_file() -> String {
    format!("{}/commands.toml", crate::config::workspace_channel_dir())
}

/// Parsed .plexi/commands.toml
#[derive(Deserialize)]
pub struct PlexiCommands {
    #[serde(default)]
    pub secrets: SecretsConfig,
    #[serde(default)]
    pub commands: HashMap<String, CommandEntry>,
}

#[derive(Deserialize, Default)]
pub struct SecretsConfig {
    #[serde(default)]
    pub required: Vec<String>,
}

/// A command entry: either a bare string (`build = "cargo build"`) or an inline table
/// (`build = { run = "cargo build", description = "..." }`).
/// The old nested-section form (`[commands.build]\nrun = "..."`) is TOML-equivalent to the
/// inline-table form and parses identically — no migration needed.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum CommandEntry {
    Simple(String),
    Full(CommandDef),
}

impl CommandEntry {
    pub fn run(&self) -> &str {
        match self {
            CommandEntry::Simple(s) => s,
            CommandEntry::Full(d) => &d.run,
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            CommandEntry::Simple(_) => None,
            CommandEntry::Full(d) => d.description.as_deref(),
        }
    }

    pub fn secrets(&self) -> &[String] {
        match self {
            CommandEntry::Simple(_) => &[],
            CommandEntry::Full(d) => &d.secrets,
        }
    }
}

#[derive(Deserialize)]
pub struct CommandDef {
    pub run: String,
    pub description: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

/// List executable files in a scripts directory.
pub(super) fn list_global_scripts(scripts_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(scripts_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            path.is_file() && is_executable(&path)
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

pub(super) fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

/// Default starter template written when `Edit commands` creates a missing
/// workspace or global `commands.toml`.
pub const COMMANDS_TEMPLATE: &str = "\
# Plexi Commands
#
# Define named shell commands for this scope. Run them from the command
# palette (Cmd+P) or the terminal (`plexi run <name>`, `plexi run --json`).
# Workspace commands override global commands of the same name.
#
# [commands]
# hello = \"echo hi\"
# dev = { run = \"npm run dev\", description = \"Start the dev server\" }
# deploy = { run = \"./deploy.sh\", description = \"Deploy\", secrets = [\"API_KEY\"] }

[commands]
";

/// Path to the channel-global `commands.toml` (`~/.plexi[-<channel>]/commands.toml`).
pub fn global_commands_path() -> std::path::PathBuf {
    crate::config::config_dir().join("commands.toml")
}

/// Ensure the effective `commands.toml` exists on disk, creating it from
/// [`COMMANDS_TEMPLATE`] when missing, and return its path.
///
/// When `workspace_root` is `Some(root)`, targets `<root>/<workspace_channel_dir()>/commands.toml`.
/// When `None`, targets `<config_dir()>/commands.toml`.
pub fn ensure_commands_file(workspace_root: Option<&std::path::Path>) -> std::path::PathBuf {
    let path = match workspace_root {
        Some(root) => root.join(commands_file()),
        None => global_commands_path(),
    };
    if !path.exists() {
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!(
                    "commands: failed to create directory {}: {e}",
                    parent.display()
                );
                return path;
            }
        }
        match std::fs::write(&path, COMMANDS_TEMPLATE) {
            Ok(()) => log::info!("commands: created template at {}", path.display()),
            Err(e) => log::warn!("commands: failed to write {}: {e}", path.display()),
        }
    }
    path
}

/// Resolve the workspace root for `commands.toml` lookup from `cwd`.
///
/// Walks up via [`crate::app::registry::resolve_workspace_root`]; if that
/// returns `None` (for example in a scratch directory under `$HOME`), falls
/// back to `cwd` itself when `cwd/<workspace_channel_dir()>/commands.toml`
/// exists and is distinct from [`global_commands_path`].
pub fn resolve_commands_workspace_root(cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    if let Some(root) = crate::app::registry::resolve_workspace_root(cwd) {
        return Some(root);
    }
    let candidate = cwd.join(commands_file());
    if candidate.is_file() && candidate != global_commands_path() {
        return Some(cwd.to_path_buf());
    }
    None
}

pub(super) fn merged_secret_names(config: &PlexiCommands, entry: &CommandEntry) -> Vec<String> {
    let mut keys: Vec<String> = config.secrets.required.clone();
    for k in entry.secrets() {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    keys
}

/// Scope of a user-authored command surfaced in the command palette and `plexi run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UserCommandScope {
    /// Defined in the focused workspace's channel-scoped `commands.toml`.
    Workspace,
    /// Defined in the global profile (`config_dir()/commands.toml` or `config_dir()/scripts/`).
    Global,
}

impl UserCommandScope {
    pub fn as_str(self) -> &'static str {
        match self {
            UserCommandScope::Workspace => "workspace",
            UserCommandScope::Global => "global",
        }
    }
}

/// A user-authored command resolved for the command palette and `plexi run`.
///
/// Mirrors what `plexi run <name>` resolves from the given workspace cwd:
/// global commands (`config_dir()/commands.toml` and executable scripts in
/// `config_dir()/scripts/`) merge with workspace commands
/// (`<workspace>/.plexi[-channel]/commands.toml`), with workspace winning on
/// name collisions. `secrets` lists required secret names only — never values.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ResolvedUserCommand {
    pub name: String,
    pub scope: UserCommandScope,
    pub description: Option<String>,
    /// Shell snippet for `commands.toml` entries (shown as preview text).
    /// `None` for global scripts — the run target is the script file itself.
    pub run: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

/// Resolve merged global + workspace user commands, returning an error string if
/// a present `commands.toml` cannot be read or parsed.
///
/// Precedence (highest wins per command `name`):
/// 1. Workspace `<workspace_root>/<workspace_channel_dir()>/commands.toml` (`scope = workspace`)
/// 2. Global `<config_dir()>/commands.toml` (`scope = global`)
/// 3. Executable scripts in `<config_dir()>/scripts/` (`scope = global`)
pub fn resolve_user_commands_checked(
    workspace_root: Option<&std::path::Path>,
) -> Result<Vec<ResolvedUserCommand>, String> {
    let mut merged: std::collections::BTreeMap<String, ResolvedUserCommand> =
        std::collections::BTreeMap::new();

    // 1. Global scripts (`config_dir()/scripts/`).
    let scripts_dir = crate::config::config_dir().join("scripts");
    for name in list_global_scripts(&scripts_dir) {
        merged.insert(
            name.clone(),
            ResolvedUserCommand {
                name,
                scope: UserCommandScope::Global,
                description: None,
                run: None,
                secrets: Vec::new(),
            },
        );
    }

    // 2. Global `config_dir()/commands.toml` (overrides same-named global scripts).
    let global_path = global_commands_path();
    match std::fs::read_to_string(&global_path) {
        Ok(contents) => {
            let config: PlexiCommands = toml::from_str(&contents)
                .map_err(|e| format!("failed to parse {}: {e}", global_path.display()))?;
            for (name, entry) in &config.commands {
                merged.insert(
                    name.clone(),
                    ResolvedUserCommand {
                        name: name.clone(),
                        scope: UserCommandScope::Global,
                        description: entry.description().map(|s| s.to_string()),
                        run: Some(entry.run().to_string()),
                        secrets: merged_secret_names(&config, entry),
                    },
                );
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!("could not read {}: {e}", global_path.display()));
        }
    }

    // 3. Workspace `<workspace_root>/.plexi[-channel]/commands.toml` (overrides global per name).
    if let Some(root) = workspace_root {
        let rel_file = commands_file();
        let config_path = root.join(&rel_file);
        if config_path != global_path {
            match std::fs::read_to_string(&config_path) {
                Ok(contents) => {
                    let config: PlexiCommands = toml::from_str(&contents)
                        .map_err(|e| format!("failed to parse {rel_file}: {e}"))?;
                    for (name, entry) in &config.commands {
                        merged.insert(
                            name.clone(),
                            ResolvedUserCommand {
                                name: name.clone(),
                                scope: UserCommandScope::Workspace,
                                description: entry.description().map(|s| s.to_string()),
                                run: Some(entry.run().to_string()),
                                secrets: merged_secret_names(&config, entry),
                            },
                        );
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(format!("could not read {}: {e}", config_path.display()));
                }
            }
        }
    }

    let cmds: Vec<ResolvedUserCommand> = merged.into_values().collect();
    log::info!(
        "commands: resolved {} merged command(s) (workspace_root={:?})",
        cmds.len(),
        workspace_root
    );
    Ok(cmds)
}

/// Resolve user commands for the command palette, mirroring `plexi run`
/// per-name layering: global commands (`config_dir()/commands.toml` and
/// `config_dir()/scripts/`) merge with workspace commands
/// (`<workspace>/.plexi[-channel]/commands.toml`), and workspace entries win on
/// name collisions. A present-but-unparseable `commands.toml` yields no
/// commands rather than silently masking the parse error, because `plexi run`
/// errors in that state.
///
/// Reads the files fresh on every call, so the palette hot-reloads on the next
/// open with no mtime bookkeeping — same pattern as palette note loading.
pub fn resolve_user_commands(workspace_root: Option<&std::path::Path>) -> Vec<ResolvedUserCommand> {
    match resolve_user_commands_checked(workspace_root) {
        Ok(cmds) => cmds,
        Err(err) => {
            log::warn!("palette: {err}; no commands surfaced");
            Vec::new()
        }
    }
}

pub mod args;
pub mod crawl;
pub mod help;
pub mod introspect;
pub mod registry;
pub mod setup;
mod pane_caller;
#[cfg(test)]
mod skill_surface;
pub mod experiments;
pub mod skill_install;
pub mod subprocess;
#[cfg(test)]
pub mod test_env;

pub mod account;
pub(crate) mod lease;
pub mod agent;
pub mod agents_api;
pub mod ai;
pub mod app;
pub mod app_check;
pub mod changes;
pub mod app_state;
pub mod completions;
pub mod config_cli;
pub mod context_cli;
pub mod demo;
pub mod descriptor;
pub mod doctor;
pub mod events;
pub mod host;
pub mod install;
pub mod debug;
pub mod ledger;
mod install_hint;
pub mod install_host;
pub mod list;
pub mod marketplace;
pub mod notes;
pub mod notify;
pub mod open;
pub mod pane;
pub mod permissions;
pub mod registry_watch;
pub mod relay;
mod relay_crypto;
mod relay_ws;
pub mod release_resolver;
pub mod routine;
pub mod run;
pub mod updater;
pub mod validate;
pub mod workspace;

pub(super) fn print_tip(msg: &str) {
    let config = crate::config::PlexiConfig::load_with_workspace(
        crate::config::active_workspace_root().as_deref(),
    );
    let enabled = config.cli.as_ref().and_then(|c| c.tips).unwrap_or(true);
    if enabled {
        log::info!("cli:tip: {msg}");
        if std::env::var_os("NO_COLOR").is_none() {
            eprintln!("\x1b[2mtip: {msg}\x1b[0m");
        } else {
            eprintln!("tip: {msg}");
        }
    }
}

/// Prints JSON to stdout. If `jq` is in PATH, pipes through `jq .` for
/// colour and pretty-printing; otherwise falls back to serde pretty-print.
/// Content that is not JSON at all is passed through verbatim — the host's
/// reply is never withheld from the caller.
pub(super) fn print_json_output(json_str: &str) -> i32 {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let jq_available = Command::new("jq")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if jq_available {
        match Command::new("jq").arg(".").stdin(Stdio::piped()).spawn() {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    if let Err(e) = stdin.write_all(json_str.as_bytes()) {
                        log::warn!("print_json_output: failed writing to jq stdin ({e})");
                    }
                }
                match child.wait() {
                    Ok(status) if status.success() => {
                        log::info!("print_json_output: rendered via jq");
                        return 0;
                    }
                    Ok(status) => {
                        log::warn!("print_json_output: jq exited non-zero ({status}), falling back to serde");
                    }
                    Err(e) => {
                        log::warn!("print_json_output: jq wait failed ({e}), falling back to serde");
                    }
                }
            }
            Err(e) => {
                log::warn!("print_json_output: jq spawn failed ({e}), falling back to serde");
            }
        }
    }

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(v) => match serde_json::to_string_pretty(&v) {
            Ok(pretty) => {
                println!("{pretty}");
                0
            }
            Err(e) => {
                eprintln!("error: could not serialize: {e}");
                1
            }
        },
        Err(_) => {
            print!("{json_str}");
            0
        }
    }
}

#[cfg(test)]
mod reply_error_tests {
    use super::reply_error;

    /// A host-reported failure must be detectable so the CLI can exit nonzero —
    /// the poll succeeds here, because the transport worked.
    #[test]
    fn error_field_is_surfaced() {
        assert_eq!(
            reply_error(r#"{"error":"no parent context for id=Some(9)"}"#).as_deref(),
            Some("no parent context for id=Some(9)")
        );
    }

    #[test]
    fn successful_response_has_no_error() {
        assert_eq!(
            reply_error(r#"{"context_id":76,"windows":[],"panes":[317,318]}"#),
            None
        );
    }

    /// Unparseable output is not an error here: the poll owns transport
    /// failures, and swallowing an unexpected-but-valid shape would hide the
    /// response from the caller.
    #[test]
    fn unparseable_output_is_not_treated_as_an_error() {
        assert_eq!(reply_error("not json at all"), None);
    }
}

#[cfg(test)]
mod socket_resolution_tests {
    use super::{command_socket_available_from, resolve_command_socket_from};
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    #[test]
    fn binary_channel_wins_over_ambient_socket() {
        let actual = resolve_command_socket_from(
            None,
            Some("pr-2765"),
            Some(OsStr::new("/Users/test/.plexi-beta/notify.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(
            actual,
            Some(crate::platform::ipc::endpoint_in(Path::new(
                "/Users/test/.plexi-pr-2765"
            )))
        );
    }

    #[test]
    fn alpha_binary_ignores_ambient_beta_socket() {
        let actual = resolve_command_socket_from(
            None,
            Some("alpha"),
            Some(OsStr::new("/tmp/beta.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(
            actual,
            Some(crate::platform::ipc::endpoint_in(Path::new(
                "/Users/test/.plexi-alpha"
            )))
        );
    }

    #[test]
    fn bare_binary_preserves_ambient_socket() {
        let actual = resolve_command_socket_from(
            None,
            None,
            Some(OsStr::new("/tmp/ambient.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(actual, Some(PathBuf::from("/tmp/ambient.sock")));
    }

    #[test]
    fn suffixed_binary_matching_ambient_socket_resolves_same_endpoint() {
        let actual = resolve_command_socket_from(
            None,
            Some("alpha"),
            Some(OsStr::new("/Users/test/.plexi-alpha/notify.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(
            actual,
            Some(crate::platform::ipc::endpoint_in(Path::new(
                "/Users/test/.plexi-alpha"
            )))
        );
    }

    #[test]
    fn explicit_socket_override_wins_for_suffixed_binary() {
        let actual = resolve_command_socket_from(
            Some(Path::new("/tmp/explicit.sock")),
            Some("pr-2384"),
            Some(OsStr::new("/tmp/ambient.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(actual, Some(PathBuf::from("/tmp/explicit.sock")));
    }

    #[test]
    fn explicit_socket_override_enables_direct_dispatch_without_ambient_socket() {
        assert!(command_socket_available_from(true, false));
    }

    #[test]
    fn binary_suffix_alone_does_not_enable_direct_dispatch() {
        let resolved =
            resolve_command_socket_from(None, Some("alpha"), None, Path::new("/Users/test"));

        assert_eq!(
            (resolved, command_socket_available_from(false, false)),
            (
                Some(PathBuf::from("/Users/test/.plexi-alpha/notify.sock")),
                false,
            )
        );
    }

    #[test]
    fn nested_channel_matrix_routes_without_env_stripping() {
        let home = Path::new("/Users/test");
        let inherited_env = [
            ("PLEXI_CHANNEL", "beta"),
            ("PLEXI_SOCKET", "/Users/test/.plexi-beta/notify.sock"),
            ("PLEXI_PANE_ID", "42"),
            ("PLEXI_CONTEXT_ID", "7"),
            ("PLEXI_CONTEXT_ROOT", "/Users/test/workspace"),
            ("PLEXI_RUNNING", "1"),
        ];
        let env_get = |key: &str| -> Option<&str> {
            inherited_env
                .iter()
                .find_map(|(k, v)| (*k == key).then_some(*v))
        };

        // Self-identity vars stay intact for default resolution (e.g. pane slot write).
        assert_eq!(env_get("PLEXI_PANE_ID"), Some("42"));
        assert_eq!(env_get("PLEXI_CONTEXT_ID"), Some("7"));
        assert_eq!(env_get("PLEXI_CONTEXT_ROOT"), Some("/Users/test/workspace"));
        assert_eq!(env_get("PLEXI_RUNNING"), Some("1"));

        // (1) Channel-named shim with inherited beta routing + self-identity env
        // resolves both profile dir and command socket to the shim's channel.
        let shim_basename = "plexi-pr-2765";
        let shim_channel = crate::config::channel_from_basename(shim_basename);
        assert_eq!(shim_channel, Some("pr-2765"));
        let shim_profile_dir =
            crate::config::resolve_channel_dir(shim_basename, env_get("PLEXI_CHANNEL"));
        let shim_socket = resolve_command_socket_from(
            None,
            shim_channel,
            env_get("PLEXI_SOCKET").map(OsStr::new),
            home,
        );
        assert_eq!(shim_profile_dir, ".plexi-pr-2765");
        assert_eq!(
            shim_socket,
            Some(crate::platform::ipc::endpoint_in(
                &home.join(".plexi-pr-2765")
            ))
        );

        // (2) Explicit `--socket` overrides the shim's channel and ambient env.
        let explicit_socket = Path::new("/tmp/custom-override.sock");
        let overridden_socket = resolve_command_socket_from(
            Some(explicit_socket),
            shim_channel,
            env_get("PLEXI_SOCKET").map(OsStr::new),
            home,
        );
        assert_eq!(overridden_socket, Some(explicit_socket.to_path_buf()));

        // (3) Bare `plexi` adopts inherited `PLEXI_CHANNEL` and `PLEXI_SOCKET`.
        let bare_basename = "plexi";
        let bare_channel = crate::config::channel_from_basename(bare_basename);
        assert_eq!(bare_channel, None);
        let bare_profile_dir =
            crate::config::resolve_channel_dir(bare_basename, env_get("PLEXI_CHANNEL"));
        let bare_socket = resolve_command_socket_from(
            None,
            bare_channel,
            env_get("PLEXI_SOCKET").map(OsStr::new),
            home,
        );
        assert_eq!(bare_profile_dir, ".plexi-beta");
        assert_eq!(
            bare_socket,
            Some(PathBuf::from("/Users/test/.plexi-beta/notify.sock"))
        );
    }
}

/// Resolve the command socket endpoint from target-routing inputs.
///
/// Precedence: explicit `--socket` > binary channel identity (`plexi-<channel>`)
/// > ambient `PLEXI_SOCKET` (bare `plexi` only) > `None`.
///
/// See `src/cli/AGENTS.md` ("Environment classification (`self-identity` vs
/// `target-routing`)") for the canonical precedence table. Self-identity env
/// vars (`PLEXI_PANE_ID`, `PLEXI_CONTEXT_ID`, `PLEXI_CONTEXT_ROOT`,
/// `PLEXI_RUNNING`) are never consulted for socket routing.
fn resolve_command_socket_from(
    explicit_socket: Option<&std::path::Path>,
    binary_channel: Option<&str>,
    ambient_socket: Option<&std::ffi::OsStr>,
    home_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if let Some(socket) = explicit_socket {
        return Some(socket.to_path_buf());
    }
    // Binary identity outranks inherited routing env: a channel-named shim
    // self-binds to its own profile endpoint and ignores an ambient
    // PLEXI_SOCKET stamped by whichever host spawned the pane.
    if let Some(channel) = binary_channel {
        return Some(crate::platform::ipc::endpoint_in(
            &home_dir.join(format!(".plexi-{channel}")),
        ));
    }
    ambient_socket.map(std::path::PathBuf::from)
}

static COMMAND_SOCKET_OVERRIDE: std::sync::OnceLock<std::path::PathBuf> =
    std::sync::OnceLock::new();

pub fn set_command_socket_override(path: std::path::PathBuf) {
    if COMMAND_SOCKET_OVERRIDE.set(path.clone()).is_err() {
        log::warn!("cli: command socket override was already set; keeping the first path");
    } else {
        log::info!("cli: set explicit command socket override path={path:?}");
    }
}

fn command_socket_available_from(explicit_override: bool, ambient_socket: bool) -> bool {
    explicit_override || ambient_socket
}

pub(super) fn command_socket_available() -> bool {
    command_socket_available_from(
        COMMAND_SOCKET_OVERRIDE.get().is_some(),
        std::env::var_os("PLEXI_SOCKET").is_some(),
    )
}

/// Poll an RPC response file with the standard CLI error reporting: timeout
/// and read failures print `error: ...` to stderr and yield exit code 1.
/// `label` names the command in the timeout message.
pub(super) fn poll_rpc(response_file: &str, label: &str) -> Result<String, i32> {
    poll_rpc_with(response_file, label, crate::rpc::DEFAULT_TIMEOUT)
}

pub(super) fn poll_rpc_with(
    response_file: &str,
    label: &str,
    timeout: std::time::Duration,
) -> Result<String, i32> {
    match crate::rpc::poll_string(response_file, Some(timeout)) {
        Ok(content) => Ok(content),
        Err(crate::rpc::PollError::TimedOut) => {
            eprintln!("error: timed out waiting for {label} response");
            Err(1)
        }
        Err(e) => {
            log::warn!("{label}:cli: {e}");
            eprintln!("error: {e}");
            Err(1)
        }
    }
}

/// Mint a response file for `prefix`, inject it into `payload`, and send the
/// request over the command socket.
///
/// Returns the response-file path once the whole newline-framed request has
/// been accepted, or the CLI exit code when the transport failed. The one
/// place the `response_file` key is put on a request — a command that needs a
/// non-standard poll (raw bytes, slot replies) sends here and polls itself;
/// everything else uses [`request`].
pub(super) fn send_request(
    payload: serde_json::Value,
    prefix: &str,
    label: &str,
) -> Result<String, i32> {
    send_request_connected(payload, prefix, label).map(|(path, _stream)| path)
}

fn send_request_connected(
    mut payload: serde_json::Value,
    prefix: &str,
    label: &str,
) -> Result<(String, crate::platform::ipc::IpcStream), i32> {
    let response_file = crate::rpc::response_file(prefix, "json");
    let Some(fields) = payload.as_object_mut() else {
        log::error!("{label}:cli: request payload is not a JSON object");
        eprintln!("error: internal: {label} request payload is not a JSON object");
        return Err(1);
    };
    fields.insert(
        "response_file".to_string(),
        serde_json::Value::String(response_file.clone()),
    );
    log::info!("{label}:cli: request response_file={response_file:?}");
    let stream = send_to_socket_connected(payload)?;
    Ok((response_file, stream))
}

/// Send a request and wait for the host's reply: the standard CLI round trip.
/// `label` names the command in the timeout and poll error messages.
pub(super) fn request(
    payload: serde_json::Value,
    prefix: &str,
    label: &str,
) -> Result<String, i32> {
    request_with(payload, prefix, label, crate::rpc::DEFAULT_TIMEOUT)
}

/// [`request`] with an explicit poll window, for verbs the host answers on its
/// own longer deadline.
pub(super) fn request_with(
    payload: serde_json::Value,
    prefix: &str,
    label: &str,
    timeout: std::time::Duration,
) -> Result<String, i32> {
    // macOS cannot recover LOCAL_PEERPID after the client disconnects, even
    // while this CLI process is alive. Hold the connection through the reply.
    let (response_file, _stream) = send_request_connected(payload, prefix, label)?;
    poll_rpc_with(&response_file, label, timeout)
}

/// The host's top-level `error` string, if the reply is JSON and carries one.
///
/// A reply that does not parse is not an error here: transport failures are
/// already owned by [`poll_rpc`], and swallowing a valid-but-unexpected shape
/// would hide the response from the caller.
pub(super) fn reply_error(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_string)
}

/// Surface a host-reported `error` as `error: <msg>` on stderr with exit code 1.
/// The shared postlude for commands whose success payload carries no `error`.
pub(super) fn check_reply_error(content: &str) -> Result<(), i32> {
    match reply_error(content) {
        Some(msg) => {
            eprintln!("error: {msg}");
            Err(1)
        }
        None => Ok(()),
    }
}

/// Spawn-queue writer gate (stint 0532): outside a Plexi pane, a spawn
/// request may only be queued when this channel's host is actually accepting
/// IPC on its notify socket. A file queued into the void fires on some later
/// host boot as a window nothing attributes — an implicit desktop escape.
/// Fail loudly and typed instead. `plexi host start` is exempt by design: it
/// seeds the queue explicitly and launches the draining host itself in the
/// same command.
pub(super) fn require_spawn_servicing_host(intent: &str) -> Result<(), i32> {
    let socket = crate::platform::ipc::endpoint_path();
    if spawn_socket_accepting(&socket) {
        return Ok(());
    }
    let channel = channel_label_from(
        crate::config::config_dir()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(""),
    );
    log::warn!(
        "spawn_gate: refused '{intent}' — host not servicing pane spawns on channel {channel} (socket {socket:?})"
    );
    eprintln!("{}", spawn_gate_error(&channel, &socket));
    print_tip("start a host for this channel with `plexi host start`, or run this from inside a Plexi pane");
    Err(1)
}

pub(super) fn spawn_socket_accepting(socket: &std::path::Path) -> bool {
    crate::platform::ipc::IpcStream::connect(socket).is_ok()
}

/// Human channel name from a profile dir basename: `.plexi-alpha` → `alpha`,
/// bare `.plexi` → `default`.
fn channel_label_from(profile_dir_basename: &str) -> String {
    match profile_dir_basename.strip_prefix(".plexi-") {
        Some(ch) if !ch.is_empty() => ch.to_string(),
        _ => "default".to_string(),
    }
}

fn spawn_gate_error(channel: &str, socket: &std::path::Path) -> String {
    format!(
        "error: host not servicing pane spawns on channel {channel} (no running host at {}); nothing was spawned",
        socket.display()
    )
}

/// Unix-epoch milliseconds stamp for spawn-queue file attribution.
pub(super) fn spawn_queued_at_ms() -> u64 {
    crate::platform::clock::now_millis() as u64
}

fn command_binary_channel() -> Option<String> {
    let channel = crate::config::build_channel();
    #[cfg(test)]
    {
        let is_cargo_test_artifact = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.ends_with("deps")))
            .unwrap_or(false);
        if is_cargo_test_artifact {
            return None;
        }
    }
    channel
}

pub(super) fn resolve_command_socket() -> Option<std::path::PathBuf> {
    let channel = command_binary_channel();
    let ambient = std::env::var_os("PLEXI_SOCKET");
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let explicit = COMMAND_SOCKET_OVERRIDE.get();
    let socket = resolve_command_socket_from(
        explicit.map(std::path::PathBuf::as_path),
        channel.as_deref(),
        ambient.as_deref(),
        &home,
    );
    if let Some(path) = socket.as_ref() {
        let source = if explicit.is_some() {
            "--socket"
        } else if channel.is_some() {
            "binary-channel"
        } else {
            "PLEXI_SOCKET"
        };
        if explicit.is_none() && channel.is_some() {
            if let Some(ignored) = ambient.as_ref() {
                if ignored.as_os_str() != path.as_os_str() {
                    log::info!(
                        "cli: ignored ambient PLEXI_SOCKET={ignored:?} because binary identity names channel {channel:?}"
                    );
                }
            }
        }
        log::info!(
            "cli: resolved command socket source={source} binary_channel={channel:?} path={path:?}"
        );
    }
    socket
}

const SOCKET_TRANSPORT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug)]
enum SocketTransportError {
    ConnectTimeout,
    Connect(std::io::Error),
    /// Only the Unix raw-fd transport can stop part-way through a frame and
    /// report how far it got; the Windows path hands the whole line to the
    /// pipe buffer in one call.
    #[cfg(unix)]
    WriteTimeout {
        bytes_written: usize,
    },
    Write {
        source: std::io::Error,
        bytes_written: usize,
    },
}

/// Bounded connect + write for a fire-and-forget CLI request.
///
/// Unix takes the raw-fd path below rather than `UnixStream`: std offers no
/// `connect_timeout` for `AF_UNIX`, and a plain `connect` against a listener
/// with a full backlog blocks forever — a hung terminal for the caller. A
/// non-blocking socket driven by `poll` is the only way to hold a deadline
/// across both the connect and the write.
#[cfg(unix)]
fn wait_for_socket(
    fd: std::os::fd::RawFd,
    events: libc::c_short,
    deadline: std::time::Instant,
) -> std::io::Result<bool> {
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        let remaining = deadline.saturating_duration_since(now);
        let timeout_ms = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
        let mut poll_fd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
        if rc > 0 {
            return Ok(true);
        }
        if rc == 0 {
            return Ok(false);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(unix)]
fn classify_connect_wait(result: std::io::Result<bool>) -> Result<(), SocketTransportError> {
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(SocketTransportError::ConnectTimeout),
        Err(error) => Err(SocketTransportError::Connect(error)),
    }
}

#[cfg(unix)]
fn connect_unix_deadline(
    socket_path: &std::path::Path,
    deadline: std::time::Instant,
) -> Result<std::os::fd::OwnedFd, SocketTransportError> {
    use std::os::fd::FromRawFd as _;
    use std::os::unix::ffi::OsStrExt as _;

    let path = socket_path.as_os_str().as_bytes();
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.len() >= addr.sun_path.len() {
        return Err(SocketTransportError::Connect(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Unix socket path is too long",
        )));
    }
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        addr.sun_len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1) as u8;
    }
    for (dst, src) in addr.sun_path.iter_mut().zip(path.iter().copied()) {
        *dst = src as libc::c_char;
    }

    let addr_len =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1) as libc::socklen_t;

    // A full accept queue is answered with EAGAIN, not EINPROGRESS, and the
    // difference matters: EINPROGRESS means the connect started and will
    // complete, while EAGAIN means it never started and must be reissued
    // (unix(7)). Waiting for POLLOUT on a socket that is not connecting
    // returns immediately — writable, with SO_ERROR 0 — so treating EAGAIN
    // like EINPROGRESS hands back an unconnected fd whose first write fails
    // ENOTCONN, reporting a saturated host as a write-phase failure. Each
    // retry gets a fresh socket; the loop is bounded by the caller's
    // deadline.
    let mut backlog_retries = 0u32;
    loop {
        let raw_fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        if raw_fd < 0 {
            return Err(SocketTransportError::Connect(
                std::io::Error::last_os_error(),
            ));
        }
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw_fd) };
        let flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(raw_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(SocketTransportError::Connect(
                std::io::Error::last_os_error(),
            ));
        }

        let rc = unsafe {
            libc::connect(
                raw_fd,
                &addr as *const libc::sockaddr_un as *const libc::sockaddr,
                addr_len,
            )
        };
        if rc == 0 {
            return Ok(fd);
        }
        let error = std::io::Error::last_os_error();
        let code = error.raw_os_error();
        if code.is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK) {
            if std::time::Instant::now() >= deadline {
                log::info!(
                    "cli_transport: listener backlog full at {} after {backlog_retries} retries — connect deadline expired",
                    socket_path.display()
                );
                return Err(SocketTransportError::ConnectTimeout);
            }
            backlog_retries += 1;
            drop(fd);
            std::thread::sleep(std::time::Duration::from_millis(1));
            continue;
        }
        if code != Some(libc::EINPROGRESS) {
            return Err(SocketTransportError::Connect(error));
        }

        classify_connect_wait(wait_for_socket(raw_fd, libc::POLLOUT, deadline))?;
        let mut socket_error: libc::c_int = 0;
        let mut socket_error_len = std::mem::size_of_val(&socket_error) as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                raw_fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                &mut socket_error as *mut _ as *mut libc::c_void,
                &mut socket_error_len,
            )
        };
        if rc < 0 {
            return Err(SocketTransportError::Connect(
                std::io::Error::last_os_error(),
            ));
        }
        if socket_error != 0 {
            return Err(SocketTransportError::Connect(
                std::io::Error::from_raw_os_error(socket_error),
            ));
        }
        return Ok(fd);
    }
}

#[cfg(unix)]
fn send_line_to_socket(
    socket_path: &std::path::Path,
    line: &[u8],
    timeout: std::time::Duration,
) -> Result<crate::platform::ipc::IpcStream, SocketTransportError> {
    use std::os::fd::AsRawFd as _;

    let deadline = std::time::Instant::now() + timeout;
    let fd = connect_unix_deadline(socket_path, deadline)?;
    let raw_fd = fd.as_raw_fd();
    let mut written = 0;
    while written < line.len() {
        if std::time::Instant::now() >= deadline {
            return Err(SocketTransportError::WriteTimeout {
                bytes_written: written,
            });
        }
        let rc = unsafe {
            libc::write(
                raw_fd,
                line[written..].as_ptr() as *const libc::c_void,
                line.len() - written,
            )
        };
        if rc > 0 {
            written += rc as usize;
            continue;
        }
        if rc == 0 {
            return Err(SocketTransportError::Write {
                source: std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "socket accepted zero bytes",
                ),
                bytes_written: written,
            });
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if error
            .raw_os_error()
            .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
        {
            match wait_for_socket(raw_fd, libc::POLLOUT, deadline) {
                Ok(true) => continue,
                Ok(false) => {
                    return Err(SocketTransportError::WriteTimeout {
                        bytes_written: written,
                    });
                }
                Err(source) => {
                    return Err(SocketTransportError::Write {
                        source,
                        bytes_written: written,
                    });
                }
            }
        }
        return Err(SocketTransportError::Write {
            source: error,
            bytes_written: written,
        });
    }
    Ok(fd.into())
}

/// Windows counterpart of the raw-fd path above.
///
/// `ipc::connect_timeout` already bounds the connect (`WaitNamedPipeW` with
/// the remaining budget), so only the write needs covering here. A request is
/// one JSON line — far under the pipe's 64 KiB buffer — so the write lands in
/// the kernel buffer and returns without waiting for the host to read it.
/// There is therefore no blocking window for a write deadline to police, and
/// `WriteTimeout` is unreachable on this path.
#[cfg(windows)]
fn send_line_to_socket(
    socket_path: &std::path::Path,
    line: &[u8],
    timeout: std::time::Duration,
) -> Result<crate::platform::ipc::IpcStream, SocketTransportError> {
    use std::io::Write as _;

    let mut stream = crate::platform::ipc::connect_timeout(socket_path, timeout).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            SocketTransportError::ConnectTimeout
        } else {
            SocketTransportError::Connect(e)
        }
    })?;
    stream
        .write_all(line)
        .and_then(|()| stream.flush())
        .map_err(|source| SocketTransportError::Write {
            source,
            // `write_all` does not report a partial count; the frame either
            // landed whole or the caller must retry the whole thing.
            bytes_written: 0,
        })?;
    Ok(stream)
}

pub(super) fn send_to_socket(payload: serde_json::Value) -> i32 {
    match send_to_socket_connected(payload) {
        Ok(_) => 0,
        Err(code) => code,
    }
}

fn send_to_socket_connected(payload: serde_json::Value) -> Result<crate::platform::ipc::IpcStream, i32> {
    let socket_path = match resolve_command_socket() {
        Some(path) => path,
        None => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return Err(1);
        }
    };
    let line = format!("{}\n", payload);
    let started = std::time::Instant::now();
    match send_line_to_socket(&socket_path, line.as_bytes(), SOCKET_TRANSPORT_DEADLINE) {
        Ok(stream) => {
            log::info!(
                "cli: socket transport complete path={socket_path:?} bytes={} elapsed_ms={}",
                line.len(),
                started.elapsed().as_millis()
            );
            Ok(stream)
        }
        Err(SocketTransportError::Connect(e))
            if e.kind() == std::io::ErrorKind::ConnectionRefused =>
        {
            crate::platform::ipc::remove_stale_endpoint(&socket_path);
            eprintln!("error: Plexi is not responding (stale socket removed). Is Plexi running?");
            Err(1)
        }
        Err(SocketTransportError::ConnectTimeout) => {
            log::warn!(
                "cli: socket connect timed out path={socket_path:?} deadline_ms={}",
                SOCKET_TRANSPORT_DEADLINE.as_millis()
            );
            eprintln!(
                "error: timed out connecting to PLEXI_SOCKET {socket_path:?}; request was not delivered"
            );
            Err(1)
        }
        Err(SocketTransportError::Connect(e)) => {
            log::warn!("cli: socket connect failed path={socket_path:?}: {e}");
            eprintln!("error: could not connect to PLEXI_SOCKET {socket_path:?}: {e}");
            Err(1)
        }
        #[cfg(unix)]
        Err(SocketTransportError::WriteTimeout { bytes_written }) => {
            log::warn!(
                "cli: socket write timed out path={socket_path:?} bytes_written={bytes_written} payload_bytes={} deadline_ms={}",
                line.len(),
                SOCKET_TRANSPORT_DEADLINE.as_millis()
            );
            if bytes_written == 0 {
                eprintln!(
                    "error: timed out writing request to PLEXI_SOCKET {socket_path:?}; the incomplete frame was not delivered and is safe to retry"
                );
            } else {
                eprintln!(
                    "error: timed out writing request to PLEXI_SOCKET {socket_path:?} after {bytes_written}/{} bytes; the incomplete frame was not delivered and is safe to retry",
                    line.len()
                );
            }
            Err(1)
        }
        Err(SocketTransportError::Write {
            source,
            bytes_written,
        }) => {
            log::warn!(
                "cli: socket write failed path={socket_path:?} bytes_written={bytes_written} payload_bytes={}: {source}",
                line.len()
            );
            if bytes_written == 0 {
                eprintln!("error: could not write request to socket: {source}");
            } else {
                eprintln!(
                    "error: socket write failed after {bytes_written}/{} bytes: {source}; the incomplete frame was not delivered and is safe to retry",
                    line.len()
                );
            }
            Err(1)
        }
    }
}

#[cfg(test)]
mod transport_deadline_tests {
    use super::{connect_unix_deadline, send_line_to_socket, SocketTransportError};
    use std::io::{BufRead as _, BufReader};
    use std::os::fd::AsRawFd as _;
    use std::os::unix::net::UnixListener;
    use std::time::{Duration, Instant};

    fn unique_socket_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pxt-{label}-{}-{:x}",
            std::process::id(),
            crate::platform::clock::now_nanos() & 0xffff_ffff
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn stalled_write_returns_with_phase_and_partial_byte_count() {
        let dir = unique_socket_dir("write");
        let socket = dir.join("notify.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let peer = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept");
            std::thread::sleep(Duration::from_millis(500));
        });

        let started = Instant::now();
        let line = vec![b'x'; 32 * 1024 * 1024];
        let error = send_line_to_socket(&socket, &line, Duration::from_millis(100))
            .expect_err("stalled write must time out");
        let elapsed = started.elapsed();
        peer.join().expect("peer");

        assert!(matches!(
            error,
            SocketTransportError::WriteTimeout { bytes_written } if bytes_written > 0
        ));
        assert!(
            elapsed < Duration::from_millis(300),
            "write exceeded strict deadline: {elapsed:?}"
        );
    }

    #[test]
    fn saturated_listener_returns_connect_phase_within_deadline() {
        let dir = unique_socket_dir("connect");
        let socket = dir.join("notify.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        // How many connects a kernel queues before refusing is not portable:
        // `UnixListener::bind` picks its own backlog hint, and Linux caps it
        // at `net.core.somaxconn` (4096 by default), so the queue here is far
        // deeper than the 512 probes below. Re-`listen` with the smallest
        // backlog so saturation is a property of this test rather than of the
        // host kernel.
        let rc = unsafe { libc::listen(listener.as_raw_fd(), 0) };
        assert_eq!(
            rc,
            0,
            "shrinking the listen backlog: {}",
            std::io::Error::last_os_error()
        );
        let mut queued = Vec::new();
        let started = Instant::now();
        let error = loop {
            match connect_unix_deadline(&socket, Instant::now() + Duration::from_millis(25)) {
                Ok(fd) => queued.push(fd),
                Err(error) => break error,
            }
            assert!(queued.len() < 512, "listener backlog did not saturate");
        };
        assert!(matches!(
            error,
            SocketTransportError::ConnectTimeout | SocketTransportError::Connect(_)
        ));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "connect saturation probe exceeded bound"
        );
    }

    #[test]
    fn connect_poll_expiry_is_typed_as_connect_timeout() {
        let error = super::classify_connect_wait(Ok(false)).expect_err("timeout");
        assert!(matches!(error, SocketTransportError::ConnectTimeout));
    }

    #[test]
    fn normal_listener_receives_exact_payload_once() {
        let dir = unique_socket_dir("exact");
        let socket = dir.join("notify.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read request");
            line
        });
        let payload = b"{\"type\":\"transport_exactly_once\"}\n";
        send_line_to_socket(&socket, payload, Duration::from_secs(1)).expect("send");
        assert_eq!(
            handle.join().expect("listener"),
            String::from_utf8_lossy(payload)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn connected_request_preserves_peer_pid_until_reply() {
        use std::os::fd::AsRawFd;
        let dir = unique_socket_dir("peer-pid");
        let socket = dir.join("notify.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let client = send_line_to_socket(&socket, b"{}\n", Duration::from_secs(1)).unwrap();
        // Deliberately accept after the complete frame has been sent: this was
        // ENOTCONN when the sender dropped its descriptor before reply polling.
        let (server, _) = listener.accept().unwrap();
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of_val(&pid) as libc::socklen_t;
        let rc = unsafe { libc::getsockopt(server.as_raw_fd(), libc::SOL_LOCAL,
            libc::LOCAL_PEERPID, &mut pid as *mut _ as *mut libc::c_void, &mut len) };
        assert_eq!(rc, 0, "{}", std::io::Error::last_os_error());
        assert_eq!(pid as u32, std::process::id());
        drop(client);
    }
}

/// Best-effort wake of a running instance after a spawn-queue file write.
///
/// A fully idle Plexi produces zero frames and only drains the spawn-queue
/// during a frame, so without this nudge an outside-shell `plexi pane new`
/// hangs until the user touches the window. Connects to the channel
/// profile's `notify.sock` and sends a no-op `AppRequest::Wake`; the socket
/// listener requests a repaint, the resulting frame drains the queue.
///
/// All errors are ignored silently and intentionally: no instance running
/// means the queue is drained at next startup — the designed fallback.
pub(super) fn nudge_running_instance() {
    use crate::platform::ipc::{self, IpcStream};
    use std::io::Write;
    let path = ipc::endpoint_path();
    let Ok(mut stream) = IpcStream::connect(&path) else {
        return;
    };
    let Ok(payload) = serde_json::to_string(&crate::protocol::AppRequest::Wake) else {
        return;
    };
    if stream.write_all(format!("{payload}\n").as_bytes()).is_ok() {
        log::debug!("cli: sent wake nudge to running instance at {path:?}");
    }
}

/// Park a spawn request in this channel's spawn queue for the running host to
/// drain, and tell the caller what was queued.
///
/// The one writer for the queue: it stamps `origin` and `queued_at_ms` onto the
/// payload, writes a collision-proof `{nanos}-{uuid}.json`, and nudges the host
/// awake. Callers gate on [`require_spawn_servicing_host`] first — never queue
/// into a channel with no host to drain it. `intent` names the command in the
/// queue record; `summary` is what the caller sees echoed back
/// ("open <type_id>").
pub(super) fn queue_spawn(
    mut payload: serde_json::Value,
    intent: &str,
    summary: &str,
) -> Result<(), i32> {
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        log::warn!("queue_spawn: could not create {queue_dir:?}: {e}");
        eprintln!("error: could not create spawn queue: {e}");
        return Err(1);
    }

    let Some(fields) = payload.as_object_mut() else {
        log::error!("queue_spawn: '{intent}' payload is not a JSON object");
        eprintln!("error: could not write spawn request: payload is not a JSON object");
        return Err(1);
    };
    fields.insert(
        "origin".to_string(),
        serde_json::Value::String(intent.to_string()),
    );
    fields.insert("queued_at_ms".to_string(), spawn_queued_at_ms().into());

    // Nanosecond stamps alone collide when two spawns are queued in the same
    // instant, so every record carries a uuid as well.
    let nanos = crate::platform::clock::now_nanos();
    let file = queue_dir.join(format!("{nanos}-{}.json", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::write(&file, payload.to_string()) {
        log::warn!("queue_spawn: could not write {file:?}: {e}");
        eprintln!("error: could not write spawn request: {e}");
        return Err(1);
    }
    nudge_running_instance();
    log::info!("queue_spawn: queued '{intent}' as {file:?}");
    println!("queued: {summary}");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    Ok(())
}

/// Queue a spawn for a running host and wait for the host's launch result.
///
/// An outside-Plexi CLI has no command socket in its environment, but it can
/// still prove a host is servicing this channel and wake it through the notify
/// socket.  Giving that queued request a response file makes this path observe
/// the same typed launch error as an in-pane spawn instead of reporting a
/// misleading successful queue write.
pub(super) fn queue_spawn_and_wait(
    mut payload: serde_json::Value,
    intent: &str,
    summary: &str,
) -> Result<String, i32> {
    let response_file = crate::rpc::response_file("queued-spawn-response", "json");
    let Some(fields) = payload.as_object_mut() else {
        log::error!("queue_spawn_and_wait: '{intent}' payload is not a JSON object");
        eprintln!("error: could not write spawn request: payload is not a JSON object");
        return Err(1);
    };
    fields.insert(
        "response_file".to_string(),
        serde_json::Value::String(response_file.clone()),
    );
    queue_spawn(payload, intent, summary)?;
    poll_rpc_with(
        &response_file,
        "queued pane",
        std::time::Duration::from_secs(10),
    )
}

/// Decode the definitive reply to an outside-pane queued spawn.
///
/// This is deliberately shared with the queue caller instead of treating a
/// queue-file write as success: the host may reject the app after it drains
/// the file (for example when the CPython bundle is unavailable).
pub(super) fn queued_spawn_pane_id(content: &str) -> Result<u64, String> {
    if let Some(error) = reply_error(content) {
        return Err(error);
    }
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|reply| reply.get("pane_id").and_then(serde_json::Value::as_u64))
        .ok_or_else(|| format!("spawn response carried no pane id: {content}"))
}

#[cfg(test)]
mod queued_spawn_tests {
    use super::queued_spawn_pane_id;

    #[test]
    fn queued_spawn_surfaces_host_launch_error() {
        assert_eq!(
            queued_spawn_pane_id(r#"{"error":"CPython WASM bundle unavailable"}"#),
            Err("CPython WASM bundle unavailable".to_string())
        );
    }
}

pub(super) fn binary_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

// ── Public re-exports (preserve crate::cli::fn_name() call sites in main.rs) ──
pub use account::{account_login_cli, account_logout_cli, account_status_cli};
pub use agent::{
    agent_ask_cli, agent_create_cli, agent_done_cli, agent_history_cli, agent_hook_install_cli,
    agent_hook_uninstall_cli, agent_list_cli, agent_reopen_cli, agent_report_cli, agent_show_cli,
    agent_spawn_cli, agent_status_cli, agent_next_cli, agent_fleet_cli, AgentHarness,
};
pub use agents_api::{
    agent_team_dispatch, agent_assign_cli, agent_cancel_cli, agent_conversation_cli, agent_delegate_cli,
    agent_grants_dispatch, agent_head_dispatch, agent_run_dispatch, assistant_open_head_cli,
    command_view_cancel_cli, command_view_cli, command_view_follow_cli, command_view_refused,
    command_view_send_cli,
};
pub use ai::{ai_doctor_cli, ai_onboard_cli, ai_setup_cli};
pub use app::{
    app_action_cli, app_adopt, app_call_cli, app_tools_cli, assistant_permission_cli,
    assistant_send_cli, assistant_tool_cli, needs_you_api_cli, needs_you_cli, app_info, app_init,
    app_inspect_cli, app_install_package, app_install_with_pin, app_list, app_package_cli,
    app_prune_cli, app_render, app_test_cli, app_uninstall, app_update_cli, InstallConfirm,
};
pub use app_check::app_check_cli;
pub use changes::{
    changes_accept_cli, changes_allow_cli, changes_preview_cli, changes_profile_cli,
    changes_propose_cli, changes_refresh_cli, changes_revert_cli,
};
pub use completions::{complete_open_cli, complete_run_cli, completions_cli};
pub use config_cli::{
    config_check, config_edit, config_get, config_list, config_reset, config_set,
};
pub use context_cli::{
    context_current_cli, context_describe_cli, context_list_cli, context_new_cli, context_open_cli,
    context_push_cli, context_set_root_cli, context_sub_cli, context_zoom_cli,
    context_zoom_out_cli,
};
pub use demo::demo_cli;
pub use doctor::doctor_cli;
pub use events::{
    events_declare_cli, events_emit_cli, events_list_cli, events_mcp_config_cli,
    events_subscribe_cli, pane_lifecycle_wait_cli, pane_lifecycle_follow_cli, EmitArgs,
};
pub use host::{host_log_cli, host_screenshot_cli, host_start_cli, host_status_cli, host_stop_cli};
pub use install::{
    install_cli, install_pack_cli, install_workspace_pack_cli, plexi_uninstall_cli,
    self_update_cli,
};
pub use ledger::ledger_summary_cli;
pub use list::{freeze_cli, parse_notify_choice};
pub use marketplace::{app_browse_cli, app_publish_cli, app_search_cli};
pub use notify::{dismiss_notify_cli, notify_cli, parse_notify_scope, NotifyCliArgs};
pub use open::{
    app_trust_cli, mcp_pane_title, open_cli, open_cli_by_name, open_mcp_by_name, pane_new_cli,
    parse_prefix, AgentBootRequest, OpenPrefix, PaneNewCliArgs,
};
pub use permissions::permissions_cli;
pub use pane::{
    pane_capture_cli, pane_click_cli, pane_click_node_cli, pane_close_cli, pane_drag_cli,
    pane_drop_cli, pane_focus_cli, pane_heartbeat_cli, pane_info_cli, pane_key_cli, pane_list_cli,
    pane_message_recv_cli, pane_message_send_cli, pane_result_read_cli, pane_result_schema_cli,
    pane_result_write_cli, pane_self_cli, pane_send_cli, pane_set_title_cli, pane_slot_delete_cli,
    pane_slot_list_cli, pane_slot_read_cli, pane_slot_wait_cli, pane_slot_write_cli,
    pane_state_cli, pane_status_cli,
};
pub use routine::{routine_add, routine_list, routine_remove, routine_run, routine_set_enabled};
pub use run::{run_command, run_list_commands};
#[cfg(test)]
pub(crate) use run::format_list_commands_for_root;
pub use validate::validate_cli;
pub use workspace::{
    folder_secret_exec, folder_secret_grant, folder_secret_read, folder_secret_rm,
    folder_secret_set, workspace_clean_cli, workspace_init, workspace_secret_delete,
    workspace_secret_get, workspace_secret_list, workspace_secret_set,
};

#[cfg(test)]
mod resolve_user_commands_tests {
    use super::{
        ensure_commands_file, resolve_user_commands, UserCommandScope, COMMANDS_TEMPLATE,
    };

    /// Build a workspace tempdir whose channel-scoped commands.toml holds `body`.
    fn workspace_with_commands(body: &str) -> (tempfile::TempDir, crate::config::TestChannelGuard) {
        let guard = crate::config::set_test_channel("test");
        let tmp = tempfile::tempdir().unwrap();
        let chan_dir = tmp.path().join(".plexi-test");
        std::fs::create_dir_all(&chan_dir).unwrap();
        std::fs::write(chan_dir.join("commands.toml"), body).unwrap();
        (tmp, guard)
    }

    #[test]
    fn workspace_commands_resolve_sorted_with_scope() {
        let (tmp, _g) = workspace_with_commands(
            r#"
[secrets]
required = ["GLOBAL_SECRET"]

[commands]
zeta = "echo z"
build = { run = "cargo build", description = "Build it", secrets = ["API_KEY"] }
"#,
        );
        let profile = tempfile::tempdir().unwrap();
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let cmds = resolve_user_commands(Some(tmp.path()));
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].name, "build");
        assert_eq!(cmds[0].description.as_deref(), Some("Build it"));
        assert_eq!(cmds[0].run.as_deref(), Some("cargo build"));
        assert_eq!(cmds[0].scope, UserCommandScope::Workspace);
        assert_eq!(cmds[0].secrets, vec!["GLOBAL_SECRET", "API_KEY"]);
        assert_eq!(cmds[1].name, "zeta");
        assert_eq!(cmds[1].secrets, vec!["GLOBAL_SECRET"]);
    }

    #[test]
    fn workspace_and_global_commands_merge_with_workspace_override() {
        // Done-When: global `foo` and workspace `bar` are both listed;
        // workspace `foo` overrides global `foo`.
        let (tmp, _g) = workspace_with_commands(
            r#"
[commands]
bar = { run = "echo ws-bar", description = "Workspace bar" }
foo = { run = "echo ws-foo", description = "Workspace foo override" }
"#,
        );
        let profile = tempfile::tempdir().unwrap();
        std::fs::write(
            profile.path().join("commands.toml"),
            r#"
[commands]
foo = { run = "echo global-foo", description = "Global foo" }
global_cmd = "echo global-toml"
"#,
        )
        .unwrap();
        let scripts = profile.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        write_executable(&scripts.join("global-script"));
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());

        let cmds = resolve_user_commands(Some(tmp.path()));
        let names: Vec<(&str, UserCommandScope, Option<&str>)> = cmds
            .iter()
            .map(|c| (c.name.as_str(), c.scope, c.run.as_deref()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("bar", UserCommandScope::Workspace, Some("echo ws-bar")),
                ("foo", UserCommandScope::Workspace, Some("echo ws-foo")),
                ("global-script", UserCommandScope::Global, None),
                (
                    "global_cmd",
                    UserCommandScope::Global,
                    Some("echo global-toml")
                ),
            ]
        );
    }

    #[test]
    fn ensure_commands_file_creates_template_when_missing() {
        let _g = crate::config::set_test_channel("test");
        let ws = tempfile::tempdir().unwrap();
        let profile = tempfile::tempdir().unwrap();
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());

        let ws_path = ensure_commands_file(Some(ws.path()));
        assert_eq!(ws_path, ws.path().join(".plexi-test").join("commands.toml"));
        assert!(ws_path.is_file());
        assert_eq!(std::fs::read_to_string(&ws_path).unwrap(), COMMANDS_TEMPLATE);
        assert!(resolve_user_commands(Some(ws.path())).is_empty());

        let global_path = ensure_commands_file(None);
        assert_eq!(global_path, profile.path().join("commands.toml"));
        assert!(global_path.is_file());
        assert_eq!(
            std::fs::read_to_string(&global_path).unwrap(),
            COMMANDS_TEMPLATE
        );
    }

    #[test]
    fn hot_reload_rereads_file_each_call() {
        let (tmp, _g) = workspace_with_commands("[commands]\nfirst = \"echo 1\"\n");
        let profile = tempfile::tempdir().unwrap();
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let first = resolve_user_commands(Some(tmp.path()));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "first");

        std::fs::write(
            tmp.path().join(".plexi-test").join("commands.toml"),
            "[commands]\nfirst = \"echo 1\"\nsecond = \"echo 2\"\n",
        )
        .unwrap();
        let second = resolve_user_commands(Some(tmp.path()));
        assert_eq!(second.len(), 2);
        assert!(second.iter().any(|c| c.name == "second"));
    }

    #[test]
    fn global_scripts_when_no_commands_toml() {
        let _g = crate::config::set_test_channel("test");
        let tmp = tempfile::tempdir().unwrap(); // workspace root with no commands.toml
        let profile = tempfile::tempdir().unwrap();
        let scripts = profile.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        write_executable(&scripts.join("deploy"));
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());

        let cmds = resolve_user_commands(Some(tmp.path()));
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "deploy");
        assert_eq!(cmds[0].scope, UserCommandScope::Global);
        assert!(cmds[0].run.is_none());
    }

    #[test]
    fn unparseable_commands_toml_yields_no_commands() {
        // Present-but-broken commands.toml → empty, never silent global fallback.
        let (tmp, _g) = workspace_with_commands("this is = not valid toml [[[\n");
        let profile = tempfile::tempdir().unwrap();
        let scripts = profile.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        write_executable(&scripts.join("should-not-appear"));
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());

        let cmds = resolve_user_commands(Some(tmp.path()));
        assert!(cmds.is_empty());
    }

    fn write_executable(path: &std::path::Path) {
        std::fs::write(path, "#!/bin/sh\necho hi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).unwrap();
        }
    }
}

#[cfg(test)]
mod spawn_gate_tests {
    use super::{channel_label_from, spawn_gate_error, spawn_socket_accepting};

    #[test]
    fn missing_socket_is_not_servicing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!spawn_socket_accepting(&tmp.path().join("notify.sock")));
    }

    #[test]
    fn bound_socket_is_servicing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notify.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(spawn_socket_accepting(&path));
    }

    #[test]
    fn gate_error_is_typed_and_names_channel_and_socket() {
        let msg = spawn_gate_error("alpha", std::path::Path::new("/tmp/x/notify.sock"));
        assert!(msg.contains("host not servicing pane spawns on channel alpha"));
        assert!(msg.contains("/tmp/x/notify.sock"));
        assert!(msg.contains("nothing was spawned"));
    }

    #[test]
    fn channel_label_derivation() {
        assert_eq!(channel_label_from(".plexi-alpha"), "alpha");
        assert_eq!(channel_label_from(".plexi-pr-2470"), "pr-2470");
        assert_eq!(channel_label_from(".plexi"), "default");
        assert_eq!(channel_label_from(""), "default");
    }
}
