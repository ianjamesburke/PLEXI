//! ContextState — rolled-up status summary for a context and its children.
//!
//! Reads existing status channels only (StatusSummary for apps, pty_title for
//! terminals). No new protocol messages.

use crate::spatial::tiling::PaneId;

/// Rolled-up status for a single context, including recursive children.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ContextState {
    pub context_id: u64,
    pub label: String,
    pub pane_count: u32,
    pub active_agents: u32,
    pub status: ContextStatus,
    pub pane_summaries: Vec<PaneSummary>,
    pub children: Vec<ContextState>,
    /// Panes in this context (not its children) whose agent state is `Completed`.
    #[serde(default)]
    pub done_agents: u32,
}

/// Overall status of a context, derived from its pane summaries and child contexts.
/// Precedence: `NeedsYou > Blocked > Error > Working > Idle`.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub enum ContextStatus {
    Idle,
    Working,
    Error,
    Blocked,
    NeedsYou,
    Done,
}

/// Summary of a single pane within a context.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct PaneSummary {
    pub pane_id: PaneId,
    pub label: String,
    pub status: PaneSummaryStatus,
}

/// Status of an individual pane.
/// Precedence: `NeedsYou > Blocked > Error > Active > Idle`.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub enum PaneSummaryStatus {
    Idle,
    Active,
    Error,
    Blocked,
    NeedsYou,
}

impl ContextState {
    /// Compute the context state for a given context by walking panes and
    /// recursing into child contexts.
    ///
    /// `contexts` is the full router slice, `windows` is all windows.
    /// Precedence: `NeedsYou > Blocked > Error > Working (Active) > Idle`.
    pub fn compute(
        context_id: u64,
        contexts: &[crate::host::context::Context],
        windows: &[crate::host::context::Window],
    ) -> Self {
        use crate::protocol::AgentState;

        let label = contexts
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|| "(unknown)".to_string());

        let mut pane_summaries = Vec::new();
        let mut active_agents: u32 = 0;
        let mut has_error = false;
        let mut has_blocked = false;
        let mut has_needs_you = false;
        let mut done_agents: u32 = 0;

