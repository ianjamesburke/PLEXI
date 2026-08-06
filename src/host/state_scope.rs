//! User-data scope resolution — the addressing layer for files a user owns.
//!
//! One `StateScope` pair, applied per [`UserDataKind`]. An app declares which
//! scopes it uses in its manifest (`[state] scopes`); the host owns path
//! construction. Two rules, both predictable from outside the process so an
//! external agent can find and write the bytes without asking Plexi anything:
//!
//! ```text
//! global   →  ~/.plexi/<kind>/…
//! context  →  <context.root>/.plexi/<kind>/…
//! ```
//!
//! `app_states` was the first kind; `notes` joined it in stint 0746. This is
//! one of Plexi's three scope models and the only one that addresses files —
//! see `src/host/AGENTS.md` for how it differs from `host/scope.rs`'s runtime
//! reachability and `app/registry.rs`'s shadowing config discovery.
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

/// The directory name holding kept notes in either tier. Same no-rename rule as
/// `APP_STATES_DIR`: it is a user-data address, not an implementation detail.
const NOTES_DIR: &str = "notes";

/// The `.plexi` directory both tiers nest under. Deliberately channel-NEUTRAL —
/// never `workspace_channel_dir()`/`config_dir()`, which are channel-scoped.
/// User data must not fork when the user runs a beta or PR build; config may.
const PLEXI_DIR: &str = ".plexi";

/// A class of user-owned data addressed by [`StateScope`]. Each kind is one leaf
/// directory name under `.plexi/`, present identically in both tiers.
///
/// Adding a kind is how a subsystem joins this scope model. It is not a place to
/// put host config or anything channel-scoped — those belong to
/// `app/registry.rs`'s discovery model instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UserDataKind {
    /// Per-app persisted state, addressed by app id (`[state] scopes`).
    AppStates,
    /// Kept notes — a flat directory of Markdown files (stint 0746).
    Notes,
}

impl UserDataKind {
    /// The leaf directory name under `.plexi/`.
    pub fn dir(self) -> &'static str {
        match self {
            Self::AppStates => APP_STATES_DIR,
            Self::Notes => NOTES_DIR,
        }
    }
}

