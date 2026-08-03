//! Host-owned scope/ownership model (stint 0724, Phase A).
//!
//! Pure new infrastructure: types, a reachability predicate, a layered-source
//! resolver, and lifecycle invalidation plumbing. Nothing outside this module
//! and the emit-site wiring in `PlexiApp`/`pane_ops` changes behavior in this
//! phase — later phases (state routing, connector tools, event bus,
//! notifications, deletion sweep) migrate real consumers onto these types.
//!
//! # Why a host-owned origin
//!
//! Every prior resource-scoping bug in this codebase traced back to the same
//! root cause: some caller derived "which context/pane is this for" from
//! ambient state (`active_window`, `router.active()`, a client-supplied id) at
//! the moment of use, rather than from an identity the host established once,
//! at the trust boundary. `ScopeOrigin` is that identity. It is constructed
//! only by the host model (`PlexiApp::origin_for_pane`,
//! `PlexiApp::origin_for_socket_peer`) from host-observed facts — the pane
//! table, the window's `context_id`, the router's `Context.root` — never from
//! a request payload, cwd, or env var. Socket-peer resolution reuses stint
//! 0636's `resolve_socket_peer_pane`, which already establishes trustworthy
//! sender identity from kernel-captured ancestry, not the wire.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Host-established identity of the acting pane at the dispatch boundary.
///
/// Constructed ONLY by the host model or from socket-peer resolution (stint
/// 0636's `resolve_socket_peer_pane`) — never from client payloads, active
/// focus, `active_window`, `router.active()`, pane cwd, or ambient env vars.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeOrigin {
    pub context_id: u64,
    pub context_root: PathBuf,
    pub window_id: u64,
    pub pane_id: u64,
    pub app_id: Option<String>,
}

/// The scope a resource (app instance, event stream, notification, layered
/// config entry, ...) is owned at. Distinct from `crate::host::state_scope::StateScope`,
/// which is app-declared addressing for on-disk state files; `Scope` is the
/// host's ownership/visibility model for in-memory and wire resources.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Visible everywhere, in every context.
    Global,
    /// Visible only within the owning context.
    Context { context_id: u64 },
    /// Visible only within the owning context — never cross-context even if
    /// another context happens to have a window with the same `window_id`.
    Window { window_id: u64, context_id: u64 },
    /// Visible only within the owning context — never cross-context even if
    /// another context happens to have a pane with the same `pane_id`.
    Pane { pane_id: u64, context_id: u64 },
    /// Visible only within the owning context — never cross-context even if
    /// another context happens to have a pane/app_id pair that collides.
    AppInstance {
        pane_id: u64,
        app_id: String,
        context_id: u64,
    },
}

/// Minimal in-memory attributable grant carrier. NOT persisted — persistence
/// for cross-context grants is a different, out-of-scope stint. This exists
/// only so `evaluate_reach` has a grant path to evaluate against in Phase A
/// and the phases that migrate consumers onto it.
#[derive(Debug, Clone)]
pub struct CrossContextGrant {
    pub granting_context_id: u64,
    pub grantee_context_id: u64,
    pub reason: String,
}

/// Outcome of `evaluate_reach`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Owner scope is inherently visible to this viewer (global, or the
    /// viewer's own context).
    Allowed,
    /// Owner scope is a different context, but a matching `CrossContextGrant`
    /// makes it visible anyway.
    AllowedByGrant,
    /// Owner scope is a different context and no matching grant exists.
    Rejected,
}

