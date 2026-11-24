//! Launch plans (stint 1632). A terminal `spawn_pane` or an `agent call` is
//! admitted once into a [`LaunchPlan`]: who asked, what authority the new pane
//! gets, where it goes, and in which directory. [`PlexiApp::apply_launch`]
//! commits a plan. It re-checks the destination and the authority, refuses
//! when either changed, and returns the pane it actually created.
//!
//! A destination names what the caller selected (a window inside a context,
//! an anchor pane, an empty tree) at the window's [`TreeHistory`] generation,
//! never a bare `TileId` or a window index: tile ids restart when a tree is
//! rebuilt and indices shift. Pane, window and context ids are allocated
//! monotonically for the host's lifetime, and a window leaves its context
//! only when that context is dissolved, so `(window_id, context_id)` cannot be
//! satisfied by a moved window.
//!
//! App and path `spawn_pane` launches are not planned: they hold no resource
//! between admission and commit.
//!
//! [`TreeHistory`]: crate::host::context::TreeHistory

use std::path::PathBuf;

use egui_tiles::{Tile, TileId};
use serde_json::{json, Value};

use super::agent_tools::{AgentCallRequest, AgentCaller};
use super::launch_spec::PaneLaunchSpec;
use super::PlexiApp;
use crate::host::worktree::ManagedWorktree;
use crate::pane_ops::create::{AgentOrigin, SubcontextTarget};
use crate::spatial::tiling::PaneId;

/// A window as the caller selected it, at its tree generation then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowMembership {
    pub(crate) window_id: u64,
    pub(crate) context_id: u64,
    pub(crate) generation: u64,
}

/// The node a split lands beside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SplitAnchor {
    /// A pane that has not left its window since admission.
    Pane(PaneId),
    /// A container node, valid only while its window's topology and
    /// membership are exactly as admitted: no rebuild, departure, occupant
    /// change or structural edit anywhere in the window. Conservative for
    /// slice 1: any change to the window refuses a container target.
    Node { tile: TileId, topology: u64 },
}

/// The tab group a tab joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabAnchor {
    Pane(PaneId),
    EmptyRoot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalDestination {
    Split {
        at: WindowMembership,
        anchor: SplitAnchor,
        vertical: bool,
        new_pane_first: bool,
        keep_focus: bool,
    },
    Tab {
        at: WindowMembership,
        anchor: TabAnchor,
        keep_focus: bool,
    },
    SeedEmptyRoot {
        at: WindowMembership,
    },
    NewWindow {
        context_id: u64,
        grid_y: u32,
        /// The window to return to when the caller asked for no focus.
        restore_active: Option<u64>,
    },
}

impl TerminalDestination {
    /// The context the destination is in.
    pub(crate) fn context_id(&self) -> u64 {
        match self {
            Self::Split { at, .. } | Self::Tab { at, .. } | Self::SeedEmptyRoot { at } => {
                at.context_id
            }
            Self::NewWindow { context_id, .. } => *context_id,
        }
    }
}

/// A launch's submitter as the host established it: the one answer every
/// launch route (`spawn_pane` of any target, `agent spawn`, `agent open`,
/// `context new`) gets from [`PlexiApp::resolve_launch_source`] (stint 1632).
#[derive(Clone, Debug)]
pub(crate) struct LaunchCaller {
    /// The verified pane that asked (or the pane in-process host code
    /// names). Only a verified pane supplies parent authority; a claimed
    /// pane id from the wire never does.
    pub(crate) caller_pane: Option<PaneId>,
    /// The source the new pane's grant is admitted from.
    pub(crate) source: crate::broker::pane_authority::GrantSource,
    pub(crate) spawned_by: String,
}

/// The caller incarnation a plan was admitted for. Commit refuses unless
/// the same incarnation still holds the same grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdmittedIncarnation {
    pub(crate) pane_id: PaneId,
    pub(crate) incarnation: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct TerminalLaunchPlan {
    /// The new pane's grant, admitted for its reserved id. It fixes the
    /// pane's credential scope; commit refuses if the destination's scope no
    /// longer matches it.
    pub(crate) grant: crate::broker::pane_authority::PaneAuthority,
    /// The destination context's scope revision at admission.
    pub(crate) scope_revision: u64,
    pub(crate) caller: Option<AdmittedIncarnation>,
    pub(crate) destination: TerminalDestination,
    /// The working directory, resolved once at admission from the explicit
    /// cwd or the destination's default. A prepared checkout replaces it.
    pub(crate) cwd: Option<PathBuf>,
    /// `(branch, repository anchor)` when the pane runs in a new checkout.
    pub(crate) worktree: Option<(String, PathBuf)>,
    pub(crate) initial_cmd: Option<String>,
    pub(crate) ephemeral: bool,
    pub(crate) name: Option<String>,
    pub(crate) result_schema: Option<Value>,
}

/// The caller of an `agent call`, kept so commit can authenticate it again.
#[derive(Clone, Debug)]
pub(crate) struct AdmittedAgentCaller {
    origin: crate::broker::caller::RequestOrigin,
    credential: Option<String>,
}

/// Everything that authorizes and shapes an agent launch: the caller's grant,
/// the role's resolved definition as it reaches the worker (command, harness,
/// lifetime, result schema) and the ceiling. Admission records it; commit
/// derives it again from the same inputs and refuses on any difference.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentCallAuthority {
    pub(crate) parent: Option<PaneId>,
    /// Context root the caller's `can_call`/`can_handoff` grant was read from.
    pub(crate) grant_root: PathBuf,
    /// The role id the definition resolved to.
    pub(crate) role: String,
    pub(crate) harness: String,
    pub(crate) command: String,
    pub(crate) ephemeral: bool,
    pub(crate) result_schema: Option<Value>,
    /// The role as admitted for the worker's grant.
    pub(crate) worker_role: crate::broker::pane_authority::AdmittedRole,
    /// The caller's grant, which bounds the worker's.
    pub(crate) caller_grant: Option<crate::broker::pane_authority::PaneAuthority>,
}

/// The existing window an agent worker lands in, and the node it splits
/// beside, as the caller selected them at admission (stint 1632). `anchor`
/// is `None` for the empty tree the worker seeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentAnchor {
    pub(crate) at: WindowMembership,
    pub(crate) anchor: Option<SplitAnchor>,
}

#[derive(Clone, Debug)]
pub(crate) enum AdmittedSubcontext {
    Existing {
        name: String,
        context_id: u64,
    },
    Create {
        name: String,
        parent_context_id: u64,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct AgentCallPlan {
    caller: AdmittedAgentCaller,
    requested_role: String,
    pub(crate) handoff: bool,
    pub(crate) authority: AgentCallAuthority,
    /// The task as the worker receives it (with the restore packet on handoff).
    task_text: String,
    pub(crate) task: String,
    pub(crate) lookup_root: PathBuf,
    /// The pane the worker hangs under (the caller, or its parent on handoff).
    pub(crate) child_parent: Option<PaneId>,
    pub(crate) subcontext: Option<AdmittedSubcontext>,
    /// The caller's window, or the window active when an outside caller asked.
    pub(crate) origin: WindowMembership,
    /// Where in an existing window the worker lands: an existing
    /// sub-context's window, or the origin window of a worker with no
    /// parent. `None` when the worker creates its sub-context.
    pub(crate) destination: Option<AgentAnchor>,
    /// The credential scope of an existing destination, bound at admission.
    /// Commit refuses if that context's root moved since (stint 1632).
    pub(crate) destination_scope: Option<(crate::broker::pane_authority::CredentialScope, u64)>,
    pub(crate) worktree: Option<String>,
    /// A handoff successor's id, reserved from the monotonic allocator at
    /// admission and carried through construction.
    pub(crate) successor: Option<PaneId>,
}

/// An admitted launch, consumed by [`PlexiApp::apply_launch`].
#[derive(Clone, Debug)]
pub(crate) enum LaunchPlan {
    Terminal(Box<TerminalLaunchPlan>),
    Agent(Box<AgentCallPlan>),
}

impl LaunchPlan {
    /// `(branch, repository anchor)` of the checkout this plan runs in.
    fn checkout(&self) -> Option<(&str, &std::path::Path)> {
        match self {
            Self::Terminal(plan) => plan
                .worktree
                .as_ref()
                .map(|(branch, anchor)| (branch.as_str(), anchor.as_path())),
            Self::Agent(plan) => plan
                .worktree
                .as_deref()
                .map(|branch| (branch, plan.lookup_root.as_path())),
        }
    }
}

/// A launch that did not install a pane, and what became of the checkout
/// prepared for it.
#[derive(Debug)]
pub(crate) struct LaunchRefused {
    pub(crate) error: String,
    pub(crate) worktree: Option<crate::host::worktree::WorktreeCloseReport>,
}

impl std::fmt::Display for LaunchRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error)?;
        if let Some(report) = &self.worktree {
            write!(f, "; worktree {} {}", report.action, report.path.display())?;
            if let Some(reason) = &report.reason {
                write!(f, " ({reason})")?;
            }
        }
        Ok(())
    }
}

/// Where a revalidated terminal plan installs, resolved in the same turn as
/// construction: window indices and tiles never outlive it.
enum TerminalPlacement {
    Split(crate::pane_ops::create::SplitPlacement),
    Tab {
        win_idx: usize,
        anchor: Option<PaneId>,
        keep_focus: bool,
    },
    Root {
        win_idx: usize,
    },
    NewWindow {
        grid_x: u32,
        grid_y: u32,
        context_id: u64,
        restore_active: Option<u64>,
    },
}

fn refused(what: String) -> String {
    format!("launch refused: {what}")
}

/// The completion a handed-off caller publishes, naming its successor.
pub(crate) fn handoff_completion(role: &str, task: &str, successor_pane_id: PaneId) -> String {
    json!({
        "status": "handed_off",
        "role": role,
        "task": task,
        "successor_pane_id": successor_pane_id,
    })
    .to_string()
}

impl PlexiApp {
    /// Commit an admitted plan: the one owner from admission through
    /// installation for every terminal and agent launch. It prepares the
    /// plan's checkout, revalidates the caller, authority and destination,
    /// constructs the pane under its admitted authority, installs it, and
    /// only then hands the checkout to the pane. A refusal at any step after
    /// preparation reclaims or retains the checkout and reports which.
    pub(crate) fn apply_launch(&mut self, plan: LaunchPlan) -> Result<PaneId, LaunchRefused> {
        let prepared = match plan.checkout() {
            Some((branch, anchor)) => {
                match crate::host::worktree::create_worktree(anchor, branch) {
                    Ok(wt) => Some(wt),
                    Err(error) => {
                        log::info!("launch_plan: checkout '{branch}' not prepared: {error}");
                        return Err(LaunchRefused {
                            error,
                            worktree: None,
                        });
                    }
                }
            }
            None => None,
        };
        #[cfg(test)]
        probes::prepared(self, prepared.as_ref());
        let outcome = match &plan {
            LaunchPlan::Terminal(plan) => self.apply_terminal_launch(plan, prepared.as_ref()),
            LaunchPlan::Agent(plan) => self.apply_agent_call(plan, prepared.as_ref()),
        };
        match outcome {
            Ok(pane_id) => {
                if let Some(wt) = prepared {
                    log::info!(
                        "pane_worktree: registered pane_id={pane_id} branch='{}' path='{}'",
                        wt.branch,
                        wt.path.display()
                    );
                    self.pane_worktrees.insert(pane_id, wt);
                }
                log::info!("launch_plan: applied pane_id={pane_id}");
                Ok(pane_id)
            }
            Err(error) => {
                let worktree = prepared.map(|wt| {
                    let report = crate::host::worktree::reclaim_or_retain_worktree(&wt);
                    log::info!("pane_worktree: launch refused; worktree disposition {report:?}");
                    report
                });
                let refused = LaunchRefused { error, worktree };
                log::info!("launch_plan: refused at commit: {refused}");
                Err(refused)
            }
        }
    }

    fn window_membership(&self, win_idx: usize) -> WindowMembership {
        let win = &self.windows[win_idx];
        WindowMembership {
            window_id: win.window_id,
            context_id: win.context_id,
            generation: win.tree_history.generation(),
        }
    }

    fn split_anchor(&self, win_idx: usize, tile: TileId) -> SplitAnchor {
        let win = &self.windows[win_idx];
        match win.tree.tiles.get(tile) {
            Some(Tile::Pane(pane_id)) => SplitAnchor::Pane(*pane_id),
            _ => SplitAnchor::Node {
                tile,
                topology: tree_topology(&win.tree),
            },
        }
    }

    /// The window index of an admitted membership, or why it no longer holds.
    fn locate_window(&self, at: WindowMembership) -> Result<usize, String> {
        let win_idx = self
            .windows
            .iter()
            .position(|w| w.window_id == at.window_id)
            .ok_or_else(|| refused(format!("window {} closed", at.window_id)))?;
        let now = self.windows[win_idx].context_id;
        if now != at.context_id {
            return Err(refused(format!(
                "window {} moved from context {} to context {now}",
                at.window_id, at.context_id
            )));
        }
        Ok(win_idx)
    }

    fn locate_split_anchor(
        &self,
        win_idx: usize,
        at: WindowMembership,
        anchor: &SplitAnchor,
    ) -> Result<TileId, String> {
        let win = &self.windows[win_idx];
        let history = &win.tree_history;
        if history.rebuilt_since(at.generation) {
            return Err(refused(format!(
                "anchor pane's window {} was rebuilt after admission",
                win.window_id
            )));
        }
        match anchor {
            SplitAnchor::Pane(pane_id) => {
                let tile = win
                    .panes
                    .contains_key(pane_id)
                    .then(|| win.tree.tiles.find_pane(pane_id))
                    .flatten();
                match tile {
                    Some(tile) if !history.departed_since(*pane_id, at.generation) => Ok(tile),
                    _ => Err(refused(format!(
                        "anchor pane {pane_id} left window {} after admission",
                        win.window_id
                    ))),
                }
            }
            SplitAnchor::Node { tile, topology } => {
                let same = matches!(win.tree.tiles.get(*tile), Some(Tile::Container(_)))
                    && history.generation() == at.generation
                    && tree_topology(&win.tree) == *topology;
                same.then_some(*tile).ok_or_else(|| {
                    refused(format!(
                        "node {tile:?} of window {} changed after admission",
                        win.window_id
                    ))
                })
            }
        }
    }

    /// An empty tree is the one admitted only while nothing joined and left
    /// it and it was not rebuilt: its generation is unchanged.
    fn require_empty_tree(&self, win_idx: usize, at: WindowMembership) -> Result<(), String> {
        let win = &self.windows[win_idx];
        let now = win.tree_history.generation();
        if win.tree.root.is_some() || now != at.generation {
            return Err(refused(format!(
                "window {} is no longer the empty tree the launch was admitted for (generation {} -> {now})",
                win.window_id, at.generation
            )));
        }
        Ok(())
    }

    /// The window holding an explicit origin pane. A named origin that is gone
    /// is an error, never a fall back to focus.
    fn explicit_origin(
        &self,
        from_pane_id: Option<PaneId>,
    ) -> Result<Option<(usize, TileId)>, String> {
        from_pane_id
            .map(|id| {
                self.find_pane_in_any_window(id)
                    .ok_or_else(|| format!("origin pane {id} not found"))
            })
            .transpose()
    }

    /// Resolve who submitted a launch from its origin (stint 1285). A socket
    /// caller in a pane bounds the new pane by that pane's grant; an outside
    /// process is an outside process; an unidentified caller is refused.
    /// Host-owned input acts on the person's own gesture: the pane it names
    /// is placement and tree parent, never a grant to inherit.
    pub(crate) fn resolve_launch_source(
        &self,
        origin: &crate::broker::caller::RequestOrigin,
        claimed: Option<PaneId>,
    ) -> Result<LaunchCaller, String> {
        use crate::broker::caller::{RequestOrigin, SocketCaller};
        use crate::broker::pane_authority::GrantSource;
        let (caller_pane, grant_source) = match origin {
            RequestOrigin::Socket(SocketCaller::Unidentified(reason)) => {
                return Err(refused(format!(
                    "the launch's caller could not be identified: {reason}"
                )))
            }
            RequestOrigin::Socket(SocketCaller::Pane(pane)) => (
                Some(pane.terminal_pane),
                self.caller_grant_source(pane.terminal_pane),
            ),
            RequestOrigin::Socket(SocketCaller::External(_)) => {
                (None, GrantSource::OutsideProcess)
            }
            RequestOrigin::HostUi => (claimed, GrantSource::DesktopInput),
            RequestOrigin::HostSystem => (claimed, GrantSource::HostSystem),
            RequestOrigin::SpawnQueue => (None, GrantSource::SpawnQueue),
            RequestOrigin::PluginGuest(instance) => (
                None,
                GrantSource::PluginInstance {
                    pane_id: instance.instance_id.unwrap_or_default(),
                },
            ),
        };
        let spawned_by = self.spawned_by_for(&grant_source);
        log::info!(
            "launch_plan: resolved origin={} claimed={claimed:?} -> caller={caller_pane:?} spawned_by={spawned_by}",
            origin.class()
        );
        Ok(LaunchCaller {
            caller_pane,
            source: grant_source,
            spawned_by,
        })
    }

    /// The caller incarnation a plan binds: a verified pane caller only.
    pub(crate) fn admitted_incarnation(
        &self,
        source: &crate::broker::pane_authority::GrantSource,
    ) -> Option<AdmittedIncarnation> {
        match source {
            crate::broker::pane_authority::GrantSource::VerifiedPane(caller) => self
                .pane_authority(caller.pane_id)
                .map(|live| AdmittedIncarnation {
                    pane_id: caller.pane_id,
                    incarnation: live.incarnation(),
                }),
            _ => None,
        }
    }

