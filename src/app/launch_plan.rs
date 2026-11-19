//! Launch plans (stint 1632). A terminal `spawn_pane` or an `agent call` is
//! admitted once into a [`LaunchPlan`]: who asked, what authority the new pane
//! gets, and where it goes. [`PlexiApp::apply_launch`] commits a plan. It
//! re-checks the destination and the authority, refuses when either changed,
//! and returns the pane it actually created.
//!
//! A destination names what the caller selected (a window inside a context,
//! an anchor pane, an empty tree at a generation), never a `TileId` or a
//! window index: tile ids restart when a tree is rebuilt and indices shift.
//! Pane, window and context ids are allocated monotonically for the host's
//! lifetime, and a window leaves its context only when that context is
//! dissolved, so `(window_id, context_id)` cannot be satisfied by a moved
//! window.

use std::path::PathBuf;

use egui_tiles::{Tile, TileId};
use serde_json::{json, Value};

use super::agent_tools::{AgentCallRequest, AgentCaller};
use super::launch_spec::PaneLaunchSpec;
use super::PlexiApp;
use crate::broker::PermissionPosture;
use crate::host::worktree::ManagedWorktree;
use crate::pane_ops::create::{AgentWorkerAuthority, SubcontextTarget};
use crate::spatial::tiling::PaneId;

/// A window as the caller selected it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowMembership {
    pub(crate) window_id: u64,
    pub(crate) context_id: u64,
}

/// The node a split lands beside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SplitAnchor {
    /// A pane, wherever its tile is at commit.
    Pane(PaneId),
    /// A container node, valid only in the tree it was selected from.
    Node { tile: TileId, tree_generation: u64 },
}

/// The tab group a tab joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabAnchor {
    Pane(PaneId),
    EmptyRoot { tree_generation: u64 },
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
        tree_generation: u64,
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
    /// Explicit cwd, or the default the destination had at admission.
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

/// The inputs that authorize an agent launch. Admission records them; commit
/// derives them again and refuses on any difference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentCallAuthority {
    pub(crate) parent: Option<PaneId>,
    /// Context root the caller's `can_call`/`can_handoff` grant was read from.
    pub(crate) grant_root: PathBuf,
    pub(crate) harness: String,
    pub(crate) worker: AgentWorkerAuthority,
}

#[derive(Clone, Debug)]
pub(crate) enum AdmittedSubcontext {
    Existing { name: String, context_id: u64 },
    Create { name: String },
}

