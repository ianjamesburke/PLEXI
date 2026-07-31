//! App state scope resolution — the addressing layer for app state.
//!
//! An app declares which scopes it uses in its manifest (`[state] scopes`);
//! the host owns path construction. Two rules, both predictable from outside
//! the process so an external agent can find and write an app's state without
//! asking Plexi anything:
//!
//! ```text
//! global   →  ~/.plexi/app_states/<app_id>.<ext>
//! context  →  <context.root>/.plexi/app_states/<app_id>.<ext>
//! ```
//!
//! Both tiers are deliberately channel-neutral (`.plexi`, never
//! `.plexi-<channel>`): user data must not fork when the user runs a beta or
//! PR build. Config stays channel-scoped; state and config diverge here on
//! purpose. Context-scoped state lives *inside* the context root — never in a
//! central store keyed by a sanitized root path — so it is discoverable,
//! greppable, and moves with the project.
//!
//! Scope is a property of the app, not of an instance or a launch: no pane,
//! placement, or restore decision may change where an app's bytes go.
//! Resolution happens against the pane's context root at call time, so
//! `plexi context set-root` immediately redirects where context-scoped state
//! resolves.

use std::path::{Path, PathBuf};

/// The directory name shared by both tiers. Consciously kept as `app_states`;
/// do not rename it — one data-loss bug from moving a state path is enough.
const APP_STATES_DIR: &str = "app_states";

/// A state scope an app may declare. Ordered lists come from the manifest;
/// the first declared scope is the app's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateScope {
    /// Cross-project user state: `~/.plexi/app_states/`.
    Global,
    /// Project state anchored to the pane's context root:
    /// `<context.root>/.plexi/app_states/`.
    Context,
}

impl StateScope {
    /// Manifest / wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Context => "context",
        }
    }

    /// Parse a manifest or wire scope name. `Err` carries the unknown value;
    /// callers turn it into a loud install/launch/read error — never a
    /// silent fallback to another scope.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "global" => Ok(Self::Global),
            "context" => Ok(Self::Context),
            other => Err(format!(
                "unknown state scope '{other}' — valid scopes: global, context"
            )),
        }
    }
}

/// The on-disk format of an app's state file. Declared in the manifest
/// (`[state] format`); the host is format-blind for anything but JSON — a
/// markdown file's bytes pass through verbatim under a single `document` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StateFormat {
    /// Pretty-printed JSON object — the default.
    #[default]
    Json,
    /// A plain-text markdown document; the host never parses it.
    Markdown,
}

impl StateFormat {
    /// Manifest / wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Markdown => "markdown",
        }
    }

    /// State-file extension, without the dot.
    pub fn ext(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Markdown => "md",
        }
    }

    /// Parse a manifest format name. Unknown values are a loud error, never a
    /// silent fallback to JSON.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "json" => Ok(Self::Json),
            "markdown" => Ok(Self::Markdown),
            other => Err(format!(
                "unknown state format '{other}' — valid formats: json, markdown"
            )),
        }
    }
}

/// The scopes an app that omits `[state]` gets.
pub fn default_scopes() -> Vec<StateScope> {
    vec![StateScope::Global]
}

/// Validate an app id before it is used as a state-file name component.
/// The id becomes `<dir>/<app_id>.<ext>` on disk, so anything that could
/// escape the state directory (separators, traversal, hidden-file prefixes)
/// is rejected loudly.
pub fn validate_app_id(app_id: &str) -> Result<(), String> {
    if app_id.is_empty() {
        return Err("app id is empty — cannot resolve a state file".to_string());
    }
    if app_id.starts_with('.') {
        return Err(format!(
            "app id '{app_id}' starts with '.' — state file names must not be hidden or traversal paths"
        ));
    }
    if app_id.contains("..") {
        return Err(format!(
            "app id '{app_id}' contains '..' — path traversal is not allowed in state file names"
        ));
    }
    if let Some(bad) = app_id
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!(
            "app id '{app_id}' contains invalid character '{bad}' — allowed: A-Z a-z 0-9 . _ -"
        ));
    }
    Ok(())
}

/// Resolve a scope + format to the concrete state file for `app_id`,
/// validating the app id first. New callers route through this; `state_path`
/// stays as the raw two-rule resolver.
pub fn state_file(
    scope: StateScope,
    app_id: &str,
    format: StateFormat,
    context_root: &Path,
) -> Result<PathBuf, String> {
    validate_app_id(app_id)?;
    Ok(state_path(scope, app_id, format.ext(), context_root))
}

