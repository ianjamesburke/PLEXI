//! `agent open <name>` (stint 1396): the one resolver for opening a named agent,
//! and New chat (stint 1397), which archives the chat it would return to.
//! Focus a running/standby pane, else resume its newest open exited chat, else resume
//! its last closed chat, else start a new chat. The CLI reaches it through `HostRequest::AgentOpen`; the palette's
//! Agents stop calls [`PlexiApp::open_named_agent`] directly.
//!
//! Each owner call (ledger row 104) sits behind one function, so changing a
//! pick is a local edit: [`PlexiApp::running_named_agent_pane`] (which pane
//! counts as running) and [`PlexiApp::last_named_agent_chat`] (which history
//! record is the last chat). A resumed or started pane opens beside its
//! authenticated caller, or for a caller with no pane beside the focused pane,
//! which is placement only (stint 1632).

use std::path::Path;

use super::PlexiApp;
use crate::spatial::tiling::PaneId;
use crate::workspace::AgentHistoryRecord;

/// Proof of admission for one request. Only the admission function can create
/// it; downstream launch helpers consume it instead of checking config again.
pub(crate) struct AdmittedAgentMeshRoute {
    _private: (),
}

/// Host-owned admission, using the current global experiment configuration.
pub(crate) fn admit_agent_mesh_route(request: &str) -> Result<AdmittedAgentMeshRoute, String> {
    let feature = crate::release::ReleaseFeature::AgentMesh;
    if crate::release::feature_enabled(feature) {
        log::info!(
            "release_gate: admitted request={request} experiment={}",
            feature.key()
        );
        return Ok(AdmittedAgentMeshRoute { _private: () });
    }
    log::info!(
        "release_gate: denied request={request} experiment={}",
        feature.key()
    );
    Err(crate::release::feature_unavailable_text(feature))
}

/// What `agent open` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentOpenOutcome {
    /// An open pane holds the agent's chat; it now has focus.
    Focused { pane_id: PaneId },
    /// The agent's last chat was resumed in a new pane.
    Resumed { pane_id: PaneId, session_id: String },
    /// No resumable chat: a new chat was started.
    Started { pane_id: PaneId },
    /// New chat (stint 1397): the current chat, if any, was archived and an
    /// empty chat started.
    NewChat {
        pane_id: PaneId,
        archived_run_id: Option<String>,
    },
}

impl AgentOpenOutcome {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Focused { .. } => "focused",
            Self::Resumed { .. } => "resumed",
            Self::Started { .. } => "started",
            Self::NewChat { .. } => "new_chat",
        }
    }

    pub(crate) fn pane_id(&self) -> PaneId {
        match self {
            Self::Focused { pane_id }
            | Self::Resumed { pane_id, .. }
            | Self::Started { pane_id }
            | Self::NewChat { pane_id, .. } => *pane_id,
        }
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        let mut reply = serde_json::json!({
            "ok": true,
            "outcome": self.as_str(),
            "pane_id": self.pane_id(),
        });
        match self {
            Self::Resumed { session_id, .. } => {
                reply["session_id"] = serde_json::Value::String(session_id.clone());
            }
            Self::NewChat {
                archived_run_id, ..
            } => {
                reply["archived_run_id"] = serde_json::json!(archived_run_id);
            }
            Self::Focused { .. } | Self::Started { .. } => {}
        }
        reply
    }
}

impl PlexiApp {
    /// Open the named agent `name` for a request made from `cwd` by `caller`.
    /// `origin` is where the request entered the host: the caller is whoever
    /// [`PlexiApp::resolve_launch_source`] resolves it to, for both launch
    /// branches (same rule as `agent start`); an unidentified socket caller is refused.
    pub(crate) fn open_named_agent(
        &mut self,
        name: &str,
        cwd: &Path,
        caller: Option<u64>,
        origin: &crate::broker::caller::RequestOrigin,
    ) -> Result<AgentOpenOutcome, String> {
        let admitted = admit_agent_mesh_route("AgentOpen")?;
        self.open_admitted_named_agent(name, cwd, caller, origin, admitted)
            .inspect_err(|error| log::warn!("agent_open: name={name} failed: {error}"))
    }