    /// Admit a terminal `spawn_pane`: the new pane's grant for a reserved
    /// id, where it goes and its directory. Names, focus and defaults are
    /// resolved here, once.
    pub(crate) fn admit_terminal_launch(
        &mut self,
        spec: &PaneLaunchSpec,
    ) -> Result<TerminalLaunchPlan, String> {
        let layout = spec.layout.as_deref().unwrap_or("split_h");
        let active = self.active_window;
        // The grant comes from the host-stamped source. A wire
        // `from_pane_id` is a placement origin only (stint 1632).
        let LaunchCaller {
            source, spawned_by, ..
        } = self.resolve_launch_source(&spec.origin, spec.from_pane_id)?;
        let caller = self.admitted_incarnation(&source);
        let vertical = matches!(layout, "split_h" | "split_right" | "split_left");
        let new_pane_first = matches!(layout, "split_above" | "split_left");
        let origin = self.explicit_origin(spec.from_pane_id)?;
        let context_root = |win: usize| {
            self.pane_context_env_for_context(self.windows[win].context_id)
                .root
        };
        let home = || dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let (destination, default_cwd) =
            if let Some(ctx_name) = spec.context_name.as_deref().filter(|c| !c.is_empty()) {
                // Named-context targeting (stint 0574): a hard target that never
                // touches active_window or focus state.
                if layout == "new_window" || layout == "tab" {
                    return Err(format!(
                        "context_name does not support layout '{layout}' — use a split layout"
                    ));
                }
                let context_id = self
                    .router
                    .iter()
                    .find(|c| c.name == ctx_name)
                    .map(|c| c.context_id)
                    .ok_or_else(|| format!("context '{ctx_name}' does not exist"))?;
                let (win, tile) = self
                    .context_spawn_target(context_id)
                    .ok_or_else(|| format!("context '{ctx_name}' has no pane to spawn beside"))?;
                (
                    TerminalDestination::Split {
                        at: self.window_membership(win),
                        anchor: self.split_anchor(win, tile),
                        vertical,
                        new_pane_first,
                        keep_focus: spec.no_focus,
                    },
                    self.windows[win].get_focused_pane_cwd(tile),
                )
            } else if layout == "new_window" {
                let win = origin.map_or(active, |(win, _)| win);
                (
                    TerminalDestination::NewWindow {
                        context_id: self.windows[win].context_id,
                        grid_y: self.windows[win].grid_y,
                        restore_active: spec.no_focus.then_some(self.windows[active].window_id),
                    },
                    // The origin window's context, not whichever is active.
                    Some(context_root(win).unwrap_or_else(home)),
                )
            } else if layout == "tab" {
                let win = origin.map_or(active, |(win, _)| win);
                let (anchor, anchor_cwd) = if self.windows[win].panes.is_empty() {
                    (TabAnchor::EmptyRoot, None)
                } else {
                    let pane = self.focused_tab_anchor(win).ok_or_else(|| {
                        format!(
                            "window {} has no focused pane to open a tab beside",
                            self.windows[win].window_id
                        )
                    })?;
                    let tile = self.windows[win].tree.tiles.find_pane(&pane);
                    (
                        TabAnchor::Pane(pane),
                        tile.and_then(|tile| self.windows[win].get_focused_pane_cwd(tile)),
                    )
                };
                (
                    TerminalDestination::Tab {
                        at: self.window_membership(win),
                        anchor,
                        keep_focus: spec.no_focus,
                    },
                    Some(context_root(win).or(anchor_cwd).unwrap_or_else(home)),
                )
            } else {
                // An explicit origin wins (cross-window), then the active
                // window's focus, then an empty active window's root.
                let split_target = origin.or_else(|| {
                    self.windows[active]
                        .focused_pane
                        .or(self.windows[active].tree.root)
                        .map(|tile| (active, tile))
                });
                match split_target {
                    Some((win, tile)) => (
                        TerminalDestination::Split {
                            at: self.window_membership(win),
                            anchor: self.split_anchor(win, tile),
                            vertical,
                            new_pane_first,
                            keep_focus: spec.no_focus || spec.from_pane_id.is_some(),
                        },
                        self.windows[win].get_focused_pane_cwd(tile),
                    ),
                    None => (
                        TerminalDestination::SeedEmptyRoot {
                            at: self.window_membership(active),
                        },
                        Some(context_root(active).unwrap_or_else(home)),
                    ),
                }
            };
        let cwd = spec.cwd.clone().or(default_cwd);
        let worktree = spec
            .worktree
            .as_ref()
            .map(|branch| (branch.clone(), self.resolve_worktree_anchor(spec)));
        // The credential scope is the destination context's, bound now.
        let context = self.pane_context_env_for_context(destination.context_id());
        let scope = crate::pane_ops::grant::credential_scope(&context, cwd.as_deref());
        // Secrets belong to one directory decided now. A checkout's
        // directory does not exist yet, so a checkout launch gets none.
        let secret_directory = if worktree.is_some() {
            None
        } else {
            cwd.clone()
        };
        let pane_id = self.host.alloc_pane_id();
        let parent = match &source {
            crate::broker::pane_authority::GrantSource::VerifiedPane(caller) => {
                Some(caller.pane_id)
            }
            _ => None,
        };
        let grant = crate::broker::pane_authority::PaneAuthority::admit(
            &crate::config::config_dir(),
            crate::broker::pane_authority::GrantRequest {
                pane_id,
                source,
                parent,
                role: None,
                scope,
                secret_directory,
                folder_secrets: true,
                agent_command: spec.agent.is_some(),
                spawned_by,
            },
        );
        log::info!(
            "launch_plan: admitted terminal pane_id={pane_id} destination={destination:?} cwd={cwd:?} caller={caller:?} scope={:?} worktree={:?}",
            grant.scope,
            worktree.as_ref().map(|(branch, _)| branch)
        );
        let scope_revision = self.context_scope_revision(grant.scope.context_id);
        Ok(TerminalLaunchPlan {
            grant,
            scope_revision,
            caller,
            destination,
            cwd,
            worktree,
            initial_cmd: super::cmd_from_args(&spec.args),
            ephemeral: spec.ephemeral,
            name: spec.name.clone().filter(|name| !name.is_empty()),
            result_schema: spec.result_schema.clone(),
        })
    }

    /// Serve a terminal `spawn_pane`: admit it, commit it, and answer the
    /// caller. A refusal reports what became of a prepared checkout.
    pub(crate) fn launch_terminal_spec(&mut self, spec: &PaneLaunchSpec) {
        let plan = match self.admit_terminal_launch(spec) {
            Ok(plan) => plan,
            Err(error) => {
                log::warn!("pane_ipc: spawn_pane rejected: {error}");
                self.reply_spawn_pane(spec, &Err(error));
                return;
            }
        };
        match self.apply_launch(LaunchPlan::Terminal(Box::new(plan))) {
            Ok(pane_id) => self.reply_spawn_pane(spec, &Ok(pane_id)),
            Err(refused) => {
                log::warn!("pane_ipc: spawn_pane refused: {refused}");
                if let Some(rf) = &spec.response_file {
                    let mut reply = json!({ "error": refused.to_string() });
                    if let Some(report) = &refused.worktree {
                        reply["worktree"] = serde_json::to_value(report).unwrap_or(Value::Null);
                    }
                    crate::rpc::write_json_response(rf, reply);
                }
            }
        }
    }

    /// The admitted caller incarnation is still live and holds the grant the
    /// plan was admitted from. A caller that closed or was replaced refuses.
    pub(crate) fn verify_caller_incarnation(
        &self,
        caller: Option<&AdmittedIncarnation>,
    ) -> Result<(), String> {
        let Some(caller) = caller else {
            return Ok(());
        };
        let live = self
            .pane_authority(caller.pane_id)
            .filter(|live| live.incarnation() == caller.incarnation && !live.is_revoked());
        if live.is_none() {
            return Err(refused(format!(
                "caller pane {} closed or was replaced after admission",
                caller.pane_id
            )));
        }
        Ok(())
    }

    /// The destination still has the credential scope the grant was
    /// admitted with. Checked before any secret is read, credential issued
    /// or backend started.
    fn verify_admitted_scope(
        &self,
        grant: &crate::broker::pane_authority::PaneAuthority,
        context: &crate::pane_ops::create::PaneContextEnv,
        cwd: Option<&std::path::Path>,
    ) -> Result<(), String> {
        let now = crate::pane_ops::grant::credential_scope(context, cwd);
        if now != grant.scope {
            return Err(refused(format!(
                "the destination's scope changed after admission: context {} root {} -> context {} root {}",
                grant.scope.context_id,
                grant.scope.workspace_root.display(),
                now.context_id,
                now.workspace_root.display()
            )));
        }
        Ok(())
    }

    /// Revalidate a terminal plan's destination and resolve where it installs,
    /// changing nothing. Every stale target refuses here, before anything is
    /// constructed or any credential exists.
    fn resolve_terminal_placement(
        &self,
        destination: &TerminalDestination,
    ) -> Result<TerminalPlacement, String> {
        Ok(match destination {
            TerminalDestination::Split {
                at,
                anchor,
                vertical,
                new_pane_first,
                keep_focus,
            } => {
                let win_idx = self.locate_window(*at)?;
                let target_tile = self.locate_split_anchor(win_idx, *at, anchor)?;
                TerminalPlacement::Split(crate::pane_ops::create::SplitPlacement {
                    win_idx,
                    target_tile,
                    vertical: *vertical,
                    new_pane_first: *new_pane_first,
                    keep_focus: *keep_focus,
                })
            }
            TerminalDestination::Tab {
                at,
                anchor,
                keep_focus,
            } => {
                let win_idx = self.locate_window(*at)?;
                let anchor = match anchor {
                    TabAnchor::Pane(pane_id) => {
                        self.locate_split_anchor(win_idx, *at, &SplitAnchor::Pane(*pane_id))?;
                        Some(*pane_id)
                    }
                    TabAnchor::EmptyRoot => {
                        self.require_empty_tree(win_idx, *at)?;
                        None
                    }
                };
                TerminalPlacement::Tab {
                    win_idx,
                    anchor,
                    keep_focus: *keep_focus,
                }
            }
            TerminalDestination::SeedEmptyRoot { at } => {
                let win_idx = self.locate_window(*at)?;
                self.require_empty_tree(win_idx, *at)?;
                TerminalPlacement::Root { win_idx }
            }
            TerminalDestination::NewWindow {
                context_id,
                grid_y,
                restore_active,
            } => {
                let (context_id, grid_y) = (*context_id, *grid_y);
                if !self.router.iter().any(|c| c.context_id == context_id) {
                    return Err(refused(format!("context {context_id} was deleted")));
                }
                let grid_x = self
                    .windows
                    .iter()
                    .filter(|w| w.context_id == context_id && w.grid_y == grid_y)
                    .map(|w| w.grid_x)
                    .max()
                    .map_or(1, |x| x + 1);
                TerminalPlacement::NewWindow {
                    grid_x,
                    grid_y,
                    context_id,
                    restore_active: *restore_active,
                }
            }
        })
    }

    fn apply_terminal_launch(
        &mut self,
        plan: &TerminalLaunchPlan,
        prepared: Option<&ManagedWorktree>,
    ) -> Result<PaneId, String> {
        self.verify_caller_incarnation(plan.caller.as_ref())?;
        let placement = self.resolve_terminal_placement(&plan.destination)?;
        let context = match &placement {
            TerminalPlacement::Split(at) => self.pane_context_env_for_window(at.win_idx),
            TerminalPlacement::Tab { win_idx, .. } | TerminalPlacement::Root { win_idx } => {
                self.pane_context_env_for_window(*win_idx)
            }
            TerminalPlacement::NewWindow { context_id, .. } => {
                self.pane_context_env_for_context(*context_id)
            }
        };
        self.verify_admitted_scope(&plan.grant, &context, plan.cwd.as_deref())?;
        let revision = self.context_scope_revision(plan.grant.scope.context_id);
        if revision != plan.scope_revision {
            return Err(refused(format!(
                "the destination's scope changed after admission (revision {} -> {revision})",
                plan.scope_revision
            )));
        }
        // The directory is the admitted one, or the checkout prepared for
        // this plan; everything else about the pane is its admitted grant.
        let launch = crate::pane_ops::construct::TerminalLaunch {
            authority: crate::broker::pane_authority::LivePaneAuthority::admitted(
                plan.grant.clone(),
            ),
            context,
            cwd: prepared.map(|wt| wt.path.clone()).or(plan.cwd.clone()),
            command: plan.initial_cmd.clone().map(|command| {
                crate::pane_ops::construct::LaunchCommand {
                    command,
                    close_on_exit: plan.ephemeral,
                }
            }),
            ephemeral: plan.ephemeral,
            env: Vec::new(),
            mcp: crate::pane_ops::construct::McpBearer::Pane,
            profile_dir: crate::config::config_dir(),
        };
        let pane = self
            .construct_terminal(launch)
            .map_err(|error| format!("terminal failed to start: {error}"))?;
        let pane_id = match placement {
            TerminalPlacement::Split(at) => self.install_split(at, pane)?,
            TerminalPlacement::Tab {
                win_idx,
                anchor,
                keep_focus,
            } => {
                let original_focus = self.windows[win_idx].focused_pane;
                let pane_id = self.install_tab(win_idx, anchor, pane)?;
                // active_window is left alone: a tab never steals the user's
                // window, only the target window's own focus.
                if keep_focus {
                    self.restore_window_focused_pane(win_idx, original_focus);
                }
                pane_id
            }
            TerminalPlacement::Root { win_idx } => {
                let pane_id = pane.id;
                self.install_window_root(win_idx, pane);
                pane_id
            }
            TerminalPlacement::NewWindow {
                grid_x,
                grid_y,
                context_id,
                restore_active,
            } => {
                let pane_id = self.install_new_window(grid_x, grid_y, context_id, pane);
                if let Some(window_id) = restore_active {
                    if let Some(win_idx) =
                        self.windows.iter().position(|w| w.window_id == window_id)
                    {
                        self.active_window = win_idx;
                    }
                }
                pane_id
            }
        };
        if let Some(ref name) = plan.name {
            self.apply_inline_pane_name(pane_id, name);
        }
        self.record_pane_provenance(pane_id);
        if let Some(ref schema) = plan.result_schema {
            self.pane_results.entry(pane_id).or_default().schema = Some(schema.clone());
            log::info!("pane_result: registered schema at spawn for pane_id={pane_id}");
        }
        Ok(pane_id)
    }

    /// Everything that authorizes and shapes `role` for `parent`, read from
    /// current state. Admission records it and commit compares it.
    fn agent_call_authority(
        &self,
        parent: Option<PaneId>,
        origin_context_id: u64,
        role: &str,
        handoff: bool,
        lookup_root: &std::path::Path,
        task_text: &str,
    ) -> Result<AgentCallAuthority, String> {
        let grant_root =
            self.check_agent_tool_permission(parent, origin_context_id, role, handoff)?;
        let workspace = crate::app::registry::resolve_agent_lookup_workspace_root(lookup_root);
        let (def, _) = crate::cli::agent::resolve_agent_definition_in(
            role,
            &crate::config::config_dir(),
            workspace.as_deref(),
        )?;
        let harness = def
            .harness
            .as_ref()
            .ok_or("agent role has no CLI harness")?;
        let adapter = crate::cli::agent::resolve_harness_adapter(harness)?;
        let memory = crate::cli::agent::named_agent_memory(&def, harness.ephemeral)?;
        if memory.is_some() {
            crate::agent::memory::ensure_memory_dir(&def)?;
        }
        let command = crate::cli::agent::with_auto_state_hooks(
            adapter,
            crate::cli::agent::build_agent_spawn_command(
                &def,
                adapter,
                task_text,
                memory.as_deref(),
            )?,
        );
        let caller_grant = match parent {
            Some(id) => Some(
                self.pane_authority(id)
                    .ok_or_else(|| format!("permission_denied: caller pane {id} has no grant"))?
                    .grant()
                    .clone(),
            ),
            None => None,
        };
        Ok(AgentCallAuthority {
            parent,
            grant_root,
            role: def.id.clone(),
            harness: adapter.harness.as_str().to_string(),
            command,
            ephemeral: harness.ephemeral,
            result_schema: harness.result_schema.clone(),
            worker_role: crate::broker::pane_authority::AdmittedRole {
                id: def.id.clone(),
                posture: def.posture.clone(),
                can_call: def.can_call.clone(),
                can_handoff: def.can_handoff.clone(),
            },
            caller_grant,
        })
    }

    /// Admit an `agent call`: the authenticated caller, the role's grant,
    /// definition and ceiling, and the destination the caller selected.
    pub(crate) fn admit_agent_call(
        &mut self,
        request: &AgentCallRequest<'_>,
    ) -> Result<AgentCallPlan, String> {
        if request.task.trim().is_empty() {
            return Err("task must be non-empty".into());
        }
        let caller = AdmittedAgentCaller {
            origin: request.origin.clone(),
            credential: request.credential.map(str::to_string),
        };
        let parent = self.agent_tool_caller(AgentCaller {
            origin: &caller.origin,
            credential: caller.credential.as_deref(),
        })?;
        if request.handoff && parent.is_none() {
            return Err("handoff requires a live caller pane".into());
        }
        // The caller's own window, or for an outside caller the window the
        // user had active. Bound before the first permission check; nothing
        // after admission re-derives it from focus.
        let origin_win = parent
            .and_then(|id| self.find_pane_in_any_window(id))
            .map_or(self.active_window, |(win, _)| win);
        let origin = self.window_membership(origin_win);
        let grant_root = self.check_agent_tool_permission(
            parent,
            origin.context_id,
            request.role,
            request.handoff,
        )?;
        self.admit_outside_use(
            &caller.origin,
            "agent.call",
            &json!({"role": request.role, "task": request.task, "cwd": request.cwd}),
            &grant_root,
            origin.context_id,
        )?;
        let lookup_root = match (parent, request.cwd) {
            (None, Some(cwd)) => PathBuf::from(cwd),
            _ => grant_root,
        };
        let parent_terminal = parent.and_then(|id| self.terminal_incarnation(id));
        let parent_of_parent = parent_terminal.and_then(|t| t.parent_pane_id);
        let task_text = match (request.handoff, parent) {
            (true, Some(old)) => {
                let packet = json!({"from_pane_id":old,"from_role":parent_terminal.and_then(|t| t.agent_role.as_deref()),
                    "session":parent_terminal.and_then(|t| t.build_saved_agent_session()),"task":request.task});
                format!(
                    "# Restore packet\n{packet}\n\n# Successor task\n{}",
                    request.task
                )
            }
            _ => request.task.to_string(),
        };
        let authority = self.agent_call_authority(
            parent,
            origin.context_id,
            request.role,
            request.handoff,
            &lookup_root,
            &task_text,
        )?;
        // The completion names the successor, so reserve the successor's id
        // before validating it: the worker is constructed under exactly this
        // id, whatever is allocated in between (stint 1632).
        let successor = match (request.handoff, parent) {
            (true, Some(old)) => {
                let successor = self.host.alloc_pane_id();
                self.validate_handoff_completion(old, &authority.role, request.task, successor)?;
                Some(successor)
            }
            _ => None,
        };
        let child_parent = if request.handoff {
            parent_of_parent
        } else {
            parent
        };
        let subcontext = match child_parent {
            Some(id) => {
                let name = format!("agent-tools-{id}");
                let under = self.agent_spawn_parent(
                    Some(id),
                    AgentOrigin::Window {
                        window_id: origin.window_id,
                        context_id: origin.context_id,
                    },
                )?;
                Some(match self.agent_subcontext_under(under.context_id, &name) {
                    Some(context_id) => AdmittedSubcontext::Existing { name, context_id },
                    None => AdmittedSubcontext::Create {
                        name,
                        parent_context_id: under.context_id,
                    },
                })
            }
            None => None,
        };
        // The existing window the worker lands in and its anchor, bound now:
        // commit never picks them from whatever is focused then.
        let destination = match &subcontext {
            Some(AdmittedSubcontext::Create { .. }) => None,
            Some(AdmittedSubcontext::Existing { context_id, .. }) => {
                let win = self
                    .windows
                    .iter()
                    .position(|w| w.context_id == *context_id)
                    .ok_or_else(|| format!("sub-context {context_id} has no window"))?;
                Some(self.agent_anchor(win))
            }
            None => Some(self.agent_anchor(origin_win)),
        };
        let destination_scope = destination.as_ref().map(|anchor| {
            (
                crate::pane_ops::grant::credential_scope(
                    &self.pane_context_env_for_context(anchor.at.context_id),
                    Some(&lookup_root),
                ),
                self.context_scope_revision(anchor.at.context_id),
            )
        });
        log::info!(
            "launch_plan: admitted agent_call role={} parent={parent:?} child_parent={child_parent:?} origin={origin:?} subcontext={subcontext:?} destination={destination:?} destination_scope={destination_scope:?} handoff={} worktree={:?}",
            authority.role,
            request.handoff,
            request.worktree
        );
        Ok(AgentCallPlan {
            caller,
            requested_role: request.role.to_string(),
            handoff: request.handoff,
            authority,
            task_text,
            task: request.task.to_string(),
            lookup_root,
            child_parent,
            subcontext,
            origin,
            destination,
            destination_scope,
            worktree: request.worktree.map(str::to_string),
            successor,
        })
    }

    /// `win_idx` and the node a worker splits beside there now: its focus, or
    /// its root, or nothing for an empty tree.
    fn agent_anchor(&self, win_idx: usize) -> AgentAnchor {
        let win = &self.windows[win_idx];
        AgentAnchor {
            at: self.window_membership(win_idx),
            anchor: win
                .focused_pane
                .or(win.tree.root)
                .map(|tile| self.split_anchor(win_idx, tile)),
        }
    }

    /// The handoff completion `old` would publish for `successor`, checked
    /// against its current write-once result state without writing it.
    fn validate_handoff_completion(
        &self,
        old: PaneId,
        role: &str,
        task: &str,
        successor: PaneId,
    ) -> Result<(), String> {
        let raw = handoff_completion(role, task, successor);
        let mut result_state = self.pane_results.get(&old).cloned().unwrap_or_default();
        result_state
            .write_payload(&raw)
            .map(|_| ())
            .map_err(|error| format!("handoff completion: {}", error.message()))
    }

    /// Revalidate an admitted window and anchor and place the worker exactly
    /// there. A window that closed, moved context or is not the placement's,
    /// or an anchor that left, changed or filled, refuses.
    fn bind_agent_destination(
        &self,
        placement: &mut crate::pane_ops::create::AgentPlacement,
        destination: &AgentAnchor,
    ) -> Result<(), String> {
        let at = destination.at;
        let win_idx = self.locate_window(at)?;
        let tile = match &destination.anchor {
            Some(anchor) => Some(self.locate_split_anchor(win_idx, at, anchor)?),
            None => {
                self.require_empty_tree(win_idx, at)?;
                None
            }
        };
        match &mut placement.target {
            crate::pane_ops::create::AgentTarget::Window {
                win_idx: target,
                context_id,
                split_at,
            } if *context_id == at.context_id => {
                *target = win_idx;
                *split_at = tile;
                Ok(())
            }
            other => Err(refused(format!(
                "the worker's destination is no longer window {} of context {}: {other:?}",
                at.window_id, at.context_id
            ))),
        }
    }

