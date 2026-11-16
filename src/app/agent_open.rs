//! `agent open <name>` (stint 1396): the one resolver for opening a named agent.
//! Focus it if a pane is running it, else resume its last chat, else start a new
//! chat. The CLI reaches it through `HostRequest::AgentOpen`; the palette's
//! Agents stop calls [`PlexiApp::open_named_agent`] directly.
//!
//! Each owner call (ledger row 104) sits behind one function, so changing a
//! pick is a local edit: [`PlexiApp::running_named_agent_pane`] (which pane
//! counts as running), [`PlexiApp::last_named_agent_chat`] (which history record
//! is the last chat) and [`PlexiApp::agent_open_parent`] (where a resumed or
//! started pane opens).

use std::path::Path;

use super::PlexiApp;
use crate::spatial::tiling::PaneId;
use crate::workspace::AgentHistoryRecord;

/// What `agent open` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentOpenOutcome {
    /// A pane was already running the agent; it now has focus.
    Focused { pane_id: PaneId },
    /// The agent's last chat was resumed in a new pane.
    Resumed { pane_id: PaneId, session_id: String },
    /// No resumable chat: a new chat was started.
    Started { pane_id: PaneId },
}

impl AgentOpenOutcome {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Focused { .. } => "focused",
            Self::Resumed { .. } => "resumed",
            Self::Started { .. } => "started",
        }
    }

    pub(crate) fn pane_id(&self) -> PaneId {
        match self {
            Self::Focused { pane_id }
            | Self::Resumed { pane_id, .. }
            | Self::Started { pane_id } => *pane_id,
        }
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        let mut reply = serde_json::json!({
            "ok": true,
            "outcome": self.as_str(),
            "pane_id": self.pane_id(),
        });
        if let Self::Resumed { session_id, .. } = self {
            reply["session_id"] = serde_json::Value::String(session_id.clone());
        }
        reply
    }
}

/// A palette `agent open`, queued for `App::logic` (the resolver probes
/// processes, which must not run in `App::ui`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingAgentOpen {
    pub(crate) name: String,
    pub(crate) cwd: std::path::PathBuf,
    pub(crate) caller: Option<u64>,
}

impl PlexiApp {
    /// Run the palette's queued `agent open` requests, in order, so back to
    /// back opens of one agent resolve one after the other.
    pub(crate) fn drain_pending_agent_opens(&mut self) {
        for open in std::mem::take(&mut self.pending_agent_opens) {
            match self.open_named_agent(&open.name, &open.cwd, open.caller, None) {
                Ok(outcome) => log::info!(
                    "palette: agent open {} -> {} {}",
                    open.name,
                    outcome.as_str(),
                    outcome.pane_id()
                ),
                Err(error) => log::warn!("palette: agent open {} failed: {error}", open.name),
            }
        }
    }

    /// Open the named agent `name` for a request made from `cwd` by `caller`.
    /// `peer_ancestry` is the socket peer stamp: when it resolves to a live
    /// pane, that pane is the caller and the claimed `caller` is ignored, for
    /// both launch branches (same rule as `agent start`).
    pub(crate) fn open_named_agent(
        &mut self,
        name: &str,
        cwd: &Path,
        caller: Option<u64>,
        peer_ancestry: Option<&[u32]>,
    ) -> Result<AgentOpenOutcome, String> {
        if let Some(pane_id) = self.running_named_agent_pane(name) {
            // A standby pane is focused and left in Standby: it resumes only
            // on Enter or `pane send` (src/cli/AGENTS.md, owner rule).
            if !self.pane_navigate(pane_id) {
                return Err(format!(
                    "agent open: pane {pane_id} running '{name}' could not be focused"
                ));
            }
            let outcome = AgentOpenOutcome::Focused { pane_id };
            log::info!("agent_open: name={name} outcome=focused pane={pane_id}");
            return Ok(outcome);
        }

        // The definition decides the harness; unknown agents fail here, before
        // anything is launched (same resolution as `agent start`).
        let stops = crate::app::registry::workspace_walk_stops();
        let (verified_caller, _) = self.resolve_unspoofable_spawner(caller, peer_ancestry);
        let parent = self.agent_open_parent(verified_caller);
        let payload = crate::cli::agent::resolve_agent_start_in_stopping_at(
            name,
            &crate::config::config_dir(),
            cwd,
            None,
            parent,
            &stops,
        )?;
        let harness = payload
            .get("harness")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("agent open: start payload for '{name}' has no harness"))?
            .to_string();

        if let Some(record) = self.last_named_agent_chat(name, &harness, cwd) {
            let session_id = record.session_id.clone().unwrap_or_default();
            let reply = self.reopen_agent_record(&record, None, parent);
            let pane_id = spawned_pane_id(&reply)?;
            self.focus_started_agent_pane(pane_id, parent);
            log::info!(
                "agent_open: name={name} outcome=resumed pane={pane_id} run_id={} session_id={session_id}",
                record.run_id
            );
            return Ok(AgentOpenOutcome::Resumed {
                pane_id,
                session_id,
            });
        }