/// Assert that `path`'s parent directory really is the scope's state
/// directory, resolving symlinks. Catches a symlinked `.plexi/` or
/// `app_states/` that would silently redirect writes outside the scope.
///
/// The comparison anchors on the scope's trusted *base* (the context root, or
/// the home dir for global scope): the base is canonicalized, the
/// `.plexi/app_states` tail is appended literally, and the actual parent's
/// fully-resolved form must match exactly. Components of the parent that do
/// not exist yet (first write) cannot hide a symlink and are appended
/// literally too.
pub fn assert_within_scope(
    path: &Path,
    scope: StateScope,
    context_root: &Path,
) -> Result<(), String> {
    let (base, plexi_dir) = match scope {
        StateScope::Global => (
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            ".plexi",
        ),
        StateScope::Context => (context_root.to_path_buf(), ".plexi"),
    };
    let expected = resolve_existing_prefix(&base)
        .map_err(|error| format!("resolve scope base {}: {error}", base.display()))?
        .join(plexi_dir)
        .join(APP_STATES_DIR);
    let parent = path
        .parent()
        .ok_or_else(|| format!("state path {} has no parent directory", path.display()))?;
    let actual = resolve_existing_prefix(parent)
        .map_err(|error| format!("resolve state dir {}: {error}", parent.display()))?;
    if actual != expected {
        return Err(format!(
            "state path {} escapes its scope: parent resolves to {} but the {} scope \
             directory is {} — refusing to follow a redirected app_states directory",
            path.display(),
            actual.display(),
            scope.as_str(),
            expected.display()
        ));
    }
    Ok(())
}

