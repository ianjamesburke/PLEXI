//! The workspace TOML config model (`workspace.toml`, `secrets.toml`) and the
//! canonical-name resolver every consumer shares: PGAP `secrets.get`,
//! `plexi run`, terminal PTY env injection, and host integrations.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use zeroize::Zeroizing;

use super::reconcile::canonical_friendly;
use super::store::NonDestructiveStore;
use super::{keychain_user_name, keychain_workspace_name};

#[derive(Deserialize, Debug, Clone)]
pub struct WorkspaceConfig {
    pub id: String,
    /// Optional `[context]` section — default name/description for the root
    /// context when this workspace is first opened. User overrides always win.
    #[serde(default)]
    pub context: Option<WorkspaceContextConfig>,
}

/// `[context]` section in workspace.toml. Provides default name and
/// description for the anchor's root context. Both fields are optional.
#[derive(Deserialize, Debug, Clone)]
pub struct WorkspaceContextConfig {
    pub name: Option<String>,
    pub description: Option<String>,
}

impl WorkspaceConfig {
    /// Read `<root>/<channel_dir>/workspace.toml`. Returns `None` if the file does
    /// not exist; returns `Err` only on a present-but-invalid file.
    pub fn load(workspace_root: &Path) -> Result<Option<Self>, String> {
        let path = workspace_root
            .join(crate::config::workspace_channel_dir())
            .join("workspace.toml");
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(format!("read {}: {e}", path.display()));
            }
        };
        toml::from_str::<WorkspaceConfig>(&raw)
            .map(Some)
            .map_err(|e| format!("parse {}: {e}", path.display()))
    }
}

// ── secrets.toml (router) ────────────────────────────────────────────────────