/// The one central reachability predicate every later phase's connector
/// tools, event bus, notifications, and deletion sweep call to decide whether
/// a viewer in `viewer_context_id` may see a resource owned at `owner`.
///
/// Rule: context-owned is visible only in the owning context; global is
/// visible everywhere; anything else (`Window`/`Pane`/`AppInstance`) needs an
/// explicit grant to cross a context boundary — and even with a grant,
/// reachability is decided purely by `context_id`. A `Window`/`Pane` scope is
/// never reachable from another context merely because the numeric
/// `window_id`/`pane_id` happens to match one there; ids are only unique
/// within their owning context.
///
/// Every `Rejected` outcome is logged at `info!` so a denied cross-context
/// access is observable without ever logging the resource's payload — only
/// ids and the scope's shape (`{:?}` on `Scope`/`Reach` never includes
/// secrets, notification bodies, or event payloads by construction, since
/// `Scope` carries no such fields).
pub fn evaluate_reach(
    owner: &Scope,
    viewer_context_id: u64,
    grant: Option<&CrossContextGrant>,
) -> Reach {
    let owner_context_id = match owner {
        Scope::Global => return Reach::Allowed,
        Scope::Context { context_id }
        | Scope::Window { context_id, .. }
        | Scope::Pane { context_id, .. }
        | Scope::AppInstance { context_id, .. } => *context_id,
    };

    if owner_context_id == viewer_context_id {
        return Reach::Allowed;
    }

    if let Some(grant) = grant {
        if grant.granting_context_id == owner_context_id
            && grant.grantee_context_id == viewer_context_id
        {
            return Reach::AllowedByGrant;
        }
    }

    log::info!(
        target: "plexi::scope",
        "reach rejected: owner_context={owner_context_id} viewer_context={viewer_context_id} scope={owner:?}"
    );
    Reach::Rejected
}

/// A resolved layered value plus the provenance a caller needs to explain
/// where it came from: which scope it's owned at, which file it was read
/// from (when the layer has one), a stable identity fingerprint for the
/// (path, key) pair, and the generation (layer index) it resolved from.
#[derive(Debug, Clone)]
pub struct ResolvedSource<T> {
    pub value: T,
    pub scope: Scope,
    pub source_path: Option<PathBuf>,
    pub fingerprint: u64,
    pub generation: u64,
}

/// One layer of a layered-source resolution — e.g. a global config file and a
/// context-local override, both declaring entries keyed by the same
/// namespace. `resolve_layers` shadows earlier layers with later ones,
/// whole-entry.
pub struct SourceLayer<T> {
    pub scope: Scope,
    pub path: PathBuf,
    pub entries: BTreeMap<String, T>,
}

/// Deterministic identity fingerprint for a (source path, key) pair. Not a
/// content hash — Phase A has no consumer that needs change detection on
/// value contents, only a stable identifier tying a resolved entry back to
/// the layer + key it came from. `DefaultHasher` is documented to start from
/// a fixed, non-randomized state, so this is deterministic across calls and
/// across process runs.
fn fingerprint_for(path: &std::path::Path, key: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    key.hash(&mut hasher);
    hasher.finish()
}

/// Resolve a stack of layers into one map keyed by entry name. Later layers
/// (higher index in `layers`) shadow earlier ones WHOLE-ENTRY: when two
/// layers declare the same key, the later layer's entire value replaces the
/// earlier one — individual fields of `T` are never merged across layers.
/// Deterministic for a given `layers` slice regardless of the entries' own
/// (BTreeMap) iteration — key order inside a layer never affects the result,
/// only the layer order the caller passes in does.
///
/// Every shadowing event (a later layer overwriting an earlier layer's entry
/// for the same key) is logged at `info!` with both source paths and the key
/// — never the value, which may be app-controlled and is not the host's to
/// name in a shared log line.
pub fn resolve_layers<T: Clone>(layers: &[SourceLayer<T>]) -> BTreeMap<String, ResolvedSource<T>> {
    let mut resolved: BTreeMap<String, ResolvedSource<T>> = BTreeMap::new();
    for (generation, layer) in layers.iter().enumerate() {
        let generation = generation as u64;
        for (key, value) in &layer.entries {
            if let Some(previous) = resolved.get(key) {
                log::info!(
                    target: "plexi::scope",
                    "layer shadow: key={key} previous_source={:?} new_source={:?}",
                    previous.source_path,
                    layer.path
                );
            }
            resolved.insert(
                key.clone(),
                ResolvedSource {
                    value: value.clone(),
                    scope: layer.scope.clone(),
                    source_path: Some(layer.path.clone()),
                    fingerprint: fingerprint_for(&layer.path, key),
                    generation,
                },
            );
        }
    }
    resolved
}