/// Fully resolve `path` by canonicalizing its nearest existing ancestor and
/// re-appending the not-yet-existing tail components literally (they cannot
/// contain symlinks because they do not exist).
fn resolve_existing_prefix(path: &Path) -> Result<PathBuf, String> {
    let mut probe = path;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !probe.exists() {
        let name = probe
            .file_name()
            .ok_or_else(|| format!("no existing ancestor for {}", path.display()))?;
        tail.push(name.to_os_string());
        probe = probe
            .parent()
            .ok_or_else(|| format!("no existing ancestor for {}", path.display()))?;
    }
    let mut resolved = probe
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", probe.display()))?;
    for name in tail.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

/// Write `bytes` to `path` atomically: sibling temp file, fsync, rename.
/// A reader never observes a partial file; a crash leaves at worst an
/// orphaned `.{name}.tmp-{uuid}` sibling. Pattern shared with
/// `crate::assistant::store`.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| format!("write {}: missing parent", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut file =
            std::fs::File::create(&temp).map_err(|e| format!("create {}: {e}", temp.display()))?;
        file.write_all(bytes)
            .map_err(|e| format!("write {}: {e}", temp.display()))?;
        file.sync_all()
            .map_err(|e| format!("sync {}: {e}", temp.display()))?;
        std::fs::rename(&temp, path)
            .map_err(|e| format!("rename {} to {}: {e}", temp.display(), path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Validate a manifest `[state] scopes` list. Rules (fail loud, no silent
/// fallback — the `join_group` free-form key is the cautionary precedent):
/// an empty list is an error, an unknown value is an error, a duplicate is an
/// error. Order is preserved; the first entry is the app's default scope.
pub fn parse_scopes(raw: &[String]) -> Result<Vec<StateScope>, String> {
    if raw.is_empty() {
        return Err(
            "manifest [state] scopes is empty — declare at least one of: global, context"
                .to_string(),
        );
    }
    let mut scopes = Vec::with_capacity(raw.len());
    for value in raw {
        let scope = StateScope::parse(value)
            .map_err(|error| format!("manifest [state] scopes: {error}"))?;
        if scopes.contains(&scope) {
            return Err(format!(
                "manifest [state] scopes declares '{}' more than once",
                scope.as_str()
            ));
        }
        scopes.push(scope);
    }
    Ok(scopes)
}

/// The channel-neutral global state directory: `~/.plexi/app_states/`.
/// Deliberately NOT `config_dir()` — the profile dir is channel-scoped and
/// user state must not fork per channel.
pub fn global_state_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".plexi")
        .join(APP_STATES_DIR)
}

/// The context-scoped state directory inside a context root.
pub fn context_state_dir(context_root: &Path) -> PathBuf {
    context_root.join(".plexi").join(APP_STATES_DIR)
}

/// Resolve a scope to the concrete state file for `app_id`. `ext` is the
/// app's state file extension without the dot (the host's Python runtime
/// persists JSON, so it passes `json`). `context_root` is the root of the
/// pane's context *at call time*, never a launch-captured copy.
pub fn state_path(scope: StateScope, app_id: &str, ext: &str, context_root: &Path) -> PathBuf {
    let dir = match scope {
        StateScope::Global => global_state_dir(),
        StateScope::Context => context_state_dir(context_root),
    };
    dir.join(format!("{app_id}.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scopes_accepts_ordered_valid_lists() {
        let scopes = parse_scopes(&["context".to_string(), "global".to_string()]).unwrap();
        assert_eq!(scopes, vec![StateScope::Context, StateScope::Global]);
    }

    #[test]
    fn parse_scopes_rejects_empty_unknown_and_duplicates() {
        assert!(parse_scopes(&[]).unwrap_err().contains("empty"));
        assert!(parse_scopes(&["workspace".to_string()])
            .unwrap_err()
            .contains("unknown state scope 'workspace'"));
        assert!(parse_scopes(&["global".to_string(), "global".to_string()])
            .unwrap_err()
            .contains("more than once"));
    }

    #[test]
    fn state_paths_follow_the_two_rules() {
        let root = Path::new("/projects/demo");
        assert_eq!(
            state_path(StateScope::Context, "todo", "json", root),
            PathBuf::from("/projects/demo/.plexi/app_states/todo.json")
        );
        let global = state_path(StateScope::Global, "todo", "json", root);
        assert!(
            global.ends_with(".plexi/app_states/todo.json"),
            "global path must live under ~/.plexi/app_states: {global:?}"
        );
        assert!(
            !global.starts_with(root),
            "global scope must ignore the context root"
        );
    }

    #[test]
    fn validate_app_id_rejects_traversal_separators_and_empties() {
        assert!(validate_app_id("").unwrap_err().contains("empty"));
        assert!(validate_app_id(".hidden").unwrap_err().contains("'.'"));
        assert!(validate_app_id("a..b").unwrap_err().contains(".."));
        assert!(validate_app_id("a/b")
            .unwrap_err()
            .contains("invalid character"));
        assert!(validate_app_id("a\\b")
            .unwrap_err()
            .contains("invalid character"));
        assert!(validate_app_id("a b")
            .unwrap_err()
            .contains("invalid character"));
        assert!(validate_app_id("todo").is_ok());
        assert!(validate_app_id("acme.todo-2").is_ok());
    }

    #[test]
    fn state_file_refuses_bad_app_ids_and_uses_format_ext() {
        let root = Path::new("/projects/demo");
        assert!(state_file(StateScope::Context, "../todo", StateFormat::Json, root).is_err());
        assert!(state_file(StateScope::Context, "a/b", StateFormat::Json, root).is_err());
        assert_eq!(
            state_file(StateScope::Context, "todo", StateFormat::Markdown, root).unwrap(),
            PathBuf::from("/projects/demo/.plexi/app_states/todo.md")
        );
        assert_eq!(
            state_file(StateScope::Context, "todo", StateFormat::Json, root).unwrap(),
            PathBuf::from("/projects/demo/.plexi/app_states/todo.json")
        );
    }

    #[test]
    fn state_format_parses_and_defaults() {
        assert_eq!(StateFormat::parse("json").unwrap(), StateFormat::Json);
        assert_eq!(
            StateFormat::parse("markdown").unwrap(),
            StateFormat::Markdown
        );
        assert!(StateFormat::parse("yaml")
            .unwrap_err()
            .contains("valid formats: json, markdown"));
        assert_eq!(StateFormat::default(), StateFormat::Json);
        assert_eq!(StateFormat::Markdown.ext(), "md");
        assert_eq!(StateFormat::Markdown.as_str(), "markdown");
    }

    #[test]
    fn assert_within_scope_rejects_a_symlinked_app_states_dir() {
        let root = tempfile::tempdir().expect("context root");
        let elsewhere = tempfile::tempdir().expect("attacker-controlled dir");
        let plexi_dir = root.path().join(".plexi");
        std::fs::create_dir_all(&plexi_dir).expect("create .plexi");
        std::os::unix::fs::symlink(elsewhere.path(), plexi_dir.join(APP_STATES_DIR))
            .expect("symlink app_states elsewhere");

        let path = state_file(StateScope::Context, "todo", StateFormat::Json, root.path())
            .expect("resolve state file");
        let err = assert_within_scope(&path, StateScope::Context, root.path())
            .expect_err("a symlinked app_states dir must be rejected");
        assert!(err.contains("escapes its scope"), "unexpected error: {err}");

        // A genuine directory passes.
        let honest = tempfile::tempdir().expect("honest root");
        let honest_dir = context_state_dir(honest.path());
        std::fs::create_dir_all(&honest_dir).expect("create app_states");
        let honest_path = state_file(
            StateScope::Context,
            "todo",
            StateFormat::Json,
            honest.path(),
        )
        .expect("resolve state file");
        assert_within_scope(&honest_path, StateScope::Context, honest.path())
            .expect("real app_states dir is within scope");

        // Not-yet-created app_states dir is fine too (first write).
        let fresh = tempfile::tempdir().expect("fresh root");
        let fresh_path = state_file(StateScope::Context, "todo", StateFormat::Json, fresh.path())
            .expect("resolve state file");
        assert_within_scope(&fresh_path, StateScope::Context, fresh.path())
            .expect("missing app_states dir is within scope before first write");
    }

    #[test]
    fn atomic_write_leaves_no_temp_residue() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("app_states").join("todo.json");
        atomic_write(&path, b"{\"k\":1}").expect("atomic write");
        assert_eq!(std::fs::read(&path).expect("read back"), b"{\"k\":1}");
        let residue: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(residue.is_empty(), "temp residue left behind: {residue:?}");
    }

    #[test]
    fn global_dir_is_channel_neutral() {
        let dir = global_state_dir();
        let plexi_component = dir
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .find(|c| c.starts_with(".plexi"))
            .expect("path contains a .plexi component");
        assert_eq!(
            plexi_component, ".plexi",
            "global state dir must not be channel-suffixed"
        );
    }
}