        for win in windows.iter().filter(|w| w.context_id == context_id) {
            let mut entries: Vec<_> = win.panes.iter().collect();
            entries.sort_by_key(|(id, _)| *id);
            for (&pid, pane) in entries {
                let (plabel, pstatus) = match pane {
                    crate::host::pane::Pane::Terminal(t) => {
                        let name = t
                            .name
                            .clone()
                            .or_else(|| t.pty_title.clone())
                            .unwrap_or_else(|| "Terminal".to_string());
                        let status = if t.exited && t.exit_status.is_some_and(|s| s.is_failure()) {
                            has_error = true;
                            PaneSummaryStatus::Error
                        } else if let Some(agent) = pane.agent() {
                            match agent.state {
                                AgentState::NeedsYou => {
                                    has_needs_you = true;
                                    PaneSummaryStatus::NeedsYou
                                }
                                AgentState::Failed => {
                                    has_error = true;
                                    PaneSummaryStatus::Error
                                }
                                AgentState::Blocked => {
                                    has_blocked = true;
                                    PaneSummaryStatus::Blocked
                                }
                                AgentState::Working => {
                                    if t.exited {
                                        PaneSummaryStatus::Idle
                                    } else {
                                        active_agents += 1;
                                        PaneSummaryStatus::Active
                                    }
                                }
                                AgentState::Completed => {
                                    done_agents += 1;
                                    PaneSummaryStatus::Idle
                                }
                                AgentState::Idle | AgentState::Interrupted => PaneSummaryStatus::Idle,
                            }
                        } else if t.exited {
                            PaneSummaryStatus::Idle
                        } else if let Some(ref act) = t.activity {
                            match act {
                                AgentState::NeedsYou => {
                                    has_needs_you = true;
                                    PaneSummaryStatus::NeedsYou
                                }
                                AgentState::Failed => {
                                    has_error = true;
                                    PaneSummaryStatus::Error
                                }
                                AgentState::Blocked => {
                                    has_blocked = true;
                                    PaneSummaryStatus::Blocked
                                }
                                AgentState::Working => {
                                    active_agents += 1;
                                    PaneSummaryStatus::Active
                                }
                                AgentState::Completed => {
                                    done_agents += 1;
                                    PaneSummaryStatus::Idle
                                }
                                AgentState::Idle | AgentState::Interrupted => PaneSummaryStatus::Idle,
                            }
                        } else if let Some(ref sb) = t.standby_session {
                            match sb.last_state {
                                AgentState::NeedsYou => {
                                    has_needs_you = true;
                                    PaneSummaryStatus::NeedsYou
                                }
                                AgentState::Failed => {
                                    has_error = true;
                                    PaneSummaryStatus::Error
                                }
                                AgentState::Blocked => {
                                    has_blocked = true;
                                    PaneSummaryStatus::Blocked
                                }
                                AgentState::Working => {
                                    active_agents += 1;
                                    PaneSummaryStatus::Active
                                }
                                AgentState::Completed => {
                                    done_agents += 1;
                                    PaneSummaryStatus::Idle
                                }
                                AgentState::Idle | AgentState::Interrupted => PaneSummaryStatus::Idle,
                            }
                        } else {
                            PaneSummaryStatus::Idle
                        };
                        (name, status)
                    }
                    crate::host::pane::Pane::App(a) => {
                        let status = if let Some(agent) = pane.agent() {
                            match agent.state {
                                AgentState::NeedsYou => {
                                    has_needs_you = true;
                                    PaneSummaryStatus::NeedsYou
                                }
                                AgentState::Failed => {
                                    has_error = true;
                                    PaneSummaryStatus::Error
                                }
                                AgentState::Blocked => {
                                    has_blocked = true;
                                    PaneSummaryStatus::Blocked
                                }
                                AgentState::Working => {
                                    active_agents += 1;
                                    PaneSummaryStatus::Active
                                }
                                AgentState::Completed => {
                                    done_agents += 1;
                                    PaneSummaryStatus::Idle
                                }
                                AgentState::Idle | AgentState::Interrupted => PaneSummaryStatus::Idle,
                            }
                        } else {
                            PaneSummaryStatus::Idle
                        };
                        (a.name.clone(), status)
                    }
                    crate::host::pane::Pane::Portal(_) => continue,
                };
                pane_summaries.push(PaneSummary {
                    pane_id: pid,
                    label: plabel,
                    status: pstatus,
                });
            }
        }

        // Recurse into child contexts.
        let child_ids: Vec<u64> = contexts
            .iter()
            .filter(|c| c.parent_id == Some(context_id))
            .map(|c| c.context_id)
            .collect();

        let children: Vec<ContextState> = child_ids
            .iter()
            .map(|&cid| Self::compute(cid, contexts, windows))
            .collect();

        // Roll up child status (NeedsYou > Blocked > Error > Working > Idle).
        for child in &children {
            match child.status {
                ContextStatus::NeedsYou => has_needs_you = true,
                ContextStatus::Blocked => has_blocked = true,
                ContextStatus::Error => has_error = true,
                ContextStatus::Working | ContextStatus::Idle | ContextStatus::Done => {}
            }
            active_agents += child.active_agents;
        }

        let pane_count = pane_summaries.len() as u32;
        let status = if has_needs_you {
            ContextStatus::NeedsYou
        } else if has_blocked {
            ContextStatus::Blocked
        } else if has_error {
            ContextStatus::Error
        } else if active_agents > 0 {
            ContextStatus::Working
        } else {
            ContextStatus::Idle
        };