    fn apply_agent_call(
        &mut self,
        plan: &AgentCallPlan,
        prepared: Option<&ManagedWorktree>,
    ) -> Result<PaneId, String> {
        // The admitted caller again, at commit: the same live incarnation,
        // or nothing launches (stint 1285).
        self.deferred_origin_is_current(&plan.caller.origin, None)
            .map_err(refused)?;
        let parent = self.agent_tool_caller(AgentCaller {
            origin: &plan.caller.origin,
            credential: plan.caller.credential.as_deref(),
        })?;
        if parent != plan.authority.parent {
            return Err(refused(format!(
                "caller identity changed from {:?} to {parent:?}",
                plan.authority.parent
            )));
        }
        if let Some(required) = plan.child_parent {
            if self.find_pane_in_any_window(required).is_none() {
                return Err(refused(format!("parent pane {required} closed")));
            }
        }
        let now = self.agent_call_authority(
            parent,
            plan.origin.context_id,
            &plan.requested_role,
            plan.handoff,
            &plan.lookup_root,
            &plan.task_text,
        )?;
        if now != plan.authority {
            return Err(refused(format!(
                "permission_denied: the authority or definition of role '{}' changed after admission",
                plan.authority.role
            )));
        }
        // Immediately before startup: the same completion against the
        // predecessor's current result state. A refusal starts no worker.
        if let (Some(old), Some(successor)) = (parent, plan.successor) {
            self.validate_handoff_completion(old, &plan.authority.role, &plan.task, successor)?;
        }
        let cwd = prepared
            .map(|wt| wt.path.clone())
            .unwrap_or_else(|| plan.lookup_root.clone());
        let subcontext = plan.subcontext.as_ref().map(|sub| match sub {
            AdmittedSubcontext::Existing { name, context_id } => SubcontextTarget::Existing {
                name,
                context_id: *context_id,
            },
            AdmittedSubcontext::Create {
                name,
                parent_context_id,
            } => SubcontextTarget::Create {
                name,
                parent_context_id: *parent_context_id,
            },
        });
        let mut placement = self.resolve_agent_spawn_location(
            subcontext,
            plan.child_parent,
            Some(cwd),
            true,
            AgentOrigin::Window {
                window_id: plan.origin.window_id,
                context_id: plan.origin.context_id,
            },
        )?;
        if let Some(destination) = &plan.destination {
            self.bind_agent_destination(&mut placement, destination)?;
        }
        if let (
            Some((admitted, admitted_revision)),
            crate::pane_ops::create::AgentTarget::Window { win_idx, .. },
        ) = (&plan.destination_scope, &placement.target)
        {
            let now = crate::pane_ops::grant::credential_scope(
                &self.pane_context_env_for_window(*win_idx),
                Some(&placement.effective_cwd),
            );
            let revision = self.context_scope_revision(admitted.context_id);
            if &now != admitted || revision != *admitted_revision {
                return Err(refused(format!(
                    "the worker's destination scope changed after admission: root {} -> {} (revision {admitted_revision} -> {revision})",
                    admitted.workspace_root.display(),
                    now.workspace_root.display()
                )));
            }
        }
        let authority = &plan.authority;
        let (pane_id, _) = self.launch_agent_worker(
            crate::pane_ops::create::AgentWorkerLaunch {
                grant: crate::pane_ops::create::AgentWorkerGrant {
                    source: match &authority.caller_grant {
                        Some(caller) => crate::broker::pane_authority::GrantSource::VerifiedPane(
                            Box::new(caller.clone()),
                        ),
                        None => crate::broker::pane_authority::GrantSource::OutsideProcess,
                    },
                    role: Some(authority.worker_role.clone()),
                    recorded: None,
                },
                pane_id: plan.successor,
                spawned_by: None,
                role: &authority.role,
                harness: &authority.harness,
                command: &authority.command,
                task: &plan.task,
                ephemeral: authority.ephemeral,
                result_schema: authority.result_schema.clone(),
                run_token: None,
            },
            placement,
        )?;
        Ok(pane_id)
    }
}

/// The test seam between checkout preparation and revalidation: a callback
/// that runs synchronously, with the host and the prepared checkout (if the
/// plan has one), before the plan is revalidated.
#[cfg(test)]
pub(crate) mod probes {
    use std::cell::RefCell;

    type Prepared =
        Box<dyn FnMut(&mut super::PlexiApp, Option<&crate::host::worktree::ManagedWorktree>)>;

    thread_local! {
        static PREPARED: RefCell<Option<Prepared>> = RefCell::new(None);
    }

    /// Run `probe` at that point of every launch this thread applies, until
    /// the guard drops.
    pub(crate) fn on_prepared(
        probe: impl FnMut(&mut super::PlexiApp, Option<&crate::host::worktree::ManagedWorktree>)
            + 'static,
    ) -> Guard {
        PREPARED.with(|p| *p.borrow_mut() = Some(Box::new(probe)));
        Guard
    }

    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            PREPARED.with(|p| *p.borrow_mut() = None);
        }
    }

    pub(super) fn prepared(
        app: &mut super::PlexiApp,
        wt: Option<&crate::host::worktree::ManagedWorktree>,
    ) {
        let probe = PREPARED.with(|p| p.borrow_mut().take());
        if let Some(mut probe) = probe {
            probe(app, wt);
            PREPARED.with(|p| {
                let mut slot = p.borrow_mut();
                if slot.is_none() {
                    *slot = Some(probe);
                }
            });
        }
    }
}

