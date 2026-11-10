//! `plexi app state get/set` — the sanctioned surface for reading and writing
//! a file-backed app's state.
//!
//! An agent could write the state file directly; the
//! file is on disk. These verbs exist to make that *correct*: they resolve the
//! app's declared state path so no caller ever hardcodes one, they validate the
//! document in the app's declared format before it reaches disk, they write
//! through the same atomic path the app's own writes use, and every attempt is
//! traced. Direct file writes stay possible and stay unsupported.
//!
//! File access is disk-direct. Inside a pane the host supplies its current root;
//! outside a pane the nearest `.plexi` ancestor of cwd supplies file scope.
//! A running app picks writes up through its state watcher.
//!
//! Entitlement here is structural, not checked: there is no path argument and
//! no `--context` flag, so the only reachable files are the calling context's
//! own state files for an app that declared `[state]`.

use crate::host::state_scope::{StateFormat, StateScope};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Everything resolving an invocation produced: which file, in which format,
/// under which scope.
#[derive(Debug)]
struct ResolvedState {
    path: PathBuf,
    scope: StateScope,
    format: StateFormat,
    context_root: PathBuf,
}

/// Who is driving. Only used for the log line — a caller that exports neither
/// pane nor context identity is `external`, which is a normal case (a plain
/// shell outside any pane), not an error.
fn caller_label() -> String {
    let pane = std::env::var("PLEXI_PANE_ID")
        .ok()
        .filter(|v| !v.is_empty());
    let context = std::env::var("PLEXI_CONTEXT_ID")
        .ok()
        .filter(|v| !v.is_empty());
    match (pane, context) {
        (Some(pane), Some(context)) => format!("pane={pane} context={context}"),
        (Some(pane), None) => format!("pane={pane}"),
        (None, Some(context)) => format!("context={context}"),
        (None, None) => "external".to_string(),
    }
}

/// Resolve current scope from the host in a pane, or the filesystem outside one.
fn cli_context_root() -> Result<PathBuf, String> {
    let root = match std::env::var_os("PLEXI_PANE_ID") {
        Some(pane) => {
            let pane_id = pane.to_str().and_then(|v| v.parse::<u64>().ok())
                .ok_or_else(|| "PLEXI_PANE_ID is not a valid pane id".to_string())?;
            let content = super::request(
                serde_json::json!({"type":"get_pane_info", "pane_id":pane_id}),
                "app-state-scope", "app state scope",
            ).map_err(|code| format!("cannot query current scope for pane {pane_id} (exit {code})"))?;
            let reply: serde_json::Value = serde_json::from_str(&content)
                .map_err(|error| format!("invalid app state scope reply: {error}"))?;
            if let Some(error) = reply.get("error").and_then(|v| v.as_str()) { return Err(error.to_string()); }
            let root = reply.get("context_root").and_then(|v| v.as_str()).filter(|v| !v.is_empty())
                .ok_or_else(|| "host reply has no current context_root".to_string())?;
            log::info!("app_state: scope pane={pane_id} current_root={root}");
            PathBuf::from(root)
        }
        None => {
            let cwd = std::env::current_dir().map_err(|error| format!("app state cwd: {error}"))?;
            file_context_root(&cwd)?
        }
    };
    if !root.is_dir() {
        return Err(format!("context root {} does not exist — refusing to create it", root.display()));
    }
    Ok(root)
}

fn file_context_root(cwd: &Path) -> Result<PathBuf, String> {
    let root = crate::notes::anchored_root_for(cwd)
        .ok_or_else(|| "no context file scope: run beneath a directory containing .plexi".to_string())?;
    log::info!("app_state: file scope cwd={} root={}", cwd.display(), root.display());
    Ok(root)
}

fn cli_registry() -> Result<crate::app::registry::AppRegistry, String> {
    let root = if std::env::var_os("PLEXI_PANE_ID").is_some() {
        cli_context_root()?
    } else {
        std::env::current_dir().map_err(|error| format!("app state cwd: {error}"))?
    };
    Ok(crate::app::registry::AppRegistry::load(&root))
}