        let mut request: crate::protocol::HostRequest = {
            let mut payload = payload;
            payload["response_file"] = serde_json::Value::String(String::new());
            serde_json::from_value(payload)
                .map_err(|e| format!("agent open: start payload for '{name}' is invalid: {e}"))?
        };
        if let crate::protocol::HostRequest::AgentSpawn {
            peer_ancestry: stamped,
            ..
        } = &mut request
        {
            *stamped = peer_ancestry.map(<[u32]>::to_vec);
        }
        let reply = self.agent_spawn_reply(&request);
        let pane_id = spawned_pane_id(&reply)?;
        log::info!("agent_open: name={name} outcome=started pane={pane_id}");
        Ok(AgentOpenOutcome::Started { pane_id })
    }

    /// Owner call 1: which pane counts as running `name`. Any context. Among
    /// the panes holding the role (ephemeral workers are task runs, not the
    /// named agent's chat, and never count), a pane runs the agent when it is
    /// in Standby (focused, not woken) or its processes say so
    /// ([`crate::host::pane_liveness::agent_process_liveness`]): its launch
    /// command is still running, which also covers a pane an earlier open
    /// started that has not yet reached its harness, or a known agent process
    /// runs below its shell. Hook reports never decide it. Several match: the
    /// most recently focused (the current focus first), else the newest pane.
    /// Probes processes: call from `App::logic` only.
    pub(crate) fn running_named_agent_pane(&self, name: &str) -> Option<PaneId> {
        use crate::host::pane_liveness::{agent_process_liveness, AgentProcessLiveness};
        let shell = crate::host::shell::detect_shell();
        let candidates: Vec<PaneId> = self
            .windows
            .iter()
            .flat_map(|win| win.panes.iter())
            .filter_map(|(id, pane)| {
                let t = pane.as_terminal()?;
                let role = t.agent_role.as_deref().or_else(|| {
                    t.standby_session
                        .as_ref()
                        .and_then(|s| s.config_name.as_deref())
                })?;
                if role != name || t.ephemeral {
                    return None;
                }
                if t.is_standby() {
                    return Some(*id);
                }
                let excluded = |reason: &str| {
                    log::info!("agent_open: name={name} pane={id} not running: {reason}");
                    None
                };
                if t.exited {
                    return excluded("its process exited");
                }
                match agent_process_liveness(t.backend.child_pid(), &shell) {
                    AgentProcessLiveness::Launching | AgentProcessLiveness::Agent(_) => Some(*id),
                    AgentProcessLiveness::Dead => excluded("its process is gone"),
                    AgentProcessLiveness::NoAgent => {
                        excluded("no agent process runs below its shell")
                    }
                }
            })
            .collect();
        let recency = self.panes_by_focus_recency();
        let focus_rank = |id: &PaneId| recency.iter().rposition(|pane| pane == id);
        candidates
            .into_iter()
            .max_by_key(|id| (focus_rank(id), *id))
    }

    /// Panes in the order they last held focus, oldest first, ending with the
    /// pane focused now. Focus history stores `(window_id, tile_id)`; each entry
    /// is resolved to the pane its tile still holds, and dropped if gone.
    fn panes_by_focus_recency(&self) -> Vec<PaneId> {
        let mut panes: Vec<PaneId> = self
            .pane_focus_history
            .iter()
            .filter_map(|(window_id, tile_id)| {
                let win = self.windows.iter().find(|w| w.window_id == *window_id)?;
                match win.tree.tiles.get(*tile_id)? {
                    egui_tiles::Tile::Pane(pane_id) => Some(*pane_id),
                    egui_tiles::Tile::Container(_) => None,
                }
            })
            .collect();
        panes.extend(self.focused_pane_id());
        panes
    }

    /// Owner call 2: which closed run is `name`'s last chat. The newest
    /// non-ephemeral record of this agent run in the same workspace as `cwd`,
    /// on its current `harness`, with a session the harness can still resume.
    /// Newer records that fail a check are skipped, each with its reason logged;
    /// none left starts a new chat.
    pub(crate) fn last_named_agent_chat(
        &self,
        name: &str,
        harness: &str,
        cwd: &Path,
    ) -> Option<AgentHistoryRecord> {
        let workspace = agent_chat_workspace(cwd);
        let adapter = crate::cli::agent::harness_adapter(harness);
        let history = self.fleet_history.borrow();
        let found = history
            .iter()
            .rev()
            .filter(|r| {
                r.config_name.as_deref() == Some(name)
                    && !r.ephemeral
                    && agent_chat_workspace(&r.cwd) == workspace
            })
            .find(|r| {
                let skip = |reason: &str| {
                    log::info!(
                        "agent_open: name={name} not resuming run_id={}: {reason}",
                        r.run_id
                    );
                    false
                };
                if r.harness != harness {
                    return skip(&format!("ran on {}, agent now uses {harness}", r.harness));
                }
                let Some(session_id) = r.session_id.as_deref().filter(|s| !s.trim().is_empty())
                else {
                    return skip("no session id was captured");
                };
                let Some(adapter) = adapter else {
                    return skip("unknown harness");
                };
                adapter.harness.session_exists(session_id)
                    || skip("the harness has no saved session for it")
            })
            .cloned();
        if found.is_none() {
            log::info!(
                "agent_open: name={name} has no resumable chat in {}",
                workspace.display()
            );
        }
        found
    }

    /// Owner call 3: where a resumed or started pane opens. Beside the calling
    /// pane, in its context, as `agent start` does; with no caller, the focused
    /// pane of the active context.
    pub(crate) fn agent_open_parent(&self, caller: Option<u64>) -> Option<u64> {
        caller.or_else(|| self.focused_pane_id())
    }
}

/// The workspace a chat belongs to: the nearest project root (`.plexi` or
/// `.git`), else the directory itself, so a global agent used in two projects
/// keeps one last chat per project.
fn agent_chat_workspace(cwd: &Path) -> std::path::PathBuf {
    crate::app::registry::resolve_agent_create_workspace_root(cwd)
        .unwrap_or_else(|| cwd.to_path_buf())
}

fn spawned_pane_id(reply: &serde_json::Value) -> Result<PaneId, String> {
    if reply.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(reply
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("agent open: launch failed")
            .to_string());
    }
    reply
        .get("pane_id")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("agent open: launch reply has no pane_id: {reply}"))
}