/// A digest of a window's tree structure: every tile, its kind, children in
/// order and pane occupant. Split shares and the active tab are not
/// structure. Combined with the membership generation it identifies a
/// container node (stint 1632).
fn tree_topology(tree: &egui_tiles::Tree<PaneId>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut tiles: Vec<_> = tree.tiles.iter().collect();
    tiles.sort_unstable_by_key(|(id, _)| id.0);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tree.root.hash(&mut hasher);
    for (id, tile) in tiles {
        id.hash(&mut hasher);
        match tile {
            Tile::Pane(pane) => pane.hash(&mut hasher),
            Tile::Container(container) => {
                format!("{:?}", container.kind()).hash(&mut hasher);
                container
                    .children()
                    .for_each(|child| child.hash(&mut hasher));
            }
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::pane_authority::Provenance;
    use crate::broker::PermissionPosture;
    use crate::host::context::{Context, Window};
    use crate::host::keys::Direction;
    use crate::host::pane::{Pane, PortalPane, ViewPane, ViewRuntime};
    use crate::protocol::HostRequest;
    use crate::testing::HostHarness;
    use std::collections::HashMap;

    fn view_pane(h: &HostHarness, pane_id: PaneId) -> Pane {
        let root = h.workspace_root();
        Pane::View(Box::new(ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id: pane_id,
            runtime: ViewRuntime::Builtin(Box::new(crate::file_browser::FileBrowserView::new(
                root.clone(),
            ))),
            workspace_root: root,
            permissions: crate::app::permissions::PluginPermissions::builtin(),
            manifest_id: "test".to_string(),
            name: "Test App".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: None,
            slots: HashMap::new(),
            semantic_state: Default::default(),
        }))
    }

    /// Replace window `win_idx`'s layout with `panes` side by side, focused
    /// on the first. Returns the pane ids.
    fn lay_out(h: &mut HostHarness, win_idx: usize, panes: Vec<(PaneId, Pane)>) -> Vec<PaneId> {
        let mut tiles = egui_tiles::Tiles::default();
        let ids: Vec<PaneId> = panes.iter().map(|(id, _)| *id).collect();
        let tile_ids: Vec<TileId> = ids.iter().map(|id| tiles.insert_pane(*id)).collect();
        let root = tiles.insert_horizontal_tile(tile_ids.clone());
        let win = &mut h.app.windows[win_idx];
        win.tree = egui_tiles::Tree::new("launch-plan-test", root, tiles);
        win.panes = panes.into_iter().collect();
        // Left unsimplified so the first pane keeps tile id 1, the id a
        // rebuilt single-pane tree gives its own root pane.
        win.focused_pane = Some(tile_ids[0]);
        ids
    }

    fn views(h: &mut HostHarness, n: usize) -> Vec<(PaneId, Pane)> {
        (0..n)
            .map(|_| {
                let id = h.app.host.alloc_pane_id();
                (id, view_pane(h, id))
            })
            .collect()
    }

    /// Push an empty window into `context_id` and return its index.
    fn push_window(h: &mut HostHarness, context_id: u64, grid_x: u32) -> usize {
        let window_id = h.app.next_window_id;
        h.app.next_window_id += 1;
        h.app.windows.push(Window {
            name: String::new(),
            path: h.workspace_root(),
            tree: egui_tiles::Tree::empty("launch-plan-test"),
            panes: Default::default(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x,
            grid_y: 0,
            window_id,
            context_id,
            tree_history: Default::default(),
        });
        h.app.windows.len() - 1
    }

    fn push_context(h: &mut HostHarness, name: &str, parent_id: Option<u64>) -> u64 {
        let context_id = h.app.next_window_id;
        h.app.next_window_id += 1;
        h.app.router.push(Context {
            name: name.to_string().into(),
            root: h.workspace_root(),
            description: None,
            context_id,
            parent_id,
            depth: u32::from(parent_id.is_some()),
            parked: false,
        });
        context_id
    }

    fn spec(
        h: &HostHarness,
        layout: &str,
        from: Option<PaneId>,
        ctx: Option<&str>,
    ) -> PaneLaunchSpec {
        PaneLaunchSpec::from_spawn_pane(&HostRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some(layout.to_string()),
            args: Vec::new(),
            from_pane_id: from,
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: Some(h.workspace_root().to_string_lossy().into_owned()),
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            context_name: ctx.map(str::to_string),
            name: None,
            agent_cmd: None,
            boot_timeout_secs: None,
            worktree: None,
            result_schema: None,
        }, crate::broker::caller::RequestOrigin::HostUi)
        .expect("valid terminal spec")
    }

    /// A terminal launch from `caller`'s own process, as the socket handler
    /// stamps it.
    fn from_caller(h: &HostHarness, layout: &str, caller: PaneId) -> PaneLaunchSpec {
        let pid = h
            .app
            .terminal_incarnation(caller)
            .expect("a live terminal caller")
            .backend
            .child_pid();
        let mut launch = spec(h, layout, Some(caller), None);
        launch.origin = h.app.socket_origin_for_pids(&[pid]);
        launch
    }

    fn apply(h: &mut HostHarness, plan: &TerminalLaunchPlan) -> Result<PaneId, String> {
        h.app
            .apply_launch(LaunchPlan::Terminal(Box::new(plan.clone())))
            .map_err(|refused| refused.to_string())
    }

    fn apply_agent(h: &mut HostHarness, plan: &AgentCallPlan) -> Result<PaneId, String> {
        h.app
            .apply_launch(LaunchPlan::Agent(Box::new(plan.clone())))
            .map_err(|refused| refused.to_string())
    }

    fn window_of(h: &HostHarness, pane_id: PaneId) -> usize {
        h.app
            .find_pane_in_any_window(pane_id)
            .map(|(win, _)| win)
            .expect("pane is live")
    }

    /// Finding 3: rebuilding a window restarts its tile ids, so the anchor's
    /// old tile id names a different node. Split, tab and named-context
    /// splits bind the anchor pane and refuse.
    #[test]
    fn launch_plan_refuses_split_tab_and_named_context_after_tile_reuse() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 2);
        let [a, _b] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let main_ctx = h.app.windows[0].context_id;
        let work_ctx = push_context(&mut h, "work", None);
        let work_win = push_window(&mut h, work_ctx, 0);
        let work_panes = views(&mut h, 1);
        lay_out(&mut h, work_win, work_panes);

        let split = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(a), None))
            .unwrap();
        let tab = h
            .app
            .admit_terminal_launch(&spec(&h, "tab", Some(a), None))
            .unwrap();
        let named = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", None, Some("work")))
            .unwrap();

        let a_tile = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
        // Rebuild both trees in place through the production seed path.
        for win_idx in [0, work_win] {
            let context = h.app.pane_context_env_for_window(win_idx);
            let launch = h
                .app
                .desktop_terminal_launch(context, Some(h.workspace_root()), None);
            h.app
                .seed_window_root_pane(win_idx, launch)
                .expect("rebuilt root terminal");
        }
        assert_eq!(h.app.windows[0].context_id, main_ctx);
        assert!(
            h.app.windows[0].tree.tiles.get(a_tile).is_some(),
            "fixture: the anchor's old tile id names a node of the rebuilt tree"
        );
        for (label, plan) in [("split", &split), ("tab", &tab), ("named", &named)] {
            let before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
            let error = apply(&mut h, plan).expect_err(label);
            assert!(error.contains("anchor pane"), "{label}: {error}");
            let after = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
            assert_eq!(before, after, "{label}: a refused launch creates nothing");
        }
    }

    /// Finding 3: dissolving a portal promotes the child's secondary window
    /// into the parent context with the same window, tile and pane ids. A
    /// launch admitted into the child context refuses instead of landing in
    /// the parent.
    #[test]
    fn launch_plan_refuses_window_promoted_out_of_its_context() {
        let mut h = HostHarness::new();
        let parent_ctx = h.app.windows[0].context_id;
        let child_ctx = push_context(&mut h, "child", Some(parent_ctx));
        let portal_id = h.app.host.alloc_pane_id();
        let mut parent_panes = views(&mut h, 1);
        parent_panes.push((
            portal_id,
            Pane::Portal(Box::new(PortalPane {
                pane_id: portal_id,
                target_context_id: child_ctx,
                context_state: None,
            })),
        ));
        lay_out(&mut h, 0, parent_panes);
        let primary = push_window(&mut h, child_ctx, 0);
        let primary_panes = views(&mut h, 1);
        lay_out(&mut h, primary, primary_panes);
        let secondary = push_window(&mut h, child_ctx, 1);
        let secondary_panes = views(&mut h, 1);
        let [y] = lay_out(&mut h, secondary, secondary_panes)[..] else {
            unreachable!()
        };
        let secondary_id = h.app.windows[secondary].window_id;
        h.app
            .context_active_window
            .insert(child_ctx, h.app.windows[primary].window_id);

        let split = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(y), None))
            .unwrap();
        let tab = h
            .app
            .admit_terminal_launch(&spec(&h, "tab", Some(y), None))
            .unwrap();
        h.app.dissolve_portal(child_ctx);
        let promoted = h
            .app
            .windows
            .iter()
            .find(|w| w.window_id == secondary_id)
            .expect("secondary window survives the dissolve");
        assert_eq!(
            promoted.context_id, parent_ctx,
            "fixture: window was promoted"
        );
        assert!(promoted.panes.contains_key(&y), "fixture: anchor pane kept");

        for (label, plan) in [("split", &split), ("tab", &tab)] {
            let error = apply(&mut h, plan).expect_err(label);
            assert!(error.contains("moved from context"), "{label}: {error}");
        }
    }

    /// An empty-root seed binds its window and empty-tree generation: focus
    /// moving elsewhere does not redirect it, and a fill-and-empty cycle in
    /// between refuses it.
    #[test]
    fn launch_plan_empty_root_ignores_focus_and_refuses_fill_and_empty() {
        let mut h = HostHarness::new();
        let other_ctx = push_context(&mut h, "other", None);
        let other = push_window(&mut h, other_ctx, 0);
        let other_panes = views(&mut h, 1);
        lay_out(&mut h, other, other_panes);
        h.app.active_window = 0;
        let empty_id = h.app.windows[0].window_id;
        let focus_moved = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", None, None))
            .unwrap();
        let aba = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", None, None))
            .unwrap();
        assert!(matches!(
            focus_moved.destination,
            TerminalDestination::SeedEmptyRoot { .. }
        ));

        // Fill and empty window 0 in place: an empty-window tab inserts into
        // the existing tree, and closing it leaves the tree empty again.
        let context = h.app.pane_context_env_for_window(0);
        let launch = h
            .app
            .desktop_terminal_launch(context, Some(h.workspace_root()), None);
        let filler = h.app.new_tab(0, None, launch).expect("filler tab");
        h.app.close_pane_by_id(filler);
        assert!(
            h.app.windows[0].tree.root.is_none(),
            "fixture: emptied again"
        );
        assert_eq!(h.app.windows[0].window_id, empty_id, "fixture: same window");
        let error = apply(&mut h, &aba).expect_err("fill-and-empty refuses");
        assert!(error.contains("no longer the empty tree"), "{error}");

        // Positive control on a fresh admission: focus moves to the other
        // window and the seed still lands in the window it was admitted for.
        let focus_moved = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", None, None))
            .unwrap();
        h.app.active_window = other;
        let other_before = h.app.windows[other].panes.len();
        let pane = apply(&mut h, &focus_moved).expect("seed lands");
        assert_eq!(h.app.windows[window_of(&h, pane)].window_id, empty_id);
        assert_eq!(h.app.windows[other].panes.len(), other_before);
    }

    /// Positive control: focus and the active window move after admission
    /// while the anchor survives, and the launch lands beside the anchor.
    #[test]
    fn launch_plan_follows_surviving_anchor_when_focus_moves() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 2);
        let [a, b] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let other_ctx = push_context(&mut h, "other", None);
        let other = push_window(&mut h, other_ctx, 0);
        let other_panes = views(&mut h, 1);
        lay_out(&mut h, other, other_panes);
        h.app.active_window = 0;
        // Stacked below a, so a and the new pane share a node b is outside.
        let split = h
            .app
            .admit_terminal_launch(&spec(&h, "split_v", None, None))
            .unwrap();
        let tab = h
            .app
            .admit_terminal_launch(&spec(&h, "tab", None, None))
            .unwrap();

        let b_tile = h.app.windows[0].tree.tiles.find_pane(&b).unwrap();
        h.app.set_window_focused_pane(0, b_tile);
        h.app.active_window = other;

        // The nearest node holding both panes: deeper for neighbours.
        let common = |h: &HostHarness, x: PaneId, y: PaneId| {
            let tiles = &h.app.windows[0].tree.tiles;
            let chain = |pane: PaneId| {
                let mut chain = vec![tiles.find_pane(&pane).unwrap()];
                while let Some(parent) = tiles.parent_of(*chain.last().unwrap()) {
                    chain.push(parent);
                }
                chain
            };
            let ys = chain(y);
            chain(x).into_iter().find(|tile| ys.contains(tile))
        };
        let split_pane = apply(&mut h, &split).expect("split lands");
        assert_eq!(window_of(&h, split_pane), 0);
        assert_ne!(
            common(&h, split_pane, a),
            common(&h, split_pane, b),
            "split lands beside a, not b"
        );

        let tab_pane = apply(&mut h, &tab).expect("tab lands");
        assert_eq!(window_of(&h, tab_pane), 0);
        let win = &h.app.windows[0];
        let tabs_of = |pane: PaneId| {
            win.find_ancestor_tabs(win.tree.tiles.find_pane(&pane).unwrap())
                .map(|(tabs, _)| tabs)
        };
        assert!(tabs_of(tab_pane).is_some());
        assert_eq!(tabs_of(tab_pane), tabs_of(a), "tab joins a's group");
        assert_ne!(tabs_of(tab_pane), tabs_of(b), "not b's group");
    }

    fn posture(deny: &[&str]) -> PermissionPosture {
        PermissionPosture {
            default_posture: crate::broker::Decision::Ask,
            allow: Vec::new(),
            ask: Vec::new(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn write_role(root: &std::path::Path, deny: &str) {
        let dir = crate::agent::workspace_agents_dir(root).join("tester");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AGENT.md"), "Test").unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            format!(
                "[agent]\nid='tester'\ndisplay_name='Tester'\n[harness]\ncli='pi'\nephemeral=false\n[permissions]\ndefault_posture='ask'\ndeny=[{deny}]\n"
            ),
        )
        .unwrap();
    }

    /// A tester worker constructed under its role's ceiling (deny
    /// `tool.secret`) through the production worker path.
    fn spawn_tester(h: &mut HostHarness, root: &std::path::Path) -> PaneId {
        spawn_tester_calling(h, root, "")
    }

    /// A tester worker whose role lists `can_call` when it is admitted.
    fn spawn_tester_calling(h: &mut HostHarness, root: &std::path::Path, can_call: &str) -> PaneId {
        write_role(root, "'tool.secret'");
        if !can_call.is_empty() {
            let path = crate::agent::workspace_agents_dir(root)
                .join("tester")
                .join("settings.toml");
            let settings = std::fs::read_to_string(&path).unwrap().replace(
                "ephemeral=false\n",
                &format!("ephemeral=false\ncan_call=[{can_call}]\n"),
            );
            std::fs::write(&path, settings).unwrap();
        }
        h.app
            .spawn_agent_worker_pane(
                "tester",
                "pi",
                "sleep 30",
                "Run",
                None,
                false,
                None,
                Some(root.to_path_buf()),
                None,
                None,
                false,
                None,
            )
            .expect("tester worker")
            .0
    }

    fn admit_tester_call_in(
        h: &mut HostHarness,
        root: &std::path::Path,
        worktree: Option<&str>,
    ) -> AgentCallPlan {
        h.app
            .admit_agent_call(&AgentCallRequest {
                role: "tester",
                task: "Verify",
                origin: &crate::broker::caller::RequestOrigin::HostUi,
                credential: None,
                // An outside-pane caller: authenticated socket, no pane.
                cwd: Some(root.to_str().unwrap()),
                worktree,
                asynchronous: true,
                handoff: false,
                response_file: "",
            })
            .expect("agent call admits")
    }

    fn admit_tester_call(h: &mut HostHarness, root: &std::path::Path) -> AgentCallPlan {
        admit_tester_call_in(h, root, None)
    }

    /// A branch name no other test run uses: `wtp` places checkouts in a
    /// directory shared by every repository under the same parent.
    fn unique_branch(stem: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{stem}-{}-{nanos}", std::process::id())
    }

    /// Remove a checkout a test left on disk (a retained, dirty one).
    fn remove_checkout(root: &std::path::Path, path: &std::path::Path, branch: &str) {
        for args in [
            vec!["worktree", "remove", "--force", path.to_str().unwrap()],
            vec!["branch", "-D", branch],
        ] {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(&args)
                .status();
        }
    }

    /// A git repository at `root` with one commit, for real checkouts.
    fn init_repo(root: &std::path::Path) {
        for args in [
            &["init", "-q", "-b", "main"][..],
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ][..],
        ] {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .expect("git runs");
            assert!(status.success(), "git {args:?}");
        }
    }

    /// Finding 1, terminal: a caller's ceiling is the new pane's, installed
    /// at construction from the caller's own binding.
    #[test]
    fn launch_plan_terminal_inherits_the_callers_constructed_ceiling() {
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let caller = spawn_tester(&mut h, &root);
        let ceiling = h.app.pane_ceiling_or_none(caller).expect("caller ceiling");
        assert_eq!(ceiling.deny, vec!["tool.secret".to_string()]);
        let plan = h
            .app
            .admit_terminal_launch(&from_caller(&h, "split_h", caller))
            .unwrap();
        assert_eq!(plan.grant.posture().unwrap(), Some(&ceiling));
        let pane = apply(&mut h, &plan).expect("inherited terminal launches");
        assert_eq!(h.app.pane_ceiling_or_none(pane), Some(ceiling));
    }

    /// Finding 6, agent call: the worker's ceiling is the one admitted. A
    /// broader definition of the same role inside the prepared checkout does
    /// not apply, and a definition broadened at the lookup root after
    /// admission refuses the launch.
    #[test]
    fn launch_plan_agent_call_keeps_admitted_ceiling() {
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        init_repo(&root);
        write_role(&root, "'tool.secret'");

        let branch = unique_branch("ceiling");
        let plan = admit_tester_call_in(&mut h, &root, Some(&branch));
        let checkout = std::rc::Rc::new(std::cell::RefCell::new(None));
        let seen = checkout.clone();
        let _seam = probes::on_prepared(move |_, wt| {
            let wt = wt.expect("checkout prepared");
            write_role(&wt.path, "");
            *seen.borrow_mut() = Some(wt.path.clone());
        });
        let pane = apply_agent(&mut h, &plan).expect("agent launches in the checkout");
        drop(_seam);
        let path = checkout.borrow().clone().expect("checkout prepared");
        let term = h.app.windows[window_of(&h, pane)].panes[&pane]
            .as_terminal()
            .unwrap();
        assert_eq!(term.initial_cwd.as_deref(), Some(path.as_path()));
        let installed = h.app.pane_ceiling_or_none(pane).expect("ceiling installed");
        assert_eq!(installed.deny, vec!["tool.secret".to_string()]);
        assert!(
            h.app.pane_worktrees.contains_key(&pane),
            "the pane owns its checkout"
        );
        h.app.close_pane_by_id(pane);
        remove_checkout(&root, &path, &branch);

        let plan = admit_tester_call(&mut h, &root);
        write_role(&root, "");
        let error = apply_agent(&mut h, &plan).expect_err("broadened role refuses");
        assert!(error.contains("changed after admission"), "{error}");
    }

    /// Finding 6: a named origin pane that is gone is an error for every
    /// layout, never a fall back to focus or the active window.
    #[test]
    fn launch_plan_refuses_a_missing_explicit_origin() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 1);
        lay_out(&mut h, 0, panes);
        let gone = h.app.host.alloc_pane_id();
        for layout in ["split_h", "tab", "new_window"] {
            let error = h
                .app
                .admit_terminal_launch(&spec(&h, layout, Some(gone), None))
                .expect_err(layout);
            assert!(
                error.contains(&format!("origin pane {gone} not found")),
                "{layout}: {error}"
            );
        }
    }

    /// Finding 6: a pane that left its window and came back is not the
    /// placement the caller selected.
    #[test]
    fn launch_plan_refuses_an_anchor_that_left_and_returned() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 2);
        let [a, _b] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let ctx = h.app.windows[0].context_id;
        let right = push_window(&mut h, ctx, 1);
        let right_panes = views(&mut h, 1);
        lay_out(&mut h, right, right_panes);
        h.app.active_window = 0;
        let split = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(a), None))
            .unwrap();
        let tab = h
            .app
            .admit_terminal_launch(&spec(&h, "tab", Some(a), None))
            .unwrap();

        assert!(h.app.move_focused_pane_to_adjacent_window(Direction::Right));
        let (win, tile) = h.app.find_pane_in_any_window(a).unwrap();
        assert_ne!(win, 0, "fixture: a left window 0");
        h.app.active_window = win;
        h.app.set_window_focused_pane(win, tile);
        assert!(h.app.move_focused_pane_to_adjacent_window(Direction::Left));
        assert_eq!(window_of(&h, a), 0, "fixture: a is back in window 0");

        for (label, plan) in [("split", &split), ("tab", &tab)] {
            let error = apply(&mut h, plan).expect_err(label);
            assert!(error.contains("left window"), "{label}: {error}");
        }
    }

    /// Finding 6: a container anchor is the node holding the panes it held at
    /// admission, not whatever occupies its tile id later.
    #[test]
    fn launch_plan_container_anchor_refuses_a_changed_node() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 2);
        lay_out(&mut h, 0, panes);
        h.app.windows[0].focused_pane = None;
        let unchanged = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", None, None))
            .unwrap();
        let changed = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", None, None))
            .unwrap();
        assert!(matches!(
            changed.destination,
            TerminalDestination::Split {
                anchor: SplitAnchor::Node { .. },
                ..
            }
        ));
        let first = apply(&mut h, &unchanged).expect("unchanged node accepts");
        assert_eq!(window_of(&h, first), 0);
        let error = apply(&mut h, &changed).expect_err("node gained a pane");
        assert!(error.contains("changed after admission"), "{error}");
    }

    /// Round 3 item 7: swapping two panes and swapping them back restores
    /// the same topology digest. The swaps still changed what the container
    /// held, so a container anchor admitted before them refuses; a pane
    /// anchor that only moved still launches.
    #[test]
    fn launch_plan_container_anchor_refuses_a_swap_and_swap_back() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 2);
        let [a, _b] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        h.app.active_window = 0;
        // Laid out, so swaps can find a neighbour by geometry.
        h.run_frames(2);
        let a_tile = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
        let pane_anchor = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(a), None))
            .unwrap();
        h.app.windows[0].focused_pane = None;
        let node = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", None, None))
            .unwrap();
        assert!(matches!(
            node.destination,
            TerminalDestination::Split {
                anchor: SplitAnchor::Node { .. },
                ..
            }
        ));
        let digest = tree_topology(&h.app.windows[0].tree);
        h.app.set_window_focused_pane(0, a_tile);
        for dir in [Direction::Right, Direction::Left] {
            assert!(matches!(
                h.app.swap_pane(dir),
                crate::pane_ops::SwapResult::Swapped { .. }
            ));
        }
        assert_eq!(
            tree_topology(&h.app.windows[0].tree),
            digest,
            "swap and swap back restore the digest"
        );
        let error = apply(&mut h, &node).expect_err("the container's occupants changed");
        assert!(error.contains("changed after admission"), "{error}");
        apply(&mut h, &pane_anchor).expect("a pane that only moved is still the anchor");
    }

    /// Finding 6: an overlay replaces the occupant of a slot under the same
    /// pane and tile ids. Pushing one, popping one, or both, after admission
    /// make split, tab and container targets stale; so does a plan admitted
    /// while covered and applied after the pop. A plan admitted after the
    /// restore launches.
    #[test]
    fn launch_plan_overlay_push_and_pop_change_the_occupant() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 2);
        let [a, _b] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        h.app.active_window = 0;
        let a_tile = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
        let admit_all = |h: &mut HostHarness| {
            let split = h
                .app
                .admit_terminal_launch(&spec(h, "split_h", Some(a), None))
                .unwrap();
            let tab = h
                .app
                .admit_terminal_launch(&spec(h, "tab", Some(a), None))
                .unwrap();
            h.app.windows[0].focused_pane = None;
            let node = h
                .app
                .admit_terminal_launch(&spec(h, "split_h", None, None))
                .unwrap();
            assert!(matches!(
                node.destination,
                TerminalDestination::Split {
                    anchor: SplitAnchor::Node { .. },
                    ..
                }
            ));
            h.app.set_window_focused_pane(0, a_tile);
            [("split", split), ("tab", tab), ("node", node)]
        };
        let push = |h: &mut HostHarness| {
            let root = h.workspace_root();
            h.app.open_builtin_view_pane(
                Box::new(crate::file_browser::FileBrowserView::new(root.clone())),
                crate::app::permissions::PluginPermissions::builtin(),
                root,
                None,
                Some("overlay"),
                None,
            );
            let covered = h.app.windows[0].panes[&a]
                .as_view()
                .is_some_and(|v| v.overlay_replaced.is_some());
            assert!(covered, "fixture: a is covered by an overlay");
        };
        let pop = |h: &mut HostHarness| {
            h.app.set_window_focused_pane(0, a_tile);
            h.app.close_focused();
            let covered = h.app.windows[0].panes[&a]
                .as_view()
                .is_some_and(|v| v.overlay_replaced.is_some());
            assert!(!covered, "fixture: the overlay was popped");
        };
        let refuse_all = |h: &mut HostHarness, plans: &[(&str, TerminalLaunchPlan)], when: &str| {
            for (label, plan) in plans {
                let before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
                let error = apply(h, plan).expect_err(&format!("{label} {when}"));
                assert!(
                    error.starts_with("launch refused"),
                    "{label} {when}: {error}"
                );
                let after = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
                assert_eq!(before, after, "{label} {when}: refused before building");
            }
        };

        let before_push = admit_all(&mut h);
        let before_round_trip = admit_all(&mut h);
        push(&mut h);
        refuse_all(&mut h, &before_push, "after a push");
        let while_covered = admit_all(&mut h);
        pop(&mut h);
        refuse_all(&mut h, &before_round_trip, "after a push and pop");
        refuse_all(
            &mut h,
            &while_covered,
            "admitted while covered, after the pop",
        );

        let fresh = admit_all(&mut h);
        for (label, plan) in &fresh[..1] {
            let pane = apply(&mut h, plan).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(window_of(&h, pane), 0);
        }
    }

    /// Finding 6: the pane collection cannot replace an occupant behind the
    /// window's back. Installing over an occupied id is refused and keeps
    /// the occupant; taking a pane out records its departure.
    #[test]
    fn launch_plan_pane_map_refuses_occupied_slots() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 1);
        let [a] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let generation = h.app.windows[0].tree_history.generation();
        let mut impostor = view_pane(&h, a);
        if let Pane::View(view) = &mut impostor {
            view.name = "impostor".into();
        }
        assert_eq!(
            h.app.windows[0].panes.insert(a, impostor),
            Err(crate::host::context::OccupiedSlot(a))
        );
        assert_eq!(
            h.app.windows[0].panes[&a]
                .as_view()
                .map(|v| v.name.as_str()),
            Some("Test App")
        );
        assert!(!h.app.windows[0].tree_history.departed_since(a, generation));
        assert!(h.app.windows[0].take_pane(a).is_some());
        assert!(h.app.windows[0].tree_history.departed_since(a, generation));
    }

    /// Finding 3 (named context): deleting the named context and creating a
    /// new one with the same name does not satisfy the admitted destination.
    #[test]
    fn launch_plan_refuses_a_recreated_named_context() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 1);
        lay_out(&mut h, 0, panes);
        let work = push_context(&mut h, "work", None);
        let work_win = push_window(&mut h, work, 0);
        let work_panes = views(&mut h, 1);
        lay_out(&mut h, work_win, work_panes);
        let named = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", None, Some("work")))
            .unwrap();
        let idx = h.app.router.position(|c| c.context_id == work).unwrap();
        h.app.delete_context(idx);
        let again = push_context(&mut h, "work", None);
        let again_win = push_window(&mut h, again, 0);
        let again_panes = views(&mut h, 1);
        lay_out(&mut h, again_win, again_panes);
        let error = apply(&mut h, &named).expect_err("recreated context refuses");
        assert!(error.contains("closed"), "{error}");
    }

    /// Round 4, admission scope: a plan binds its credential scope at
    /// admission. A destination root changed from A to B between admission
    /// and construction refuses before any secret is read, any credential
    /// is issued or any backend starts. With A stable, an intentional cwd C
    /// starts, and both credentials resolve scope A, not the cwd.
    #[test]
    fn launch_plan_binds_the_credential_scope_at_admission() {
        crate::app::host_mcp::ensure_test_discovery();
        let mut h = HostHarness::new();
        let panes = views(&mut h, 1);
        let [a] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();
        let cwd_c = tempfile::tempdir().unwrap();
        h.app.set_context_root(root_a.path().to_path_buf(), None);
        let starts = std::rc::Rc::new(std::cell::Cell::new(0));
        let issued = std::rc::Rc::new(std::cell::Cell::new(0));
        let (started, published) = (starts.clone(), issued.clone());
        let _entry = crate::pane_ops::construct::probes::on_backend_entry(move |_| {
            started.set(started.get() + 1);
            Ok(())
        });
        let _publish = crate::pane_ops::construct::probes::on_publish(move |_, _| {
            published.set(published.get() + 1)
        });
        crate::host::shell::secret_reads::take();

        let plan = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(a), None))
            .unwrap();
        assert_eq!(
            plan.grant.scope.workspace_root,
            crate::platform::path::canonical_or_self(root_a.path())
        );
        h.app.set_context_root(root_b.path().to_path_buf(), None);
        let before = h.app.windows[0].panes.len();
        let error = apply(&mut h, &plan).expect_err("a moved scope refuses");
        assert!(error.contains("scope changed after admission"), "{error}");
        assert_eq!(h.app.windows[0].panes.len(), before, "nothing installed");
        assert_eq!(issued.get(), 0, "no credential issued");
        assert_eq!(starts.get(), 0, "no backend started");
        assert_eq!(
            crate::host::shell::secret_reads::take(),
            Vec::new(),
            "no secret read"
        );

        // A to B and back to A between admission and commit: refused by
        // revision, though the path is the admitted one.
        h.app.set_context_root(root_a.path().to_path_buf(), None);
        let plan = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(a), None))
            .unwrap();
        h.app.set_context_root(root_b.path().to_path_buf(), None);
        h.app.set_context_root(root_a.path().to_path_buf(), None);
        let error = apply(&mut h, &plan).expect_err("away and back refuses");
        assert!(error.contains("revision"), "{error}");
        assert_eq!(issued.get(), 0, "no credential issued");
        assert_eq!(starts.get(), 0, "no backend started");

        let mut in_c = spec(&h, "split_h", Some(a), None);
        in_c.cwd = Some(cwd_c.path().to_path_buf());
        let plan = h.app.admit_terminal_launch(&in_c).unwrap();
        let pane = apply(&mut h, &plan).expect("an intentional cwd starts");
        let term = h.app.terminal_incarnation(pane).unwrap();
        assert_eq!(term.initial_cwd.as_deref(), Some(cwd_c.path()));
        let credentials = term.credentials().unwrap();
        let canonical_a = crate::platform::path::canonical_or_self(root_a.path());
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let call = monitor
            .authenticate_credential_for_test(credentials.call_token().expect("credential"))
            .unwrap();
        assert_eq!(call.workspace_root, canonical_a, "call credential scope");
        assert_eq!(
            crate::app::host_mcp::authenticated_workspace_for_test(
                credentials.mcp_token().unwrap()
            ),
            Some(canonical_a),
            "MCP bearer scope"
        );
    }

    /// Finding 4: an outside caller (socket ancestry that resolves to no pane)
    /// cannot take a pane's provenance or ceiling by naming it on the wire.
    #[test]
    fn launch_plan_wire_origin_is_not_provenance() {
        let mut h = HostHarness::new();
        let agent = h.add_focused_terminal();
        h.app.windows[0]
            .panes
            .get_mut(&agent)
            .and_then(|p| p.as_terminal_mut())
            .unwrap()
            .agent_role = Some("lead".into());
        let mut outside = spec(&h, "split_h", Some(agent), None);
        outside.origin = crate::broker::caller::RequestOrigin::outside_for_test();
        let plan = h.app.admit_terminal_launch(&outside).unwrap();
        assert_eq!(plan.caller, None);
        assert_eq!(plan.grant.provenance, Provenance::OutsideProcess);
        assert_eq!(plan.grant.parent, None);
        assert_eq!(plan.grant.spawned_by, "outside");
    }

    /// Finding 4: a launch's source is stamped by the socket handler. A
    /// missing peer capture refuses before anything is built, whatever the
    /// wire names as origin, and no wire field selects or forges a source. A
    /// verified pane, a captured outside peer and a genuine in-process call
    /// each get their own provenance.
    #[test]
    fn launch_plan_socket_source_is_host_stamped() {
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let agent = spawn_tester(&mut h, &root);
        let agent_pid = h.app.windows[window_of(&h, agent)].panes[&agent]
            .as_terminal()
            .unwrap()
            .backend
            .child_pid();
        let wake = std::sync::Arc::new(crate::app::ui_mailbox::RecordingWake::new());
        let (mailbox, rx) = crate::app::ui_mailbox::UiMailbox::channel(wake, "launch-plan-test");
        let response = h.workspace_root().join("unverified-spawn.json");
        let line = |extra: &str| {
            format!(
                r#"{{"type":"spawn_pane","type_id":"terminal","layout":"split_h","from_pane_id":{agent},"response_file":"{}"{extra}}}"#,
                response.display()
            )
        };
        let profile = h.profile_dir().to_path_buf();
        // One line through the socket entry, as the listener delivers it.
        let stamp = |peer: Option<crate::broker::caller::PeerWitness>, extra: &str| {
            super::super::handle_socket_line_in(
                &profile,
                &line(extra),
                &mailbox,
                peer.as_ref(),
                &mut Vec::new(),
            );
            rx.try_recv().ok()
        };

        for forged in [r#","source":"in_process""#, r#","peer_ancestry":[1]"#] {
            assert!(
                stamp(None, forged).is_none(),
                "wire field {forged} is rejected"
            );
        }
        let unverified = stamp(None, "").expect("queued");
        let before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        h.app.service_pane_ipc_message(unverified);
        let reply: Value =
            serde_json::from_str(&std::fs::read_to_string(&response).unwrap()).unwrap();
        assert!(
            reply["error"]
                .as_str()
                .is_some_and(|e| e.contains("could not be")),
            "{reply}"
        );
        let after = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        assert_eq!(before, after, "a refused launch builds nothing");

        // The grant a queued socket launch gets, from its classified caller.
        let admit = |h: &mut HostHarness, message: crate::app::socket_caller::PaneIpcMessage| {
            let crate::app::socket_caller::PaneIpcMessage::Request {
                ingress: crate::app::socket_caller::Ingress::Socket { peer, .. },
                request,
            } = message
            else {
                panic!("expected a socket request");
            };
            let origin = crate::broker::caller::RequestOrigin::Socket(
                h.app.classify_socket_peer(peer.as_ref()),
            );
            h.app
                .admit_terminal_launch(&PaneLaunchSpec::from_spawn_pane(&request, origin).unwrap())
                .unwrap()
                .grant
        };
        let verified = admit(
            &mut h,
            stamp(Some(crate::broker::caller::PeerWitness::for_live_pids(&[agent_pid])), "")
                .unwrap(),
        );
        assert_eq!(
            verified.provenance,
            Provenance::VerifiedPane { pane_id: agent }
        );
        assert_eq!(verified.parent, Some(agent));
        assert_eq!(verified.spawned_by, "agent:tester");
        assert_eq!(
            verified.posture().unwrap(),
            Some(&posture(&["tool.secret"]))
        );
        let outside = admit(
            &mut h,
            stamp(
                Some(crate::broker::caller::PeerWitness::capture(std::process::id())),
                "",
            )
            .unwrap(),
        );
        assert_eq!(
            (outside.provenance.clone(), outside.spawned_by.as_str()),
            (Provenance::OutsideProcess, "outside")
        );
        assert_eq!(outside.posture().unwrap(), None);
        // In-process host code acts on the person's input: the pane it names
        // is placement, not a grant to inherit.
        let internal = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(agent), None))
            .unwrap()
            .grant;
        assert_eq!(internal.provenance, Provenance::DesktopInput);
        assert_eq!(internal.spawned_by, "human");
    }

    /// Finding 5: an outside caller's call is admitted under its origin
    /// window's context. Focus moving to a context with a different root does
    /// not change its grant or its placement; a change to the origin's own
    /// authority root refuses.
    #[test]
    fn launch_plan_outside_agent_keeps_its_origin_across_focus() {
        let mut h = HostHarness::new();
        let first = views(&mut h, 1);
        lay_out(&mut h, 0, first);
        let root_a = h.workspace_root();
        h.app.set_context_root(root_a.clone(), None);
        write_role(&root_a, "");
        let ctx_a = h.app.windows[0].context_id;
        let second = push_window(&mut h, ctx_a, 1);
        let second_panes = views(&mut h, 1);
        lay_out(&mut h, second, second_panes);
        let root_b = tempfile::tempdir().unwrap();
        let ctx_b = push_context(&mut h, "b", None);
        let pos_b = h.app.router.position(|c| c.context_id == ctx_b).unwrap();
        h.app.router.get_mut(pos_b).root = root_b.path().to_path_buf();
        let win_b = push_window(&mut h, ctx_b, 0);
        let b_panes = views(&mut h, 1);
        lay_out(&mut h, win_b, b_panes);

        h.app.active_window = second;
        let plan = admit_tester_call(&mut h, &root_a);
        let refused_plan = admit_tester_call(&mut h, &root_a);
        h.app.active_window = win_b;
        let pane = apply_agent(&mut h, &plan)
            .expect("focus elsewhere does not change the admitted origin");
        assert_eq!(window_of(&h, pane), second);

        let moved = tempfile::tempdir().unwrap();
        let pos_a = h.app.router.position(|c| c.context_id == ctx_a).unwrap();
        h.app.router.get_mut(pos_a).root = moved.path().to_path_buf();
        let error = apply_agent(&mut h, &refused_plan).expect_err("the origin's own root changed");
        assert!(error.contains("changed after admission"), "{error}");
    }

    /// The pane placed just before `pane` in the split that holds it (its
    /// tab wrapper, if any, is the split's child).
    fn placed_after(h: &HostHarness, pane: PaneId) -> Option<PaneId> {
        let win = &h.app.windows[window_of(h, pane)];
        let mut unit = win.tree.tiles.find_pane(&pane)?;
        let split = loop {
            let parent = win.tree.tiles.parent_of(unit)?;
            match win.tree.tiles.get(parent) {
                Some(Tile::Container(egui_tiles::Container::Tabs(_))) => unit = parent,
                Some(Tile::Container(container)) => break container,
                _ => return None,
            }
        };
        let children: Vec<TileId> = split.children().copied().collect();
        let at = children.iter().position(|child| *child == unit)?;
        let before = win.find_first_pane_in(*children.get(at.checked_sub(1)?)?)?;
        match win.tree.tiles.get(before) {
            Some(Tile::Pane(before)) => Some(*before),
            _ => None,
        }
    }

    /// Round 3 item 5: an agent worker lands beside the anchor bound at
    /// admission, in the window bound then, for an unparented call and for a
    /// call into an existing sub-context. Focus moving inside that window
    /// before commit does not move the worker.
    #[test]
    fn launch_plan_agent_worker_lands_beside_its_admitted_anchor() {
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        let panes = views(&mut h, 2);
        let [a, b] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        write_role(&root, "");
        h.app.active_window = 0;
        let a_tile = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
        let b_tile = h.app.windows[0].tree.tiles.find_pane(&b).unwrap();
        h.app.set_window_focused_pane(0, a_tile);
        let plan = admit_tester_call(&mut h, &root);
        h.app.set_window_focused_pane(0, b_tile);
        let worker = apply_agent(&mut h, &plan).expect("launches");
        assert_eq!(window_of(&h, worker), 0);
        assert_eq!(
            placed_after(&h, worker),
            Some(a),
            "beside the admitted anchor, not focus"
        );

        // A caller's second call reuses its sub-context, whose window holds
        // the first worker and one more pane by then.
        // The caller is admitted with `can_call = ['tester']`: delegation is
        // fixed in its grant at admission (stint 1632).
        let caller = spawn_tester_calling(&mut h, &root, "'tester'");
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let token = monitor.issue_credential(
            Some(caller),
            "agent:tester",


        );
        let call = |h: &mut HostHarness| {
            h.app
                .admit_agent_call(&AgentCallRequest {
                    role: "tester",
                    task: "Sub",
                    origin: &h.origin_from_pane(caller),
                    credential: Some(&token),
                    cwd: None,
                    worktree: None,
                    asynchronous: true,
                    handoff: false,
                    response_file: "",
                })
                .expect("child call admits")
        };
        let first = call(&mut h);
        let first_worker = apply_agent(&mut h, &first).expect("first worker");
        let sub_win = window_of(&h, first_worker);
        let first_pane = h.app.windows[sub_win].take_pane(first_worker).unwrap();
        let mut sub_panes = vec![(first_worker, first_pane)];
        sub_panes.extend(views(&mut h, 1));
        let [first_worker_again, other] = lay_out(&mut h, sub_win, sub_panes)[..] else {
            unreachable!()
        };
        assert_eq!(first_worker_again, first_worker);
        let second = call(&mut h);
        assert!(matches!(
            second.subcontext,
            Some(AdmittedSubcontext::Existing { .. })
        ));
        let other_tile = h.app.windows[sub_win].tree.tiles.find_pane(&other).unwrap();
        h.app.set_window_focused_pane(sub_win, other_tile);
        let second_worker = apply_agent(&mut h, &second).expect("second worker");
        assert_eq!(window_of(&h, second_worker), sub_win);
        assert_eq!(
            placed_after(&h, second_worker),
            Some(first_worker),
            "beside the sub-context anchor admitted, not its focus at commit"
        );
    }

    /// Evaluate `target` for `posture` against an empty grant store.
    fn decide(target: &str, posture: Option<&PermissionPosture>) -> crate::broker::Decision {
        let request = crate::broker::PermissionRequest::new(
            crate::broker::ActorType::Agent,
            "probe",
            crate::broker::TargetType::HostTool,
            target,
            None,
        );
        crate::broker::GrantStore::default().evaluate_at(&request, posture, 0)
    }

    /// What the construction probes saw for one pane.
    #[derive(Default)]
    struct Observed {
        published: Option<(PaneId, String, Option<String>)>,
        backend: Option<(Option<PathBuf>, HashMap<String, String>)>,
    }

    /// Install probes that, at the instant credentials publish, authenticate
    /// both through the real call and MCP authenticators and decide an
    /// explicitly denied operation under the binding they resolve; and that
    /// record the backend's cwd and environment at entry. `fail_backend`
    /// fails the start after recording.
    fn observe_construction(
        h: &HostHarness,
        fail_backend: bool,
    ) -> (
        std::rc::Rc<std::cell::RefCell<Observed>>,
        [crate::pane_ops::construct::probes::Guard; 2],
    ) {
        let observed = std::rc::Rc::new(std::cell::RefCell::new(Observed::default()));
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let publish = observed.clone();
        let published = crate::pane_ops::construct::probes::on_publish(move |pane_id, env| {
            let call = env["PLEXI_CALL_CREDENTIAL"].clone();
            let verified = monitor
                .authenticate_credential_for_test(&call)
                .expect("the call credential authenticates at publication");
            assert_eq!(verified.pane_id, Some(pane_id));
            let binding = monitor
                .credential_authority(&call)
                .expect("the call credential carries the pane's incarnation");
            assert_eq!(binding.pane_id(), pane_id);
            assert_eq!(
                decide("tool.secret", binding.grant().posture().unwrap()),
                crate::broker::Decision::Deny,
                "an explicitly denied operation is denied at the first instant"
            );
            let mcp = env.get("PLEXI_HOST_MCP_TOKEN").cloned();
            let mcp_posture = crate::app::host_mcp::authenticated_posture_for_test(
                mcp.as_deref().expect("a pane bearer is published"),
            )
            .expect("the bearer authenticates at publication");
            assert_eq!(mcp_posture, binding.grant().posture().unwrap().cloned());
            publish.borrow_mut().published = Some((pane_id, call, mcp));
        });
        let backend = observed.clone();
        let entered = crate::pane_ops::construct::probes::on_backend_entry(move |settings| {
            backend.borrow_mut().backend =
                Some((settings.working_directory.clone(), settings.env.clone()));
            if fail_backend {
                Err("injected backend failure".into())
            } else {
                Ok(())
            }
        });
        (observed, [published, entered])
    }

    fn start_mcp(h: &HostHarness) {
        let (tx, _rx) = crate::app::ui_mailbox::UiMailbox::channel(
            std::sync::Arc::new(crate::app::ui_mailbox::RecordingWake::new()),
            "launch-plan-mcp",
        );
        crate::app::host_mcp::start_host_mcp_server(tx, h.profile_dir()).expect("mcp server");
    }

    /// Finding 1: credentials authenticate only under the authority binding
    /// installed for the real pane id, from the first instant they exist, and
    /// the backend starts in the bound directory with them. Covered for an
    /// inherited terminal and for a worker.
    #[test]
    fn launch_plan_credentials_publish_under_installed_authority() {
        let mut h = HostHarness::new();
        start_mcp(&h);
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let caller = spawn_tester(&mut h, &root);

        let terminal = h
            .app
            .admit_terminal_launch(&from_caller(&h, "split_h", caller))
            .unwrap();
        let worker = admit_tester_call(&mut h, &root);
        for (label, plan) in [
            (
                "inherited terminal",
                LaunchPlan::Terminal(Box::new(terminal)),
            ),
            ("worker", LaunchPlan::Agent(Box::new(worker))),
        ] {
            let (observed, _guard) = observe_construction(&h, false);
            crate::host::shell::secret_reads::take();
            let pane = h.app.apply_launch(plan).expect(label);
            assert_eq!(
                crate::host::shell::secret_reads::take(),
                Vec::new(),
                "{label}: an agent-requested pane resolves no secret value"
            );
            let observed = observed.borrow();
            let (published, call, _) = observed.published.clone().expect("published");
            assert_eq!(
                published, pane,
                "{label}: credentials of the installed pane"
            );
            let (cwd, env) = observed.backend.clone().expect("backend entered");
            assert_eq!(cwd.as_deref(), Some(root.as_path()), "{label}: bound cwd");
            assert_eq!(env.get("PLEXI_PANE_ID"), Some(&pane.to_string()), "{label}");
            assert_eq!(env.get("PLEXI_CALL_CREDENTIAL"), Some(&call), "{label}");
        }
    }

    /// Finding 1: a backend that fails to start leaves nothing behind: both
    /// credentials captured at publication are stale, the binding is gone,
    /// and no pane, sub-context, window or portal was installed.
    #[test]
    fn launch_plan_failed_backend_revokes_both_credentials() {
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        start_mcp(&h);
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        // The caller is admitted with `can_call = ['tester']`: delegation is
        // fixed in its grant at admission (stint 1632).
        let caller = spawn_tester_calling(&mut h, &root, "'tester'");
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let token = monitor.issue_credential(
            Some(caller),
            "agent:tester",


        );
        let plan = h
            .app
            .admit_agent_call(&AgentCallRequest {
                role: "tester",
                task: "Sub",
                origin: &h.origin_from_pane(caller),
                credential: Some(&token),
                cwd: None,
                worktree: None,
                asynchronous: true,
                handoff: false,
                response_file: "",
            })
            .expect("child call admits");
        assert!(matches!(
            plan.subcontext,
            Some(AdmittedSubcontext::Create { .. })
        ));
        let panes_before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        let (windows_before, contexts_before) = (h.app.windows.len(), h.app.router.len());

        let (observed, _guard) = observe_construction(&h, true);
        let error = apply_agent(&mut h, &plan).expect_err("backend failure refuses");
        assert!(error.contains("injected backend failure"), "{error}");
        let observed = observed.borrow();
        let (pane, call, mcp) = observed
            .published
            .clone()
            .expect("credentials were published");
        assert!(matches!(
            monitor.authenticate_credential_for_test(&call),
            Err(crate::broker::gate::IdentityError::StaleCredential)
        ));
        assert_eq!(
            crate::app::host_mcp::authenticated_posture_for_test(&mcp.unwrap()),
            None,
            "the pane bearer is revoked"
        );
        assert!(monitor.credential_authority(&call).is_none());
        assert!(h.app.find_pane_in_any_window(pane).is_none());
        let panes_after = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        assert_eq!(panes_before, panes_after, "no worker and no portal");
        assert_eq!(
            (h.app.windows.len(), h.app.router.len()),
            (windows_before, contexts_before),
            "no provisional sub-context or window"
        );
    }

    /// What a launch's backend received and which secret values were read.
    struct Launched {
        pane: PaneId,
        cwd: Option<PathBuf>,
        env: HashMap<String, String>,
        reads: Vec<(&'static str, PathBuf)>,
    }

    fn launch_observed(h: &mut HostHarness, spec: &PaneLaunchSpec) -> Launched {
        let plan = h.app.admit_terminal_launch(spec).expect("admits");
        let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
        let record = seen.clone();
        let _entry = crate::pane_ops::construct::probes::on_backend_entry(move |settings| {
            *record.borrow_mut() = Some((settings.working_directory.clone(), settings.env.clone()));
            Ok(())
        });
        crate::host::shell::secret_reads::take();
        let pane = h
            .app
            .apply_launch(LaunchPlan::Terminal(Box::new(plan)))
            .expect("launches");
        let reads = crate::host::shell::secret_reads::take();
        let (cwd, env) = seen.borrow_mut().take().expect("backend entered");
        Launched {
            pane,
            cwd,
            env,
            reads,
        }
    }

    /// Cover `pane` with a builtin overlay, as a user opening an app over
    /// it does.
    fn cover_with_overlay(h: &mut HostHarness, pane: PaneId) {
        let win = window_of(h, pane);
        let tile = h.app.windows[win].tree.tiles.find_pane(&pane).unwrap();
        h.app.active_window = win;
        h.app.set_window_focused_pane(win, tile);
        let root = h.workspace_root();
        h.app.open_builtin_view_pane(
            Box::new(crate::file_browser::FileBrowserView::new(root.clone())),
            crate::app::permissions::PluginPermissions::builtin(),
            root,
            None,
            Some("overlay"),
            None,
        );
        let covered = h.app.windows[win].panes[&pane]
            .as_view()
            .is_some_and(|v| v.overlay_replaced.is_some());
        assert!(covered, "fixture: {pane} is covered by an overlay");
    }

    /// Round 3 item 1: covering a terminal with an overlay does not change
    /// how its live process is authorized. Its socket peer still resolves to
    /// it (so its launches are a pane's: no folder secrets, its ceiling
    /// inherited), and its ceiling and `can_call` are its own, not the
    /// overlay's.
    #[test]
    fn launch_plan_covered_terminal_keeps_its_identity_and_authority() {
        let _features =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let caller = spawn_tester(&mut h, &root);
        let posture = h.app.pane_ceiling_or_none(caller);
        assert!(
            posture.is_some(),
            "fixture: the tester runs under a ceiling"
        );
        let pid = h
            .app
            .terminal_incarnation(caller)
            .unwrap()
            .backend
            .child_pid();
        let context_id = h.app.windows[window_of(&h, caller)].context_id;
        cover_with_overlay(&mut h, caller);

        assert!(
            matches!(
                h.app.classify_socket_peer(Some(
                    &crate::broker::caller::PeerWitness::for_live_pids(&[pid])
                )),
                crate::broker::caller::SocketCaller::Pane(ref found) if found.terminal_pane == caller
            ),
            "the covered terminal's process is still that pane"
        );
        assert_eq!(h.app.pane_ceiling_or_none(caller), posture);
        let mut socket = spec(&h, "split_h", None, None);
        socket.origin = h.app.socket_origin_for_pids(&[pid]);
        let plan = h.app.admit_terminal_launch(&socket).expect("admits");
        assert_eq!(plan.caller.as_ref().map(|c| c.pane_id), Some(caller));
        assert!(
            !plan.grant.secrets.folder && !plan.grant.secrets.workspace_terminal_env,
            "a pane caller's launch gets no secrets"
        );
        assert_eq!(plan.grant.posture().unwrap().cloned(), posture);
        let error = h
            .app
            .check_agent_tool_permission(Some(caller), context_id, "tester", false)
            .expect_err("the tester lists no role in can_call");
        assert!(error.contains("can_call"), "{error}");
    }

    /// Round 3 item 6a: `agent spawn` and `agent open` take their caller
    /// from the host-stamped source through the one resolver. An uncaptured
    /// peer is refused, and a captured peer that is no pane never borrows the
    /// claimed parent's authority or provenance.
    #[test]
    fn launch_plan_agent_spawn_and_open_resolve_the_stamped_source() {
        let _features =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let claimed = spawn_tester(&mut h, &root);
        let request = HostRequest::AgentSpawn {
            anchor_pane: None,
            role: "tester".to_string(),
            harness: "pi".to_string(),
            command: "sleep 30".to_string(),
            task: None,
            subcontext: None,
            ephemeral: false,
            name: None,
            cwd: Some(root.to_string_lossy().into_owned()),
            worktree: None,
            result_schema: None,
            initial_slots: Vec::new(),
            focus: false,
            response_file: String::new(),
        };
        let panes = |h: &HostHarness| h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        let before = panes(&h);
        let unverified = h.app.agent_spawn_reply(&crate::broker::caller::RequestOrigin::unidentified_for_test(), &request);
        assert_eq!(unverified["ok"], false, "{unverified}");
        assert!(
            unverified["error"]
                .as_str()
                .unwrap_or("")
                .contains("could not be identified"),
            "{unverified}"
        );
        assert_eq!(panes(&h), before, "an unverified spawn starts nothing");

        let outside = h
            .app
            .agent_spawn_reply(&crate::broker::caller::RequestOrigin::outside_for_test(), &request);
        assert_eq!(outside["ok"], true, "{outside}");
        assert_eq!(
            outside["parent_pane_id"],
            Value::Null,
            "a captured peer that is no pane is not the claimed parent: {outside}"
        );
        assert_eq!(outside["spawned_by"], "outside", "{outside}");

        // A host gesture names no parent through the request (stint 1285).
        let in_process = h.app.agent_spawn_reply(&crate::broker::caller::RequestOrigin::HostUi, &request);
        assert_eq!(in_process["parent_pane_id"], Value::Null, "{in_process}");
        let _ = claimed;

        let error = h
            .app
            .open_named_agent("tester", &root, Some(claimed), &crate::broker::caller::RequestOrigin::unidentified_for_test())
            .expect_err("an unverified open is refused");
        assert!(error.contains("could not be identified"), "{error}");
    }

    /// Round 3 item 8: a spawn-queue launch has no verifiable identity, so it
    /// resolves no secret of either class; and an app or path `spawn_pane`
    /// is refused on an unverified source exactly as a terminal is.
    #[test]
    fn launch_plan_queue_gets_no_secrets_and_every_target_refuses_unverified() {
        let mut h = HostHarness::new();
        h.add_test_pane();
        // A workspace, so an eligible launch resolves its terminal secrets.
        h.app.set_context_root(h.workspace_root(), None);
        h.app.active_window = 0;
        h.app.windows[0].focused_pane = None;
        // Positive control: the host's own input resolves terminal secrets.
        let desktop = spec(&h, "new_window", None, None);
        let launched = launch_observed(&mut h, &desktop);
        assert!(
            launched.reads.iter().any(|(class, _)| *class == "terminal"),
            "fixture: a desktop launch resolves terminal secrets: {:?}",
            launched.reads
        );
        // An identified outside caller is a socket caller: no secret.
        let mut outside = spec(&h, "new_window", None, None);
        outside.origin = crate::broker::caller::RequestOrigin::outside_for_test();
        let launched = launch_observed(&mut h, &outside);
        assert!(
            launched.reads.is_empty(),
            "an outside caller's launch resolves no secret: {:?}",
            launched.reads
        );
        let mut queued = spec(&h, "new_window", None, None);
        queued.origin = crate::broker::caller::RequestOrigin::SpawnQueue;
        let launched = launch_observed(&mut h, &queued);
        assert!(
            launched.reads.is_empty(),
            "a queue launch resolves no secret: {:?}",
            launched.reads
        );

        for (type_id, path) in [("file_browser", None), ("", Some(h.workspace_root()))] {
            let reply = h
                .workspace_root()
                .join(format!("unverified-{type_id}.json"));
            let mut request: HostRequest = serde_json::from_value(json!({
                "type": "spawn_pane",
                "type_id": type_id,
                "args": [],
                "response_file": reply,
            }))
            .unwrap();
            if let (Some(app_path), HostRequest::SpawnPane { path, .. }) = (&path, &mut request) {
                *path = Some(app_path.to_string_lossy().into_owned());
            }
            let before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
            h.inject_socket(request, None);
            h.hidden_frame();
            let body: Value =
                serde_json::from_str(&std::fs::read_to_string(&reply).expect("a reply")).unwrap();
            assert!(
                body["error"]
                    .as_str()
                    .unwrap_or("")
                    .contains("could not be"),
                "{type_id}: {body}"
            );
            assert_eq!(
                h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>(),
                before,
                "{type_id}: nothing launched"
            );
        }
    }

    /// Finding 4: the directory and secret classes a launch resolves are the
    /// admitted ones, through every placement. An explicit `/` stays `/` for a
    /// new window and an empty root (no home stand-in, so a home-only folder
    /// secret never arrives); a cross-context new window with no cwd starts in
    /// its origin context's root; split, tab and backend-default directories
    /// are the admitted ones; a pane caller's launch reads no folder secret and
    /// an agent caller's reads no secret at all.
    #[test]
    fn launch_plan_directory_and_secrets_follow_admission_through_every_placement() {
        let _features =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let home = dirs::home_dir().expect("home");
        let sentinel = "PLEXI_HOME_ONLY_SENTINEL_1632";
        crate::workspace::secrets::set_folder_secret(
            crate::workspace::secrets::folder_store(),
            sentinel,
            &home,
            "home-only-1632",
        )
        .expect("in-memory folder store");
        assert_eq!(crate::workspace::secrets::backend_label(), "in-memory");

        let mut h = HostHarness::new();
        let panes = views(&mut h, 1);
        let [a] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let root_a = h.workspace_root();
        h.app.set_context_root(root_a.clone(), None);
        let slash = PathBuf::from("/");
        let not_home = |label: &str, launched: &Launched| {
            assert!(
                !launched.env.contains_key(sentinel),
                "{label}: home secret arrived"
            );
            for (class, path) in &launched.reads {
                assert_ne!(path, &home, "{label}: {class} secrets read for home");
            }
        };

        // Explicit `/`: new window and empty root keep it.
        // No origin pane: a view origin would make this an agent-requested
        // launch, which resolves no secret at all.
        h.app.active_window = 0;
        let mut new_window = spec(&h, "new_window", None, None);
        new_window.cwd = Some(slash.clone());
        let launched = launch_observed(&mut h, &new_window);
        assert_eq!(
            launched.cwd.as_deref(),
            Some(slash.as_path()),
            "new window keeps /"
        );
        assert!(
            launched.reads.contains(&("folder", slash.clone())),
            "an eligible launch resolves folder secrets for its own directory: {:?}",
            launched.reads
        );
        not_home("new window", &launched);

        let empty_ctx = push_context(&mut h, "empty", None);
        let empty_win = push_window(&mut h, empty_ctx, 0);
        h.app.active_window = empty_win;
        let mut seed = spec(&h, "split_h", None, None);
        seed.cwd = Some(slash.clone());
        let plan = h.app.admit_terminal_launch(&seed).unwrap();
        assert!(matches!(
            plan.destination,
            TerminalDestination::SeedEmptyRoot { .. }
        ));
        let launched = launch_observed(&mut h, &seed);
        assert_eq!(
            launched.cwd.as_deref(),
            Some(slash.as_path()),
            "empty root keeps /"
        );
        not_home("empty root", &launched);

        // Cross-context new window with no cwd: the origin context's root,
        // whatever context is active.
        let root_b = tempfile::tempdir().unwrap();
        let ctx_b = push_context(&mut h, "b", None);
        let pos_b = h.app.router.position(|c| c.context_id == ctx_b).unwrap();
        h.app.router.get_mut(pos_b).root = root_b.path().to_path_buf();
        let win_b = push_window(&mut h, ctx_b, 0);
        let b_panes = views(&mut h, 1);
        let [b] = lay_out(&mut h, win_b, b_panes)[..] else {
            unreachable!()
        };
        h.app.active_window = 0;
        let mut cross = spec(&h, "new_window", Some(b), None);
        cross.cwd = None;
        let launched = launch_observed(&mut h, &cross);
        assert_eq!(
            launched.cwd.as_deref(),
            Some(root_b.path()),
            "origin context root"
        );

        // Split and tab with no cwd: the admitted anchor directory and the
        // context root.
        h.app.active_window = 0;
        let mut split = spec(&h, "split_h", Some(a), None);
        split.cwd = None;
        let launched = launch_observed(&mut h, &split);
        assert_eq!(
            launched.cwd.as_deref(),
            Some(root_a.as_path()),
            "split anchor cwd"
        );
        let mut tab = spec(&h, "tab", Some(a), None);
        tab.cwd = None;
        let launched = launch_observed(&mut h, &tab);
        assert_eq!(
            launched.cwd.as_deref(),
            Some(root_a.as_path()),
            "tab context root"
        );

        // A split beside a pane with no directory: the backend default, and
        // no directory-derived secret read at all.
        let bare = h.app.host.alloc_pane_id();
        let bare_pane = Pane::Portal(Box::new(PortalPane {
            pane_id: bare,
            target_context_id: empty_ctx,
            context_state: None,
        }));
        let bare_win = push_window(&mut h, ctx_b, 1);
        lay_out(&mut h, bare_win, vec![(bare, bare_pane)]);
        let mut default_cwd = spec(&h, "split_h", Some(bare), None);
        default_cwd.cwd = None;
        let launched = launch_observed(&mut h, &default_cwd);
        assert_eq!(launched.cwd, None, "backend default cwd");
        assert_eq!(launched.reads, Vec::new(), "no directory, no secret read");

        // A socket caller, human-owned pane or agent, gets no secret: only
        // host-owned input does (stint 1285's rule, applied here).
        let human = h.add_focused_terminal();
        let human_pid = h.app.windows[window_of(&h, human)].panes[&human]
            .as_terminal()
            .unwrap()
            .backend
            .child_pid();
        let mut from_pane = spec(&h, "split_h", Some(human), None);
        from_pane.origin = h.app.socket_origin_for_pids(&[human_pid]);
        let launched = launch_observed(&mut h, &from_pane);
        assert_eq!(launched.reads, Vec::new(), "pane caller: no secret read");
        let agent = spawn_tester(&mut h, &root_a);
        let mut from_agent = from_caller(&h, "split_h", agent);
        from_agent.cwd = Some(root_a.clone());
        let launched = launch_observed(&mut h, &from_agent);
        assert_eq!(launched.reads, Vec::new(), "agent caller: no secret read");
        let _ = launched.pane;

        crate::workspace::secrets::remove_folder_secret(
            crate::workspace::secrets::folder_store(),
            sentinel,
            &home,
        )
        .expect("remove the sentinel");
    }

    /// Finding 3: an edit that changes what the worker runs (a harness flag)
    /// refuses the launch even though the posture is unchanged.
    #[test]
    fn launch_plan_agent_call_refuses_a_changed_harness_definition() {
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        write_role(&root, "'tool.secret'");
        let plan = admit_tester_call(&mut h, &root);
        let settings = crate::agent::workspace_agents_dir(&root)
            .join("tester")
            .join("settings.toml");
        let edited = std::fs::read_to_string(&settings).unwrap().replace(
            "ephemeral=false\n",
            "ephemeral=false\nmodel='edited-model'\n",
        );
        std::fs::write(&settings, edited).unwrap();
        let error = apply_agent(&mut h, &plan).expect_err("changed definition refuses");
        assert!(error.contains("changed after admission"), "{error}");
    }

    /// Finding 5: an outside caller's worker lands in the window that was
    /// active at admission, even when it is not its context's first window.
    #[test]
    fn launch_plan_agent_call_lands_in_the_selected_window() {
        let mut h = HostHarness::new();
        let first = views(&mut h, 1);
        lay_out(&mut h, 0, first);
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        write_role(&root, "");
        let ctx = h.app.windows[0].context_id;
        let second = push_window(&mut h, ctx, 1);
        let second_panes = views(&mut h, 1);
        lay_out(&mut h, second, second_panes);
        h.app.active_window = second;
        let plan = admit_tester_call(&mut h, &root);
        h.app.active_window = 0;
        let pane = apply_agent(&mut h, &plan).expect("worker launches");
        assert_eq!(window_of(&h, pane), second);
    }

    /// Finding 5: a create-sub-context intent is bound to the parent context
    /// it was admitted under; a parent now elsewhere refuses.
    #[test]
    fn launch_plan_subcontext_create_is_bound_to_its_parent_context() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 1);
        let [parent] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let elsewhere = h.app.windows[0].context_id + 1_000_000;
        let error = h
            .app
            .resolve_agent_spawn_location(
                Some(SubcontextTarget::Create {
                    name: "agent-tools-x",
                    parent_context_id: elsewhere,
                }),
                Some(parent),
                None,
                false,
                AgentOrigin::Active,
            )
            .expect_err("moved parent refuses");
        assert!(error.contains("was admitted under context"), "{error}");
    }

    /// Finding 7: a handoff publishes the successor's id in the caller's
    /// completion and in the reply, before the caller closes.
    #[test]
    fn launch_plan_handoff_publishes_the_successor() {
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        for role in ["lead", "writer"] {
            let dir = crate::agent::workspace_agents_dir(&root).join(role);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("AGENT.md"), "Work").unwrap();
            std::fs::write(dir.join("settings.toml"), format!("[agent]\nid='{role}'\ndisplay_name='{role}'\n[harness]\ncli='pi'\nephemeral=false\ncan_handoff=['writer']")).unwrap();
        }
        let (caller, _) = h
            .app
            .spawn_agent_worker_pane(
                "lead",
                "pi",
                "sleep 30",
                "Start",
                None,
                false,
                None,
                Some(root.clone()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let token = monitor.issue_credential(
            Some(caller),
            "agent:lead",


        );
        let response = root.join("handoff-successor.json");
        h.app
            .start_agent_tool_call(AgentCallRequest {
                role: "writer",
                task: "Continue",
                origin: &h.origin_from_pane(caller),
                credential: Some(&token),
                cwd: None,
                worktree: None,
                asynchronous: false,
                handoff: true,
                response_file: response.to_str().unwrap(),
            })
            .unwrap();
        let reply: Value =
            serde_json::from_str(&std::fs::read_to_string(&response).unwrap()).unwrap();
        let successor = reply["successor_pane_id"]
            .as_u64()
            .expect("reply names successor");
        assert_eq!(reply["result"]["successor_pane_id"], json!(successor));
        let committed = h
            .app
            .pane_results
            .get(&caller)
            .and_then(|r| r.committed.clone());
        assert_eq!(
            committed.map(|p| p["successor_pane_id"].clone()),
            Some(json!(successor)),
            "the caller's stored completion names the successor"
        );
        let role = h
            .app
            .windows
            .iter()
            .find_map(|w| w.panes.get(&successor))
            .and_then(|p| p.as_terminal())
            .and_then(|t| t.agent_role.clone());
        assert_eq!(role.as_deref(), Some("writer"));
        assert!(h.app.find_pane_in_any_window(caller).is_none());
    }

    /// A lead worker with `can_handoff = ['writer']` and a credential, for
    /// handoff tests.
    fn spawn_lead(h: &mut HostHarness, root: &std::path::Path) -> (PaneId, String) {
        for role in ["lead", "writer"] {
            let dir = crate::agent::workspace_agents_dir(root).join(role);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("AGENT.md"), "Work").unwrap();
            std::fs::write(
                dir.join("settings.toml"),
                format!("[agent]\nid='{role}'\ndisplay_name='{role}'\n[harness]\ncli='pi'\nephemeral=false\ncan_handoff=['writer']"),
            )
            .unwrap();
        }
        let (caller, _) = h
            .app
            .spawn_agent_worker_pane(
                "lead",
                "pi",
                "sleep 30",
                "Start",
                None,
                false,
                None,
                Some(root.to_path_buf()),
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let token = monitor.issue_credential(Some(caller), "agent:lead");
        (caller, token)
    }

    fn require_successor(h: &mut HostHarness, caller: PaneId, successor: Value) {
        require_successor_in(&mut h.app, caller, successor);
    }

    fn require_successor_in(app: &mut PlexiApp, caller: PaneId, successor: Value) {
        app.pane_results.entry(caller).or_default().schema = Some(json!({
            "type": "object",
            "properties": {"successor_pane_id": {"const": successor}},
            "required": ["successor_pane_id"],
        }));
    }

    /// Finding 7: the successor's id is reserved at admission. An unrelated
    /// pane allocated before construction does not move it: the worker, the
    /// committed completion and the reply all name the reserved id. A
    /// completion the predecessor's schema rejects starts no worker and
    /// leaves the predecessor live.
    #[test]
    fn launch_plan_handoff_reserves_its_successor() {
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let (caller, token) = spawn_lead(&mut h, &root);
        fn request<'a>(
            origin: &'a crate::broker::caller::RequestOrigin,
            token: &'a str,
            response: &'a str,
        ) -> AgentCallRequest<'a> {
            AgentCallRequest {
                role: "writer",
                task: "Continue",
                origin,
                credential: Some(token),
                cwd: None,
                worktree: None,
                asynchronous: false,
                handoff: true,
                response_file: response,
            }
        }

        // A schema that names a different successor, written after admission:
        // the check immediately before startup refuses it.
        let rejecting = probes::on_prepared(move |app, _| {
            require_successor_in(app, caller, json!(u64::MAX));
        });
        let starts = std::rc::Rc::new(std::cell::Cell::new(0));
        let counted = starts.clone();
        let _entry = crate::pane_ops::construct::probes::on_backend_entry(move |_| {
            counted.set(counted.get() + 1);
            Ok(())
        });
        let rejected = root.join("handoff-rejected.json");
        let error = h
            .app
            .start_agent_tool_call(request(&h.origin_from_pane(caller), &token, rejected.to_str().unwrap()))
            .expect_err("schema mismatch refuses");
        assert!(error.contains("handoff completion"), "{error}");
        assert_eq!(starts.get(), 0, "no worker started");
        assert!(
            h.app.find_pane_in_any_window(caller).is_some(),
            "predecessor live"
        );
        drop(rejecting);
        h.app.pane_results.entry(caller).or_default().schema = None;

        // The reserved id survives an unrelated allocation before startup.
        let reserved = h.app.host.next_pane_id();
        require_successor(&mut h, caller, json!(reserved));
        let _seam = probes::on_prepared(|app, _| {
            let unrelated = app.host.alloc_pane_id();
            assert_ne!(unrelated, 0);
        });
        let response = root.join("handoff-successor.json");
        h.app
            .start_agent_tool_call(request(&h.origin_from_pane(caller), &token, response.to_str().unwrap()))
            .expect("handoff succeeds");
        let reply: Value =
            serde_json::from_str(&std::fs::read_to_string(&response).unwrap()).unwrap();
        assert_eq!(reply["successor_pane_id"], json!(reserved));
        assert_eq!(reply["result"]["successor_pane_id"], json!(reserved));
        let committed = h
            .app
            .pane_results
            .get(&caller)
            .and_then(|r| r.committed.clone());
        assert_eq!(
            committed.map(|p| p["successor_pane_id"].clone()),
            Some(json!(reserved))
        );
        let role = h
            .app
            .windows
            .iter()
            .find_map(|w| w.panes.get(&reserved))
            .and_then(|p| p.as_terminal())
            .and_then(|t| t.agent_role.clone());
        assert_eq!(
            role.as_deref(),
            Some("writer"),
            "the worker is the reserved pane"
        );
        assert!(h.app.find_pane_in_any_window(caller).is_none());
    }

    fn branch_exists(root: &std::path::Path, branch: &str) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["show-ref", "--verify", "--quiet"])
            .arg(format!("refs/heads/{branch}"))
            .status()
            .expect("git runs")
            .success()
    }

    /// A refused launch disposes of the checkout prepared for it exactly once,
    /// for both routes: a clean checkout and its new branch are reclaimed, a
    /// dirty one is retained with its path and reason, a pre-existing branch
    /// survives; a successful launch hands the checkout to the pane, and one
    /// ordinary close reclaims it.
    #[test]
    fn launch_plan_refused_launch_disposes_of_its_checkout() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 3);
        let [a, b, c] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        init_repo(&root);
        let terminal = |h: &mut HostHarness, from: PaneId, branch: &str| {
            let mut spec = spec(h, "split_h", Some(from), None);
            spec.worktree = Some(branch.to_string());
            LaunchPlan::Terminal(Box::new(h.app.admit_terminal_launch(&spec).unwrap()))
        };
        let refuse = |h: &mut HostHarness, plan: LaunchPlan, invalidate: PaneId| {
            let _seam = probes::on_prepared(move |app, wt| {
                assert!(wt.is_some(), "checkout prepared before revalidation");
                app.close_pane_by_id(invalidate);
            });
            h.app
                .apply_launch(plan)
                .expect_err("invalidated plan refuses")
        };

        // Terminal, clean, new branch: reclaimed, branch deleted.
        let clean = unique_branch("clean");
        let plan = terminal(&mut h, a, &clean);
        let refused = refuse(&mut h, plan, a);
        let report = refused.worktree.expect("one disposition");
        assert_eq!(report.action, "reclaimed", "{report:?}");
        assert!(!report.path.exists(), "checkout removed");
        assert!(!branch_exists(&root, &clean), "new branch deleted");

        // Terminal, pre-existing branch: the checkout goes, the branch stays.
        let kept = unique_branch("kept");
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["branch", &kept])
            .status()
            .unwrap();
        assert!(status.success());
        let plan = terminal(&mut h, b, &kept);
        let refused = refuse(&mut h, plan, b);
        let report = refused.worktree.expect("one disposition");
        assert_eq!(report.action, "reclaimed", "{report:?}");
        assert!(
            branch_exists(&root, &kept),
            "a pre-existing branch survives"
        );

        // Agent, dirty: retained with its path and reason.
        write_role(&root, "'tool.secret'");
        let dirty = unique_branch("dirty");
        let plan = admit_tester_call_in(&mut h, &root, Some(&dirty));
        let root_for_seam = root.clone();
        let _seam = probes::on_prepared(move |_, wt| {
            let wt = wt.expect("checkout prepared");
            std::fs::write(wt.path.join("uncommitted.txt"), "work").unwrap();
            write_role(&root_for_seam, "");
        });
        let refused = h
            .app
            .apply_launch(LaunchPlan::Agent(Box::new(plan)))
            .expect_err("broadened role refuses");
        drop(_seam);
        assert!(
            refused.error.contains("changed after admission"),
            "{}",
            refused.error
        );
        let message = refused.to_string();
        let report = refused.worktree.expect("one disposition, not an orphan");
        assert_eq!(report.action, "retained", "{report:?}");
        assert!(report.reason.is_some(), "{report:?}");
        assert!(report.path.exists(), "the dirty checkout is kept");
        assert!(
            message.contains(&report.path.display().to_string()),
            "{message}"
        );
        remove_checkout(&root, &report.path, &dirty);

        // Success: the pane owns the checkout; one ordinary close reclaims it.
        let owned = unique_branch("owned");
        let plan = terminal(&mut h, c, &owned);
        let pane = h.app.apply_launch(plan).expect("launches in its checkout");
        let path = h
            .app
            .pane_worktrees
            .get(&pane)
            .expect("transferred")
            .path
            .clone();
        assert!(path.exists());
        h.app.close_pane_by_id(pane);
        assert!(!h.app.pane_worktrees.contains_key(&pane));
        assert!(!path.exists(), "the ordinary close reclaimed it");
    }
    // ── Round 3/4 acceptance: one durable grant per pane (DESIGN-R2) ────────

    fn edit_saved_workspace(
        profile: &std::path::Path,
        edit: impl FnOnce(&mut crate::workspace::WorkspaceFile),
    ) {
        let mut ws = crate::workspace::WorkspaceFile::load_from(profile).expect("saved workspace");
        edit(&mut ws);
        ws.save_to(profile).expect("rewrite workspace");
    }

    fn saved_pane_mut(
        ws: &mut crate::workspace::WorkspaceFile,
        pane: PaneId,
    ) -> &mut crate::workspace::SavedPane {
        ws.windows
            .iter_mut()
            .flat_map(|w| w.panes.iter_mut())
            .find(|p| p.id == pane)
            .expect("pane was saved")
    }

    /// A role directory with this settings body.
    fn write_role_settings(root: &std::path::Path, id: &str, body: &str) {
        let dir = crate::agent::workspace_agents_dir(root).join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AGENT.md"), id).unwrap();
        std::fs::write(
            dir.join("settings.toml"),
            format!("[agent]\nid='{id}'\ndisplay_name='{id}'\n[harness]\ncli='pi'\nephemeral=false\n{body}"),
        )
        .unwrap();
    }

    /// A named worker for `role`, under `parent` when given.
    fn worker(
        h: &mut HostHarness,
        root: &std::path::Path,
        role: &str,
        parent: Option<PaneId>,
        sub: Option<&str>,
    ) -> PaneId {
        h.app
            .spawn_agent_worker_pane(
                role,
                "pi",
                "sleep 30",
                "Run",
                sub,
                false,
                parent,
                Some(root.to_path_buf()),
                None,
                None,
                false,
                None,
            )
            .expect("worker starts")
            .0
    }

    /// A plain shell `caller`'s own process asks for over the socket.
    fn shell_from(h: &mut HostHarness, caller: PaneId) -> PaneId {
        let launch = from_caller(h, "split_h", caller);
        let plan = h.app.admit_terminal_launch(&launch).expect("admits");
        apply(h, &plan).expect("plain shell starts")
    }

    fn pid_of(h: &mut HostHarness, pane: PaneId) -> u32 {
        h.app
            .terminal_incarnation(pane)
            .expect("a live terminal")
            .backend
            .child_pid()
    }

    /// The grant `pane` runs under.
    fn grant_of(h: &HostHarness, pane: PaneId) -> crate::broker::pane_authority::PaneAuthority {
        h.app
            .pane_authority(pane)
            .expect("a constructed terminal")
            .grant()
            .clone()
    }

    /// A second context holding one workspace secret; returns its id string.
    fn secret_context(h: &mut HostHarness, root: &std::path::Path) -> String {
        let mut target = h.app.router.active().clone();
        target.context_id = crate::testing::reserve_test_context_id();
        target.root = root.to_path_buf();
        target.name = crate::host::context::ContextName::custom("grant-inventory");
        let id = target.context_id.to_string();
        h.app.router.push(target);
        let cfg = crate::workspace::secrets::init_workspace(
            root,
            &crate::config::workspace_channel_dir(),
        )
        .unwrap();
        let account = crate::workspace::secrets::keychain_workspace_name(&cfg.id, "GRANT_1632");
        crate::workspace::secrets::system_store()
            .set(&account, "not-for-output")
            .unwrap();
        id
    }

    /// The broker's answer to a cross-context secret inventory asked by
    /// `pid`'s pane: `true` when the names were listed without admission.
    fn lists_other_context(h: &mut HostHarness, context: &str, pid: u32) -> bool {
        let response_file = crate::rpc::response_file("grant-1632-list", "json");
        let request = HostRequest::PaneSecretList {
            context: context.into(),
            response_file: response_file.clone(),
        };
        let line = serde_json::to_string(&request).unwrap();
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(&[pid]), &mut Vec::new());
        h.hidden_frame();
        let body: Value =
            serde_json::from_str(&std::fs::read_to_string(response_file).unwrap()).unwrap();
        body["entries"]
            .as_array()
            .is_some_and(|e| e.iter().any(|entry| entry["name"] == "GRANT_1632"))
    }

    /// Round 3 items 2a and 8, round 4 ownership: an agent's plain child and
    /// a spawn-queue shell are restricted from the first instant and after a
    /// restart with no agent-session metadata: no human ownership, no secret
    /// read, and the next cross-context inventory goes through admission. A
    /// desktop shell is the control.
    #[test]
    fn grant_plain_child_and_queue_shell_stay_restricted_through_restart() {
        let profile = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let (desktop, child, queued, grandchild) = {
            let mut h = HostHarness::from_profile_dir(profile.path());
            let desktop = h.add_focused_terminal();
            h.app.set_context_root(root.path().to_path_buf(), None);
            let agent = spawn_tester(&mut h, root.path());
            crate::host::shell::secret_reads::take();
            let child = shell_from(&mut h, agent);
            let mut queue = spec(&h, "new_window", None, None);
            queue.origin = crate::broker::caller::RequestOrigin::SpawnQueue;
            let plan = h.app.admit_terminal_launch(&queue).unwrap();
            let queued = apply(&mut h, &plan).unwrap();
            // The child's own next launch.
            let grandchild = shell_from(&mut h, child);
            assert_eq!(
                crate::host::shell::secret_reads::take(),
                Vec::new(),
                "no ineligible secret resolver read"
            );
            let context = secret_context(&mut h, other.path());
            for (pane, human) in [
                (desktop, true),
                (child, false),
                (queued, false),
                (grandchild, false),
            ] {
                assert_eq!(h.app.terminal_incarnation(pane).is_some_and(|t| t.is_desktop_granted()), human, "pane {pane} live");
                let pid = pid_of(&mut h, pane);
                // Ownership decides nothing: every pane is brokered (stint 1285).
                assert!(!lists_other_context(&mut h, &context, pid), "pane {pane} live");
            }
            assert!(h.app.save_workspace_now(), "saved");
            (desktop, child, queued, grandchild)
        };
        let mut h = HostHarness::from_profile_dir(profile.path());
        let context = secret_context(&mut h, other.path());
        for (pane, human) in [
            (desktop, true),
            (child, false),
            (queued, false),
            (grandchild, false),
        ] {
            assert!(
                h.app
                    .terminal_incarnation(pane)
                    .is_some_and(|t| !t.is_agent_pane()),
                "fixture: pane {pane} restored with no agent session"
            );
            assert_eq!(
                h.app.terminal_incarnation(pane).is_some_and(|t| t.is_desktop_granted()),
                human,
                "pane {pane} restored"
            );
            let pid = pid_of(&mut h, pane);
            assert!(!lists_other_context(&mut h, &context, pid), "pane {pane} restored");
        }
        assert_eq!(
            grant_of(&h, child).secrets,
            crate::broker::pane_authority::SecretEligibility::none()
        );
    }

    /// Round 4 delegation: a plain child and grandchild carry their parent's
    /// `can_call` and `can_handoff` limits, independently, live and after a
    /// restart. A broader role-file edit does not widen a running pane.
    #[test]
    fn grant_delegation_limits_travel_with_plain_children() {
        let profile = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let check = |h: &HostHarness, pane: PaneId, label: &str| {
            let ctx = h.app.windows[window_of(h, pane)].context_id;
            let decide = |role: &str, handoff: bool| {
                h.app
                    .check_agent_tool_permission(Some(pane), ctx, role, handoff)
                    .is_ok()
            };
            assert!(decide("tester", false), "{label}: may call tester");
            assert!(!decide("writer", false), "{label}: may not call writer");
            assert!(decide("writer", true), "{label}: may hand off to writer");
            assert!(
                !decide("tester", true),
                "{label}: may not hand off to tester"
            );
        };
        let (lead, child, grandchild) = {
            let mut h = HostHarness::from_profile_dir(profile.path());
            h.add_test_pane();
            h.app.set_context_root(root.path().to_path_buf(), None);
            write_role_settings(
                root.path(),
                "lead",
                "can_call=['tester']\ncan_handoff=['writer']\n",
            );
            let lead = worker(&mut h, root.path(), "lead", None, None);
            let child = shell_from(&mut h, lead);
            let grandchild = shell_from(&mut h, child);
            for (pane, label) in [(lead, "lead"), (child, "child"), (grandchild, "grandchild")] {
                check(&h, pane, label);
            }
            // A broader definition after admission widens nothing.
            write_role_settings(
                root.path(),
                "lead",
                "can_call=['tester','writer']\ncan_handoff=['writer','tester']\n",
            );
            for (pane, label) in [(lead, "lead"), (child, "child"), (grandchild, "grandchild")] {
                check(&h, pane, label);
            }
            assert!(h.app.save_workspace_now(), "saved");
            (lead, child, grandchild)
        };
        let h = HostHarness::from_profile_dir(profile.path());
        for (pane, label) in [(lead, "lead"), (child, "child"), (grandchild, "grandchild")] {
            check(&h, pane, &format!("{label} restored"));
        }
    }

    /// Round 3 item 2b, round 4 missing role and legacy ancestry. A legacy
    /// grandparent, parent and child saved across windows, with the deny
    /// only on the grandparent's role and no saved parent posture, restores
    /// the child denied at first use, even with an exact allow grant
    /// present. An unreadable child role, a missing parent and a cycle seal
    /// the pane.
    #[test]
    fn grant_legacy_import_keeps_ancestor_restrictions() {
        let profile = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        write_role_settings(
            root.path(),
            "elder",
            "[permissions]\ndefault_posture='ask'\ndeny=['tool.secret']\n",
        );
        write_role_settings(
            root.path(),
            "helper",
            "[permissions]\ndefault_posture='ask'\n",
        );
        let (grandparent, parent, child, plain) = {
            let mut h = HostHarness::from_profile_dir(profile.path());
            h.add_test_pane();
            h.app.set_context_root(root.path().to_path_buf(), None);
            let grandparent = worker(&mut h, root.path(), "elder", None, None);
            let parent = worker(
                &mut h,
                root.path(),
                "helper",
                Some(grandparent),
                Some("mid"),
            );
            let child = worker(&mut h, root.path(), "helper", Some(parent), Some("leaf"));
            let plain = h.add_focused_terminal();
            assert_ne!(window_of(&h, grandparent), window_of(&h, child));
            assert!(h.app.save_workspace_now(), "saved");
            (grandparent, parent, child, plain)
        };
        // A pre-grant save: no records, and no saved posture below the
        // grandparent's role.
        let legacy = |extra: &dyn Fn(&mut crate::workspace::WorkspaceFile)| {
            edit_saved_workspace(profile.path(), |ws| {
                for pane in ws.windows.iter_mut().flat_map(|w| w.panes.iter_mut()) {
                    pane.authority = None;
                    if let Some(session) = pane.agent_session.as_mut() {
                        session.permission_posture = None;
                    }
                }
                extra(ws);
            });
        };
        let exact_allow = |h: &HostHarness, pane: PaneId| {
            let binding = crate::broker::ExactBinding {
                agent_run: None,
                actor: crate::broker::principal::Principal::Pane(
                    h.app.pane_principal(pane).unwrap(),
                )
                .into(),
                actor_scope: crate::broker::ActorScope::Workspace,
                trust_origin: "host".to_string(),
                workspace_root: root.path().to_path_buf(),
                target_type: crate::broker::TargetType::PluginConnector,
                target_id: "tool.secret".to_string(),
                resource_scope: crate::broker::ResourceScope::Game,
                resource_id: Some("game-1".to_string()),
                args_fingerprint: "fp-1632".to_string(),
                session_id: None,
                package_id: "grant-1632".to_string(),
                instance_id: Some(7),
                context_id: Some(3),
                call_id: "grant-1632".to_string(),
                operation_id: String::new(),
            };
            let mut store = crate::broker::GrantStore::default();
            store.record(crate::broker::GrantRecord::from_binding(
                &binding,
                crate::broker::Decision::Allow,
                crate::broker::GrantDuration::Always,
                crate::broker::GrantSource::User,
                "exact allow",
            ));
            let request = crate::broker::PermissionRequest::exact(&binding);
            match h.app.pane_ceiling(pane) {
                Ok(posture) => {
                    assert_eq!(
                        store.evaluate_at(&request, None, 0),
                        crate::broker::Decision::Allow,
                        "fixture: the exact grant allows without a ceiling"
                    );
                    store.evaluate_at(&request, posture.as_ref(), 0)
                }
                Err(_) => crate::broker::Decision::Deny,
            }
        };
        legacy(&|_| {});
        let h = HostHarness::from_profile_dir(profile.path());
        assert_eq!(
            exact_allow(&h, child),
            crate::broker::Decision::Deny,
            "the grandparent's deny beats an exact allow at first use"
        );
        assert!(
            h.app.pane_ceiling(plain).is_err(),
            "a legacy plain shell is sealed"
        );
        assert!(!h.app.terminal_incarnation(plain).is_some_and(|t| t.is_desktop_granted()));
        drop(h);

        // Unreadable child role.
        legacy(&|_| {});
        let broken = crate::agent::workspace_agents_dir(root.path()).join("helper");
        std::fs::write(broken.join("settings.toml"), "not toml [").unwrap();
        let h = HostHarness::from_profile_dir(profile.path());
        assert!(
            h.app.pane_ceiling(child).is_err(),
            "unreadable role: sealed"
        );
        drop(h);
        write_role_settings(
            root.path(),
            "helper",
            "[permissions]\ndefault_posture='ask'\n",
        );

        // Missing parent: the child names a pane that is not in the save.
        legacy(&|ws| {
            saved_pane_mut(ws, child)
                .agent_session
                .as_mut()
                .unwrap()
                .parent_pane_id = Some(parent + 1_000_000);
        });
        let h = HostHarness::from_profile_dir(profile.path());
        assert!(h.app.pane_ceiling(child).is_err(), "missing parent: sealed");
        drop(h);

        // A cycle: the grandparent claims the child as its parent.
        legacy(&|ws| {
            saved_pane_mut(ws, grandparent)
                .agent_session
                .as_mut()
                .unwrap()
                .parent_pane_id = Some(child);
        });
        let h = HostHarness::from_profile_dir(profile.path());
        assert!(h.app.pane_ceiling(child).is_err(), "cycle: sealed");
    }

    /// Round 4 missing role: a direct named-worker launch whose role does
    /// not resolve starts no backend; and a history reopen under a broader
    /// current definition keeps the recorded restriction.
    #[test]
    fn grant_unresolved_named_worker_starts_nothing_and_reopen_keeps_its_record() {
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let starts = std::rc::Rc::new(std::cell::Cell::new(0));
        let started = starts.clone();
        let entry = crate::pane_ops::construct::probes::on_backend_entry(move |_| {
            started.set(started.get() + 1);
            Ok(())
        });
        let before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        let error = h
            .app
            .admit_agent_call(&AgentCallRequest {
                role: "ghost",
                task: "Haunt",
                origin: &crate::broker::caller::RequestOrigin::HostUi,
                credential: None,
                cwd: Some(root.to_str().unwrap()),
                worktree: None,
                asynchronous: true,
                handoff: false,
                response_file: "",
            })
            .expect_err("an unresolved role is refused");
        assert!(error.contains("ghost"), "{error}");
        assert_eq!(starts.get(), 0, "no backend started");
        assert_eq!(
            h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>(),
            before
        );
        drop(entry);

        // A raw spawn whose role label has no definition runs unconfigured,
        // still bounded by its parent's grant.
        let parent = spawn_tester(&mut h, &root);
        let labelled = worker(&mut h, &root, "unlisted-label", Some(parent), None);
        assert!(
            h.app
                .pane_ceiling(labelled)
                .unwrap()
                .is_some_and(|p| p.denies("tool.secret")),
            "an unconfigured worker keeps its parent's ceiling"
        );

        write_role_settings(&root, "tester", "[permissions]\ndefault_posture='ask'\n");
        let recorded = {
            let mut grant = crate::broker::pane_authority::PaneAuthority::for_test(
                7_001,
                crate::broker::pane_authority::Ceiling::Posture {
                    posture: posture(&["record.only"]),
                },
                0,
                root.clone(),
            );
            grant.can_call = crate::broker::pane_authority::Delegation::none();
            serde_json::to_value(grant).unwrap()
        };
        let record = crate::workspace::AgentHistoryRecord {
            fleet_seen: false,
            fleet_dismissed: false,
            chat_archived: false,
            run_id: "run-1632-reopen".into(),
            pane_id: 7_001,
            context_id: h.app.windows[0].context_id,
            context_name: "Test".into(),
            parent_pane_id: None,
            spawned_by: None,
            pane_name: None,
            harness: "pi".into(),
            config_name: Some("tester".into()),
            permission_posture: None,
            authority: Some(recorded),
            session_id: None,
            cwd: root.clone(),
            ephemeral: false,
            last_state: crate::protocol::AgentState::Completed,
            last_blocked_reason: None,
            last_detail: None,
            summary: None,
            result: None,
            tail_preview: None,
            slots: HashMap::new(),
            closed_at: 0,
        };
        let reply = h.app.reopen_agent_record(
            &record,
            None,
            None,
            crate::broker::pane_authority::GrantSource::OutsideProcess,
        );
        let reopened = reply["pane_id"].as_u64().expect("reopened");
        let grant = grant_of(&h, reopened);
        assert!(
            grant
                .posture()
                .unwrap()
                .is_some_and(|p| p.denies("record.only")),
            "the recorded restriction is kept: {grant:?}"
        );
        assert_eq!(
            grant.can_call,
            crate::broker::pane_authority::Delegation::none()
        );
    }

    /// Round 3 item 6a, round 4 history parent: an outside caller's history
    /// resume lands beside the focused pane, with no authenticated parent and
    /// nothing borrowed from that pane.
    #[test]
    fn grant_outside_history_resume_borrows_no_parent() {
        let _features =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        write_role_settings(&root, "helper", "[permissions]\ndefault_posture='ask'\n");
        let focused = spawn_tester(&mut h, &root);
        assert!(h.app.pane_navigate(focused));
        let record = crate::workspace::AgentHistoryRecord {
            fleet_seen: false,
            fleet_dismissed: false,
            chat_archived: false,
            run_id: "run-1632-outside".into(),
            pane_id: 7_002,
            context_id: h.app.windows[0].context_id,
            context_name: "Test".into(),
            parent_pane_id: Some(focused),
            spawned_by: None,
            pane_name: None,
            harness: "pi".into(),
            config_name: Some("helper".into()),
            permission_posture: None,
            authority: None,
            session_id: None,
            cwd: root.clone(),
            ephemeral: false,
            last_state: crate::protocol::AgentState::Completed,
            last_blocked_reason: None,
            last_detail: None,
            summary: None,
            result: None,
            tail_preview: None,
            slots: HashMap::new(),
            closed_at: 0,
        };
        crate::workspace::append_agent_history_to(h.profile_dir(), &record).unwrap();
        let reply_file = h.workspace_root().join("reopen-1632.json");
        // A captured peer that is no pane.
        h.inject_socket(
            HostRequest::AgentReopen {
                query: record.run_id.clone(),
                subcontext: None,
                response_file: reply_file.to_string_lossy().into_owned(),
            },
            Some(crate::broker::caller::PeerWitness::capture(std::process::id())),
        );
        h.hidden_frame();
        let reply: Value =
            serde_json::from_str(&std::fs::read_to_string(&reply_file).unwrap()).unwrap();
        let reopened = reply["pane_id"]
            .as_u64()
            .unwrap_or_else(|| panic!("{reply}"));
        let grant = grant_of(&h, reopened);
        assert_eq!(grant.parent, None, "no authenticated parent");
        assert_eq!(grant.provenance, Provenance::OutsideProcess);
        assert!(
            !grant
                .posture()
                .unwrap()
                .is_some_and(|p| p.denies("tool.secret")),
            "nothing borrowed from the focused tester: {grant:?}"
        );
        assert_eq!(
            h.app.terminal_incarnation(reopened).unwrap().parent_pane_id,
            None
        );
        assert_eq!(
            window_of(&h, reopened),
            window_of(&h, focused),
            "placed beside the focused pane"
        );
    }

    /// Round 4, verified CLI subscription: a raw CLI subscribe from a
    /// verified pane is admitted under that pane's grant, so an explicit
    /// pane deny beats an exact allow grant for the same subscriber, as it
    /// does over MCP. A desktop pane with the same grant is the control.
    #[test]
    fn grant_verified_cli_subscription_is_bounded_by_the_pane() {
        let mut h = HostHarness::new();
        let desktop = h.add_focused_terminal();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let app_id = format!("cli-1632-app-{}", uuid::Uuid::new_v4().simple());
        let target = format!("{app_id}::cli.tick");
        write_role_settings(
            &root,
            "watcher",
            &format!("[permissions]\ndefault_posture='ask'\ndeny=['{target}']\n"),
        );
        let watcher = worker(&mut h, &root, "watcher", None, None);
        let timeline = crate::host::plugin_timeline::global();
        let mut store = crate::broker::GrantStore::default();
        for pane in [desktop, watcher] {
            let context_id = h.app.windows[window_of(&h, pane)].context_id;
            timeline
                .lock()
                .unwrap()
                .declare_streams(
                    context_id,
                    &app_id,
                    vec![crate::protocol::EventStreamDecl {
                        name: "cli.tick".to_string(),
                        schema: json!({"type": "object"}),
                        description: None,
                    }],
                )
                .unwrap();
            let scope = h.app.pane_authority(pane).unwrap().scope().workspace_root;
            store.record(crate::broker::GrantRecord::event_stream_allow(
                &crate::broker::principal::Principal::Pane(h.app.pane_principal(pane).unwrap()),
                crate::broker::ActorScope::User,
                &target,
                &scope,
                crate::broker::GrantDuration::Always,
                crate::broker::GrantSource::User,
                None,
            ));
        }
        h.app.host_subscriptions =
            crate::host::event_subscriptions::HostSubscriptionService::new_for_test(
                store, timeline,
            );
        let subscribe = |h: &mut HostHarness, pane: PaneId| {
            let pid = pid_of(h, pane);
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            h.app
                .admit_subscribe_request(crate::host::event_subscriptions::HostSubscribeRequest {
                    publisher_plugin_id: app_id.clone(),
                    event_names: vec!["cli.tick".into()],
                    payload_mode: crate::protocol::PayloadMode::Full,
                    trigger_mode: crate::protocol::TriggerMode::Conversation,
                    resource_id: None,
                    from_pane_id: None,
                    subscriber_override: Some(format!("cli-conn-{pane}")),
                    principal: crate::broker::principal::Principal::Pane(
                        h.app.pane_principal(pane).unwrap(),
                    ),
                    workspace_root_override: None,
                    subscriber_authority:
                        crate::host::event_subscriptions::SubscriberAuthority::External,
                    context_id_override: None,
                    socket_caller: Some(h.app.classify_socket_peer(Some(
                        &crate::broker::caller::PeerWitness::for_live_pids(&[pid]),
                    ))),
                    cancelled: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                        false,
                    ))),
                    reply,
                });
            receiver.try_recv().expect("answered inline")
        };
        assert!(
            matches!(
                subscribe(&mut h, desktop),
                crate::host::event_subscriptions::HostSubscribeReply::Ok { .. }
            ),
            "fixture: the exact grant admits a pane with no ceiling"
        );
        assert!(
            matches!(
                subscribe(&mut h, watcher),
                crate::host::event_subscriptions::HostSubscribeReply::Err { .. }
            ),
            "the watcher's explicit deny beats the exact grant"
        );
    }

    /// Round 4, live scope: a covered terminal's call credential and MCP
    /// bearer resolve one scope through the pane's handle. A context-root
    /// change publishes a new revision both observe at once, so an exact
    /// grant bound to root A stops applying and one bound to B starts, on
    /// either transport.
    #[test]
    fn grant_live_scope_change_reaches_both_credentials_at_once() {
        crate::app::host_mcp::ensure_test_discovery();
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();
        h.app.set_context_root(root_a.path().to_path_buf(), None);
        let pane = spawn_tester(&mut h, root_a.path());
        cover_with_overlay(&mut h, pane);
        let (call, mcp) = {
            let credentials = h
                .app
                .terminal_incarnation(pane)
                .unwrap()
                .credentials()
                .unwrap();
            (
                credentials.call_token().unwrap().to_string(),
                credentials.mcp_token().unwrap().to_string(),
            )
        };
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        // The workspace each transport reports, and the revision behind it.
        let observe = |h: &HostHarness| {
            let verified = monitor
                .authenticate_credential_for_test(&call)
                .unwrap();
            let over_mcp = crate::app::host_mcp::authenticated_workspace_for_test(&mcp).unwrap();
            let revision = h.app.pane_authority(pane).unwrap().scope().revision;
            (verified.workspace_root, over_mcp, revision)
        };
        let owner = h.app.pane_principal(pane).unwrap();
        let allowed_in = |workspace: &std::path::Path, granted: &std::path::Path| {
            let binding = |root: &std::path::Path| crate::broker::ExactBinding {
                agent_run: None,
                actor: crate::broker::principal::Principal::Pane(owner.clone()).into(),
                actor_scope: crate::broker::ActorScope::Workspace,
                trust_origin: "host".to_string(),
                workspace_root: root.to_path_buf(),
                target_type: crate::broker::TargetType::PluginConnector,
                target_id: "scoped.op".to_string(),
                resource_scope: crate::broker::ResourceScope::Game,
                resource_id: Some("game-1".to_string()),
                args_fingerprint: "fp-scope".to_string(),
                session_id: None,
                package_id: "scope-1632".to_string(),
                instance_id: Some(7),
                context_id: Some(3),
                call_id: "scope-1632".to_string(),
                operation_id: String::new(),
            };
            let mut store = crate::broker::GrantStore::default();
            store.record(crate::broker::GrantRecord::from_binding(
                &binding(granted),
                crate::broker::Decision::Allow,
                crate::broker::GrantDuration::Always,
                crate::broker::GrantSource::User,
                "scoped",
            ));
            store.evaluate_at(
                &crate::broker::PermissionRequest::exact(&binding(workspace)),
                None,
                0,
            ) == crate::broker::Decision::Allow
        };
        let (a, b) = (
            crate::platform::path::canonical_or_self(root_a.path()),
            crate::platform::path::canonical_or_self(root_b.path()),
        );
        let (call_ws, mcp_ws, before) = observe(&h);
        assert_eq!((&call_ws, &mcp_ws), (&a, &a), "both transports start in A");
        assert!(allowed_in(&call_ws, &a) && allowed_in(&mcp_ws, &a));

        let context = h.app.windows[window_of(&h, pane)].context_id;
        h.app
            .set_context_root(root_b.path().to_path_buf(), Some(context));
        let (call_ws, mcp_ws, after) = observe(&h);
        assert_eq!(after, before + 1, "one committed revision");
        assert_eq!((&call_ws, &mcp_ws), (&b, &b), "both transports moved to B");
        assert!(
            !allowed_in(&call_ws, &a) && !allowed_in(&mcp_ws, &a),
            "A's grant no longer applies"
        );
        assert!(
            allowed_in(&call_ws, &b) && allowed_in(&mcp_ws, &b),
            "B's grant applies"
        );
    }

    /// Ledger row 155: a plain shell restored from an older save is sealed,
    /// and says so. Its notice names the reason, a brokered call from it is
    /// refused with that reason rather than a generic permission error, and
    /// the notice's way out opens a desktop pane beside it.
    #[test]
    fn grant_sealed_legacy_shell_names_its_reason_and_offers_a_new_pane() {
        let profile = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let shell = {
            let mut h = HostHarness::from_profile_dir(profile.path());
            let shell = h.add_focused_terminal();
            assert!(h.app.save_workspace_now(), "saved");
            shell
        };
        edit_saved_workspace(profile.path(), |ws| {
            saved_pane_mut(ws, shell).authority = None
        });
        let mut h = HostHarness::from_profile_dir(profile.path());
        let reason = "this pane was restored from an older save; open a new pane";
        assert_eq!(
            h.app.terminal_incarnation(shell).unwrap().sealed_reason(),
            Some(crate::broker::pane_authority::SealReason::OlderSave)
        );
        assert_eq!(
            crate::broker::pane_authority::SealReason::OlderSave.explanation(),
            reason
        );
        // A brokered call over the socket from the sealed shell's process.
        let context = secret_context(&mut h, other.path());
        let pid = pid_of(&mut h, shell);
        let response_file = crate::rpc::response_file("grant-1632-sealed", "json");
        let line = serde_json::to_string(&HostRequest::PaneSecretList {
            context,
            response_file: response_file.clone(),
        })
        .unwrap();
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(&[pid]), &mut Vec::new());
        h.hidden_frame();
        let body = std::fs::read_to_string(response_file).unwrap();
        assert!(
            body.contains(reason),
            "the refusal names the reason: {body}"
        );
        let error = h
            .app
            .check_agent_tool_permission(Some(shell), h.app.windows[0].context_id, "tester", false)
            .expect_err("a sealed pane delegates nothing");
        assert!(error.contains(reason), "{error}");

        let opened = h
            .app
            .open_pane_beside_sealed(shell)
            .expect("a new pane opens");
        assert!(
            h.app.terminal_incarnation(opened).is_some_and(|t| t.is_desktop_granted()),
            "the new pane is the person's"
        );
        assert_eq!(
            h.app.terminal_incarnation(opened).unwrap().sealed_reason(),
            None
        );
        assert!(
            h.app.terminal_incarnation(shell).is_some(),
            "the sealed pane stays"
        );
    }

    /// Round 5 P1-6: a child a shell opens over the socket is never the
    /// person's, before an agent is detected in it (stint 1285) or after, and
    /// its cross-context inventory goes through admission.
    #[test]
    fn grant_detected_agents_plain_child_is_agent_spawned() {
        let mut h = HostHarness::new();
        let shell = h.add_focused_terminal();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let other = tempfile::tempdir().unwrap();
        let context = secret_context(&mut h, other.path());
        let before = shell_from(&mut h, shell);
        assert!(
            !h.app.terminal_incarnation(before).is_some_and(|t| t.is_desktop_granted()),
            "a socket request is never the person, from any shell"
        );
        let win = window_of(&h, shell);
        h.app.windows[win]
            .panes
            .get_mut(&shell)
            .and_then(|p| p.as_terminal_mut())
            .unwrap()
            .agent = Some(crate::protocol::PaneAgentState {
            pane_id: shell,
            state: crate::protocol::AgentState::Working,
            agent: "claude".into(),
            detail: None,
            session_id: None,
        });
        assert!(
            h.app.terminal_incarnation(shell).is_some_and(|t| t.is_agent_pane()),
            "fixture: an agent was detected"
        );
        let child = shell_from(&mut h, shell);
        assert!(
            !h.app.terminal_incarnation(child).is_some_and(|t| t.is_desktop_granted()),
            "the child is agent-spawned"
        );
        let pid = pid_of(&mut h, child);
        assert!(
            !lists_other_context(&mut h, &context, pid),
            "no cross-context inventory without admission"
        );
    }

    /// Round 5 P2-3: a worker started under the team harness still runs
    /// under its defined role's ceiling.
    #[test]
    fn grant_team_harness_worker_keeps_its_roles_ceiling() {
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        write_role_settings(
            &root,
            "squad",
            "[permissions]\ndefault_posture='ask'\ndeny=['tool.secret']\n",
        );
        let (pane, _) = h
            .app
            .spawn_agent_worker_pane(
                "squad",
                "team",
                ":",
                "Coordinate",
                None,
                false,
                None,
                Some(root.clone()),
                None,
                None,
                false,
                None,
            )
            .expect("team coordinator starts");
        assert!(
            h.app
                .pane_ceiling(pane)
                .unwrap()
                .is_some_and(|p| p.denies("tool.secret")),
            "the role's deny applies under the team harness"
        );
    }

    /// Round 5 P1-4: wrapping a live pane into a sub-context, and dissolving
    /// that sub-context back, moves the pane's grant with it as a scope
    /// revision: its bearer reports the context it lives in, and a restart
    /// restores it under its grant instead of sealing it.
    #[test]
    fn grant_wrap_and_dissolve_move_the_grant_and_survive_restart() {
        crate::app::host_mcp::ensure_test_discovery();
        let profile = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let bearer_context = |h: &HostHarness, pane: PaneId| {
            let token = h
                .app
                .terminal_incarnation(pane)
                .unwrap()
                .credentials()
                .unwrap()
                .mcp_token()
                .unwrap()
                .to_string();
            crate::app::host_mcp::authenticated_context_for_test(&token)
        };
        let restarted_ceiling = |pane: PaneId| {
            let h = HostHarness::from_profile_dir(profile.path());
            h.app.pane_ceiling(pane)
        };
        let pane = {
            let mut h = HostHarness::from_profile_dir(profile.path());
            h.add_test_pane();
            h.app.set_context_root(root.path().to_path_buf(), None);
            let pane = spawn_tester(&mut h, root.path());
            let win = window_of(&h, pane);
            h.app.active_window = win;
            let parent = h.app.windows[win].context_id;
            assert!(h
                .app
                .wrap_panes_to_subcontext(&[pane], pane, Some("wrapped".into())));
            let child = h.app.windows[window_of(&h, pane)].context_id;
            assert_ne!(child, parent, "fixture: the pane moved");
            assert_eq!(
                bearer_context(&h, pane),
                Some(child),
                "the bearer follows the wrap"
            );
            assert!(h.app.save_workspace_now(), "saved");
            pane
        };
        let ceiling = restarted_ceiling(pane).expect("a wrapped pane is not sealed on restart");
        assert!(ceiling.is_some_and(|p| p.denies("tool.secret")));
        {
            let mut h = HostHarness::from_profile_dir(profile.path());
            let child = h.app.windows[window_of(&h, pane)].context_id;
            h.app.dissolve_portal(child);
            let now = h.app.windows[window_of(&h, pane)].context_id;
            assert_ne!(now, child, "fixture: the pane moved back");
            assert_eq!(
                bearer_context(&h, pane),
                Some(now),
                "the bearer follows the dissolve"
            );
            assert!(h.app.save_workspace_now(), "saved");
        }
        let ceiling = restarted_ceiling(pane).expect("a dissolved pane is not sealed on restart");
        assert!(ceiling.is_some_and(|p| p.denies("tool.secret")));
    }

    /// Round 5 P2-1: an agent call into an existing sub-context binds that
    /// destination's credential root at admission. A root moved between
    /// admission and commit refuses before any worker starts, although the
    /// caller's own grant root is unchanged; an unchanged root launches.
    #[test]
    fn grant_agent_destination_root_is_bound_at_admission() {
        let _features =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        h.add_test_pane();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        let caller = spawn_tester_calling(&mut h, &root, "'tester'");
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir()).unwrap();
        let token = monitor.issue_credential(
            Some(caller),
            "agent:tester",


        );
        let call = |h: &mut HostHarness| {
            h.app
                .admit_agent_call(&AgentCallRequest {
                    role: "tester",
                    task: "Sub",
                    origin: &h.origin_from_pane(caller),
                    credential: Some(&token),
                    cwd: None,
                    worktree: None,
                    asynchronous: true,
                    handoff: false,
                    response_file: "",
                })
                .expect("child call admits")
        };
        let first = call(&mut h);
        let first_worker = apply_agent(&mut h, &first).expect("first worker");
        let sub_context = h.app.windows[window_of(&h, first_worker)].context_id;
        let second = call(&mut h);
        assert!(matches!(
            second.subcontext,
            Some(AdmittedSubcontext::Existing { .. })
        ));
        let moved = tempfile::tempdir().unwrap();
        let sub_root = h
            .app
            .router
            .iter()
            .find(|c| c.context_id == sub_context)
            .unwrap()
            .root
            .clone();
        // A to B and back to A: the same path, a later revision.
        h.app
            .set_context_root(moved.path().to_path_buf(), Some(sub_context));
        h.app.set_context_root(sub_root, Some(sub_context));
        let before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        let error = apply_agent(&mut h, &second).expect_err("a moved destination root refuses");
        assert!(error.contains("destination scope changed"), "{error}");
        assert_eq!(
            h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>(),
            before,
            "no worker started"
        );
        let third = call(&mut h);
        apply_agent(&mut h, &third).expect("an unchanged destination launches");
    }

    /// Round 6: an outside call into an empty window binds that window's
    /// context revision; its root moving away and back refuses.
    #[test]
    fn grant_empty_destination_window_binds_its_scope_revision() {
        let _features =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
        let mut h = HostHarness::new();
        let root = h.workspace_root();
        h.app.set_context_root(root.clone(), None);
        write_role(&root, "");
        assert!(
            h.app.windows[0].tree.root.is_none(),
            "fixture: an empty window"
        );
        let plan = admit_tester_call(&mut h, &root);
        assert!(matches!(
            plan.destination,
            Some(AgentAnchor { anchor: None, .. })
        ));
        let elsewhere = tempfile::tempdir().unwrap();
        h.app.set_context_root(elsewhere.path().to_path_buf(), None);
        h.app.set_context_root(root.clone(), None);
        let error = apply_agent(&mut h, &plan).expect_err("away and back refuses");
        assert!(error.contains("revision"), "{error}");
        let fresh = admit_tester_call(&mut h, &root);
        apply_agent(&mut h, &fresh).expect("a fresh admission launches");
    }

    /// Round 6, regression against alpha: a desktop shell saved while it ran
    /// a detected agent resumes that agent restricted, with no terminal or
    /// folder secret; a plain desktop shell saved beside it keeps its
    /// eligibility.
    #[test]
    fn grant_restored_detected_agent_session_resolves_no_secrets() {
        let profile = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let (agent, plain) = {
            let mut h = HostHarness::from_profile_dir(profile.path());
            let plain = h.add_focused_terminal();
            let agent = h.add_focused_terminal();
            h.app.set_context_root(root.path().to_path_buf(), None);
            let win = window_of(&h, agent);
            let terminal = h.app.windows[win]
                .panes
                .get_mut(&agent)
                .and_then(|p| p.as_terminal_mut())
                .unwrap();
            terminal.agent_harness = Some("pi".into());
            terminal.agent = Some(crate::protocol::PaneAgentState {
                pane_id: agent,
                state: crate::protocol::AgentState::Idle,
                agent: "pi".into(),
                detail: None,
                session_id: None,
            });
            let saved = grant_of(&h, agent);
            assert!(
                saved.is_desktop() && saved.secrets.workspace_terminal_env,
                "fixture: the shell's grant was desktop and secret-eligible"
            );
            assert!(h.app.save_workspace_now(), "saved");
            (agent, plain)
        };
        let h = HostHarness::from_profile_dir(profile.path());
        let restored = grant_of(&h, agent);
        assert_eq!(
            restored.secrets.policy(),
            crate::host::shell::SecretPolicy::NONE,
            "a resumed agent resolves no terminal or folder secret"
        );
        assert!(!restored.is_desktop(), "a resumed agent is restricted");
        assert!(
            grant_of(&h, plain).secrets.workspace_terminal_env,
            "control"
        );
    }

    /// Round 7: with `restore_mode = "off"` the saved session is dropped and
    /// the pane restores as a plain shell. It keeps its saved desktop grant,
    /// across a save and another restart; only a kept session narrows it.
    #[test]
    fn grant_restore_mode_off_keeps_the_shells_saved_grant() {
        let profile = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let agent = {
            let mut h = HostHarness::from_profile_dir(profile.path());
            let agent = h.add_focused_terminal();
            h.app.set_context_root(root.path().to_path_buf(), None);
            let win = window_of(&h, agent);
            let terminal = h.app.windows[win]
                .panes
                .get_mut(&agent)
                .and_then(|p| p.as_terminal_mut())
                .unwrap();
            terminal.agent_harness = Some("pi".into());
            terminal.agent = Some(crate::protocol::PaneAgentState {
                pane_id: agent,
                state: crate::protocol::AgentState::Idle,
                agent: "pi".into(),
                detail: None,
                session_id: None,
            });
            assert!(h.app.save_workspace_now(), "saved");
            agent
        };
        std::fs::write(
            profile.path().join("config.toml"),
            "[agents]\nrestore_mode = \"off\"\n",
        )
        .unwrap();
        for pass in ["first restart", "after a save and another restart"] {
            let mut h = HostHarness::from_profile_dir(profile.path());
            assert!(
                !h.app.terminal_incarnation(agent).unwrap().is_agent_pane(),
                "fixture: the session was dropped ({pass})"
            );
            let grant = grant_of(&h, agent);
            assert!(
                grant.is_desktop() && grant.secrets.workspace_terminal_env,
                "the plain shell keeps its saved grant ({pass}): {grant:?}"
            );
            assert!(h.app.save_workspace_now(), "saved");
        }
    }
}