        Self {
            context_id,
            label,
            pane_count,
            active_agents,
            status,
            pane_summaries,
            children,
            done_agents,
        }
    }

    /// Count of panes currently in `Active` (`Working`) state across this context and its children.
    pub fn working_count(&self) -> u32 {
        let own = self
            .pane_summaries
            .iter()
            .filter(|s| s.status == PaneSummaryStatus::Active)
            .count() as u32;
        let children: u32 = self.children.iter().map(Self::working_count).sum();
        (own + children).max(self.active_agents)
    }

    /// Count of panes currently in `Blocked` (Tier 1, master-owned) state across this context and its children.
    pub fn blocked_count(&self) -> u32 {
        let own = self
            .pane_summaries
            .iter()
            .filter(|s| s.status == PaneSummaryStatus::Blocked)
            .count() as u32;
        let children: u32 = self.children.iter().map(Self::blocked_count).sum();
        own + children
    }

    /// Count of panes currently in `NeedsYou` (Tier 2, human-owned) state across this context and its children.
    pub fn needs_you_count(&self) -> u32 {
        let own = self
            .pane_summaries
            .iter()
            .filter(|s| s.status == PaneSummaryStatus::NeedsYou)
            .count() as u32;
        let children: u32 = self.children.iter().map(Self::needs_you_count).sum();
        own + children
    }

    /// Count of panes currently in `Error` state across this context and its children.
    pub fn error_count(&self) -> u32 {
        let own = self
            .pane_summaries
            .iter()
            .filter(|s| s.status == PaneSummaryStatus::Error)
            .count() as u32;
        let children: u32 = self.children.iter().map(Self::error_count).sum();
        own + children
    }

    /// Panes whose agent finished, including nested contexts.
    pub fn done_count(&self) -> u32 {
        let children: u32 = self.children.iter().map(Self::done_count).sum();
        self.done_agents + children
    }

    /// One badge line for every sub-context header.
    /// Words are `working`, `needs you`, `done`, and `failed`.
    pub fn status_summary_badge(&self) -> Option<String> {
        format_state_counts(
            self.working_count(),
            self.needs_you_count() + self.blocked_count(),
            self.done_count(),
            self.error_count(),
        )
        .or_else(|| match self.status {
            ContextStatus::NeedsYou | ContextStatus::Blocked => Some("1 needs you".to_string()),
            ContextStatus::Error => Some("1 failed".to_string()),
            ContextStatus::Working => Some(format!("{} working", self.active_agents.max(1))),
            ContextStatus::Done => Some("done".to_string()),
            ContextStatus::Idle => None,
        })
    }
}

/// Shared words for sub-context headers and pane badges.
pub fn format_state_counts(working: u32, needs_you: u32, done: u32, failed: u32) -> Option<String> {
    let mut parts = Vec::new();
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if needs_you > 0 {
        parts.push(format!("{needs_you} needs you"));
    }
    if done > 0 {
        parts.push(format!("{done} done"));
    }
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" \u{b7} "))
    }
}

/// Chrome shared by a portal card and the bar shown after entering that space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubcontextHeader {
    pub context_id: u64,
    pub name: String,
    pub owner: Option<String>,
    pub working: u32,
    pub needs_you: u32,
    pub done: u32,
    pub failed: u32,
    pub elapsed_secs: Option<u64>,
}

impl Default for SubcontextHeader {
    fn default() -> Self {
        Self {
            context_id: 0,
            name: "(deleted)".to_string(),
            owner: None,
            working: 0,
            needs_you: 0,
            done: 0,
            failed: 0,
            elapsed_secs: None,
        }
    }
}

impl SubcontextHeader {
    pub fn counts_label(&self) -> Option<String> {
        format_state_counts(self.working, self.needs_you, self.done, self.failed)
    }
}

/// One pane's contribution to a sub-context header.
pub struct HeaderPane<'a> {
    pub name: Option<&'a str>,
    pub task: Option<&'a str>,
    pub has_parent: bool,
    pub outside_owner: Option<&'a str>,
    pub reported_at: Option<std::time::Instant>,
}