/// Parsed `<workspace_root>/.plexi/secrets.toml`. The `fallback` field has
/// **no** serde default — a missing-fallback file is rejected loudly so
/// users have to declare their stance explicitly.
#[derive(Deserialize, Debug, Clone)]
pub struct WorkspaceSecrets {
    pub fallback: bool,
    #[serde(default)]
    pub apps: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    pub default: HashMap<String, String>,
    #[serde(default)]
    pub terminal: TerminalSecrets,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct TerminalSecrets {
    #[serde(default)]
    pub env: TerminalEnvSecrets,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct TerminalEnvSecrets {
    #[serde(default)]
    pub inject: Vec<String>,
}

impl WorkspaceSecrets {
    pub fn parse(raw: &str) -> Result<Self, String> {
        toml::from_str::<Self>(raw).map_err(|e| format!("parse secrets.toml: {e}"))
    }

    /// Read `<root>/<channel_dir>/secrets.toml`. Returns `None` if the file does
    /// not exist; returns `Err` if it exists but is malformed (incl. missing
    /// `fallback`).
    pub fn load(workspace_root: &Path) -> Result<Option<Self>, String> {
        let path = workspace_root
            .join(crate::config::workspace_channel_dir())
            .join("secrets.toml");
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("read {}: {e}", path.display())),
        };
        Self::parse(&raw)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Look up a route for `(app_id, canonical_name)`. Returns the friendly
    /// Keychain name when an explicit `[apps.<app_id>]` route exists, or a
    /// `[default]` route otherwise. `None` means no route is defined.
    pub fn route_for(&self, app_id: &str, canonical_name: &str) -> Option<&str> {
        if let Some(app_routes) = self.apps.get(app_id) {
            if let Some(friendly) = app_routes.get(canonical_name) {
                return Some(friendly.as_str());
            }
        }
        self.default.get(canonical_name).map(|s| s.as_str())
    }
}

// ── Resolution result ────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ResolveOutcome {
    /// Found a value via app/default route or user-scope fallback.
    Found(Zeroizing<String>),
    /// `fallback = false` and no route defined — surface as a hard in-pane
    /// error. The host should NOT show a "create new" modal.
    HardMissing { reason: String },
    /// Route defined but no Keychain entry, OR no route + `fallback = true`
    /// + no user-scope entry. Show the missing-secret prompt modal.
    PromptUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedSecretSource {
    WorkspaceRoute,
    WorkspaceCanonical,
    GlobalCanonical,
}

#[derive(Debug)]
pub struct ResolvedSecret {
    pub value: Zeroizing<String>,
    pub source: ResolvedSecretSource,
}

/// 4-step runtime resolution. Pure function — no I/O beyond the `SecretStore`
/// trait calls. Tests use `InMemoryKeychain`; production uses `MacKeychain`.
pub fn resolve(
    workspace_id: &str,
    app_id: &str,
    canonical_name: &str,
    router: &WorkspaceSecrets,
    store: &dyn NonDestructiveStore,
) -> ResolveOutcome {
    match resolve_with_source(workspace_id, app_id, canonical_name, router, store) {
        ResolveWithSourceOutcome::Found(found) => ResolveOutcome::Found(found.value),
        ResolveWithSourceOutcome::HardMissing { reason } => ResolveOutcome::HardMissing { reason },
        ResolveWithSourceOutcome::PromptUser => ResolveOutcome::PromptUser,
    }
}

#[derive(Debug)]
pub enum ResolveWithSourceOutcome {
    Found(ResolvedSecret),
    HardMissing { reason: String },
    PromptUser,
}

/// Canonical-name resolver used by PGAP, `plexi run`, PTY env injection, and
/// host integrations. An explicit route remains an alias override; without a
/// route, the canonical env var name is the workspace Keychain suffix.
pub fn resolve_with_source(
    workspace_id: &str,
    app_id: &str,
    canonical_name: &str,
    router: &WorkspaceSecrets,
    store: &dyn NonDestructiveStore,
) -> ResolveWithSourceOutcome {
    // Step 1+2: workspace route (apps.<id> first, then [default]).
    if let Some(friendly) = router.route_for(app_id, canonical_name) {
        let account = keychain_workspace_name(workspace_id, friendly);
        if let Some(value) = store.get(&account) {
            return ResolveWithSourceOutcome::Found(ResolvedSecret {
                value,
                source: ResolvedSecretSource::WorkspaceRoute,
            });
        }
        // The route value is a persisted friendly name that reconcile cannot
        // rewrite (secrets.toml lives per-workspace; reconcile is
        // workspace-blind). If it is a legacy spelling whose keychain account
        // was renamed to canonical, honor the route through the same alias
        // table that renamed it — loudly, until the file is fixed.
        if let Some(canonical) = canonical_friendly(friendly) {
            let renamed = keychain_workspace_name(workspace_id, canonical);
            if let Some(value) = store.get(&renamed) {
                log::warn!(
                    "workspace_secrets: route for '{canonical_name}' (app '{app_id}') points at \
                     legacy spelling '{friendly}' but the keychain account is now '{renamed}' — \
                     resolving anyway; update the route value in .plexi/secrets.toml to '{canonical}'"
                );
                return ResolveWithSourceOutcome::Found(ResolvedSecret {
                    value,
                    source: ResolvedSecretSource::WorkspaceRoute,
                });
            }
        }
        // Route declared but Keychain is empty — prompt the user (don't
        // silently fall through to user-scope; the route was explicit).
        return ResolveWithSourceOutcome::PromptUser;
    }

    // Step 2.5: no alias route means the canonical name is the workspace key.
    let workspace_account = keychain_workspace_name(workspace_id, canonical_name);
    if let Some(value) = store.get(&workspace_account) {
        return ResolveWithSourceOutcome::Found(ResolvedSecret {
            value,
            source: ResolvedSecretSource::WorkspaceCanonical,
        });
    }

    // Step 3: user-scope fallback when allowed.
    if router.fallback {
        let user_account = keychain_user_name(canonical_name);
        if let Some(value) = store.get(&user_account) {
            return ResolveWithSourceOutcome::Found(ResolvedSecret {
                value,
                source: ResolvedSecretSource::GlobalCanonical,
            });
        }
        return ResolveWithSourceOutcome::PromptUser;
    }

    // Step 4: no route + fallback disabled → hard error.
    ResolveWithSourceOutcome::HardMissing {
        reason: format!(
            "no workspace or route value in .plexi/secrets.toml for app '{app_id}' / secret \
             '{canonical_name}', and fallback = false"
        ),
    }
}

pub fn resolve_terminal_env(
    workspace_root: &Path,
    store: &dyn NonDestructiveStore,
) -> Result<HashMap<String, Zeroizing<String>>, String> {
    let cfg = WorkspaceConfig::load(workspace_root)?
        .ok_or_else(|| format!("workspace.toml missing at {}", workspace_root.display()))?;
    let router = WorkspaceSecrets::load(workspace_root)?
        .ok_or_else(|| format!("secrets.toml missing at {}", workspace_root.display()))?;

    let mut env = HashMap::new();
    for canonical_name in &router.terminal.env.inject {
        match resolve_with_source(&cfg.id, "terminal", canonical_name, &router, store) {
            ResolveWithSourceOutcome::Found(found) => {
                log::info!(
                    "workspace_secrets: terminal env injecting {canonical_name} source={:?}",
                    found.source
                );
                env.insert(canonical_name.clone(), found.value);
            }
            ResolveWithSourceOutcome::PromptUser => {
                log::info!(
                    "workspace_secrets: terminal env skipped missing allowlisted secret {canonical_name}"
                );
            }
            ResolveWithSourceOutcome::HardMissing { reason } => {
                log::warn!("workspace_secrets: terminal env skipped {canonical_name}: {reason}");
            }
        }
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::secrets::reconcile::reconcile;
    use crate::workspace::secrets::{accounts, InMemoryKeychain, SecretStore};

    fn router(toml_src: &str) -> WorkspaceSecrets {
        WorkspaceSecrets::parse(toml_src).expect("router parses")
    }

    fn write_terminal_env_workspace(root: &Path, workspace_id: &str) {
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(root.join(&channel_dir)).unwrap();
        std::fs::write(
            root.join(&channel_dir).join("workspace.toml"),
            format!("id = \"{workspace_id}\"\n"),
        )
        .unwrap();
        std::fs::write(
            root.join(&channel_dir).join("secrets.toml"),
            "fallback = true\n\n[terminal.env]\ninject = [\"OPENROUTER_API_KEY\"]\n",
        )
        .unwrap();
    }

    #[test]
    fn routed_workspace_still_resolves_after_reconcile_renames_the_account() {
        // `secret set --alias openrouter-api-key` writes a route whose value
        // is the legacy friendly name. Reconcile renames the keychain account
        // to canonical but cannot rewrite per-workspace secrets.toml — the
        // resolver must honor the routed legacy spelling through the alias
        // table, or the secret silently vanishes from PTY env injection,
        // PGAP, and `plexi run` (found live by tester-6 on PR 2503).
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:openrouter-api-key", "sk-routed")
            .unwrap();
        let scanned = accounts(&["plexi:ws-1:openrouter-api-key"]);
        let report = reconcile(&scanned, &[], &store);
        assert_eq!(report.renamed.len(), 1, "{report:?}");
        assert!(
            store.get("plexi:ws-1:openrouter-api-key").is_none(),
            "precondition: the legacy account was renamed"
        );

        let router =
            router("fallback = false\n\n[default]\nOPENROUTER_API_KEY = \"openrouter-api-key\"\n");
        let outcome =
            resolve_with_source("ws-1", "terminal", "OPENROUTER_API_KEY", &router, &store);
        match outcome {
            ResolveWithSourceOutcome::Found(found) => {
                assert_eq!(found.value.to_string(), "sk-routed");
                assert!(matches!(found.source, ResolvedSecretSource::WorkspaceRoute));
            }
            other => panic!("routed secret must survive the rename, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_missing_fallback() {
        let err = WorkspaceSecrets::parse("[apps.foo]\nX = \"y\"\n")
            .expect_err("missing fallback should error");
        assert!(
            err.contains("fallback") || err.contains("missing field"),
            "expected fallback-related error, got: {err}"
        );
    }

    #[test]
    fn layer_1_app_route_returns_workspace_namespaced_value() {
        let store = InMemoryKeychain::new();
        store.set("plexi:ws-1:openai_prod", "sk-abc").unwrap();
        let r = router("fallback = false\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_prod\"\n");
        match resolve("ws-1", "claude-code", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "sk-abc"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn layer_2_default_route_used_when_no_app_route() {
        let store = InMemoryKeychain::new();
        store.set("plexi:ws-1:gh_team", "ghp-team").unwrap();
        let r = router("fallback = false\n[default]\nGITHUB_TOKEN = \"gh_team\"\n");
        match resolve("ws-1", "any-app", "GITHUB_TOKEN", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "ghp-team"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn fallback_false_with_no_route_is_hard_missing() {
        let store = InMemoryKeychain::new();
        // Even with a user-scope value present, fallback=false must NOT use it.
        store.set("plexi:user:OPENAI_API_KEY", "sk-user").unwrap();
        let r = router("fallback = false\n");
        match resolve("ws-1", "claude-code", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::HardMissing { reason } => {
                assert!(reason.contains("fallback = false"), "reason: {reason}");
            }
            other => panic!("expected HardMissing, got {other:?}"),
        }
    }

    #[test]
    fn fallback_true_reads_user_scope_when_no_route() {
        let store = InMemoryKeychain::new();
        store.set("plexi:user:GITHUB_TOKEN", "ghp-user").unwrap();
        let r = router("fallback = true\n");
        match resolve("ws-1", "claude-code", "GITHUB_TOKEN", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "ghp-user"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn canonical_workspace_name_resolves_without_alias_route() {
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENROUTER_API_KEY", "sk-workspace")
            .unwrap();
        let r = router("fallback = true\n");
        match resolve("ws-1", "terminal", "OPENROUTER_API_KEY", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "sk-workspace"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn workspace_canonical_value_overrides_global_fallback() {
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENAI_API_KEY", "sk-workspace")
            .unwrap();
        store.set("plexi:user:OPENAI_API_KEY", "sk-global").unwrap();
        let r = router("fallback = true\n");
        match resolve("ws-1", "terminal", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "sk-workspace"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn explicit_alias_route_takes_precedence_over_canonical_workspace_name() {
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENAI_API_KEY", "sk-canonical")
            .unwrap();
        store.set("plexi:ws-1:openai_personal", "sk-alias").unwrap();
        let r = router("fallback = true\n[default]\nOPENAI_API_KEY = \"openai_personal\"\n");
        match resolve_with_source("ws-1", "terminal", "OPENAI_API_KEY", &r, &store) {
            ResolveWithSourceOutcome::Found(found) => {
                assert_eq!(found.value.as_str(), "sk-alias");
                assert_eq!(found.source, ResolvedSecretSource::WorkspaceRoute);
            }
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn terminal_env_injects_only_allowlisted_names() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("workspace.toml"),
            "id = \"ws-1\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("secrets.toml"),
            "fallback = true\n\n[terminal.env]\ninject = [\"OPENROUTER_API_KEY\"]\n",
        )
        .unwrap();
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENROUTER_API_KEY", "sk-openrouter")
            .unwrap();
        store.set("plexi:ws-1:OPENAI_API_KEY", "sk-openai").unwrap();

        let env = resolve_terminal_env(tmp.path(), &store).expect("terminal env resolves");

        assert_eq!(
            env.get("OPENROUTER_API_KEY").map(|v| v.as_str()),
            Some("sk-openrouter")
        );
        assert!(
            !env.contains_key("OPENAI_API_KEY"),
            "non-allowlisted secret must not be injected"
        );
    }

    #[test]
    fn terminal_env_uses_global_fallback_when_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("workspace.toml"),
            "id = \"ws-1\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("secrets.toml"),
            "fallback = true\n\n[terminal.env]\ninject = [\"OPENAI_API_KEY\"]\n",
        )
        .unwrap();
        let store = InMemoryKeychain::new();
        store.set("plexi:user:OPENAI_API_KEY", "sk-global").unwrap();

        let env = resolve_terminal_env(tmp.path(), &store).expect("terminal env resolves");

        assert_eq!(
            env.get("OPENAI_API_KEY").map(|v| v.as_str()),
            Some("sk-global")
        );
    }

    #[test]
    fn terminal_env_resolves_same_openrouter_name_per_workspace() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        write_terminal_env_workspace(ws_a.path(), "ws-a");
        write_terminal_env_workspace(ws_b.path(), "ws-b");

        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-a:OPENROUTER_API_KEY", "sk-openrouter-a")
            .unwrap();
        store
            .set("plexi:ws-b:OPENROUTER_API_KEY", "sk-openrouter-b")
            .unwrap();

        let env_a = resolve_terminal_env(ws_a.path(), &store).expect("workspace A env");
        let env_b = resolve_terminal_env(ws_b.path(), &store).expect("workspace B env");

        assert_eq!(
            env_a.get("OPENROUTER_API_KEY").map(|v| v.as_str()),
            Some("sk-openrouter-a")
        );
        assert_eq!(
            env_b.get("OPENROUTER_API_KEY").map(|v| v.as_str()),
            Some("sk-openrouter-b")
        );
    }

    #[test]
    fn terminal_env_injects_nothing_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("workspace.toml"),
            "id = \"ws-1\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("secrets.toml"),
            "fallback = true\n",
        )
        .unwrap();
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENROUTER_API_KEY", "sk-workspace")
            .unwrap();
        store.set("plexi:user:OPENAI_API_KEY", "sk-global").unwrap();

        let env = resolve_terminal_env(tmp.path(), &store).expect("terminal env resolves");

        assert!(env.is_empty(), "terminal injection must be opt-in");
    }

    #[test]
    fn same_canonical_name_two_workspaces_returns_different_values() {
        // The whole point of workspace-scoping: same OPENAI_API_KEY in the
        // app, different bills downstream.
        let store = InMemoryKeychain::new();
        store.set("plexi:work:openai_prod", "sk-work").unwrap();
        store
            .set("plexi:personal:openai_personal", "sk-personal")
            .unwrap();
        let work_router =
            router("fallback = false\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_prod\"\n");
        let personal_router =
            router("fallback = false\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_personal\"\n");
        let work = match resolve(
            "work",
            "claude-code",
            "OPENAI_API_KEY",
            &work_router,
            &store,
        ) {
            ResolveOutcome::Found(v) => v.to_string(),
            other => panic!("work: {other:?}"),
        };
        let personal = match resolve(
            "personal",
            "claude-code",
            "OPENAI_API_KEY",
            &personal_router,
            &store,
        ) {
            ResolveOutcome::Found(v) => v.to_string(),
            other => panic!("personal: {other:?}"),
        };
        assert_eq!(work, "sk-work");
        assert_eq!(personal, "sk-personal");
        assert_ne!(work, personal);
    }

    #[test]
    fn route_declared_but_keychain_empty_prompts_user() {
        let store = InMemoryKeychain::new();
        // Router points at a friendly name but no Keychain entry was set.
        let r = router("fallback = true\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_prod\"\n");
        match resolve("ws-1", "claude-code", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::PromptUser => {}
            other => panic!("expected PromptUser, got {other:?}"),
        }
    }

    #[test]
    fn app_route_takes_precedence_over_default() {
        let store = InMemoryKeychain::new();
        store.set("plexi:ws-1:per_app", "per-app-val").unwrap();
        store.set("plexi:ws-1:default_val", "default-val").unwrap();
        let r = router(
            "fallback = false\n\
             [apps.claude-code]\n\
             OPENAI_API_KEY = \"per_app\"\n\
             [default]\n\
             OPENAI_API_KEY = \"default_val\"\n",
        );
        match resolve("ws-1", "claude-code", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "per-app-val"),
            other => panic!("{other:?}"),
        }
    }
}
