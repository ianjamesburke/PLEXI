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

impl PlexiApp {
    /// Open the named agent `name` for a request made from `cwd` by `caller`.
    /// `peer_ancestry` is the socket peer stamp, used exactly as `agent start`
    /// uses it for `spawned_by` provenance.
    pub(crate) fn open_named_agent(
        &mut self,
        name: &str,
        cwd: &Path,
        caller: Option<u64>,
        peer_ancestry: Option<&[u32]>,
    ) -> Result<AgentOpenOutcome, String> {
        if let Some(pane_id) = self.running_named_agent_pane(name) {
            if self.resume_standby_pane(pane_id) {
                log::info!("agent_open: name={name} woke standby pane {pane_id}");
            }
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
        let parent = self.agent_open_parent(caller);
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

    /// Owner call 1: which pane counts as running `name`. Any context; a pane
    /// restored into standby counts. Ephemeral workers of the definition are
    /// task runs, not the named agent's chat, and never count. Several match:
    /// the most recently focused, else the newest pane.
    pub(crate) fn running_named_agent_pane(&self, name: &str) -> Option<PaneId> {
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
                (role == name && !t.ephemeral).then_some(*id)
            })
            .collect();
        let focus_rank = |id: &PaneId| {
            self.pane_focus_history
                .iter()
                .rposition(|(pane, _)| pane == id)
        };
        candidates
            .into_iter()
            .max_by_key(|id| (focus_rank(id), *id))
    }

    /// Owner call 2: which closed run is `name`'s last chat. The newest
    /// non-ephemeral record of this agent, on its current `harness`, with a
    /// session id the harness can still resume, run in the same workspace as
    /// `cwd`. Anything else starts a new chat; the reason is logged.
    pub(crate) fn last_named_agent_chat(
        &self,
        name: &str,
        harness: &str,
        cwd: &Path,
    ) -> Option<AgentHistoryRecord> {
        let workspace = agent_chat_workspace(cwd);
        let history = self.fleet_history.borrow();
        let newest = history.iter().rev().find(|r| {
            r.config_name.as_deref() == Some(name)
                && !r.ephemeral
                && agent_chat_workspace(&r.cwd) == workspace
        })?;
        let skip = |reason: &str| {
            log::info!(
                "agent_open: name={name} not resuming run_id={}: {reason}",
                newest.run_id
            );
            None
        };
        if newest.harness != harness {
            return skip(&format!(
                "harness changed from {} to {harness}",
                newest.harness
            ));
        }
        let Some(session_id) = newest
            .session_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        else {
            return skip("no session id was captured");
        };
        let Some(adapter) = crate::cli::agent::harness_adapter(harness) else {
            return skip("unknown harness");
        };
        if !adapter.harness.session_exists(session_id) {
            return skip("the harness has no saved session for it");
        }
        Some(newest.clone())
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