/// Human sub-context name for an app opened from a pane.
///
/// `--cli git` and `--mcp filesystem` already pass a display name. A plain
/// app id is used as-is. The renderer shell ids are not names; a path falls
/// back to its file stem.
pub fn app_anchor_name(
    explicit: Option<&str>,
    type_id: Option<&str>,
    path: Option<&std::path::Path>,
) -> String {
    if let Some(name) = explicit
        .map(str::trim)
        .filter(|name| !name.is_empty() && !is_raw_subcontext_id(name))
    {
        return name.to_string();
    }
    if let Some(id) = type_id.map(str::trim).filter(|id| !id.is_empty()) {
        if id != "cli-renderer" && id != "mcp-renderer" && !is_raw_subcontext_id(id) {
            return id.to_string();
        }
    }
    if let Some(stem) = path
        .and_then(|path| path.file_stem())
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty() && !is_raw_subcontext_id(stem))
    {
        return stem.to_string();
    }
    "App".to_string()
}

/// `team-<hex>` and `agent-tools-<digits>` are internal ids, not names.
pub fn is_raw_subcontext_id(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("team-") {
        return rest.len() >= 8 && rest.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some(rest) = name.strip_prefix("agent-tools-") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    }
    false
}

pub fn resolve_header_identity(
    stored: &str,
    panes: &[HeaderPane<'_>],
    now: std::time::Instant,
) -> (String, Option<String>, Option<u64>) {
    let owner = panes.iter().find_map(|pane| {
        pane.outside_owner
            .filter(|name| !name.is_empty() && !is_raw_subcontext_id(name))
            .map(str::to_string)
    });
    let elapsed_secs = panes
        .iter()
        .filter_map(|pane| pane.reported_at)
        .min()
        .map(|started| now.saturating_duration_since(started).as_secs());
    let name = if !stored.is_empty() && !is_raw_subcontext_id(stored) {
        stored.to_string()
    } else if let Some(root) = panes.iter().find_map(|pane| {
        (!pane.has_parent)
            .then_some(pane.name)
            .flatten()
            .filter(|name| !name.is_empty() && !is_raw_subcontext_id(name))
    }) {
        root.to_string()
    } else {
        let mut names: Vec<&str> = panes
            .iter()
            .filter_map(|pane| pane.name)
            .filter(|name| !name.is_empty() && !is_raw_subcontext_id(name))
            .collect();
        names.sort_unstable();
        names.dedup();
        if names.len() == 1 {
            names[0].to_string()
        } else if let Some(task) = panes.iter().find_map(|pane| {
            pane.task
                .map(str::trim)
                .filter(|task| !task.is_empty())
                .and_then(|task| task.lines().next())
                .map(str::trim)
                .filter(|line| !line.is_empty())
        }) {
            task.to_string()
        } else if let Some(owner) = owner.clone() {
            owner
        } else {
            "Agents".to_string()
        }
    };
    let owner = owner.filter(|owner| owner != &name);
    (name, owner, elapsed_secs)
}

fn owner_outside_context(
    windows: &[crate::host::context::Window],
    parent: u64,
    context_id: u64,
) -> Option<String> {
    let parent_pane = windows.iter().find_map(|window| window.panes.get(&parent))?;
    let parent_ctx = windows
        .iter()
        .find(|window| window.panes.contains_key(&parent))
        .map(|window| window.context_id)?;
    if parent_ctx == context_id {
        return None;
    }
    let name = match parent_pane {
        crate::host::pane::Pane::Terminal(terminal) => terminal
            .name
            .clone()
            .filter(|name| !name.is_empty())
            .or_else(|| terminal.agent_role.clone().filter(|name| !name.is_empty()))
            .unwrap_or_else(|| "terminal".to_string()),
        crate::host::pane::Pane::App(app) => {
            let trimmed = app.name.trim();
            if trimmed.is_empty() {
                "app".to_string()
            } else {
                trimmed.to_string()
            }
        }
        crate::host::pane::Pane::Portal(_) => return None,
    };
    (!is_raw_subcontext_id(&name)).then_some(name)
}

pub fn subcontext_header(
    context_id: u64,
    contexts: &[crate::host::context::Context],
    windows: &[crate::host::context::Window],
    now: std::time::Instant,
) -> SubcontextHeader {
    let stored = contexts
        .iter()
        .find(|context| context.context_id == context_id)
        .map(|context| context.name.displayed().to_string())
        .unwrap_or_else(|| "Agents".to_string());
    let state = ContextState::compute(context_id, contexts, windows);
    struct OwnedHeaderPane {
        name: Option<String>,
        task: Option<String>,
        has_parent: bool,
        outside_owner: Option<String>,
        reported_at: Option<std::time::Instant>,
    }
    let mut collected = Vec::new();
    for win in windows.iter().filter(|win| win.context_id == context_id) {
        for pane in win.panes.values() {
            match pane {
                crate::host::pane::Pane::Terminal(terminal) => {
                    collected.push(OwnedHeaderPane {
                        name: terminal.name.clone().or_else(|| terminal.agent_role.clone()),
                        task: terminal.agent.as_ref().and_then(|agent| agent.detail.clone()),
                        has_parent: terminal.parent_pane_id.is_some(),
                        outside_owner: terminal
                            .parent_pane_id
                            .and_then(|parent| owner_outside_context(windows, parent, context_id)),
                        reported_at: terminal.agent_reported_at,
                    });
                }
                crate::host::pane::Pane::App(app) => {
                    let name = {
                        let trimmed = app.name.trim();
                        (!trimmed.is_empty()).then(|| trimmed.to_string())
                    };
                    collected.push(OwnedHeaderPane {
                        name,
                        task: app.agent.as_ref().and_then(|agent| agent.detail.clone()),
                        has_parent: app.linked_pane_id.is_some(),
                        outside_owner: app
                            .linked_pane_id
                            .and_then(|parent| owner_outside_context(windows, parent, context_id)),
                        reported_at: None,
                    });
                }
                crate::host::pane::Pane::Portal(_) => {}
            }
        }
    }
    let header_panes: Vec<HeaderPane<'_>> = collected
        .iter()
        .map(|pane| HeaderPane {
            name: pane.name.as_deref(),
            task: pane.task.as_deref(),
            has_parent: pane.has_parent,
            outside_owner: pane.outside_owner.as_deref(),
            reported_at: pane.reported_at,
        })
        .collect();
    let (name, owner, elapsed_secs) = resolve_header_identity(&stored, &header_panes, now);
    SubcontextHeader {
        context_id,
        name,
        owner,
        working: state.working_count(),
        needs_you: state.needs_you_count() + state.blocked_count(),
        done: state.done_count(),
        failed: state.error_count(),
        elapsed_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::{Context, Window};
    use std::path::PathBuf;

    fn make_context(id: u64, name: &str, parent: Option<u64>) -> Context {
        Context {
            name: name.to_string().into(),
            root: PathBuf::from("/tmp"),
            description: None,
            context_id: id,
            parent_id: parent,
            depth: if parent.is_some() { 1 } else { 0 },
            parked: false,
        }
    }

    fn make_empty_window(context_id: u64, window_id: u64) -> Window {
        Window {
            name: String::new(),
            path: PathBuf::from("/tmp"),
            tree: egui_tiles::Tree::empty("test"),
            panes: std::collections::HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id,
            context_id,
        }
    }

    #[test]
    fn empty_context_is_idle() {
        let contexts = vec![make_context(1, "Root", None)];
        let windows = vec![make_empty_window(1, 100)];
        let state = ContextState::compute(1, &contexts, &windows);
        assert_eq!(state.status, ContextStatus::Idle);
        assert_eq!(state.pane_count, 0);
        assert_eq!(state.label, "Root");
    }

    #[test]
    fn child_error_rolls_up() {
        let contexts = vec![
            make_context(1, "Root", None),
            make_context(2, "Child", Some(1)),
        ];
        let windows = vec![make_empty_window(1, 100), make_empty_window(2, 101)];
        // Child has no panes but we test the rollup structure.
        let state = ContextState::compute(1, &contexts, &windows);
        assert_eq!(state.children.len(), 1);
        assert_eq!(state.children[0].label, "Child");
    }

    use crate::protocol::AgentState;

    struct DummyApp;
    impl crate::app::app_trait::App for DummyApp {
        fn type_id(&self) -> &'static str {
            "test.app"
        }
        fn display_name(&self) -> String {
            "Test App".to_string()
        }
        fn ui(
            &mut self,
            _ui: &mut egui::Ui,
            _ctx: &crate::app::app_trait::AppRenderContext<'_>,
            _pending_click: Option<crate::host::pane::PendingPaneClick>,
        ) {
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn child_status_precedence_needs_you_beats_blocked_beats_error_beats_working() {
        let contexts = vec![
            make_context(1, "Root", None),
            make_context(2, "Squad", Some(1)),
        ];
        let mut child_win = make_empty_window(2, 101);
        let make_app_pane = |id: u64, state: AgentState| {
            crate::host::pane::Pane::App(Box::new(crate::host::pane::AppPane {
                id,
                name: format!("app-{id}"),
                manifest_id: "test.app".into(),
                runtime: crate::host::pane::AppRuntime::Builtin(Box::new(DummyApp)),
                workspace_root: PathBuf::from("/tmp"),
                permissions: crate::app::permissions::AppPermissions::default(),
                pane_group: None,
                linked_pane_id: None,
                agent: Some(crate::protocol::PaneAgentState {
                    pane_id: id,
                    state,
                    agent: "codex".into(),
                    detail: None,
                    session_id: None,
                }),
                pip_status: None,
                slots: std::collections::HashMap::new(),
                hidden: false,
                overlay_replaced: None,
                semantic_state: crate::host::pane::SemanticPaneState::default(),
            }))
        };

        child_win.panes.insert(10, make_app_pane(10, AgentState::Working));
        let windows = vec![make_empty_window(1, 100), child_win];
        let state = ContextState::compute(1, &contexts, &windows);
        assert_eq!(state.children[0].status, ContextStatus::Working);
        assert_eq!(state.status, ContextStatus::Working);
        assert_eq!(
            state.children[0].status_summary_badge().as_deref(),
            Some("1 working")
        );

        let mut windows = windows;
        windows[1]
            .panes
            .insert(11, make_app_pane(11, AgentState::Working));
        windows[1]
            .panes
            .insert(12, make_app_pane(12, AgentState::Blocked));
        let state = ContextState::compute(1, &contexts, &windows);
        assert_eq!(state.children[0].status, ContextStatus::Blocked);
        assert_eq!(state.status, ContextStatus::Blocked);
        assert_eq!(
            state.children[0].status_summary_badge().as_deref(),
            Some("2 working \u{b7} 1 needs you")
        );

        let mut needs_you_win = make_empty_window(2, 101);
        needs_you_win
            .panes
            .insert(20, make_app_pane(20, AgentState::NeedsYou));
        let windows = vec![make_empty_window(1, 100), needs_you_win];
        let state = ContextState::compute(1, &contexts, &windows);
        assert_eq!(state.children[0].status, ContextStatus::NeedsYou);
        assert_eq!(state.status, ContextStatus::NeedsYou);
        assert_eq!(
            state.children[0].status_summary_badge().as_deref(),
            Some("1 needs you")
        );
    }

    #[test]
    fn raw_ids_resolve_to_a_human_name_owner_and_elapsed() {
        let now = std::time::Instant::now();
        let started = now - std::time::Duration::from_secs(125);
        let (name, owner, elapsed) = resolve_header_identity(
            "team-017f341796ab4976bee42030d6f5a6c0",
            &[HeaderPane {
                name: Some("Review"),
                task: Some("Pick a direction"),
                has_parent: false,
                outside_owner: None,
                reported_at: Some(started),
            }],
            now,
        );
        assert_eq!(name, "Review");
        assert_eq!(owner, None);
        assert_eq!(elapsed, Some(125));

        let (name, owner, _) = resolve_header_identity(
            "agent-tools-12",
            &[
                HeaderPane {
                    name: Some("researcher"),
                    task: Some("Audit the auth module"),
                    has_parent: true,
                    outside_owner: Some("lead"),
                    reported_at: None,
                },
                HeaderPane {
                    name: Some("tester"),
                    task: Some("Check the tests"),
                    has_parent: true,
                    outside_owner: Some("lead"),
                    reported_at: None,
                },
            ],
            now,
        );
        assert_eq!(name, "Audit the auth module");
        assert_eq!(owner.as_deref(), Some("lead"));

        let (name, owner, _) = resolve_header_identity(
            "auth-audit",
            &[HeaderPane {
                name: Some("researcher"),
                task: None,
                has_parent: true,
                outside_owner: Some("lead"),
                reported_at: None,
            }],
            now,
        );
        assert_eq!(name, "auth-audit");
        assert_eq!(owner.as_deref(), Some("lead"));
        assert!(!is_raw_subcontext_id("agent-tools-preview"));
    }

    #[test]
    fn app_anchor_name_prefers_a_human_label() {
        assert_eq!(
            app_anchor_name(Some("git"), Some("cli-renderer"), None),
            "git"
        );
        assert_eq!(
            app_anchor_name(Some("mcp: filesystem"), Some("mcp-renderer"), None),
            "mcp: filesystem"
        );
        assert_eq!(
            app_anchor_name(None, Some("file_browser"), None),
            "file_browser"
        );
        assert_eq!(app_anchor_name(None, Some("cli-renderer"), None), "App");
        assert_eq!(
            app_anchor_name(None, None, Some(std::path::Path::new("/tmp/notes-app"))),
            "notes-app"
        );
        assert_eq!(
            app_anchor_name(
                Some("team-017f341796ab4976bee42030d6f5a6c0"),
                Some("notes"),
                None
            ),
            "notes"
        );
    }

    #[test]
    fn app_pane_header_names_the_context_and_the_launching_pane() {
        let contexts = vec![
            make_context(1, "Root", None),
            make_context(2, "git", Some(1)),
        ];
        let mut parent = make_empty_window(1, 100);
        parent.panes.insert(
            7,
            crate::host::pane::Pane::App(Box::new(crate::host::pane::AppPane {
                id: 7,
                name: "commands".into(),
                manifest_id: "test".into(),
                runtime: crate::host::pane::AppRuntime::Builtin(Box::new(DummyApp)),
                workspace_root: PathBuf::from("/tmp"),
                permissions: crate::app::permissions::AppPermissions::default(),
                pane_group: None,
                linked_pane_id: None,
                agent: None,
                pip_status: None,
                slots: std::collections::HashMap::new(),
                hidden: false,
                overlay_replaced: None,
                semantic_state: crate::host::pane::SemanticPaneState::default(),
            })),
        );
        let mut child = make_empty_window(2, 101);
        child.panes.insert(
            8,
            crate::host::pane::Pane::App(Box::new(crate::host::pane::AppPane {
                id: 8,
                name: "git".into(),
                manifest_id: "cli-renderer".into(),
                runtime: crate::host::pane::AppRuntime::Builtin(Box::new(DummyApp)),
                workspace_root: PathBuf::from("/tmp"),
                permissions: crate::app::permissions::AppPermissions::default(),
                pane_group: None,
                linked_pane_id: Some(7),
                agent: Some(crate::protocol::PaneAgentState {
                    pane_id: 8,
                    state: AgentState::Blocked,
                    agent: "git".into(),
                    detail: None,
                    session_id: None,
                }),
                pip_status: None,
                slots: std::collections::HashMap::new(),
                hidden: false,
                overlay_replaced: None,
                semantic_state: crate::host::pane::SemanticPaneState::default(),
            })),
        );
        let header = subcontext_header(2, &contexts, &[parent, child], std::time::Instant::now());
        assert_eq!(header.name, "git");
        assert_eq!(header.owner.as_deref(), Some("commands"));
        assert_eq!(header.needs_you, 1);
    }
}