    fn open_admitted_named_agent(
        &mut self,
        name: &str,
        cwd: &Path,
        caller: Option<u64>,
        origin: &crate::broker::caller::RequestOrigin,
        admitted: AdmittedAgentMeshRoute,
    ) -> Result<AgentOpenOutcome, String> {
        let launch_caller = self.resolve_launch_source(origin, caller)?;
        let verified_caller = launch_caller.caller_pane;
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

        let (payload, harness) = named_agent_start_payload(name, cwd)?;

        let open_chat = self.exited_named_agent_chat(name, &harness, cwd);
        if let Some(record) = open_chat.or_else(|| self.last_named_agent_chat(name, &harness, cwd))
        {
            let session_id = record.session_id.clone().unwrap_or_default();
            // A reopen is admitted under the authenticated caller only. An
            // outside caller's run opens beside the focused pane (placement)
            // with no parent and no borrowed authority.
            let reply =
                self.reopen_agent_record(&record, None, verified_caller, launch_caller.source);
            let pane_id = spawned_pane_id(&reply)?;
            self.focus_started_agent_pane(pane_id, verified_caller);
            log::info!(
                "agent_open: name={name} outcome=resumed pane={pane_id} run_id={} session_id={session_id}",
                record.run_id
            );
            return Ok(AgentOpenOutcome::Resumed {
                pane_id,
                session_id,
            });
        }

        let pane_id = self.start_named_agent_chat(name, payload, origin, caller, admitted)?;
        log::info!("agent_open: name={name} outcome=started pane={pane_id}");
        Ok(AgentOpenOutcome::Started { pane_id })
    }

    /// New chat for the named agent `name` (stint 1397): archive its current
    /// chat, then start an empty one. The current chat is the one `agent open`
    /// would return to: a running pane's, else an exited pane's, else the last
    /// closed chat. A pane holding it is closed through the normal close path,
    /// which records it in history; the record is then marked archived, so
    /// `agent open` never resumes it again and `agent reopen <run-id>` still
    /// does. Nothing is deleted, and the new chat is given the same memory
    /// folder (`crate::agent::memory`). Caller and source as for
    /// [`Self::open_named_agent`].
    pub(crate) fn new_named_agent_chat(
        &mut self,
        name: &str,
        cwd: &Path,
        caller: Option<u64>,
        origin: &crate::broker::caller::RequestOrigin,
    ) -> Result<AgentOpenOutcome, String> {
        let admitted = admit_agent_mesh_route("AgentNewChat")?;
        self.new_admitted_named_agent_chat(name, cwd, caller, origin, admitted)
            .inspect_err(|error| log::warn!("agent_new_chat: name={name} failed: {error}"))
    }

    fn new_admitted_named_agent_chat(
        &mut self,
        name: &str,
        cwd: &Path,
        caller: Option<u64>,
        origin: &crate::broker::caller::RequestOrigin,
        admitted: AdmittedAgentMeshRoute,
    ) -> Result<AgentOpenOutcome, String> {
        // Everything the replacement launch needs is decided before anything
        // is closed (Codex 3290 r4): the agent, the chat to archive, and the
        // caller as it will be once that chat's pane is gone (a new chat
        // asked for from inside the chat it replaces). A launch that could
        // not proceed is refused here, with the current chat untouched.
        let (payload, harness) = named_agent_start_payload(name, cwd)?;
        let chat_pane = self.chat_pane_to_archive(name, &harness, cwd);
        let (launch_origin, launch_caller) = self.origin_without_pane(origin, caller, chat_pane);
        self.resolve_launch_source(&launch_origin, launch_caller)?;
        let archived_run_id = self.archive_named_agent_chat(name, &harness, cwd, chat_pane)?;
        let pane_id =
            self.start_named_agent_chat(name, payload, &launch_origin, launch_caller, admitted)?;
        log::info!(
            "agent_new_chat: name={name} archived_run_id={archived_run_id:?} new_pane={pane_id}"
        );
        Ok(AgentOpenOutcome::NewChat {
            pane_id,
            archived_run_id,
        })
    }