/// A state scope an app may declare. Ordered lists come from the manifest;
/// the first declared scope is the app's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateScope {
    /// Cross-project user data: `~/.plexi/<kind>/`.
    Global,
    /// Project data anchored to the pane's context root:
    /// `<context.root>/.plexi/<kind>/`.
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
    kind: UserDataKind,
    context_root: &Path,
) -> Result<(), String> {
    let (base, tail) = scope_layout(scope, kind, context_root);
    let expected = tail.iter().fold(
        resolve_existing_prefix(&base)
            .map_err(|error| format!("resolve scope base {}: {error}", base.display()))?,
        |dir, part| dir.join(part),
    );
    let parent = path.parent().ok_or_else(|| {
        format!(
            "{} path {} has no parent directory",
            kind.dir(),
            path.display()
        )
    })?;
    let actual = resolve_existing_prefix(parent)
        .map_err(|error| format!("resolve {} dir {}: {error}", kind.dir(), parent.display()))?;
    if actual != expected {
        return Err(format!(
            "{} path {} escapes its scope: parent resolves to {} but the {} scope \
             directory is {} — refusing to follow a redirected {} directory",
            kind.dir(),
            path.display(),
            actual.display(),
            scope.as_str(),
            expected.display(),
            kind.dir()
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

/// A scope's directory expressed as a trusted base to canonicalize plus tail
/// components to append *literally*. Splitting it this way is what makes the
/// symlink-escape guard work: only the base is resolved, so a symlink anywhere
/// in the tail (a redirected `.plexi/` or `<kind>/`) fails the comparison in
/// [`assert_within_scope`] instead of being silently followed.
///
/// The global tier is `crate::config::shared_dir()` — deliberately NOT
/// `config_dir()`, which is channel-scoped. User data must not fork when the
/// user runs a beta or PR build; config may. Using `shared_dir()` rather than
/// re-deriving `home_dir().join(".plexi")` also picks up its thread-local test
/// override, so both tiers are testable without touching a real profile.
fn scope_layout(
    scope: StateScope,
    kind: UserDataKind,
    context_root: &Path,
) -> (PathBuf, Vec<std::ffi::OsString>) {
    match scope {
        StateScope::Global => {
            let shared = crate::config::shared_dir();
            // Canonicalize the *parent* and keep the `.plexi` component literal,
            // so a symlinked `~/.plexi` is rejected exactly as a symlinked
            // `<kind>/` is.
            let name = shared
                .file_name()
                .map(std::ffi::OsStr::to_os_string)
                .unwrap_or_default();
            let base = shared.parent().map(Path::to_path_buf).unwrap_or(shared);
            (base, vec![name, kind.dir().into()])
        }
        StateScope::Context => (
            context_root.to_path_buf(),
            vec![PLEXI_DIR.into(), kind.dir().into()],
        ),
    }
}

/// The directory holding `kind`'s data in `scope`. The one place both tiers'
/// layout is constructed; every kind-specific helper below routes through it so
/// there is a single definition of "where does this tier live".
pub fn user_data_dir(scope: StateScope, kind: UserDataKind, context_root: &Path) -> PathBuf {
    let (base, tail) = scope_layout(scope, kind, context_root);
    tail.iter().fold(base, |dir, part| dir.join(part))
}

/// The channel-neutral global notes directory: `~/.plexi/notes/`.
pub fn global_notes_dir() -> PathBuf {
    user_data_dir(StateScope::Global, UserDataKind::Notes, Path::new(""))
}

/// The context-scoped notes directory inside a context root:
/// `<context_root>/.plexi/notes/`.
pub fn context_notes_dir(context_root: &Path) -> PathBuf {
    user_data_dir(StateScope::Context, UserDataKind::Notes, context_root)
}

/// Resolve a scope to the concrete state file for `app_id`. `ext` is the
/// app's state file extension without the dot (the host's Python runtime
/// persists JSON, so it passes `json`). `context_root` is the root of the
/// pane's context *at call time*, never a launch-captured copy.
pub fn state_path(scope: StateScope, app_id: &str, ext: &str, context_root: &Path) -> PathBuf {
    user_data_dir(scope, UserDataKind::AppStates, context_root).join(format!("{app_id}.{ext}"))
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

    /// Every kind obeys the same two rules, so a new kind cannot quietly invent
    /// its own layout — the whole point of routing them through `user_data_dir`.
    #[test]
    fn every_user_data_kind_follows_the_same_two_rules() {
        let root = Path::new("/projects/demo");
        for kind in [UserDataKind::AppStates, UserDataKind::Notes] {
            let context = user_data_dir(StateScope::Context, kind, root);
            assert_eq!(
                context,
                PathBuf::from(format!("/projects/demo/.plexi/{}", kind.dir())),
                "context tier for {kind:?}"
            );
            let global = user_data_dir(StateScope::Global, kind, root);
            assert!(
                global.ends_with(format!(".plexi/{}", kind.dir())),
                "global tier for {kind:?} must live under ~/.plexi: {global:?}"
            );
            assert!(
                !global.starts_with(root),
                "global tier for {kind:?} must ignore the context root"
            );
        }
    }

    /// Notes are channel-neutral like app state: the tier must never pick up a
    /// `.plexi-<channel>` profile dir, or a user's notes fork the moment they
    /// run a beta or PR build.
    #[test]
    fn notes_tiers_are_channel_neutral() {
        let root = Path::new("/projects/demo");
        assert_eq!(
            context_notes_dir(root),
            PathBuf::from("/projects/demo/.plexi/notes")
        );
        for dir in [context_notes_dir(root), global_notes_dir()] {
            let has_channel_component = dir.components().any(|c| {
                let name = c.as_os_str().to_string_lossy();
                name.starts_with(".plexi-")
            });
            assert!(
                !has_channel_component,
                "notes tier must be channel-neutral: {dir:?}"
            );
        }
    }

    /// The symlink-escape guard is kind-aware: a redirected `notes` directory is
    /// rejected the same way a redirected `app_states` one is, and the message
    /// names the kind rather than always saying `app_states`.
    #[test]
    fn assert_within_scope_rejects_a_symlinked_notes_dir() {
        let root = tempfile::tempdir().expect("context root");
        let elsewhere = tempfile::tempdir().expect("attacker-controlled dir");
        let plexi_dir = root.path().join(PLEXI_DIR);
        std::fs::create_dir_all(&plexi_dir).expect("create .plexi");
        std::os::unix::fs::symlink(elsewhere.path(), plexi_dir.join(NOTES_DIR))
            .expect("symlink notes elsewhere");

        let note = context_notes_dir(root.path()).join("captured.md");
        let err = assert_within_scope(&note, StateScope::Context, UserDataKind::Notes, root.path())
            .expect_err("a symlinked notes dir must be rejected");
        assert!(err.contains("escapes its scope"), "unexpected error: {err}");
        assert!(
            err.contains("notes") && !err.contains("app_states"),
            "the error must name the notes kind, not app_states: {err}"
        );

        // A real directory, and a not-yet-created one, both pass.
        let honest = tempfile::tempdir().expect("honest root");
        std::fs::create_dir_all(context_notes_dir(honest.path())).expect("create notes dir");
        assert_within_scope(
            &context_notes_dir(honest.path()).join("captured.md"),
            StateScope::Context,
            UserDataKind::Notes,
            honest.path(),
        )
        .expect("real notes dir is within scope");

        let fresh = tempfile::tempdir().expect("fresh root");
        assert_within_scope(
            &context_notes_dir(fresh.path()).join("captured.md"),
            StateScope::Context,
            UserDataKind::Notes,
            fresh.path(),
        )
        .expect("missing notes dir is within scope before first write");
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
        let err = assert_within_scope(
            &path,
            StateScope::Context,
            UserDataKind::AppStates,
            root.path(),
        )
        .expect_err("a symlinked app_states dir must be rejected");
        assert!(err.contains("escapes its scope"), "unexpected error: {err}");

        // A genuine directory passes.
        let honest = tempfile::tempdir().expect("honest root");
        let honest_dir = user_data_dir(StateScope::Context, UserDataKind::AppStates, honest.path());
        std::fs::create_dir_all(&honest_dir).expect("create app_states");
        let honest_path = state_file(
            StateScope::Context,
            "todo",
            StateFormat::Json,
            honest.path(),
        )
        .expect("resolve state file");
        assert_within_scope(
            &honest_path,
            StateScope::Context,
            UserDataKind::AppStates,
            honest.path(),
        )
        .expect("real app_states dir is within scope");

        // Not-yet-created app_states dir is fine too (first write).
        let fresh = tempfile::tempdir().expect("fresh root");
        let fresh_path = state_file(StateScope::Context, "todo", StateFormat::Json, fresh.path())
            .expect("resolve state file");
        assert_within_scope(
            &fresh_path,
            StateScope::Context,
            UserDataKind::AppStates,
            fresh.path(),
        )
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

    /// Both kinds' global tier stays `.plexi`, never `.plexi-<channel>` — a
    /// channel-suffixed tier would fork a user's data per build.
    #[test]
    fn global_dir_is_channel_neutral() {
        for kind in [UserDataKind::AppStates, UserDataKind::Notes] {
            let dir = user_data_dir(StateScope::Global, kind, Path::new(""));
            let plexi_component = dir
                .components()
                .filter_map(|c| c.as_os_str().to_str())
                .find(|c| c.starts_with(".plexi"))
                .expect("path contains a .plexi component");
            assert_eq!(
                plexi_component, ".plexi",
                "global {kind:?} dir must not be channel-suffixed"
            );
        }
    }
}