#[derive(Clone, Debug)]
pub(crate) struct AgentCallPlan {
    caller: AdmittedAgentCaller,
    requested_role: String,
    pub(crate) handoff: bool,
    pub(crate) authority: AgentCallAuthority,
    /// The role id the definition resolved to.
    pub(crate) role: String,
    pub(crate) command: String,
    pub(crate) task: String,
    pub(crate) ephemeral: bool,
    pub(crate) result_schema: Option<Value>,
    pub(crate) lookup_root: PathBuf,
    /// The pane the worker hangs under (the caller, or its parent on handoff).
    pub(crate) child_parent: Option<PaneId>,
    pub(crate) subcontext: Option<AdmittedSubcontext>,
    pub(crate) origin_context: u64,
    pub(crate) worktree: Option<String>,
    /// Prevalidated completion published for the caller on handoff.
    pub(crate) handoff_result: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum LaunchPlan<'a> {
    Terminal(&'a TerminalLaunchPlan),
    Agent(&'a AgentCallPlan),
}

fn refused(what: String) -> String {
    format!("launch refused: {what}")
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
        WindowMembership {
            window_id: self.windows[win_idx].window_id,
            context_id: self.windows[win_idx].context_id,
        }
    }

    fn split_anchor(&self, win_idx: usize, tile: TileId) -> SplitAnchor {
        let win = &self.windows[win_idx];
        match win.tree.tiles.get(tile) {
            Some(Tile::Pane(pane_id)) => SplitAnchor::Pane(*pane_id),
            _ => SplitAnchor::Node {
                tile,
                tree_generation: win.tree_generation,
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

    fn locate_split_anchor(&self, win_idx: usize, anchor: SplitAnchor) -> Result<TileId, String> {
        let win = &self.windows[win_idx];
        match anchor {
            SplitAnchor::Pane(pane_id) => win
                .panes
                .contains_key(&pane_id)
                .then(|| win.tree.tiles.find_pane(&pane_id))
                .flatten()
                .ok_or_else(|| {
                    refused(format!(
                        "anchor pane {pane_id} is no longer in window {}",
                        win.window_id
                    ))
                }),
            SplitAnchor::Node {
                tile,
                tree_generation,
            } => (win.tree_generation == tree_generation && win.tree.tiles.get(tile).is_some())
                .then_some(tile)
                .ok_or_else(|| {
                    refused(format!(
                        "node {tile:?} of window {} was rebuilt (generation {tree_generation} -> {})",
                        win.window_id, win.tree_generation
                    ))
                }),
        }
    }

    fn require_empty_tree(&self, win_idx: usize, tree_generation: u64) -> Result<(), String> {
        let win = &self.windows[win_idx];
        if win.tree.root.is_some() || win.tree_generation != tree_generation {
            return Err(refused(format!(
                "window {} is no longer the empty tree the launch was admitted for (generation {tree_generation} -> {})",
                win.window_id, win.tree_generation
            )));
        }
        Ok(())
    }

    /// Admit a terminal `spawn_pane`: what the new pane may have and where it
    /// goes. Names and implicit focus are resolved here, once.
    pub(crate) fn admit_terminal_launch(
        &self,
        spec: &PaneLaunchSpec,
    ) -> Result<TerminalLaunchPlan, String> {
        let layout = spec.layout.as_deref().unwrap_or("split_h");
        let active = self.active_window;
        let peer_pane = spec
            .peer_ancestry
            .as_deref()
            .filter(|pids| !pids.is_empty())
            .and_then(|pids| self.resolve_socket_peer_pane(pids).map(|(pid, _, _)| pid));
        let requester_is_pane = peer_pane.is_some();
        let requester_is_agent = peer_pane.is_some_and(|pid| !self.is_human_owned_pane(pid))
            || spec
                .from_pane_id
                .is_some_and(|pid| !self.is_human_owned_pane(pid));
        let agent_pane = spec.agent.is_some() || requester_is_agent;
        let inject_folder_secrets = !requester_is_pane
            && crate::release::feature_enabled(crate::release::ReleaseFeature::FolderSecrets);
        if requester_is_pane {
            log::info!(
                "pane_ipc: spawn_pane requester is a pane (peer_pane={peer_pane:?}, is_agent_pane={agent_pane}); withholding folder secrets"
            );
        }
        // Provenance comes from the verified socket peer, never the wire.
        let (caller_pane, spawned_by) =
            self.resolve_unspoofable_spawner(spec.from_pane_id, spec.peer_ancestry.as_deref());
        let inherited_posture = caller_pane.and_then(|id| self.pane_permission_posture(id));

        let vertical = matches!(layout, "split_h" | "split_right" | "split_left");
        let new_pane_first = matches!(layout, "split_above" | "split_left");
        let caller_win = spec
            .from_pane_id
            .and_then(|id| self.find_pane_in_any_window(id).map(|(win, _)| win));
        let mut default_cwd = None;
        let destination = if let Some(ctx_name) =
            spec.context_name.as_deref().filter(|c| !c.is_empty())
        {
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
            TerminalDestination::Split {
                at: self.window_membership(win),
                anchor: self.split_anchor(win, tile),
                vertical,
                new_pane_first,
                keep_focus: spec.no_focus,
            }
        } else if layout == "new_window" {
            let win = caller_win.unwrap_or(active);
            default_cwd = Some(self.router.active().root.clone());
            TerminalDestination::NewWindow {
                context_id: self.windows[win].context_id,
                grid_y: self.windows[win].grid_y,
                restore_active: spec.no_focus.then_some(self.windows[active].window_id),
            }
        } else if layout == "tab" {
            let win = caller_win.unwrap_or(active);
            let anchor = if self.windows[win].panes.is_empty() {
                TabAnchor::EmptyRoot {
                    tree_generation: self.windows[win].tree_generation,
                }
            } else {
                TabAnchor::Pane(self.focused_tab_anchor(win).ok_or_else(|| {
                    format!(
                        "window {} has no focused pane to open a tab beside",
                        self.windows[win].window_id
                    )
                })?)
            };
            TerminalDestination::Tab {
                at: self.window_membership(win),
                anchor,
                keep_focus: spec.no_focus,
            }
        } else {
            // from_pane_id wins (cross-window), then the active window's focus.
            let split_target = match spec.from_pane_id {
                Some(from_id) => match self.find_pane_in_any_window(from_id) {
                    Some(location) => Some(location),
                    None => {
                        log::warn!(
                            "pane_ipc: spawn_pane: from_pane_id={from_id} not found in any window, using focused pane"
                        );
                        None
                    }
                },
                None => None,
            }
            .or_else(|| {
                self.windows[active]
                    .focused_pane
                    .or(self.windows[active].tree.root)
                    .map(|tile| (active, tile))
            });
            match split_target {
                Some((win, tile)) => TerminalDestination::Split {
                    at: self.window_membership(win),
                    anchor: self.split_anchor(win, tile),
                    vertical,
                    new_pane_first,
                    keep_focus: spec.no_focus || spec.from_pane_id.is_some(),
                },
                None => {
                    default_cwd = Some(self.router.active().root.clone());
                    TerminalDestination::SeedEmptyRoot {
                        at: self.window_membership(active),
                        tree_generation: self.windows[active].tree_generation,
                    }
                }
            }
        };
        let worktree = spec
            .worktree
            .as_ref()
            .map(|branch| (branch.clone(), self.resolve_worktree_anchor(spec)));
        log::info!(
            "launch_plan: admitted terminal destination={destination:?} agent_pane={agent_pane} inject_folder_secrets={inject_folder_secrets} caller_pane={caller_pane:?} inherits_posture={} worktree={:?}",
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
            cwd: spec.cwd.clone().or(default_cwd),
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
        let options = |cwd_override| crate::pane_ops::create::TerminalSpawnOptions {
            initial_cmd: plan.initial_cmd.as_deref(),
            close_on_exit: plan.ephemeral,
            cwd_override,
            agent_pane: authority.agent_pane,
            inject_folder_secrets: authority.inject_folder_secrets,
        };
        let cwd = prepared.map(|wt| wt.path.clone()).or(plan.cwd.clone());
        let pane_id = match plan.destination {
            TerminalDestination::Split {
                at,
                anchor,
                vertical,
                new_pane_first,
                keep_focus,
            } => {
                let win_idx = self.locate_window(at)?;
                let target_tile = self.locate_split_anchor(win_idx, anchor)?;
                self.spawn_terminal_pane_at(crate::pane_ops::create::SpawnTerminalAtSpec {
                    win_idx,
                    target_tile,
                    vertical,
                    new_pane_first,
                    keep_focus,
                    options: options(cwd),
                })
            }
            TerminalDestination::Tab {
                at,
                anchor,
                keep_focus,
            } => {
                let win_idx = self.locate_window(at)?;
                let anchor = match anchor {
                    TabAnchor::Pane(pane_id) => {
                        self.locate_split_anchor(win_idx, SplitAnchor::Pane(pane_id))?;
                        Some(pane_id)
                    }
                    TabAnchor::EmptyRoot { tree_generation } => {
                        self.require_empty_tree(win_idx, tree_generation)?;
                        None
                    }
                };
                let original_focus = self.windows[win_idx].focused_pane;
                let pane_id = self
                    .new_tab(
                        win_idx,
                        anchor,
                        plan.initial_cmd.as_deref(),
                        plan.ephemeral,
                        cwd,
                        authority.agent_pane,
                        authority.inject_folder_secrets,
                    )
                    .ok_or_else(|| "failed to open the terminal tab".to_string())?;
                // active_window is left alone: a tab never steals the user's
                // window, only the target window's own focus.
                if keep_focus {
                    self.restore_window_focused_pane(win_idx, original_focus);
                }
                pane_id
            }
            TerminalDestination::SeedEmptyRoot {
                at,
                tree_generation,
            } => {
                let win_idx = self.locate_window(at)?;
                self.require_empty_tree(win_idx, tree_generation)?;
                self.seed_root_pane(
                    win_idx,
                    plan.initial_cmd.as_deref(),
                    plan.ephemeral,
                    cwd,
                    authority.agent_pane,
                    authority.inject_folder_secrets,
                )
                .ok_or_else(|| "failed to seed root pane".to_string())?
            }
            TerminalDestination::NewWindow {
                context_id,
                grid_y,
                restore_active,
            } => {
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
                    .create_page_at(grid_x, grid_y, context_id, options(cwd))
                    .ok_or_else(|| "failed to create the terminal window".to_string())?;
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
        if self.find_pane_in_any_window(pane_id).is_none() {
            return Err(format!("terminal pane {pane_id} failed to start"));
        }
        if let Some(ref name) = plan.name {
            self.apply_inline_pane_name(pane_id, name);
        }
        // The parent link is left alone so no result delivery is implied.
        self.set_pane_spawned_by(pane_id, authority.spawned_by.clone(), None);
        if let Some(ref ceiling) = authority.inherited_posture {
            if let Some(term) = self
                .windows
                .iter_mut()
                .find_map(|w| w.panes.get_mut(&pane_id))
                .and_then(|p| p.as_terminal_mut())
            {
                term.agent_permission_posture = Some(match term.agent_permission_posture.take() {
                    Some(own) => own.restricted_by(ceiling),
                    None => ceiling.clone(),
                });
            }
        }
        if let Some(ref schema) = plan.result_schema {
            self.pane_results.entry(pane_id).or_default().schema = Some(schema.clone());
            log::info!("pane_result: registered schema at spawn for pane_id={pane_id}");
        }
        Ok(pane_id)
    }

    /// Everything that authorizes `role` for `parent`, read from current
    /// state. Admission records it and commit compares it.
    fn agent_call_authority(
        &self,
        parent: Option<PaneId>,
        role: &str,
        handoff: bool,
        lookup_root: &std::path::Path,
    ) -> Result<(AgentCallAuthority, crate::agent::AgentDefinition), String> {
        let grant_root = self.check_agent_tool_permission(parent, role, handoff)?;
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
        let parent_posture = parent.and_then(|id| self.pane_permission_posture(id));
        let authority = AgentCallAuthority {
            parent,
            grant_root,
            harness: crate::cli::agent::resolve_harness_adapter(harness)?
                .harness
                .as_str()
                .to_string(),
            worker: AgentWorkerAuthority::ceiling(def.posture.clone(), parent_posture.as_ref()),
        };
        Ok((authority, def))
    }

    /// Admit an `agent call`: the authenticated caller, the role's grant and
    /// ceiling, the command, and where the worker goes.
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
        let grant_root = self.check_agent_tool_permission(parent, request.role, request.handoff)?;
        let lookup_root = match (parent, request.cwd) {
            (None, Some(cwd)) => PathBuf::from(cwd),
            _ => grant_root,
        };
        let (authority, def) =
            self.agent_call_authority(parent, request.role, request.handoff, &lookup_root)?;
        let harness = def
            .harness
            .as_ref()
            .ok_or("agent role has no CLI harness")?;
        let adapter = crate::cli::agent::resolve_harness_adapter(harness)?;
        let parent_terminal = parent
            .and_then(|id| self.windows.iter().find_map(|w| w.panes.get(&id)))
            .and_then(|p| p.as_terminal());
        let (task, handoff_result) = match (request.handoff, parent) {
            (true, Some(old)) => {
                let raw =
                    json!({"status":"handed_off","role":def.id,"task":request.task}).to_string();
                let mut result_state = self.pane_results.get(&old).cloned().unwrap_or_default();
                result_state
                    .write_payload(&raw)
                    .map_err(|error| format!("handoff completion: {}", error.message()))?;
                let packet = json!({"from_pane_id":old,"from_role":parent_terminal.and_then(|t| t.agent_role.as_deref()),
                    "session":parent_terminal.and_then(|t| t.build_saved_agent_session()),"task":request.task});
                (
                    format!(
                        "# Restore packet\n{packet}\n\n# Successor task\n{}",
                        request.task
                    ),
                    Some(raw),
                )
            }
            _ => (request.task.to_string(), None),
        };
        let command = crate::cli::agent::with_auto_state_hooks(
            adapter,
            crate::cli::agent::build_agent_spawn_command(&def, adapter, &task)?,
        );
        let child_parent = if request.handoff {
            parent_terminal.and_then(|t| t.parent_pane_id)
        } else {
            parent
        };
        let origin_context = parent
            .and_then(|id| self.find_pane_in_any_window(id))
            .map_or(self.windows[self.active_window].context_id, |(win, _)| {
                self.windows[win].context_id
            });
        let subcontext = match child_parent {
            Some(id) => {
                let name = format!("agent-tools-{id}");
                let under = self.agent_spawn_parent(Some(id), Some(origin_context))?;
                Some(match self.agent_subcontext_under(under.context_id, &name) {
                    Some(context_id) => AdmittedSubcontext::Existing { name, context_id },
                    None => AdmittedSubcontext::Create { name },
                })
            }
            None => None,
        };
        log::info!(
            "launch_plan: admitted agent_call role={} parent={parent:?} child_parent={child_parent:?} origin_context={origin_context} subcontext={subcontext:?} handoff={} worktree={:?}",
            def.id,
            request.handoff,
            request.worktree
        );
        Ok(AgentCallPlan {
            caller,
            requested_role: request.role.to_string(),
            handoff: request.handoff,
            authority,
            role: def.id.clone(),
            command,
            task: request.task.to_string(),
            ephemeral: harness.ephemeral,
            result_schema: harness.result_schema.clone(),
            lookup_root,
            child_parent,
            subcontext,
            origin_context,
            worktree: request.worktree.map(str::to_string),
            handoff_result,
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
        if !self
            .router
            .iter()
            .any(|c| c.context_id == plan.origin_context)
        {
            return Err(refused(format!(
                "origin context {} was deleted",
                plan.origin_context
            )));
        }
        let (now, _) = self.agent_call_authority(
            parent,
            &plan.requested_role,
            plan.handoff,
            &plan.lookup_root,
        )?;
        if now != plan.authority {
            return Err(refused(format!(
                "permission_denied: the authority for role '{}' changed after admission",
                plan.role
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
            AdmittedSubcontext::Create { name } => SubcontextTarget::Create(name),
        });
        let (pane_id, _) = self.spawn_prepared_agent_worker_pane(
            plan.authority.worker.clone(),
            &plan.role,
            &plan.authority.harness,
            &plan.command,
            &plan.task,
            subcontext,
            plan.ephemeral,
            plan.child_parent,
            Some(cwd),
            plan.result_schema.clone(),
            None,
            true,
            Some(plan.origin_context),
        )?;
        Ok(pane_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::{Context, Window};
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
            tree_generation: 0,
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
            peer_ancestry: None,
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
            .new_tab(0, None, None, false, Some(h.workspace_root()), false, false)
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
        assert!(
            error.contains("authority for role 'tester' changed"),
            "{error}"
        );
    }
}