    /// `origin` as it will be once pane `closing` is gone: unchanged unless
    /// the caller is that pane, then classified again from its kernel
    /// witness without it (no parent pane is named for it).
    fn origin_without_pane(
        &self,
        origin: &crate::broker::caller::RequestOrigin,
        caller: Option<u64>,
        closing: Option<PaneId>,
    ) -> (crate::broker::caller::RequestOrigin, Option<u64>) {
        use crate::broker::caller::{RequestOrigin, SocketCaller};
        match (origin, closing) {
            (RequestOrigin::Socket(SocketCaller::Pane(pane)), Some(closing))
                if pane.visible_pane == closing || pane.terminal_pane == closing =>
            {
                let after = self.classify_socket_peer_excluding(Some(&pane.peer), Some(closing));
                log::info!(
                    "agent_new_chat: caller pane {closing} is the chat it replaces; launching as {}",
                    after.class()
                );
                (RequestOrigin::Socket(after), None)
            }
            _ => (origin.clone(), caller),
        }
    }

    /// The pane holding `name`'s current chat, the one a new chat archives.
    fn chat_pane_to_archive(&self, name: &str, harness: &str, cwd: &Path) -> Option<PaneId> {
        self.running_named_agent_pane(name).or_else(|| {
            self.exited_named_agent_chat(name, harness, cwd)
                .map(|record| record.pane_id)
        })
    }

    /// Archive `name`'s current chat (in `open_pane`, else its last closed
    /// chat) in this workspace and return its run id,
    /// or `None` when it has no chat to archive.
    fn archive_named_agent_chat(
        &mut self,
        name: &str,
        harness: &str,
        cwd: &Path,
        open_pane: Option<PaneId>,
    ) -> Result<Option<String>, String> {
        let run_id = if let Some(pane_id) = open_pane {
            self.close_pane_by_id(pane_id);
            self.fleet_history
                .borrow()
                .iter()
                .rev()
                .find(|r| r.pane_id == pane_id)
                .map(|r| r.run_id.clone())
                .ok_or_else(|| {
                    format!(
                        "new chat: closing pane {pane_id} of '{name}' recorded no chat to archive"
                    )
                })?
        } else if let Some(record) = self.last_named_agent_chat(name, harness, cwd) {
            record.run_id
        } else {
            log::info!("agent_new_chat: name={name} has no current chat to archive");
            return Ok(None);
        };
        let mut history = self.fleet_history.borrow_mut();
        let record = history
            .iter_mut()
            .rev()
            .find(|r| r.run_id == run_id)
            .ok_or_else(|| format!("new chat: run {run_id} of '{name}' is not in history"))?;
        record.chat_archived = true;
        if let Err(error) = crate::workspace::append_agent_history(record) {
            record.chat_archived = false;
            log::error!("agent_new_chat: name={name} archive run_id={run_id} failed: {error}");
            return Err(format!(
                "new chat: could not archive run {run_id} of '{name}' in agent history: {error}"
            ));
        }
        log::info!(
            "agent_new_chat: name={name} archived run_id={run_id} session_id={:?}",
            record.session_id
        );
        Ok(Some(run_id))
    }

    /// Start a new chat from a resolved `agent start` payload, admitted for
    /// the request's own origin (stint 1285).
    fn start_named_agent_chat(
        &mut self,
        name: &str,
        mut payload: serde_json::Value,
        origin: &crate::broker::caller::RequestOrigin,
        caller: Option<u64>,
        admitted: AdmittedAgentMeshRoute,
    ) -> Result<PaneId, String> {
        payload["response_file"] = serde_json::Value::String(String::new());
        let request: crate::protocol::HostRequest = serde_json::from_value(payload)
            .map_err(|e| format!("agent open: start payload for '{name}' is invalid: {e}"))?;
        let reply = self.agent_spawn_reply_admitted(origin, caller, &request, admitted);
        spawned_pane_id(&reply)
    }

    /// Owner call 1: which pane counts as running `name`. Any context. Among
    /// the panes holding the role (ephemeral workers are task runs, not the
    /// named agent's chat, and never count), a pane runs the agent when it is
    /// in Standby (focused, not woken), its host-recorded launch has not ended
    /// (`TerminalPane::agent_launch`: set at spawn, restore and wake, cleared only when the
    /// 1 Hz activity tick sees its shell back at a bare prompt, so a pane an
    /// earlier open just started counts), or the host's agent detector sees a
    /// known agent in its foreground (`observed_agent`, e.g. a harness
    /// relaunched by hand). Reads cached host state only: no hook report
    /// decides it and nothing is probed here. Several match: the most recently
    /// focused (the current focus first), else the newest pane.
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
                if role != name || t.ephemeral {
                    return None;
                }
                let reason = if t.exited {
                    "its process exited"
                } else if t.is_standby() || t.agent_launch.is_some() || t.observed_agent.is_some() {
                    return Some(*id);
                } else {
                    "its launch ended and no agent is in its foreground"
                };
                log::info!("agent_open: name={name} pane={id} not running: {reason}");
                None
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