/// A lifecycle event that makes some previously-resolved scope/origin/source
/// stale. Phase A has no consumers — `PlexiApp::emit_scope_invalidation` only
/// logs it. Later phases (state routing, connector tools, event bus,
/// notifications, deletion sweep) add real invalidation handling behind this
/// same enum as they migrate onto the scope model, so every future consumer
/// reacts to one shared vocabulary of "what changed" instead of re-deriving
/// it from ad hoc call sites.
#[derive(Debug, Clone)]
pub enum ScopeInvalidation {
    /// `pane_ops::workspace::set_context_root` moved a context's anchor.
    ContextRootChanged {
        context_id: u64,
        old_root: PathBuf,
        new_root: PathBuf,
    },
    /// `pane_ops::workspace::delete_context` removed a context (emitted once
    /// per deleted context in a cascading delete, including descendants).
    ContextRemoved { context_id: u64 },
    /// `pane_ops::layout::close_pane_by_id` closed a pane.
    PaneClosed { pane_id: u64, context_id: u64 },
    /// An app package was installed or replaced on disk. `root` is the
    /// installed app's directory when known.
    PackageReplaced {
        app_id: String,
        root: Option<PathBuf>,
    },
    /// A filesystem-watched source (e.g. the app registry) reported a change
    /// generation the host has not yet resolved against.
    SourceGenerationChanged { source_path: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── evaluate_reach: the scope compatibility matrix ──────────────────────

    fn grant(granting: u64, grantee: u64) -> CrossContextGrant {
        CrossContextGrant {
            granting_context_id: granting,
            grantee_context_id: grantee,
            reason: "test grant".to_string(),
        }
    }

    #[test]
    fn global_is_always_allowed() {
        assert_eq!(evaluate_reach(&Scope::Global, 1, None), Reach::Allowed);
        assert_eq!(evaluate_reach(&Scope::Global, 2, None), Reach::Allowed);
        // Even a grant for an unrelated pair doesn't change global's answer.
        let g = grant(9, 9);
        assert_eq!(evaluate_reach(&Scope::Global, 42, Some(&g)), Reach::Allowed);
    }

    #[test]
    fn context_scope_same_context_is_allowed() {
        let owner = Scope::Context { context_id: 5 };
        assert_eq!(evaluate_reach(&owner, 5, None), Reach::Allowed);
    }

    #[test]
    fn context_scope_different_context_without_grant_is_rejected() {
        let owner = Scope::Context { context_id: 5 };
        assert_eq!(evaluate_reach(&owner, 6, None), Reach::Rejected);
    }

    #[test]
    fn context_scope_different_context_with_matching_grant_is_allowed_by_grant() {
        let owner = Scope::Context { context_id: 5 };
        let g = grant(5, 6);
        assert_eq!(evaluate_reach(&owner, 6, Some(&g)), Reach::AllowedByGrant);
    }

    #[test]
    fn context_scope_different_context_with_mismatched_grant_is_rejected() {
        let owner = Scope::Context { context_id: 5 };
        // Grant exists but names a different grantor/grantee pair.
        let g = grant(5, 999);
        assert_eq!(evaluate_reach(&owner, 6, Some(&g)), Reach::Rejected);
        let g2 = grant(4, 6);
        assert_eq!(evaluate_reach(&owner, 6, Some(&g2)), Reach::Rejected);
    }

    #[test]
    fn window_scope_same_context_is_allowed() {
        let owner = Scope::Window {
            window_id: 100,
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 5, None), Reach::Allowed);
    }

