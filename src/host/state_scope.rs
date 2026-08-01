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

/// The scopes an app that omits `[state]` gets.
pub fn default_scopes() -> Vec<StateScope> {
    vec![StateScope::Global]
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
        assert!(
            parse_scopes(&["global".to_string(), "global".to_string()])
                .unwrap_err()
                .contains("more than once")
        );
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