    /// After live/standby resolution, prefer the newest open chat to closed
    /// history. Pane ids are allocated in creation order. Only a live shell with
    /// a saved, resumable session is eligible. Build a transient record for the
    /// existing reopen path; never type into or mutate the leftover shell.
    /// History is persisted only when the pane closes.
    fn exited_named_agent_chat(
        &self,
        name: &str,
        harness: &str,
        cwd: &Path,
    ) -> Option<AgentHistoryRecord> {
        let workspace = agent_chat_workspace(cwd);
        let adapter = crate::cli::agent::harness_adapter(harness);
        let mut candidates: Vec<_> = self
            .windows
            .iter()
            .flat_map(|win| {
                win.panes.iter().filter_map(move |(id, pane)| {
                    let t = pane.as_terminal()?;
                    (t.agent_role.as_deref() == Some(name)).then_some((*id, t, win))
                })
            })
            .collect();
        candidates.sort_unstable_by_key(|(id, _, _)| std::cmp::Reverse(*id));
        let mut selected = None;
        for (id, t, win) in candidates {
            let session_id = t.agent_session_id.as_deref().unwrap_or_default();
            let chat_cwd = t
                .standby_cwd
                .as_deref()
                .or(t.initial_cwd.as_deref())
                .unwrap_or(&win.path);
            let reason = if t.exited {
                Some("its PTY process exited")
            } else if t.ephemeral {
                Some("ephemeral run")
            } else if agent_chat_workspace(chat_cwd) != workspace {
                Some("different workspace")
            } else if t.agent_harness.as_deref() != Some(harness) {
                Some("different harness")
            } else if session_id.trim().is_empty() {
                Some("no captured chat id")
            } else if !adapter.is_some_and(|a| a.harness.session_exists(session_id)) {
                Some("the harness has no saved session for it")
            } else if selected.is_some() {
                Some("older than selected open chat")
            } else {
                None
            };
            if let Some(reason) = reason {
                log::info!(
                    "agent_open: name={name} skipped open pane={id} session_id={session_id}: {reason}"
                );
            } else {
                let context_name = self
                    .router
                    .iter()
                    .find(|c| c.context_id == win.context_id)
                    .map(|c| c.name.displayed().to_string())
                    .unwrap_or_default();
                selected =
                    t.build_history_record(win.context_id, &context_name, chat_cwd.to_path_buf());
                log::info!(
                    "agent_open: name={name} selected open pane={id} session_id={session_id} for resume"
                );
            }
        }
        if let Some(record) = &selected {
            let pane_id = record.pane_id;
            for record in self
                .fleet_history
                .borrow()
                .iter()
                .filter(|r| r.config_name.as_deref() == Some(name))
            {
                log::info!(
                    "agent_open: name={name} skipped closed run_id={} session_id={:?}: open chat pane={pane_id} takes precedence",
                    record.run_id,
                    record.session_id
                );
            }
        }
        selected
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
                if r.chat_archived {
                    return skip("a New chat archived it");
                }
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
}

/// The `agent start` payload for `name` from `cwd` and its harness. The
/// definition decides the harness; unknown agents fail here, before anything
/// is launched. No pane id in the payload names a caller: the spawn it
/// becomes resolves its caller from the request's origin (stint 1285).
fn named_agent_start_payload(
    name: &str,
    cwd: &Path,
) -> Result<(serde_json::Value, String), String> {
    let stops = crate::app::registry::workspace_walk_stops()?;
    let payload = crate::cli::agent::resolve_agent_start_in_stopping_at(
        name,
        &crate::config::config_dir(),
        cwd,
        None,
        &stops,
    )?;
    let harness = payload
        .get("harness")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("agent open: start payload for '{name}' has no harness"))?
        .to_string();
    Ok((payload, harness))
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