/// Resolve `app_id` + an optional `--scope` to a concrete state file.
///
/// Rejects, each with a named reason: an unknown app, an app that declared no
/// `[state]` section, a scope the app did not declare, an app id that is not a
/// safe file stem, and a resolved path whose parent is not really the scope's
/// state directory (a symlinked `.plexi/` or `app_states/`).
fn resolve(
    app_id: &str,
    scope_raw: Option<&str>,
    registry: &crate::app::registry::AppRegistry,
) -> Result<ResolvedState, String> {
    let installed = registry.get(app_id).ok_or_else(|| {
        format!("app '{app_id}' not found — run `plexi app list` to see installed apps")
    })?;
    if !installed.state_declared {
        return Err(format!(
            "app '{app_id}' declares no [state] section — it has no file-backed state to address"
        ));
    }
    let declared = &installed.state_scopes;
    let scope = match scope_raw {
        Some(raw) => {
            let requested = StateScope::parse(raw)?;
            if !declared.contains(&requested) {
                return Err(format!(
                    "app '{app_id}' does not declare the '{}' scope — it declares: {}",
                    requested.as_str(),
                    declared
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            requested
        }
        // No flag reaches us as absent (src/cli/AGENTS.md) — the default is the
        // app's own first declared scope, never a CLI-side constant.
        None => *declared
            .first()
            .ok_or_else(|| format!("app '{app_id}' declares an empty scope list"))?,
    };
    let format = installed.state_format;
    let context_root = match scope {
        StateScope::Context => cli_context_root()?,
        StateScope::Global => PathBuf::new(),
    };
    let path = crate::host::state_scope::state_file(scope, app_id, format, &context_root)?;
    crate::host::state_scope::assert_within_scope(
        &path,
        scope,
        crate::host::state_scope::UserDataKind::AppStates,
        &context_root,
    )?;
    Ok(ResolvedState {
        path,
        scope,
        format,
        context_root,
    })
}

/// The document an app with no state file yet should read as. A missing file is
/// first launch, not an error — the same rule the host's loader follows.
fn empty_document(format: StateFormat) -> &'static str {
    match format {
        StateFormat::Json => "{}\n",
        StateFormat::Markdown => "",
    }
}

/// `plexi app state get <app> [--scope ...]`
pub fn get(app_id: &str, scope: Option<&str>) -> i32 {
    let registry = match cli_registry() {
        Ok(registry) => registry,
        Err(error) => {
            log::info!("app_state: registry rejected: {error}");
            eprintln!("error: {error}");
            return 1;
        }
    };
    let resolved = match resolve(app_id, scope, &registry) {
        Ok(resolved) => resolved,
        Err(error) => {
            log::info!(
                "app_state: get app={app_id} caller={} rejected — {error}",
                caller_label()
            );
            eprintln!("error: {error}");
            return 1;
        }
    };
    match std::fs::read(&resolved.path) {
        Ok(bytes) => {
            // Read-back proves the file is decodable in the declared format;
            // a corrupted file is surfaced, never printed as if it were state.
            if let Err(error) = crate::host::wasm_python::decode_state_file(&bytes, resolved.format)
            {
                log::info!(
                    "app_state: get app={app_id} scope={} caller={} path={} rejected — {error}",
                    resolved.scope.as_str(),
                    caller_label(),
                    resolved.path.display()
                );
                eprintln!(
                    "error: state file {} is not valid {}: {error}",
                    resolved.path.display(),
                    resolved.format.as_str()
                );
                return 1;
            }
            log::info!(
                "app_state: get app={app_id} scope={} format={} caller={} path={} accepted bytes={}",
                resolved.scope.as_str(),
                resolved.format.as_str(),
                caller_label(),
                resolved.path.display(),
                bytes.len()
            );
            print!("{}", String::from_utf8_lossy(&bytes));
            0
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            log::info!(
                "app_state: get app={app_id} scope={} format={} caller={} path={} accepted (no file yet)",
                resolved.scope.as_str(),
                resolved.format.as_str(),
                caller_label(),
                resolved.path.display()
            );
            print!("{}", empty_document(resolved.format));
            0
        }
        Err(error) => {
            log::info!(
                "app_state: get app={app_id} scope={} caller={} path={} rejected — read failed: {error}",
                resolved.scope.as_str(),
                caller_label(),
                resolved.path.display()
            );
            eprintln!("error: read {}: {error}", resolved.path.display());
            1
        }
    }
}

/// `plexi app state set <app> [FILE] [--scope ...]`
///
/// Replaces the document wholesale. Reads `FILE` when given, else stdin. There
/// is deliberately no partial/merge update — get/set proves the capability, and
/// the right merge semantics are clearer once something real is using it.
pub fn set(app_id: &str, file: Option<&Path>, scope: Option<&str>) -> i32 {
    let input: Box<dyn Read> = match file {
        Some(path) => match std::fs::File::open(path) {
            Ok(file) => Box::new(file),
            Err(error) => {
                eprintln!("error: open {}: {error}", path.display());
                return 1;
            }
        },
        None => Box::new(std::io::stdin()),
    };
    let registry = match cli_registry() {
        Ok(registry) => registry,
        Err(error) => {
            log::info!("app_state: registry rejected: {error}");
            eprintln!("error: {error}");
            return 1;
        }
    };
    set_from(app_id, input, scope, &registry)
}

/// `set` with its input factored out so tests can drive it without a real stdin.
fn set_from(
    app_id: &str,
    mut input: impl Read,
    scope: Option<&str>,
    registry: &crate::app::registry::AppRegistry,
) -> i32 {
    let resolved = match resolve(app_id, scope, registry) {
        Ok(resolved) => resolved,
        Err(error) => {
            log::info!(
                "app_state: set app={app_id} caller={} rejected — {error}",
                caller_label()
            );
            eprintln!("error: {error}");
            return 1;
        }
    };
    let mut bytes = Vec::new();
    if let Err(error) = input.read_to_end(&mut bytes) {
        log::info!(
            "app_state: set app={app_id} scope={} caller={} rejected — read input: {error}",
            resolved.scope.as_str(),
            caller_label()
        );
        eprintln!("error: read input: {error}");
        return 1;
    }
    // Validate before writing: a rejected document must never reach disk, or
    // the app wakes to a corrupt file it did not cause.
    if let Err(error) = crate::host::wasm_python::decode_state_file(&bytes, resolved.format) {
        log::info!(
            "app_state: set app={app_id} scope={} format={} caller={} path={} rejected — {error}",
            resolved.scope.as_str(),
            resolved.format.as_str(),
            caller_label(),
            resolved.path.display()
        );
        eprintln!(
            "error: input is not valid {} state: {error}",
            resolved.format.as_str()
        );
        return 1;
    }
    if resolved.scope == StateScope::Context {
        // A user must never be able to accidentally commit their app state with
        // a project — same standing ruling the host's own write path follows.
        if let Err(error) =
            crate::workspace::secrets::ensure_app_state_gitignore(&resolved.context_root)
        {
            log::warn!(
                "app_state: could not ensure {}/.plexi/.gitignore covers app_states/: {error}",
                resolved.context_root.display()
            );
        }
    }
    match crate::platform::fs::atomic_write(&resolved.path, &bytes) {
        Ok(()) => {
            log::info!(
                "app_state: set app={app_id} scope={} format={} caller={} path={} accepted bytes={}",
                resolved.scope.as_str(),
                resolved.format.as_str(),
                caller_label(),
                resolved.path.display(),
                bytes.len()
            );
            0
        }
        Err(error) => {
            log::info!(
                "app_state: set app={app_id} scope={} caller={} path={} rejected — write failed: {error}",
                resolved.scope.as_str(),
                caller_label(),
                resolved.path.display()
            );
            eprintln!("error: write {}: {error}", resolved.path.display());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Install a minimal app into a staged global apps dir and return a registry
    /// that sees it. Same hermetic seam `src/app/registry.rs`'s own tests use —
    /// no chdir, which is process-global and would race the rest of the suite.
    fn install_app(
        state_section: Option<&str>,
    ) -> (tempfile::TempDir, crate::app::registry::AppRegistry) {
        let apps = tempdir().expect("apps dir");
        let app_dir = apps.path().join("test.state");
        std::fs::create_dir_all(&app_dir).expect("app dir");
        std::fs::write(app_dir.join("run.sh"), "#!/bin/sh\nexit 0\n").expect("entry");
        let manifest = format!(
            "schema_version = 1\n\n[app]\nid = \"test.state\"\ntype = \"app\"\n\
             name = \"State Test\"\nversion = \"0.0.1\"\nentry = \"run.sh\"\n{}",
            state_section.unwrap_or("")
        );
        std::fs::write(app_dir.join("manifest.toml"), manifest).expect("manifest");
        let bare = tempdir().expect("bare cwd");
        let registry =
            crate::app::registry::AppRegistry::load_with_global(bare.path(), apps.path());
        (apps, registry)
    }

    /// Retain a stale spawn-time environment value for negative controls.
    fn with_stale_root<T>(root: &Path, body: impl FnOnce() -> T) -> T {
        let previous = std::env::var_os("PLEXI_CONTEXT_ROOT");
        // SAFETY: every reader of this variable in-process is serialized behind
        // `env_guard`, which each test in this module holds for its duration.
        unsafe { std::env::set_var("PLEXI_CONTEXT_ROOT", root) };
        let out = body();
        match previous {
            Some(value) => unsafe { std::env::set_var("PLEXI_CONTEXT_ROOT", value) },
            None => unsafe { std::env::remove_var("PLEXI_CONTEXT_ROOT") },
        }
        out
    }

    /// Exercise the production host-query resolver using a bounded mock endpoint.
    fn with_context_root<T>(root: &Path, body: impl FnOnce() -> T) -> T {
        use std::io::{BufRead, Write};
        use std::os::unix::net::UnixListener;
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        let dir = tempdir().unwrap();
        let socket = dir.path().join("state.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let root = root.to_path_buf();
        let server = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut line = String::new();
                        std::io::BufReader::new(&stream).read_line(&mut line).unwrap();
                        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                        assert_eq!(request["type"], "get_pane_info");
                        stream.write_all(serde_json::json!({"context_root":root}).to_string().as_bytes()).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(5)),
                    Err(error) => panic!("state scope server: {error}"),
                }
            }
        });
        let keys = ["PLEXI_PANE_ID", "PLEXI_SOCKET"];
        let previous = keys.map(std::env::var_os);
        unsafe {
            std::env::set_var(keys[0], "42");
            std::env::set_var(keys[1], socket);
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        for (key, value) in keys.into_iter().zip(previous) {
            match value {
                Some(v) => unsafe { std::env::set_var(key, v) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
        match result { Ok(value) => value, Err(error) => std::panic::resume_unwind(error) }
    }

    #[test]
    fn context_targets_external_state_uses_nearest_file_anchor() {
        let root = tempdir().unwrap();
        let nested = root.path().join("nested");
        std::fs::create_dir_all(nested.join(".plexi")).unwrap();
        std::fs::create_dir_all(root.path().join(".plexi")).unwrap();
        assert_eq!(file_context_root(&nested).unwrap(), nested);
        assert_eq!(file_context_root(root.path()).unwrap(), root.path());
    }

    /// Serializes the env-mutating tests in this module against each other.
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn context_targets_stale_root_never_substitutes_for_unreachable_host() {
        let _guard = env_guard();
        let root = tempdir().unwrap();
        let previous = std::env::var_os("PLEXI_PANE_ID");
        unsafe { std::env::set_var("PLEXI_PANE_ID", "18446744073709551615") };
        let result = with_stale_root(root.path(), cli_context_root);
        match previous {
            Some(v) => unsafe { std::env::set_var("PLEXI_PANE_ID", v) },
            None => unsafe { std::env::remove_var("PLEXI_PANE_ID") },
        }
        assert!(result.is_err(), "a pane must ask its host, never use a stale env root: {result:?}");
    }

    #[test]
    fn context_targets_app_state_after_set_root_ignores_stale_environment() {
        use std::io::{BufRead, Write};
        use std::os::unix::net::UnixListener;
        let _guard = env_guard();
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_test_pane();
        let stale = h.app.router.active().root.clone();
        let current = tempdir().unwrap();
        h.app.set_context_root(current.path().to_path_buf(), None);
        let reply_path = h.profile_dir().join("pane-scope.json");
        h.inject_ipc(crate::protocol::AppRequest::GetPaneInfo {
            pane_id: pane, response_file: reply_path.to_string_lossy().into_owned(),
        });
        h.hidden_frame();
        let reply = std::fs::read(&reply_path).unwrap();
        let socket = h.profile_dir().join("scope.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + crate::testing::load_aware_timeout(std::time::Duration::from_secs(2));
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut line = String::new();
                        std::io::BufReader::new(&stream).read_line(&mut line).unwrap();
                        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                        assert_eq!(request["type"], "get_pane_info");
                        assert_eq!(request["pane_id"], pane);
                        stream.write_all(&reply).unwrap();
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline { break; }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => panic!("scope socket: {e}"),
                }
            }
        });
        let keys = ["PLEXI_PANE_ID", "PLEXI_SOCKET"];
        let previous = keys.map(std::env::var_os);
        unsafe {
            std::env::set_var(keys[0], pane.to_string());
            std::env::set_var(keys[1], socket);
        }
        let actual = with_stale_root(&stale, cli_context_root);
        for (key, value) in keys.into_iter().zip(previous) {
            match value {
                Some(v) => unsafe { std::env::set_var(key, v) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        server.join().unwrap();
        assert_eq!(actual.unwrap(), current.path());
    }

    const CONTEXT_ONLY: &str = "\n[state]\nscopes = [\"context\"]\n";

    #[test]
    fn set_writes_the_document_to_the_apps_declared_state_file() {
        let _guard = env_guard();
        let (_apps, registry) = install_app(Some(CONTEXT_ONLY));
        let root = tempdir().expect("context root");
        with_context_root(root.path(), || {
            assert_eq!(
                set_from(
                    "test.state",
                    &br#"{"items":["buy milk"]}"#[..],
                    None,
                    &registry
                ),
                0,
                "a valid document must be accepted"
            );
        });
        let written =
            std::fs::read_to_string(root.path().join(".plexi/app_states/test.state.json"))
                .expect("state file written");
        assert!(written.contains("buy milk"));
    }

    #[test]
    fn an_app_without_a_state_section_is_not_addressable() {
        let _guard = env_guard();
        let (_apps, registry) = install_app(None);
        let root = tempdir().expect("context root");
        with_context_root(root.path(), || {
            assert_eq!(
                set_from("test.state", &b"{}"[..], None, &registry),
                1,
                "an app that declared no [state] must be refused"
            );
            let error = resolve("test.state", None, &registry).expect_err("must be refused");
            assert!(
                error.contains("declares no [state] section"),
                "the refusal must name the reason, got: {error}"
            );
        });
    }

    #[test]
    fn an_undeclared_scope_is_refused_rather_than_silently_redirected() {
        let _guard = env_guard();
        let (_apps, registry) = install_app(Some(CONTEXT_ONLY));
        let root = tempdir().expect("context root");
        with_context_root(root.path(), || {
            let error =
                resolve("test.state", Some("global"), &registry).expect_err("must be refused");
            assert!(
                error.contains("does not declare the 'global' scope"),
                "the refusal must name the declared set, got: {error}"
            );
            assert_eq!(
                set_from("test.state", &b"{}"[..], Some("global"), &registry),
                1
            );
        });
    }

    #[test]
    fn a_malformed_document_never_reaches_disk() {
        let _guard = env_guard();
        let (_apps, registry) = install_app(Some(CONTEXT_ONLY));
        let root = tempdir().expect("context root");
        with_context_root(root.path(), || {
            assert_eq!(
                set_from("test.state", &b"not json at all"[..], None, &registry),
                1
            );
            // A JSON array parses, but a state document is an object.
            assert_eq!(set_from("test.state", &b"[1,2,3]"[..], None, &registry), 1);
        });
        assert!(
            !root
                .path()
                .join(".plexi/app_states/test.state.json")
                .exists(),
            "a rejected document must leave no file behind"
        );
    }

    #[test]
    fn a_traversal_or_absolute_app_id_cannot_address_a_file_outside_the_scope() {
        let _guard = env_guard();
        let (_apps, registry) = install_app(Some(CONTEXT_ONLY));
        let root = tempdir().expect("context root");
        with_context_root(root.path(), || {
            assert_eq!(
                set_from("../../etc/passwd", &b"{}"[..], None, &registry),
                1,
                "a traversal app id must never be writable"
            );
        });
        // The id gate is the load-bearing part: even for a declared app, such an
        // id never resolves to a path at all.
        for hostile in ["../../etc/passwd", "/etc/passwd", "a/b"] {
            assert!(
                crate::host::state_scope::state_file(
                    StateScope::Context,
                    hostile,
                    StateFormat::Json,
                    root.path()
                )
                .is_err(),
                "app id {hostile:?} must never resolve to a state path"
            );
        }
    }

    #[test]
    fn a_missing_context_root_errors_instead_of_being_created() {
        let _guard = env_guard();
        let (_apps, registry) = install_app(Some(CONTEXT_ONLY));
        let root = tempdir().expect("context root");
        let missing = root.path().join("no-such-context");
        with_context_root(&missing, || {
            assert_eq!(set_from("test.state", &b"{}"[..], None, &registry), 1);
        });
        assert!(
            !missing.exists(),
            "a context root that does not exist must never be conjured by a state write"
        );
    }

    #[test]
    fn markdown_state_passes_through_verbatim() {
        let _guard = env_guard();
        let (_apps, registry) = install_app(Some(
            "\n[state]\nscopes = [\"context\"]\nformat = \"markdown\"\n",
        ));
        let root = tempdir().expect("context root");
        let document = "- [ ] buy milk\n- [x] ship 0645\n";
        with_context_root(root.path(), || {
            assert_eq!(
                set_from("test.state", document.as_bytes(), None, &registry),
                0
            );
        });
        let written = std::fs::read_to_string(root.path().join(".plexi/app_states/test.state.md"))
            .expect("markdown state file");
        assert_eq!(
            written, document,
            "markdown state must reach disk byte-for-byte — the host is format-blind"
        );
    }

    #[test]
    fn a_symlinked_app_states_dir_is_refused() {
        let _guard = env_guard();
        let (_apps, registry) = install_app(Some(CONTEXT_ONLY));
        let root = tempdir().expect("context root");
        let elsewhere = tempdir().expect("escape target");
        let plexi_dir = root.path().join(".plexi");
        std::fs::create_dir_all(&plexi_dir).expect("plexi dir");
        std::os::unix::fs::symlink(elsewhere.path(), plexi_dir.join("app_states"))
            .expect("symlink app_states");
        with_context_root(root.path(), || {
            assert_eq!(
                set_from("test.state", &b"{}"[..], None, &registry),
                1,
                "a redirected app_states directory must be refused, not followed"
            );
        });
        assert!(
            !elsewhere.path().join("test.state.json").exists(),
            "nothing may be written through the symlink"
        );
    }

    #[test]
    fn get_on_an_app_with_no_state_file_yet_prints_the_empty_document() {
        let _guard = env_guard();
        let (_apps, registry) = install_app(Some(CONTEXT_ONLY));
        let root = tempdir().expect("context root");
        with_context_root(root.path(), || {
            let resolved = resolve("test.state", None, &registry).expect("resolves");
            assert!(!resolved.path.exists(), "fixture starts with no state file");
            assert_eq!(empty_document(resolved.format), "{}\n");
        });
    }
}
