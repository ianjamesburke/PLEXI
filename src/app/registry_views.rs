//! Per-root `AppRegistry` views (stint 0724, Phase B).
//!
//! Before this module, `PlexiApp` held exactly one `AppRegistry`, rescanned
//! only when the *active* context transitioned (`apply_context_transition_effects`
//! in `pane_ops::workspace`). Any call site that resolved an app id against a
//! context other than whichever one happened to be active last — a launch
//! targeting an explicit `context_id`, the registry watcher for a background
//! context, app-state routing for a parked pane — got whatever the active
//! context's registry happened to contain, which is either stale or simply
//! the wrong workspace's apps.
//!
//! `RegistryViews` fixes this by keying the cache on the canonical root
//! itself (the host's source-address for "which app definitions apply here"),
//! not on which context is active. Two contexts whose `Context.root` is the
//! same path share one scan and one cache entry; two contexts with different
//! roots always get independently correct views, no matter which one is
//! active. This is the "shared root ⇒ shared root-backed definitions,
//! distinct runtime ownership" rule from the stint.
//!
//! Every view is produced by [`crate::app::registry::AppRegistry::load`] (or
//! `load_with_global` for the global-dir override tests need) — this module
//! never reimplements discovery, shadowing, or conflict handling; it only
//! decides which root to scan and when to rescan it.

use crate::app::registry::AppRegistry;
use crate::host::scope::ScopeInvalidation;
use crate::workspace::router::WorkspaceRouter;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Canonical root → (loaded registry, generation). `generation` increments on
/// every forced rescan so callers that only need "did this change" can diff
/// it cheaply; Phase B has no such caller yet, but it costs nothing to track
/// and later phases (or a future palette optimization) can read it.
pub(crate) struct RegistryViews {
    views: HashMap<PathBuf, (AppRegistry, u64)>,
    /// Global apps dir override. Defaults to the real `~/.plexi-<channel>/apps/`;
    /// tests substitute a tempdir so they never touch the user's real profile
    /// (mirrors `AppRegistry::load_with_global`'s existing test seam).
    global_dir: PathBuf,
}

impl RegistryViews {
    pub(crate) fn new() -> Self {
        Self {
            views: HashMap::new(),
            global_dir: crate::app::registry::apps_dir(),
        }
    }

    /// Test-only constructor mirroring `AppRegistry::load_with_global` — lets
    /// tests stage a fake global apps dir instead of the real profile.
    #[cfg(test)]
    pub(crate) fn new_with_global(global_dir: &Path) -> Self {
        Self {
            views: HashMap::new(),
            global_dir: global_dir.to_path_buf(),
        }
    }

    /// Canonicalize for use as a cache key. Falls back to the given path
    /// unchanged when canonicalization fails (root not yet created, or a
    /// synthetic test path like `/tmp/perf-gate-...` that is never scanned) —
    /// a missing root simply produces an empty `AppRegistry`, same as
    /// `AppRegistry::load` on a nonexistent cwd today.
    fn canonical_root(root: &Path) -> PathBuf {
        root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
    }

    fn load_for(&self, root: &Path) -> AppRegistry {
        AppRegistry::load_with_global(root, &self.global_dir)
    }

    /// The view for `root`, loading it on first access. Cheap on every
    /// subsequent call for the same canonical root (`HashMap` lookup only —
    /// no rescan) until an explicit [`Self::rescan_root`] or an invalidation
    /// event forces a fresh scan.
    pub(crate) fn view_for_root(&mut self, root: &Path) -> &AppRegistry {
        let key = Self::canonical_root(root);
        if !self.views.contains_key(&key) {
            log::info!("registry_views: loading view for root={}", key.display());
            let registry = self.load_for(&key);
            self.views.insert(key.clone(), (registry, 0));
        }
        &self.views.get(&key).expect("just inserted").0
    }

