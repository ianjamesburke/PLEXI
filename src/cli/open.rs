use super::pane::open_github_ephemeral;
use super::send_to_socket;
use crate::app::launch_spec::PaneLaunchSpec;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

/// Poll a response file until it appears (or timeout). Shared by all spawn paths.
///
/// `pub(super)` so `host.rs` can reference the same convention. `host.rs`'s
/// own readiness/status polling uses a bespoke variant (`poll_response_file`)
/// instead — it needs the raw JSON content and a configurable timeout rather
/// than this function's fixed 5s timeout and print-to-stdout side effect.
pub(super) fn wait_for_response(response_file: &str) -> i32 {
    let response_path = std::path::PathBuf::from(response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read_to_string(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
                            eprintln!("error: {msg}");
                            return 1;
                        }
                        if let Some(pid) = v.get("pane_id").and_then(|v| v.as_u64()) {
                            println!("{pid}");
                            return 0;
                        }
                    }
                    print!("{content}");
                    return 0;
                }
                Err(e) => {
                    log::warn!("pane_new: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Unified pane spawning. All CLI spawn paths funnel through here.
pub fn pane_new_cli(
    cmd: Option<&str>,
    name: Option<&str>,
    layout: Option<&str>,
    from_pane_id: Option<u64>,
    cwd: Option<&str>,
    ephemeral: bool,
    no_focus: bool,
    app: Option<&str>,
    mcp: &[String],
    extra_args: &[String],
) -> i32 {
    // Determine mode: app or terminal
    let is_app = app.is_some() || !mcp.is_empty();
    let type_id = if let Some(a) = app {
        a.to_string()
    } else if !mcp.is_empty() {
        "mcp-renderer".to_string()
    } else {
        "terminal".to_string()
    };

    let args: Vec<String> = if !mcp.is_empty() {
        mcp.to_vec()
    } else if is_app {
        extra_args.to_vec()
    } else if let Some(c) = cmd {
        let command = std::iter::once(c)
            .chain(extra_args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        vec![command]
    } else {
        Vec::new()
    };

    let from_pane_id = from_pane_id.or_else(|| std::env::var("PLEXI_PANE_ID").ok()?.parse().ok());

    // Socket path — inside a Plexi pane
    if super::command_socket_available() {
        let id = uuid::Uuid::new_v4();
        let response_file = crate::config::config_dir()
            .join(format!("spawn-pane-response-{id}.json"))
            .to_string_lossy()
            .into_owned();
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": type_id,
            "args": args,
            "response_file": response_file,
        });
        // Omit `layout` when unset so the host applies its placement default
        // (manifest `[launch] placement`, else a sibling split) rather than the
        // CLI forcing a value (stint 0330).
        if let Some(l) = layout {
            payload["layout"] = serde_json::Value::String(l.to_string());
        }
        if ephemeral {
            payload["ephemeral"] = serde_json::Value::Bool(true);
        }
        if let Some(pid) = from_pane_id {
            payload["from_pane_id"] = serde_json::Value::Number(pid.into());
        }
        if let Some(cwd) = cwd {
            payload["cwd"] = serde_json::Value::String(cwd.to_string());
        }
        if no_focus {
            payload["no_focus"] = serde_json::Value::Bool(true);
        }
        if let Some(n) = name {
            payload["name"] = serde_json::Value::String(n.to_string());
        }
        log::info!("pane_new:cli: sending via socket type_id={type_id} name={name:?} ephemeral={ephemeral} no_focus={no_focus} from_pane_id={from_pane_id:?} cwd={cwd:?} response_file={response_file:?}");
        let code = send_to_socket(payload);
        if code != 0 {
            return code;
        }
        return wait_for_response(&response_file);
    }

    // Fallback: spawn-queue (outside a Plexi pane)
    if from_pane_id.is_some() {
        log::warn!(
            "pane_new:cli: --from requires PLEXI_SOCKET (run inside a Plexi pane); ignoring"
        );
        eprintln!("warning: --from is ignored outside a Plexi pane");
    }
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut queue_payload = serde_json::json!({
        "type_id": type_id,
        "args": args,
    });
    if let Some(l) = layout {
        queue_payload["layout"] = serde_json::Value::String(l.to_string());
    }
    if ephemeral {
        queue_payload["ephemeral"] = serde_json::Value::Bool(true);
    }
    if let Some(cwd) = cwd {
        queue_payload["cwd"] = serde_json::Value::String(cwd.to_string());
    }
    if no_focus {
        queue_payload["no_focus"] = serde_json::Value::Bool(true);
    }
    if let Some(n) = name {
        queue_payload["name"] = serde_json::Value::String(n.to_string());
    }
    let file = queue_dir.join(format!("{id}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    crate::cli::nudge_running_instance();
    log::info!("pane_new:cli: queued type_id={type_id} name={name:?} ephemeral={ephemeral} no_focus={no_focus} cwd={cwd:?}");
    println!("queued: open {type_id}");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}

/// Derive a short display name for an MCP pane from the server command args.
///
/// Rules (matching the issue spec):
/// - Skip leading runner tokens (`npx`, `node`, `uvx`, `python`, `python3`, `bunx`)
/// - Take the first remaining arg as the package/server name
/// - Strip a leading `@scope/` prefix
/// - Strip a leading `server-` prefix from the remainder
/// - Prefix the result with `"mcp: "`
///
/// Examples:
///   `["npx", "@modelcontextprotocol/server-filesystem", "/tmp"]` → `"mcp: filesystem"`
///   `["uvx", "mcp-server-fetch"]`                               → `"mcp: fetch"`
///   `["npx", "@scope/server-git"]`                              → `"mcp: git"`
pub fn mcp_pane_title(args: &[String]) -> String {
    const RUNNERS: &[&str] = &[
        "npx", "node", "uvx", "bunx", "python", "python3", "deno", "bun",
    ];
    let name = args
        .iter()
        .find(|a| !RUNNERS.contains(&a.as_str()) && !a.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("mcp");

    // Strip leading `@scope/` if present
    let after_scope = if let Some(pos) = name.find('/') {
        &name[pos + 1..]
    } else {
        name
    };

    // Strip leading `server-` or `mcp-server-` prefix
    let short = after_scope
        .strip_prefix("mcp-server-")
        .or_else(|| after_scope.strip_prefix("server-"))
        .unwrap_or(after_scope);

    format!("mcp: {short}")
}

/// Parsed prefix from a `type_id` string like `app:snake`, `cli:git`, `mcp:filesystem`.
pub enum OpenPrefix {
    /// `app:<name>` -- explicit app registry lookup
    App(String),
    /// `cli:<name>` -- CLI registry/crawl lookup
    Cli(String),
    /// `mcp:<name>` -- MCP registry lookup
    Mcp(String),
    /// No prefix -- backwards compat (try app registry, then CLI registry)
    Bare(String),
}

/// Parse a `type_id` argument into an `OpenPrefix`.
pub fn parse_prefix(type_id: &str) -> OpenPrefix {
    if let Some(name) = type_id.strip_prefix("app:") {
        OpenPrefix::App(name.to_string())
    } else if let Some(name) = type_id.strip_prefix("cli:") {
        OpenPrefix::Cli(name.to_string())
    } else if let Some(name) = type_id.strip_prefix("mcp:") {
        OpenPrefix::Mcp(name.to_string())
    } else {
        OpenPrefix::Bare(type_id.to_string())
    }
}

/// Serialize a resolved descriptor to a temp file and open it in the native
/// `cli-renderer`. Shared by every CLI resolution path so the temp-file +
/// pane-spawn handshake lives in exactly one place.
fn open_descriptor_in_renderer(
    descriptor: &crate::app::plexi_descriptor::PlexiDescriptor,
    name: &str,
    layout: Option<&str>,
    from_pane_id: Option<u64>,
    cwd: Option<&str>,
) -> i32 {
    let json = match serde_json::to_string_pretty(descriptor) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: could not serialize descriptor for `{name}`: {e}");
            return 1;
        }
    };
    let id = uuid::Uuid::new_v4();
    let tmp = std::env::temp_dir().join(format!("plexi-descriptor-{id}.json"));
    if let Err(e) = std::fs::write(&tmp, &json) {
        eprintln!("error: could not write descriptor temp file: {e}");
        return 1;
    }
    let path = tmp.to_string_lossy().to_string();
    log::info!("open:cli: launching cli-renderer for `{name}` with descriptor at {path}");
    pane_new_cli(
        None,
        Some(name),
        layout,
        from_pane_id,
        cwd,
        false,
        false,
        Some("cli-renderer"),
        &[],
        &[path],
    )
}

/// Open a named CLI tool via the full Tier 1/2/3 resolution chain (native
/// `--plexi` → registry → recursive `--help` crawl), then render it.
pub fn open_cli_by_name(
    name: &str,
    layout: Option<&str>,
    from_pane_id: Option<u64>,
    cwd: Option<&str>,
) -> i32 {
    log::info!("open:cli: resolving `{name}`");
    match crate::cli::descriptor::resolve_cli(name) {
        Ok(resolved) => {
            open_descriptor_in_renderer(&resolved.descriptor, name, layout, from_pane_id, cwd)
        }
        Err(e) => {
            eprintln!("error: could not resolve CLI `{name}`: {e}");
            1
        }
    }
}

/// Open a named MCP server from the registry.
///
/// Looks up the `command` array from `registry/mcp/<name>.json` and opens it
/// with `mcp-renderer`.
pub fn open_mcp_by_name(
    name: &str,
    layout: Option<&str>,
    from_pane_id: Option<u64>,
    cwd: Option<&str>,
) -> i32 {
    log::info!("open:prefix: resolving mcp:{name}");

    match crate::cli::registry::lookup_mcp(name) {
        Some(entry) => {
            log::info!(
                "open:prefix: mcp:{} ({}) resolved, command={:?}",
                entry.name,
                entry.description,
                entry.command
            );
            let title = mcp_pane_title(&entry.command);
            pane_new_cli(
                None,
                Some(&title),
                layout,
                from_pane_id,
                cwd,
                false,
                false,
                Some("mcp-renderer"),
                &entry.command,
                &[],
            )
        }
        None => {
            eprintln!("error: MCP server `{name}` not found in registry");
            eprintln!("hint: use `plexi app open --mcp <command>...` for unnamed MCP servers");
            1
        }
    }
}

fn raw_wasm_app_id(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("wasm")
        .to_string()
}

fn raw_wasm_workspace_root(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn prompt_raw_wasm_review(
    app_id: &str,
    workspace_root: &Path,
    missing: &[String],
    is_tty: bool,
    reader: &mut dyn BufRead,
) -> Result<bool, String> {
    if missing.is_empty() {
        return Ok(true);
    }
    if !is_tty {
        return Err(format!(
            "raw WASM launch requires review for: {}. Run `plexi app open` from a terminal to approve these imports.",
            missing.join(", ")
        ));
    }

    eprintln!("Raw WASM component '{app_id}' requests host imports:");
    for capability_id in missing {
        eprintln!(
            "  - {capability_id}: {}",
            crate::app::permissions::wasm_capability_description(capability_id)
        );
    }
    eprintln!("Scope: {}", workspace_root.display());
    eprint!("Allow and remember for this scope? [y/N] ");
    io::stderr()
        .flush()
        .map_err(|e| format!("could not flush review prompt: {e}"))?;

    let mut answer = String::new();
    reader
        .read_line(&mut answer)
        .map_err(|e| format!("could not read review response: {e}"))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn review_raw_wasm_open_with_reader(
    path: &Path,
    is_tty: bool,
    reader: &mut dyn BufRead,
) -> Result<(), String> {
    let app_id = raw_wasm_app_id(path);
    let workspace_root = raw_wasm_workspace_root(path);
    let required_grants =
        crate::host::wasm_app::WasmApp::inspect_required_grants(path).map_err(|e| {
            format!(
                "could not inspect WASM imports for '{}': {e}",
                path.display()
            )
        })?;
    let required_caps = required_grants.capability_ids();
    if required_caps.is_empty() {
        return Ok(());
    }

    let config_dir = crate::config::config_dir();
    let mut store = crate::app::permissions::PermissionStore::load_or_default(&config_dir);
    let declared: std::collections::HashSet<String> = required_caps.iter().cloned().collect();
    let (granted, blocked) = store.build_wasm_permission_sets(&app_id, &workspace_root, &declared);
    let blocked_required: Vec<String> = required_caps
        .iter()
        .filter(|cap| blocked.contains(*cap))
        .cloned()
        .collect();
    if !blocked_required.is_empty() {
        return Err(format!(
            "raw WASM launch blocked by saved decision for: {}",
            blocked_required.join(", ")
        ));
    }
    let missing: Vec<String> = required_caps
        .iter()
        .filter(|cap| !granted.contains(*cap))
        .cloned()
        .collect();
    if !prompt_raw_wasm_review(&app_id, &workspace_root, &missing, is_tty, reader)? {
        return Err("raw WASM launch cancelled".to_string());
    }
    for capability_id in missing {
        store.set_wasm(
            &app_id,
            &workspace_root,
            &capability_id,
            crate::app::permissions::PermissionState::Green,
        );
    }
    store.save();
    log::info!(
        "open:cli: raw wasm review app={} path={} workspace={} grants={:?}",
        app_id,
        path.display(),
        workspace_root.display(),
        required_caps
    );
    Ok(())
}

fn review_raw_wasm_open(path: &Path) -> Result<(), String> {
    let is_tty = io::stdin().is_terminal();
    let mut stdin = io::stdin().lock();
    review_raw_wasm_open_with_reader(path, is_tty, &mut stdin)
}

/// Thin wrapper preserving the existing `plexi app open` call site.
pub fn open_cli(
    type_id: &str,
    args: &[String],
    layout: Option<&str>,
    from_pane_id: Option<u64>,
    cwd: Option<&str>,
) -> i32 {
    // Intercept github: prefix for ephemeral open-without-install.
    if type_id.starts_with("github:") {
        return open_github_ephemeral(type_id, layout, from_pane_id, cwd);
    }

    if type_id == "terminal" {
        log::warn!(
            "open:cli: 'plexi app open terminal' is deprecated, use 'plexi pane new' instead"
        );
        eprintln!("warning: 'plexi app open terminal' is deprecated, use 'plexi pane new' instead");
    }

    // If type_id looks like a path to an app directory (contains manifest.toml), open by path.
    let path = std::path::Path::new(type_id);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    if resolved.join("manifest.toml").exists() {
        let abs_path = resolved.to_string_lossy().to_string();
        log::info!("open:cli: detected path with manifest.toml, opening from path={abs_path}");
        return open_app_by_path(&abs_path, args, layout, from_pane_id);
    }

    // A `.wasm` file is a sandboxed component app launched through the same
    // path-spawn flow as a local app dir (the run primitive, G6).
    if resolved.extension().and_then(|e| e.to_str()) == Some("wasm") && resolved.is_file() {
        if let Err(e) = review_raw_wasm_open(&resolved) {
            log::warn!(
                "open:cli: raw wasm review failed path={}: {e}",
                resolved.display()
            );
            eprintln!("error: {e}");
            return 1;
        }
        let abs_path = resolved.to_string_lossy().to_string();
        log::info!("open:cli: detected .wasm component, opening from path={abs_path}");
        return open_app_by_path(&abs_path, args, layout, from_pane_id);
    }

    pane_new_cli(
        None,
        None,
        layout,
        from_pane_id,
        cwd,
        false,
        false,
        Some(type_id),
        &[],
        args,
    )
}

/// Open an app from a local directory path (replaces the old `app run` command).
fn open_app_by_path(
    abs_path: &str,
    args: &[String],
    layout: Option<&str>,
    from_pane_id: Option<u64>,
) -> i32 {
    // Leave layout unset when no flag is given so the host applies its placement
    // default (manifest `[launch] placement`, else a sibling split) instead of
    // the CLI forcing an overlay takeover of the caller's pane (stint 0330).
    let from_pane_id = from_pane_id.or_else(|| std::env::var("PLEXI_PANE_ID").ok()?.parse().ok());
    let spec = match PaneLaunchSpec::path(abs_path, args.to_vec()) {
        Ok(spec) => spec
            .with_layout(layout.map(str::to_string))
            .with_from_pane_id(from_pane_id),
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    if super::command_socket_available() {
        let id = uuid::Uuid::new_v4();
        let response_file = crate::config::config_dir()
            .join(format!("spawn-pane-response-{id}.json"))
            .to_string_lossy()
            .into_owned();
        let spec = spec.with_response_file(Some(response_file.clone()));
        let payload = serde_json::to_value(spec.to_spawn_pane_request()).unwrap_or_else(|e| {
            log::error!("open_app_by_path: failed to serialize spawn request: {e}");
            serde_json::json!({
                "type": "spawn_pane",
                "type_id": "",
                "path": abs_path,
                "args": args,
                "layout": layout,
                "response_file": response_file,
            })
        });
        log::info!("open_app_by_path: sending via socket path={abs_path} args={args:?} layout={layout:?} from_pane_id={from_pane_id:?}");
        let code = send_to_socket(payload);
        if code != 0 {
            return code;
        }
        return wait_for_response(&response_file);
    }

    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let queue_payload = serde_json::to_value(spec.to_spawn_pane_request()).unwrap_or_else(|e| {
        log::error!("open_app_by_path: failed to serialize queued spawn request: {e}");
        serde_json::json!({
            "type_id": "",
            "path": abs_path,
            "args": args,
            "layout": layout,
        })
    });
    let file = queue_dir.join(format!("{ts}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    crate::cli::nudge_running_instance();
    log::info!("open_app_by_path: queued path={abs_path}");
    println!("queued: open {abs_path}");
    println!("(running outside a Plexi pane; Plexi will pick this up within a second)");
    0
}

#[cfg(test)]
mod open_cli_tests {
    use super::{open_cli, review_raw_wasm_open_with_reader};
    use serde_json::Value;
    use std::io::{BufRead, BufReader, Cursor};
    use std::os::unix::net::UnixListener;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn capture_spawn_payload<F>(run_cli: F) -> (i32, Value)
    where
        F: FnOnce() -> i32,
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("open.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind open socket");
        std::env::set_var("PLEXI_SOCKET", &socket_path);

        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept open connection");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read open payload");
            let payload: Value = serde_json::from_str(&line).expect("open payload json");
            if let Some(response_file) = payload.get("response_file").and_then(|v| v.as_str()) {
                std::fs::write(response_file, r#"{"pane_id":123}"#).expect("write open response");
            }
            payload
        });

        let code = run_cli();
        std::env::remove_var("PLEXI_SOCKET");
        let payload = handle.join().expect("payload thread");
        (code, payload)
    }

    #[test]
    fn app_open_omits_layout_so_host_applies_split_default() {
        // With no directional flag the CLI must leave `layout` unset so the host
        // applies its placement default (manifest `[launch] placement`, else a
        // sibling split) rather than forcing an overlay takeover (stint 0330).
        let _guard = ENV_LOCK.lock().unwrap();
        let (code, payload) = capture_spawn_payload(|| open_cli("balls", &[], None, None, None));

        assert_eq!(code, 0);
        assert_eq!(payload["type"], "spawn_pane");
        assert_eq!(payload["type_id"], "balls");
        assert!(
            payload.get("layout").is_none(),
            "layout must be omitted when no flag is given, got {:?}",
            payload.get("layout")
        );
    }

    #[test]
    fn app_open_explicit_tab_layout_is_preserved() {
        let _guard = ENV_LOCK.lock().unwrap();
        let (code, payload) =
            capture_spawn_payload(|| open_cli("file_browser", &[], Some("tab"), None, None));

        assert_eq!(code, 0);
        assert_eq!(payload["type"], "spawn_pane");
        assert_eq!(payload["type_id"], "file_browser");
        assert_eq!(payload["layout"], "tab");
    }

    #[test]
    fn wasm_path_open_forwards_launch_args() {
        let _guard = ENV_LOCK.lock().unwrap();
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let _profile_guard = crate::config::set_test_profile_dir(config_dir.path().to_path_buf());

        // Use the real fixture — a stub `\0asm` blob is not a valid WASM component
        // and wasmtime will reject it before the socket payload is ever sent.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/wasm-fixtures/sysmon.wasm");
        let app_id = fixture
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("wasm");
        let workspace_root = fixture.parent().expect("fixture parent");

        // Pre-approve all required grants so review does not prompt stdin.
        let required = crate::host::wasm_app::WasmApp::inspect_required_grants(&fixture)
            .expect("inspect fixture grants")
            .capability_ids();
        let mut store =
            crate::app::permissions::PermissionStore::load_or_default(config_dir.path());
        for cap in required {
            store.set_wasm(
                app_id,
                workspace_root,
                &cap,
                crate::app::permissions::PermissionState::Green,
            );
        }
        store.save();

        let wasm_path_str = fixture.to_string_lossy().into_owned();
        let args = vec!["--sample".to_string(), "96".to_string()];

        let (code, payload) =
            capture_spawn_payload(|| open_cli(&wasm_path_str, &args, None, None, None));

        assert_eq!(code, 0);
        assert_eq!(payload["type"], "spawn_pane");
        assert_eq!(payload["path"], wasm_path_str);
        assert_eq!(payload["args"], serde_json::json!(["--sample", "96"]));
    }

    #[test]
    fn raw_wasm_review_requires_tty_for_unreviewed_imports() {
        let _guard = ENV_LOCK.lock().unwrap();
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let _profile_guard = crate::config::set_test_profile_dir(config_dir.path().to_path_buf());
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/wasm-fixtures/sysmon.wasm");
        let mut reader = Cursor::new(Vec::<u8>::new());

        let err = review_raw_wasm_open_with_reader(&fixture, false, &mut reader)
            .expect_err("non-tty raw wasm review should fail");

        assert!(err.contains("raw WASM launch requires review"), "{err}");
    }

    #[test]
    fn raw_wasm_review_tty_persists_required_imports() {
        let _guard = ENV_LOCK.lock().unwrap();
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let _profile_guard = crate::config::set_test_profile_dir(config_dir.path().to_path_buf());
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/wasm-fixtures/sysmon.wasm");
        let app_id = fixture
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("wasm");
        let workspace_root = fixture.parent().expect("fixture parent");
        let required = crate::host::wasm_app::WasmApp::inspect_required_grants(&fixture)
            .expect("inspect fixture grants")
            .capability_ids();
        let mut reader = Cursor::new(b"yes\n".to_vec());

        review_raw_wasm_open_with_reader(&fixture, true, &mut reader)
            .expect("tty approval should persist review");

        let store = crate::app::permissions::PermissionStore::load_or_default(config_dir.path());
        for capability_id in required {
            assert_eq!(
                store.get_wasm(app_id, workspace_root, &capability_id),
                Some(crate::app::permissions::PermissionState::Green),
                "{capability_id} should be remembered"
            );
        }
    }
}

/// Read a line from stdin with echo disabled (for password-style input).
pub(super) fn read_secret_from_stdin() -> io::Result<String> {
    // Disable echo via stty (avoids libc dependency).
    let _ = std::process::Command::new("stty").arg("-echo").status();

    let result = read_line_plain();

    // Restore echo.
    let _ = std::process::Command::new("stty").arg("echo").status();
    // Print newline since echo was off during input.
    eprintln!();

    result
}

fn read_line_plain() -> io::Result<String> {
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::{mcp_pane_title, parse_prefix, OpenPrefix};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn prefix_app() {
        match parse_prefix("app:snake") {
            OpenPrefix::App(name) => assert_eq!(name, "snake"),
            _ => panic!("expected App prefix"),
        }
    }

    #[test]
    fn prefix_cli() {
        match parse_prefix("cli:git") {
            OpenPrefix::Cli(name) => assert_eq!(name, "git"),
            _ => panic!("expected Cli prefix"),
        }
    }

    #[test]
    fn prefix_mcp() {
        match parse_prefix("mcp:filesystem") {
            OpenPrefix::Mcp(name) => assert_eq!(name, "filesystem"),
            _ => panic!("expected Mcp prefix"),
        }
    }

    #[test]
    fn prefix_bare() {
        match parse_prefix("snake") {
            OpenPrefix::Bare(name) => assert_eq!(name, "snake"),
            _ => panic!("expected Bare prefix"),
        }
    }

    #[test]
    fn prefix_github_stays_bare() {
        // github: is handled by open_cli, not the prefix router
        match parse_prefix("github:owner/repo") {
            OpenPrefix::Bare(name) => assert_eq!(name, "github:owner/repo"),
            _ => panic!("expected Bare for github: prefix"),
        }
    }

    #[test]
    fn mcp_title_npx_scoped_server() {
        assert_eq!(
            mcp_pane_title(&args(&[
                "npx",
                "@modelcontextprotocol/server-filesystem",
                "/tmp"
            ])),
            "mcp: filesystem"
        );
    }

    #[test]
    fn mcp_title_uvx_mcp_server_prefix() {
        assert_eq!(
            mcp_pane_title(&args(&["uvx", "mcp-server-fetch"])),
            "mcp: fetch"
        );
    }

    #[test]
    fn mcp_title_scoped_server_git() {
        assert_eq!(
            mcp_pane_title(&args(&["npx", "@scope/server-git"])),
            "mcp: git"
        );
    }

    #[test]
    fn mcp_title_bare_binary() {
        assert_eq!(mcp_pane_title(&args(&["my-mcp-tool"])), "mcp: my-mcp-tool");
    }

    #[test]
    fn mcp_title_empty_args() {
        assert_eq!(mcp_pane_title(&[]), "mcp: mcp");
    }
}