    #[test]
    fn window_scope_matching_window_id_in_another_context_is_still_rejected() {
        // window_id collision across contexts must never grant reach — ids
        // are only unique within their owning context.
        let owner = Scope::Window {
            window_id: 100,
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 6, None), Reach::Rejected);
    }

    #[test]
    fn window_scope_different_context_with_grant_is_allowed_by_grant() {
        let owner = Scope::Window {
            window_id: 100,
            context_id: 5,
        };
        let g = grant(5, 6);
        assert_eq!(evaluate_reach(&owner, 6, Some(&g)), Reach::AllowedByGrant);
    }

    #[test]
    fn pane_scope_same_context_is_allowed() {
        let owner = Scope::Pane {
            pane_id: 200,
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 5, None), Reach::Allowed);
    }

    #[test]
    fn pane_scope_matching_pane_id_in_another_context_is_still_rejected() {
        let owner = Scope::Pane {
            pane_id: 200,
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 6, None), Reach::Rejected);
    }

    #[test]
    fn pane_scope_different_context_with_grant_is_allowed_by_grant() {
        let owner = Scope::Pane {
            pane_id: 200,
            context_id: 5,
        };
        let g = grant(5, 6);
        assert_eq!(evaluate_reach(&owner, 6, Some(&g)), Reach::AllowedByGrant);
    }

    #[test]
    fn app_instance_scope_same_context_is_allowed() {
        let owner = Scope::AppInstance {
            pane_id: 200,
            app_id: "acme.todo".to_string(),
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 5, None), Reach::Allowed);
    }

    #[test]
    fn app_instance_scope_matching_ids_in_another_context_is_still_rejected() {
        let owner = Scope::AppInstance {
            pane_id: 200,
            app_id: "acme.todo".to_string(),
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 6, None), Reach::Rejected);
    }

    #[test]
    fn app_instance_scope_different_context_with_grant_is_allowed_by_grant() {
        let owner = Scope::AppInstance {
            pane_id: 200,
            app_id: "acme.todo".to_string(),
            context_id: 5,
        };
        let g = grant(5, 6);
        assert_eq!(evaluate_reach(&owner, 6, Some(&g)), Reach::AllowedByGrant);
    }

    // ── resolve_layers ───────────────────────────────────────────────────────

    fn layer(scope: Scope, path: &str, entries: &[(&str, &str)]) -> SourceLayer<String> {
        SourceLayer {
            scope,
            path: PathBuf::from(path),
            entries: entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn later_layer_shadows_earlier_layer_whole_entry() {
        let layers = vec![
            layer(
                Scope::Global,
                "/global/config.toml",
                &[("greeting", "hello-global"), ("only_global", "g")],
            ),
            layer(
                Scope::Context { context_id: 1 },
                "/ctx/config.toml",
                &[("greeting", "hello-context")],
            ),
        ];
        let resolved = resolve_layers(&layers);

        // The later (context) layer's whole entry wins for the shared key.
        let greeting = &resolved["greeting"];
        assert_eq!(greeting.value, "hello-context");
        assert_eq!(greeting.scope, Scope::Context { context_id: 1 });
        assert_eq!(
            greeting.source_path,
            Some(PathBuf::from("/ctx/config.toml"))
        );
        assert_eq!(greeting.generation, 1);

        // The key only the earlier layer declared survives untouched.
        let only_global = &resolved["only_global"];
        assert_eq!(only_global.value, "g");
        assert_eq!(only_global.scope, Scope::Global);
        assert_eq!(only_global.generation, 0);
    }

    #[test]
    fn resolve_layers_is_deterministic_for_the_same_slice_order() {
        let make_layers = || {
            vec![
                layer(
                    Scope::Global,
                    "/global/config.toml",
                    &[("a", "1"), ("b", "2")],
                ),
                layer(
                    Scope::Context { context_id: 7 },
                    "/ctx/config.toml",
                    &[("b", "override"), ("c", "3")],
                ),
            ]
        };

        let first = resolve_layers(&make_layers());
        let second = resolve_layers(&make_layers());

        assert_eq!(
            first.keys().collect::<Vec<_>>(),
            second.keys().collect::<Vec<_>>()
        );
        for key in first.keys() {
            assert_eq!(first[key].value, second[key].value);
            assert_eq!(first[key].scope, second[key].scope);
            assert_eq!(first[key].source_path, second[key].source_path);
            assert_eq!(first[key].fingerprint, second[key].fingerprint);
            assert_eq!(first[key].generation, second[key].generation);
        }
    }

    #[test]
    fn resolve_layers_attributes_each_entry_to_its_originating_layer() {
        let layers = vec![
            layer(Scope::Global, "/global/config.toml", &[("x", "global-x")]),
            layer(
                Scope::Context { context_id: 3 },
                "/ctx-a/config.toml",
                &[("y", "ctx-a-y")],
            ),
            layer(
                Scope::Context { context_id: 4 },
                "/ctx-b/config.toml",
                &[("z", "ctx-b-z")],
            ),
        ];
        let resolved = resolve_layers(&layers);

        assert_eq!(resolved["x"].scope, Scope::Global);
        assert_eq!(
            resolved["x"].source_path,
            Some(PathBuf::from("/global/config.toml"))
        );
        assert_eq!(resolved["x"].generation, 0);

        assert_eq!(resolved["y"].scope, Scope::Context { context_id: 3 });
        assert_eq!(
            resolved["y"].source_path,
            Some(PathBuf::from("/ctx-a/config.toml"))
        );
        assert_eq!(resolved["y"].generation, 1);

        assert_eq!(resolved["z"].scope, Scope::Context { context_id: 4 });
        assert_eq!(
            resolved["z"].source_path,
            Some(PathBuf::from("/ctx-b/config.toml"))
        );
        assert_eq!(resolved["z"].generation, 2);
    }

    #[test]
    fn resolve_layers_never_merges_fields_of_a_shadowed_struct_entry() {
        // T here is a struct-like tuple encoded as a String "field_a|field_b"
        // to prove shadowing replaces the whole value, not just one field.
        let layers = vec![
            layer(Scope::Global, "/global.toml", &[("entry", "a1|b1")]),
            layer(
                Scope::Context { context_id: 1 },
                "/ctx.toml",
                &[("entry", "a2|MISSING")], // later layer only "declares" a2 conceptually
            ),
        ];
        let resolved = resolve_layers(&layers);
        // The later layer's value replaces the earlier one entirely — this
        // would fail if resolve_layers merged per-field instead of shadowing
        // the whole entry.
        assert_eq!(resolved["entry"].value, "a2|MISSING");
        assert_ne!(resolved["entry"].value, "a1|b1");
    }

    // ── ScopeInvalidation / emit_scope_invalidation smoke test ──────────────

    /// `PlexiApp::set_context_root` must fire `emit_scope_invalidation` with a
    /// `ContextRootChanged` event. Phase A has no real consumer, so the
    /// observable proof is the test-only counter `emit_scope_invalidation`
    /// increments under `#[cfg(test)]` — a HostHarness-based smoke test per
    /// TESTING.md (host logic → HostHarness, not a TOML scene, since nothing
    /// here is pixel-observable).
    #[test]
    fn set_context_root_fires_scope_invalidation() {
        let mut harness = crate::testing::HostHarness::new();
        let before =
            crate::app::SCOPE_INVALIDATION_COUNT_FOR_TEST.load(std::sync::atomic::Ordering::SeqCst);

        let new_root = tempfile::tempdir().expect("new root tempdir");
        harness
            .app
            .set_context_root(new_root.path().to_path_buf(), None);

        let after =
            crate::app::SCOPE_INVALIDATION_COUNT_FOR_TEST.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            after > before,
            "set_context_root must fire emit_scope_invalidation (before={before}, after={after})"
        );
    }
}