    /// The view for whichever root `context_id` is anchored to. Falls back to
    /// the process cwd (logged at `warn`) when `context_id` names no context
    /// in `router` — this should only happen for a context that raced its own
    /// removal; it must never panic the host.
    pub(crate) fn view_for_context(
        &mut self,
        context_id: u64,
        router: &WorkspaceRouter,
    ) -> &AppRegistry {
        let root = router
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.root.clone())
            .unwrap_or_else(|| {
                log::warn!(
                    "registry_views: context_id={context_id} not found in router — \
                     falling back to process cwd"
                );
                std::env::current_dir()
                    .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
            });
        self.view_for_root(&root)
    }

    /// Force a fresh scan of `root` regardless of cache state — used for an
    /// explicit "rescan now" (a watched source changed, a root was just
    /// (re)assigned to a context, or a caller couldn't find an app id and
    /// wants to check disk for a just-installed one).
    pub(crate) fn rescan_root(&mut self, root: &Path) -> &AppRegistry {
        let key = Self::canonical_root(root);
        log::info!("registry_views: rescanning root={}", key.display());
        let generation = self.views.get(&key).map_or(0, |(_, gen)| gen + 1);
        let registry = self.load_for(&key);
        self.views.insert(key.clone(), (registry, generation));
        &self.views.get(&key).expect("just inserted").0
    }

    /// Drop the cached view for `root` if no live context in `router` still
    /// anchors there. No-op if the root isn't cached or is still referenced.
    fn drop_if_orphaned(&mut self, root: &Path, router: &WorkspaceRouter) {
        let key = Self::canonical_root(root);
        if !self.views.contains_key(&key) {
            return;
        }
        let still_referenced = router.iter().any(|c| Self::canonical_root(&c.root) == key);
        if !still_referenced {
            log::info!(
                "registry_views: dropping orphaned root={} (no live context references it)",
                key.display()
            );
            self.views.remove(&key);
        }
    }

    /// React to a scope-lifecycle event. Called from
    /// `PlexiApp::emit_scope_invalidation` — the single choke point every
    /// scope-affecting change already flows through.
    ///
    /// `ContextRemoved` relies on `router` already having the removed
    /// context(s) dropped (see `pane_ops::workspace::delete_context`, which
    /// emits this event after `WorkspaceRouter::remove_at`, not before) —
    /// otherwise a doomed context would still "reference" its own root and
    /// the orphan check would never fire.
    pub(crate) fn invalidate(&mut self, ev: &ScopeInvalidation, router: &WorkspaceRouter) {
        match ev {
            ScopeInvalidation::ContextRootChanged {
                old_root, new_root, ..
            } => {
                self.rescan_root(new_root);
                self.drop_if_orphaned(old_root, router);
            }
            ScopeInvalidation::ContextRemoved { .. } => {
                // A context is gone; sweep every cached root and drop any
                // that no surviving context still anchors to. Cheap: this is
                // a HashMap of roots (one per open workspace), never one per
                // context.
                let orphaned: Vec<PathBuf> = self
                    .views
                    .keys()
                    .filter(|root| {
                        !router
                            .iter()
                            .any(|c| &Self::canonical_root(&c.root) == *root)
                    })
                    .cloned()
                    .collect();
                for root in orphaned {
                    log::info!(
                        "registry_views: dropping orphaned root={} (context removed)",
                        root.display()
                    );
                    self.views.remove(&root);
                }
            }
            ScopeInvalidation::SourceGenerationChanged { source_path } => {
                self.rescan_root(source_path);
            }
            ScopeInvalidation::PaneClosed { .. } => {
                // Not a registry-shaped event: a pane closing doesn't change
                // which apps are installed.
            }
        }
    }

    /// Every canonical root currently cached — what the watcher (or any other
    /// caller enumerating "what's live") should be watching / aware of.
    pub(crate) fn roots(&self) -> Vec<PathBuf> {
        self.views.keys().cloned().collect()
    }

    /// Test-only direct injection: install `registry` as the view for `root`,
    /// bypassing a disk scan entirely. Lets tests stage a hand-built
    /// `AppRegistry` (e.g. one app with a specific `[launch] on_launch`
    /// policy) for a specific context's root without writing manifest.toml
    /// fixtures to disk for every case.
    #[cfg(test)]
    pub(crate) fn set_view_for_test(&mut self, root: &Path, registry: AppRegistry) {
        let key = Self::canonical_root(root);
        self.views.insert(key, (registry, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::Context;

    fn ctx(id: u64, root: &Path) -> Context {
        Context {
            name: format!("ctx{id}").into(),
            root: root.to_path_buf(),
            description: None,
            context_id: id,
            parent_id: None,
            depth: 0,
            parked: false,
        }
    }

    fn write_app(dir: &Path, id: &str) {
        let app_dir = dir.join(id);
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(
            app_dir.join("manifest.toml"),
            format!(
                "schema_version = 1\n\n[app]\nid = \"{id}\"\ntype = \"app\"\nname = \"{id}\"\n\
                 version = \"0.0.1\"\nentry = \"run.sh\"\n"
            ),
        )
        .unwrap();
        std::fs::write(app_dir.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    }

    #[test]
    fn view_for_root_loads_once_and_caches() {
        let global = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut views = RegistryViews::new_with_global(global.path());
        assert!(views.view_for_root(root.path()).get("nope").is_none());
        // Install after the first load; without a rescan the cached view must
        // not pick it up (proves it's actually caching, not reloading).
        let channel_dir = crate::config::workspace_channel_dir();
        let apps = root.path().join(&channel_dir).join("apps");
        write_app(&apps, "late-app");
        assert!(
            views.view_for_root(root.path()).get("late-app").is_none(),
            "cached view must not silently re-scan on every access"
        );
        views.rescan_root(root.path());
        assert!(
            views.view_for_root(root.path()).get("late-app").is_some(),
            "explicit rescan must pick up the new app"
        );
    }

    #[test]
    fn two_contexts_sharing_a_root_share_one_view() {
        let global = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        write_app(&root.path().join(&channel_dir).join("apps"), "shared-app");

        let router = WorkspaceRouter::new(vec![ctx(1, root.path()), ctx(2, root.path())], 0);
        let mut views = RegistryViews::new_with_global(global.path());
        assert!(views
            .view_for_context(1, &router)
            .get("shared-app")
            .is_some());
        assert!(views
            .view_for_context(2, &router)
            .get("shared-app")
            .is_some());
        assert_eq!(views.roots().len(), 1, "one shared root ⇒ one cached view");
    }

    #[test]
    fn different_roots_never_cross_contaminate() {
        let global = tempfile::tempdir().unwrap();
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        write_app(&root_a.path().join(&channel_dir).join("apps"), "app-a");
        write_app(&root_b.path().join(&channel_dir).join("apps"), "app-b");

        let router = WorkspaceRouter::new(vec![ctx(1, root_a.path()), ctx(2, root_b.path())], 0);
        let mut views = RegistryViews::new_with_global(global.path());
        assert!(views.view_for_context(1, &router).get("app-a").is_some());
        assert!(views.view_for_context(1, &router).get("app-b").is_none());
        assert!(views.view_for_context(2, &router).get("app-b").is_some());
        assert!(views.view_for_context(2, &router).get("app-a").is_none());
    }

    /// Same app id installed in two different roots: each view must resolve
    /// its own root's copy, never the other's.
    #[test]
    fn duplicate_app_id_in_two_roots_resolves_per_view() {
        let global = tempfile::tempdir().unwrap();
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let dir_a = root_a.path().join(&channel_dir).join("apps").join("dup");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::write(
            dir_a.join("manifest.toml"),
            "schema_version = 1\n\n[app]\nid = \"dup\"\ntype = \"app\"\nname = \"A copy\"\n\
             version = \"0.0.1\"\nentry = \"run.sh\"\n",
        )
        .unwrap();
        std::fs::write(dir_a.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();

        let dir_b = root_b.path().join(&channel_dir).join("apps").join("dup");
        std::fs::create_dir_all(&dir_b).unwrap();
        std::fs::write(
            dir_b.join("manifest.toml"),
            "schema_version = 1\n\n[app]\nid = \"dup\"\ntype = \"app\"\nname = \"B copy\"\n\
             version = \"0.0.1\"\nentry = \"run.sh\"\n",
        )
        .unwrap();
        std::fs::write(dir_b.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();

        let router = WorkspaceRouter::new(vec![ctx(1, root_a.path()), ctx(2, root_b.path())], 0);
        let mut views = RegistryViews::new_with_global(global.path());
        assert_eq!(
            views
                .view_for_context(1, &router)
                .get("dup")
                .unwrap()
                .manifest
                .name,
            "A copy"
        );
        assert_eq!(
            views
                .view_for_context(2, &router)
                .get("dup")
                .unwrap()
                .manifest
                .name,
            "B copy"
        );
    }

    #[test]
    fn context_root_changed_rescans_new_root_and_drops_orphaned_old_root() {
        let global = tempfile::tempdir().unwrap();
        let old_root = tempfile::tempdir().unwrap();
        let new_root = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        write_app(&new_root.path().join(&channel_dir).join("apps"), "new-app");

        let mut views = RegistryViews::new_with_global(global.path());
        // Prime the old root's view before the "move".
        views.view_for_root(old_root.path());
        assert_eq!(views.roots().len(), 1);

        // Router already reflects the root change (matches how
        // `set_context_root` emits the event after mutating the router).
        let router = WorkspaceRouter::new(vec![ctx(1, new_root.path())], 0);
        views.invalidate(
            &ScopeInvalidation::ContextRootChanged {
                context_id: 1,
                old_root: old_root.path().to_path_buf(),
                new_root: new_root.path().to_path_buf(),
            },
            &router,
        );

        assert!(
            views
                .view_for_root(new_root.path())
                .get("new-app")
                .is_some(),
            "new root must be rescanned"
        );
        assert_eq!(
            views.roots().len(),
            1,
            "orphaned old root must be dropped, leaving only the new root cached"
        );
    }

    #[test]
    fn context_removed_drops_root_only_when_no_context_still_references_it() {
        let global = tempfile::tempdir().unwrap();
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();

        // Two contexts share root_a, one context owns root_b.
        let router_before = WorkspaceRouter::new(
            vec![
                ctx(1, root_a.path()),
                ctx(2, root_a.path()),
                ctx(3, root_b.path()),
            ],
            0,
        );
        let mut views = RegistryViews::new_with_global(global.path());
        views.view_for_context(1, &router_before);
        views.view_for_context(3, &router_before);
        assert_eq!(views.roots().len(), 2);

        // Context 2 (sharing root_a with context 1) is removed — root_a must
        // survive because context 1 still anchors there.
        let router_after_remove_2 =
            WorkspaceRouter::new(vec![ctx(1, root_a.path()), ctx(3, root_b.path())], 0);
        views.invalidate(
            &ScopeInvalidation::ContextRemoved { context_id: 2 },
            &router_after_remove_2,
        );
        assert_eq!(
            views.roots().len(),
            2,
            "root_a is still referenced by context 1 — must not be dropped"
        );

        // Now context 1 (the last reference to root_a) is also removed.
        let router_after_remove_1 = WorkspaceRouter::new(vec![ctx(3, root_b.path())], 0);
        views.invalidate(
            &ScopeInvalidation::ContextRemoved { context_id: 1 },
            &router_after_remove_1,
        );
        assert_eq!(
            views.roots().len(),
            1,
            "root_a must be dropped once no context references it"
        );
    }

    #[test]
    fn source_generation_changed_forces_a_rescan() {
        let global = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let mut views = RegistryViews::new_with_global(global.path());
        views.view_for_root(root.path());
        assert!(views.view_for_root(root.path()).get("fresh").is_none());

        write_app(&root.path().join(&channel_dir).join("apps"), "fresh");
        let router = WorkspaceRouter::new(vec![ctx(1, root.path())], 0);
        views.invalidate(
            &ScopeInvalidation::SourceGenerationChanged {
                source_path: root.path().to_path_buf(),
            },
            &router,
        );
        assert!(views.view_for_root(root.path()).get("fresh").is_some());
    }
}
