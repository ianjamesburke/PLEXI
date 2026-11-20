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
use crate::broker::PermissionPosture;
use crate::host::pane::PaneLaunchAuthority;
use crate::host::worktree::ManagedWorktree;
use crate::pane_ops::create::{AgentOrigin, AgentWorkerAuthority, SubcontextTarget};
use crate::protocol::SpawnSource;
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

/// What a terminal launch may have, decided at admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalAuthority {
    pub(crate) agent_pane: bool,
    /// Folder secrets for exactly the plan's `cwd` (or its prepared checkout);
    /// never set without a directory bound at admission.
    pub(crate) inject_folder_secrets: bool,
    /// The verified caller pane the posture ceiling came from.
    pub(crate) caller_pane: Option<PaneId>,
    pub(crate) spawned_by: String,
    pub(crate) inherited_posture: Option<PermissionPosture>,
}

#[derive(Clone, Debug)]
pub(crate) struct TerminalLaunchPlan {
    pub(crate) authority: TerminalAuthority,
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
    pane_claim: Option<u64>,
    credential: Option<String>,
    peer_ancestry: Vec<u32>,
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
    pub(crate) worker: AgentWorkerAuthority,
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
    pub(crate) worktree: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum LaunchPlan<'a> {
    Terminal(&'a TerminalLaunchPlan),
    Agent(&'a AgentCallPlan),
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
    /// Commit an admitted plan with its prepared resource (a created
    /// checkout). One path for every terminal and agent launch.
    pub(crate) fn apply_launch(
        &mut self,
        plan: LaunchPlan<'_>,
        prepared: Option<&ManagedWorktree>,
    ) -> Result<PaneId, String> {
        let outcome = match plan {
            LaunchPlan::Terminal(plan) => self.apply_terminal_launch(plan, prepared),
            LaunchPlan::Agent(plan) => self.apply_agent_call(plan, prepared),
        };
        match &outcome {
            Ok(pane_id) => log::info!("launch_plan: applied pane_id={pane_id}"),
            Err(error) => log::info!("launch_plan: refused at commit: {error}"),
        }
        outcome
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

    /// Admit a terminal `spawn_pane`: what the new pane may have, where it
    /// goes and its directory. Names, focus and defaults are resolved here,
    /// once.
    pub(crate) fn admit_terminal_launch(
        &self,
        spec: &PaneLaunchSpec,
    ) -> Result<TerminalLaunchPlan, String> {
        let layout = spec.layout.as_deref().unwrap_or("split_h");
        let active = self.active_window;
        // Provenance, the posture ceiling and secret eligibility come from the
        // host-stamped source. A wire `from_pane_id` is a placement origin; it
        // names the caller only for an in-process request (stint 1632).
        let (caller_pane, requester_is_pane, secrets_eligible) = match &spec.source {
            SpawnSource::Unverified => {
                return Err(refused(
                    "spawn_pane arrived without a verified socket peer identity".into(),
                ))
            }
            SpawnSource::InProcess => (spec.from_pane_id, false, true),
            SpawnSource::SocketPeer(pids) => {
                let peer = self.resolve_socket_peer_pane(pids).map(|(pid, _, _)| pid);
                (peer, peer.is_some(), peer.is_none())
            }
            // Anyone who can write the profile's queue directory: no identity,
            // so no caller pane and no folder secrets.
            SpawnSource::SpawnQueue => (None, false, false),
        };
        let spawned_by = self.derive_spawned_by_for_parent(caller_pane);
        let requester_is_agent = caller_pane.is_some_and(|pid| !self.is_human_owned_pane(pid))
            || spec
                .from_pane_id
                .is_some_and(|pid| !self.is_human_owned_pane(pid));
        let agent_pane = spec.agent.is_some() || requester_is_agent;
        if requester_is_pane {
            log::info!(
                "pane_ipc: spawn_pane requester is a pane (peer_pane={caller_pane:?}, is_agent_pane={agent_pane}); withholding folder secrets"
            );
        }
        let inherited_posture = caller_pane.and_then(|id| self.pane_permission_posture(id));

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
        // Folder secrets belong to one directory decided now: the bound cwd or
        // the checkout prepared for this plan. No directory, no secrets.
        let inject_folder_secrets = secrets_eligible
            && (cwd.is_some() || worktree.is_some())
            && crate::release::feature_enabled(crate::release::ReleaseFeature::FolderSecrets);
        log::info!(
            "launch_plan: admitted terminal destination={destination:?} cwd={cwd:?} agent_pane={agent_pane} inject_folder_secrets={inject_folder_secrets} caller_pane={caller_pane:?} inherits_posture={} worktree={:?}",
            inherited_posture.is_some(),
            worktree.as_ref().map(|(branch, _)| branch)
        );
        Ok(TerminalLaunchPlan {
            authority: TerminalAuthority {
                agent_pane,
                inject_folder_secrets,
                caller_pane,
                spawned_by,
                inherited_posture,
            },
            destination,
            cwd,
            worktree,
            initial_cmd: super::cmd_from_args(&spec.args),
            ephemeral: spec.ephemeral,
            name: spec.name.clone().filter(|name| !name.is_empty()),
            result_schema: spec.result_schema.clone(),
        })
    }

    /// Serve a terminal `spawn_pane`: admit it, prepare its checkout when it
    /// asked for one, commit, and answer the caller. The checkout belongs to
    /// the pane only once the pane exists; a refused commit reclaims it.
    pub(crate) fn launch_terminal_spec(&mut self, spec: &PaneLaunchSpec) {
        let plan = match self.admit_terminal_launch(spec) {
            Ok(plan) => plan,
            Err(error) => {
                log::warn!("pane_ipc: spawn_pane rejected: {error}");
                self.reply_spawn_pane(spec, &Err(error));
                return;
            }
        };
        let prepared = match &plan.worktree {
            Some((branch, anchor)) => {
                match crate::host::worktree::create_worktree(anchor, branch) {
                    Ok(wt) => Some(wt),
                    Err(error) => {
                        log::warn!("pane_ipc: spawn_pane worktree rejected: {error}");
                        self.reply_spawn_pane(spec, &Err(error));
                        return;
                    }
                }
            }
            None => None,
        };
        let outcome = self.apply_launch(LaunchPlan::Terminal(&plan), prepared.as_ref());
        if let Some(wt) = prepared {
            match outcome {
                Ok(pane_id) => {
                    log::info!(
                        "pane_worktree: registered pane_id={pane_id} branch='{}' path='{}'",
                        wt.branch,
                        wt.path.display()
                    );
                    self.pane_worktrees.insert(pane_id, wt);
                }
                Err(_) => {
                    let report = crate::host::worktree::reclaim_or_retain_worktree(&wt);
                    log::info!("pane_worktree: launch refused; worktree disposition {report:?}");
                }
            }
        }
        self.reply_spawn_pane(spec, &outcome);
    }

    /// The posture ceiling a live caller grants now. A caller that closed
    /// keeps the admitted ceiling: it is not required, and the plan never
    /// widens beyond what it recorded.
    fn verify_terminal_authority(&self, authority: &TerminalAuthority) -> Result<(), String> {
        let Some(caller) = authority.caller_pane else {
            return Ok(());
        };
        if self.find_pane_in_any_window(caller).is_none() {
            return Ok(());
        }
        if self.pane_permission_posture(caller) != authority.inherited_posture {
            return Err(refused(format!(
                "caller pane {caller}'s permission posture changed after admission"
            )));
        }
        Ok(())
    }

    fn apply_terminal_launch(
        &mut self,
        plan: &TerminalLaunchPlan,
        prepared: Option<&ManagedWorktree>,
    ) -> Result<PaneId, String> {
        self.verify_terminal_authority(&plan.authority)?;
        let authority = &plan.authority;
        let cwd = prepared.map(|wt| wt.path.clone()).or(plan.cwd.clone());
        // The ceiling and provenance are part of construction, so the shell
        // and its credential never run under a pane without them.
        let options = crate::pane_ops::create::TerminalSpawnOptions {
            initial_cmd: plan.initial_cmd.as_deref(),
            close_on_exit: plan.ephemeral,
            cwd_override: cwd,
            agent_pane: authority.agent_pane,
            inject_folder_secrets: authority.inject_folder_secrets,
            authority: PaneLaunchAuthority {
                posture: authority.inherited_posture.clone(),
                spawned_by: Some(authority.spawned_by.clone()),
            },
        };
        let pane_id = match &plan.destination {
            TerminalDestination::Split {
                at,
                anchor,
                vertical,
                new_pane_first,
                keep_focus,
            } => {
                let win_idx = self.locate_window(*at)?;
                let target_tile = self.locate_split_anchor(win_idx, *at, anchor)?;
                self.spawn_terminal_pane_at(crate::pane_ops::create::SpawnTerminalAtSpec {
                    win_idx,
                    target_tile,
                    vertical: *vertical,
                    new_pane_first: *new_pane_first,
                    keep_focus: *keep_focus,
                    options,
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
                let original_focus = self.windows[win_idx].focused_pane;
                let pane_id = self
                    .new_tab(win_idx, anchor, options)
                    .ok_or_else(|| "failed to open the terminal tab".to_string())?;
                // active_window is left alone: a tab never steals the user's
                // window, only the target window's own focus.
                if *keep_focus {
                    self.restore_window_focused_pane(win_idx, original_focus);
                }
                pane_id
            }
            TerminalDestination::SeedEmptyRoot { at } => {
                let win_idx = self.locate_window(*at)?;
                self.require_empty_tree(win_idx, *at)?;
                self.seed_root_pane(win_idx, options)
                    .ok_or_else(|| "failed to seed root pane".to_string())?
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
                let pane_id = self
                    .create_page_at(grid_x, grid_y, context_id, options)
                    .ok_or_else(|| "failed to create the terminal window".to_string())?;
                if let Some(window_id) = restore_active {
                    if let Some(win_idx) =
                        self.windows.iter().position(|w| w.window_id == *window_id)
                    {
                        self.active_window = win_idx;
                    }
                }
                pane_id
            }
        };
        if self.find_pane_in_any_window(pane_id).is_none() {
            return Err(format!("terminal pane {pane_id} failed to start"));
        }
        if let Some(ref name) = plan.name {
            self.apply_inline_pane_name(pane_id, name);
        }
        // Stamped at construction; this records it in the session trace. The
        // parent link is left alone so no result delivery is implied.
        self.set_pane_spawned_by(pane_id, authority.spawned_by.clone(), None);
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
        let command = crate::cli::agent::with_auto_state_hooks(
            adapter,
            crate::cli::agent::build_agent_spawn_command(&def, adapter, task_text)?,
        );
        let parent_posture = parent.and_then(|id| self.pane_permission_posture(id));
        Ok(AgentCallAuthority {
            parent,
            grant_root,
            role: def.id.clone(),
            harness: adapter.harness.as_str().to_string(),
            command,
            ephemeral: harness.ephemeral,
            result_schema: harness.result_schema.clone(),
            worker: AgentWorkerAuthority::ceiling(def.posture.clone(), parent_posture.as_ref()),
        })
    }

    /// Admit an `agent call`: the authenticated caller, the role's grant,
    /// definition and ceiling, and the destination the caller selected.
    pub(crate) fn admit_agent_call(
        &self,
        request: &AgentCallRequest<'_>,
    ) -> Result<AgentCallPlan, String> {
        if request.task.trim().is_empty() {
            return Err("task must be non-empty".into());
        }
        let caller = AdmittedAgentCaller {
            pane_claim: request.parent_pane_id,
            credential: request.credential.map(str::to_string),
            peer_ancestry: request.peer_ancestry.to_vec(),
        };
        let parent = self.agent_tool_caller(AgentCaller {
            pane_id: caller.pane_claim,
            credential: caller.credential.as_deref(),
            peer_ancestry: &caller.peer_ancestry,
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
        let lookup_root = match (parent, request.cwd) {
            (None, Some(cwd)) => PathBuf::from(cwd),
            _ => grant_root,
        };
        let parent_terminal = parent
            .and_then(|id| self.windows.iter().find_map(|w| w.panes.get(&id)))
            .and_then(|p| p.as_terminal());
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
        if let (true, Some(old)) = (request.handoff, parent) {
            // The completion must name the successor. Validate its shape now,
            // with the id the successor is expected to get; commit validates
            // the real one before anything is published.
            let raw = handoff_completion(&authority.role, request.task, self.host.next_pane_id());
            let mut result_state = self.pane_results.get(&old).cloned().unwrap_or_default();
            result_state
                .write_payload(&raw)
                .map_err(|error| format!("handoff completion: {}", error.message()))?;
        }
        let child_parent = if request.handoff {
            parent_terminal.and_then(|t| t.parent_pane_id)
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
        log::info!(
            "launch_plan: admitted agent_call role={} parent={parent:?} child_parent={child_parent:?} origin={origin:?} subcontext={subcontext:?} handoff={} worktree={:?}",
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
            worktree: request.worktree.map(str::to_string),
        })
    }

    fn apply_agent_call(
        &mut self,
        plan: &AgentCallPlan,
        prepared: Option<&ManagedWorktree>,
    ) -> Result<PaneId, String> {
        let parent = self.agent_tool_caller(AgentCaller {
            pane_id: plan.caller.pane_claim,
            credential: plan.caller.credential.as_deref(),
            peer_ancestry: &plan.caller.peer_ancestry,
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
        let authority = &plan.authority;
        let (pane_id, _) = self.spawn_prepared_agent_worker_pane(
            authority.worker.clone(),
            &authority.role,
            &authority.harness,
            &authority.command,
            &plan.task,
            subcontext,
            authority.ephemeral,
            plan.child_parent,
            Some(cwd),
            authority.result_schema.clone(),
            None,
            true,
            AgentOrigin::Window {
                window_id: plan.origin.window_id,
                context_id: plan.origin.context_id,
            },
        )?;
        Ok(pane_id)
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
                container.children().for_each(|child| child.hash(&mut hasher));
            }
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::{Context, Window};
    use crate::host::keys::Direction;
    use crate::host::pane::{Pane, PortalPane, ViewPane, ViewRuntime};
    use crate::protocol::HostRequest;
    use crate::testing::HostHarness;
    use std::collections::HashMap;

    fn view_pane(h: &HostHarness, pane_id: PaneId) -> Pane {
        let root = h.workspace_root();
        Pane::View(Box::new(ViewPane {
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
            panes: HashMap::new(),
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
            source: crate::protocol::SpawnSource::InProcess,
        })
        .expect("valid terminal spec")
    }

    fn apply(h: &mut HostHarness, plan: &TerminalLaunchPlan) -> Result<PaneId, String> {
        h.app.apply_launch(LaunchPlan::Terminal(plan), None)
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
            h.app
                .seed_window_root_pane(
                    win_idx,
                    &context,
                    h.workspace_root(),
                    crate::pane_ops::create::TerminalSpawnOptions::default(),
                )
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
        let filler = h
            .app
            .new_tab(
                0,
                None,
                crate::pane_ops::create::TerminalSpawnOptions {
                    cwd_override: Some(h.workspace_root()),
                    inject_folder_secrets: false,
                    ..Default::default()
                },
            )
            .expect("filler tab");
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

    /// Finding 6, terminal: a caller's posture is the new pane's ceiling. A
    /// posture that changes between admission and commit refuses; an
    /// unchanged one is installed as admitted.
    #[test]
    fn launch_plan_terminal_authority_does_not_widen_after_admission() {
        let mut h = HostHarness::new();
        let caller = h.add_focused_terminal();
        let set_posture = |h: &mut HostHarness, p: PermissionPosture| {
            h.app.windows[0]
                .panes
                .get_mut(&caller)
                .and_then(|p| p.as_terminal_mut())
                .unwrap()
                .agent_permission_posture = Some(p);
        };
        set_posture(&mut h, posture(&["tool.secret"]));
        let widened = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(caller), None))
            .unwrap();
        let kept = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(caller), None))
            .unwrap();

        set_posture(&mut h, posture(&[]));
        let error = apply(&mut h, &widened).expect_err("broadened caller refuses");
        assert!(error.contains("posture changed"), "{error}");

        set_posture(&mut h, posture(&["tool.secret"]));
        let pane = apply(&mut h, &kept).expect("unchanged authority launches");
        assert_eq!(
            h.app.pane_permission_posture(pane),
            Some(posture(&["tool.secret"]))
        );
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

    fn admit_tester_call(h: &HostHarness, root: &std::path::Path) -> AgentCallPlan {
        h.app
            .admit_agent_call(&AgentCallRequest {
                role: "tester",
                task: "Verify",
                parent_pane_id: None,
                credential: None,
                // An outside-pane caller: authenticated socket, no pane.
                peer_ancestry: &[u32::MAX],
                cwd: Some(root.to_str().unwrap()),
                worktree: None,
                asynchronous: true,
                handoff: false,
                response_file: "",
            })
            .expect("agent call admits")
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
        write_role(&root, "'tool.secret'");
        let checkout = tempfile::tempdir().unwrap();
        write_role(checkout.path(), "");
        let prepared = ManagedWorktree {
            branch: "feature".into(),
            path: checkout.path().to_path_buf(),
            repo_root: root.clone(),
            base_head: "0".repeat(40),
            created_branch: true,
        };

        let plan = admit_tester_call(&h, &root);
        let pane = h
            .app
            .apply_launch(LaunchPlan::Agent(&plan), Some(&prepared))
            .expect("agent launches in the checkout");
        let installed = h
            .app
            .pane_permission_posture(pane)
            .expect("ceiling installed");
        assert_eq!(installed.deny, vec!["tool.secret".to_string()]);

        let plan = admit_tester_call(&h, &root);
        write_role(&root, "");
        let error = h
            .app
            .apply_launch(LaunchPlan::Agent(&plan), None)
            .expect_err("broadened role refuses");
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
                assert!(error.starts_with("launch refused"), "{label} {when}: {error}");
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
        refuse_all(&mut h, &while_covered, "admitted while covered, after the pop");

        let fresh = admit_all(&mut h);
        for (label, plan) in &fresh[..1] {
            let pane = apply(&mut h, plan).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(window_of(&h, pane), 0);
        }
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

    /// Finding 4: the directory a launch runs in (and takes folder secrets
    /// for) is decided at admission. A split's anchor or a tab's context root
    /// changing afterwards does not move it.
    #[test]
    fn launch_plan_binds_the_directory_at_admission() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 1);
        let [a] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let admitted_dir = tempfile::tempdir().unwrap();
        let later_dir = tempfile::tempdir().unwrap();
        let set_view_root = |h: &mut HostHarness, dir: &std::path::Path| {
            if let Some(Pane::View(view)) = h.app.windows[0].panes.get_mut(&a) {
                view.workspace_root = dir.to_path_buf();
            }
        };
        set_view_root(&mut h, admitted_dir.path());
        h.app
            .set_context_root(admitted_dir.path().to_path_buf(), None);
        let mut split_spec = spec(&h, "split_h", Some(a), None);
        split_spec.cwd = None;
        let mut tab_spec = spec(&h, "tab", Some(a), None);
        tab_spec.cwd = None;
        let split = h.app.admit_terminal_launch(&split_spec).unwrap();
        let tab = h.app.admit_terminal_launch(&tab_spec).unwrap();
        assert_eq!(split.cwd.as_deref(), Some(admitted_dir.path()));

        set_view_root(&mut h, later_dir.path());
        h.app.set_context_root(later_dir.path().to_path_buf(), None);
        for (label, plan) in [("split", &split), ("tab", &tab)] {
            let pane = apply(&mut h, plan).expect(label);
            let initial = h.app.windows[window_of(&h, pane)].panes[&pane]
                .as_terminal()
                .and_then(|t| t.initial_cwd.clone());
            assert_eq!(initial.as_deref(), Some(admitted_dir.path()), "{label}");
        }
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
        outside.source = SpawnSource::SocketPeer(vec![u32::MAX]);
        let plan = h.app.admit_terminal_launch(&outside).unwrap();
        assert_eq!(plan.authority.caller_pane, None);
        assert_eq!(plan.authority.spawned_by, "human");
    }

    /// Finding 4: a launch's source is stamped by the socket handler. A
    /// missing peer capture refuses before anything is built, whatever the
    /// wire names as origin, and no wire field selects or forges a source. A
    /// verified pane, a captured outside peer and a genuine in-process call
    /// each get their own provenance.
    #[test]
    fn launch_plan_socket_source_is_host_stamped() {
        let mut h = HostHarness::new();
        let agent = h.add_focused_terminal();
        let term = h.app.windows[0]
            .panes
            .get_mut(&agent)
            .and_then(|p| p.as_terminal_mut())
            .unwrap();
        term.agent_role = Some("lead".into());
        term.agent_permission_posture = Some(posture(&["tool.secret"]));
        let agent_pid = term.backend.child_pid();
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
        let stamp = |ancestry: Option<&[u32]>, extra: &str| {
            super::super::handle_socket_line_in(
                &profile,
                &line(extra),
                &mailbox,
                ancestry,
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
        let stamped_unverified = matches!(
            &unverified,
            HostRequest::SpawnPane {
                source: SpawnSource::Unverified,
                ..
            }
        );
        let before = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        h.app.handle_pane_ipc_request(unverified);
        let reply: Value =
            serde_json::from_str(&std::fs::read_to_string(&response).unwrap()).unwrap();
        assert!(
            reply["error"]
                .as_str()
                .is_some_and(|e| e.contains("without a verified socket peer")),
            "{reply}"
        );
        let after = h.app.windows.iter().map(|w| w.panes.len()).sum::<usize>();
        assert_eq!(before, after, "a refused launch builds nothing");
        assert!(
            stamped_unverified,
            "an uncaptured peer is stamped Unverified"
        );

        let admit = |h: &HostHarness, request: HostRequest| {
            h.app
                .admit_terminal_launch(&PaneLaunchSpec::from_spawn_pane(&request).unwrap())
                .unwrap()
                .authority
        };
        let verified = admit(&h, stamp(Some(&[agent_pid]), "").unwrap());
        assert_eq!(verified.caller_pane, Some(agent));
        assert_eq!(verified.spawned_by, "agent:lead");
        assert_eq!(verified.inherited_posture, Some(posture(&["tool.secret"])));
        let outside = admit(&h, stamp(Some(&[u32::MAX]), "").unwrap());
        assert_eq!(
            (outside.caller_pane, outside.spawned_by.as_str()),
            (None, "human")
        );
        assert_eq!(outside.inherited_posture, None);
        let internal = h
            .app
            .admit_terminal_launch(&spec(&h, "split_h", Some(agent), None))
            .unwrap()
            .authority;
        assert_eq!(internal.caller_pane, Some(agent));
        assert_eq!(internal.spawned_by, "agent:lead");
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
        let plan = admit_tester_call(&h, &root_a);
        let refused_plan = admit_tester_call(&h, &root_a);
        h.app.active_window = win_b;
        let pane = h
            .app
            .apply_launch(LaunchPlan::Agent(&plan), None)
            .expect("focus elsewhere does not change the admitted origin");
        assert_eq!(window_of(&h, pane), second);

        let moved = tempfile::tempdir().unwrap();
        let pos_a = h.app.router.position(|c| c.context_id == ctx_a).unwrap();
        h.app.router.get_mut(pos_a).root = moved.path().to_path_buf();
        let error = h
            .app
            .apply_launch(LaunchPlan::Agent(&refused_plan), None)
            .expect_err("the origin's own root changed");
        assert!(error.contains("changed after admission"), "{error}");
    }

    /// Finding 1: the creation primitives construct the pane with its
    /// ceiling and provenance, so no pane exists, even inside one call,
    /// without them.
    #[test]
    fn launch_plan_terminal_is_constructed_with_its_authority() {
        let mut h = HostHarness::new();
        let panes = views(&mut h, 1);
        let [a] = lay_out(&mut h, 0, panes)[..] else {
            unreachable!()
        };
        let tile = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
        let pane = h
            .app
            .spawn_terminal_pane_at(crate::pane_ops::create::SpawnTerminalAtSpec {
                win_idx: 0,
                target_tile: tile,
                vertical: true,
                new_pane_first: false,
                keep_focus: true,
                options: crate::pane_ops::create::TerminalSpawnOptions {
                    cwd_override: Some(h.workspace_root()),
                    inject_folder_secrets: false,
                    authority: PaneLaunchAuthority {
                        posture: Some(posture(&["tool.secret"])),
                        spawned_by: Some("agent:lead".into()),
                    },
                    ..Default::default()
                },
            });
        let term = h.app.windows[0].panes[&pane].as_terminal().unwrap();
        assert_eq!(
            term.agent_permission_posture,
            Some(posture(&["tool.secret"]))
        );
        assert_eq!(term.spawned_by.as_deref(), Some("agent:lead"));
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
        let plan = admit_tester_call(&h, &root);
        let settings = crate::agent::workspace_agents_dir(&root)
            .join("tester")
            .join("settings.toml");
        let edited = std::fs::read_to_string(&settings).unwrap().replace(
            "ephemeral=false\n",
            "ephemeral=false\nmodel='edited-model'\n",
        );
        std::fs::write(&settings, edited).unwrap();
        let error = h
            .app
            .apply_launch(LaunchPlan::Agent(&plan), None)
            .expect_err("changed definition refuses");
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
        let plan = admit_tester_call(&h, &root);
        h.app.active_window = 0;
        let pane = h
            .app
            .apply_launch(LaunchPlan::Agent(&plan), None)
            .expect("worker launches");
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
            h.app.windows[0].context_id,
            &root,
            "agent:lead",
        );
        let response = root.join("handoff-successor.json");
        h.app
            .start_agent_tool_call(AgentCallRequest {
                role: "writer",
                task: "Continue",
                parent_pane_id: Some(caller),
                credential: Some(&token),
                peer_ancestry: &[],
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
}
