use crate::app::permissions::PluginPermissions;
use crate::app::view_trait::{View, ViewCommand, ViewRenderContext};
use crate::spatial::tiling::PaneId;
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

pub(crate) const SEMANTIC_PANE_STATE_SCHEMA_VERSION: u32 = 2;

fn short_session_id(id: &str) -> &str {
    const MAX_SESSION_ID_CHARS: usize = 12;
    let end = id
        .char_indices()
        .nth(MAX_SESSION_ID_CHARS)
        .map(|(index, _)| index)
        .unwrap_or(id.len());
    &id[..end]
}

fn standby_session_line(
    session: &crate::workspace::SavedAgentSession,
    cached_resume_command: Option<&str>,
) -> String {
    match (
        crate::cli::AgentHarness::parse(&session.harness),
        session
            .session_id
            .as_deref()
            .filter(|id| !id.trim().is_empty()),
    ) {
        (Some(harness), Some(id))
            if cached_resume_command.is_some_and(|command| command != harness.adapter().binary) =>
        {
            format!(
                "{} · resumes session {} ({})",
                session.harness,
                short_session_id(id),
                session.last_state.as_str()
            )
        }
        (Some(crate::cli::AgentHarness::Pi), Some(id)) => format!(
            "{} · session {} not found, Enter starts a new one",
            session.harness,
            short_session_id(id)
        ),
        _ => format!(
            "{} · no saved session, Enter starts a new one",
            session.harness
        ),
    }
}

/// Runtime-neutral, read-only representation returned by `plexi pane state`.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SemanticPaneState {
    pub schema_version: u32,
    pub runtime_kind: String,
    pub roots: Vec<String>,
    pub nodes: Vec<SemanticPaneNode>,
}

impl Default for SemanticPaneState {
    fn default() -> Self {
        Self::empty("builtin")
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct SemanticPaneNode {
    pub id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub canvas_commands: Vec<serde_json::Value>,
}

impl SemanticPaneState {
    pub(crate) fn empty(runtime_kind: &str) -> Self {
        Self {
            schema_version: SEMANTIC_PANE_STATE_SCHEMA_VERSION,
            runtime_kind: runtime_kind.to_string(),
            roots: Vec::new(),
            nodes: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_process_frame(frame: &serde_json::Value) -> Self {
        let mut nodes = Vec::new();
        let roots = collect_process_semantics(frame, "frame", &mut nodes);
        Self {
            schema_version: SEMANTIC_PANE_STATE_SCHEMA_VERSION,
            runtime_kind: "process".to_string(),
            roots,
            nodes,
        }
    }

    pub(crate) fn from_accesskit(
        nodes: &egui::IdMap<egui::accesskit::Node>,
        pane_rect: egui::Rect,
    ) -> Self {
        let mut nodes: Vec<SemanticPaneNode> = nodes
            .iter()
            .filter_map(|(id, node)| {
                let bounds = node.bounds()?;
                let center = egui::pos2(
                    ((bounds.x0 + bounds.x1) / 2.0) as f32,
                    ((bounds.y0 + bounds.y1) / 2.0) as f32,
                );
                pane_rect.contains(center).then(|| SemanticPaneNode {
                    id: id.value().to_string(),
                    role: format!("{:?}", node.role()).to_ascii_lowercase(),
                    label: node.label().map(str::to_string),
                    value: node.value().map(str::to_string),
                    children: node.children().iter().map(|id| id.0.to_string()).collect(),
                    bounds: Some([bounds.x0, bounds.y0, bounds.x1, bounds.y1]),
                    canvas_commands: Vec::new(),
                })
            })
            .collect();
        let retained_ids: std::collections::HashSet<String> =
            nodes.iter().map(|node| node.id.clone()).collect();
        for node in &mut nodes {
            node.children.retain(|child| retained_ids.contains(child));
        }
        let child_ids: std::collections::HashSet<&str> = nodes
            .iter()
            .flat_map(|node| node.children.iter().map(String::as_str))
            .collect();
        let roots = nodes
            .iter()
            .filter(|node| !child_ids.contains(node.id.as_str()))
            .map(|node| node.id.clone())
            .collect();
        Self {
            schema_version: SEMANTIC_PANE_STATE_SCHEMA_VERSION,
            runtime_kind: "builtin".to_string(),
            roots,
            nodes,
        }
    }

    pub(crate) fn from_wasm_tree(tree: &crate::host::wasm_view::UiTree) -> Self {
        use crate::host::wasm_view::UiNodeData;

        let nodes = tree
            .nodes
            .iter()
            .map(|node| {
                let (role, label, value, children) = match &node.data {
                    UiNodeData::Empty => ("empty", None, None, Vec::new()),
                    UiNodeData::Text(text) => ("text", Some(text.text.clone()), None, Vec::new()),
                    UiNodeData::Button(button) => {
                        ("button", Some(button.label.clone()), None, Vec::new())
                    }
                    UiNodeData::TextInput(input) => (
                        "text_input",
                        Some(input.placeholder.clone()),
                        (!input.password).then(|| input.value.clone()),
                        Vec::new(),
                    ),
                    UiNodeData::Row(row) => (
                        "row",
                        None,
                        None,
                        row.children.iter().map(u32::to_string).collect(),
                    ),
                    UiNodeData::Column(column) => (
                        "column",
                        None,
                        None,
                        column.children.iter().map(u32::to_string).collect(),
                    ),
                    UiNodeData::ProgressBar(progress) => (
                        "progress_bar",
                        progress.label.clone(),
                        Some(progress.value.to_string()),
                        Vec::new(),
                    ),
                    UiNodeData::Badge(badge) => {
                        ("badge", Some(badge.text.clone()), None, Vec::new())
                    }
                    UiNodeData::ListView(list) => (
                        "list_view",
                        None,
                        list.selected.map(|selected| selected.to_string()),
                        list.items.iter().map(u32::to_string).collect(),
                    ),
                    UiNodeData::Scroll(scroll) => {
                        ("scroll", None, None, vec![scroll.child.to_string()])
                    }
                    UiNodeData::Padding(padding) => {
                        ("padding", None, None, vec![padding.child.to_string()])
                    }
                    UiNodeData::Divider => ("divider", None, None, Vec::new()),
                    UiNodeData::Space(space) => (
                        "space",
                        None,
                        Some(format!(
                            "{}{}",
                            space.size,
                            if space.grow { "+grow" } else { "" }
                        )),
                        Vec::new(),
                    ),
                    UiNodeData::Surface(surface) => (
                        "surface",
                        None,
                        Some(format!("{}x{}", surface.width, surface.height)),
                        Vec::new(),
                    ),
                    UiNodeData::Canvas(canvas) => (
                        "canvas",
                        None,
                        Some(format!("{}x{}", canvas.width, canvas.height)),
                        Vec::new(),
                    ),
                    UiNodeData::SurfaceHeader(surface_header) => (
                        "surface_header",
                        Some(surface_header.title.clone()),
                        (!surface_header.subtitle.is_empty())
                            .then(|| surface_header.subtitle.clone()),
                        Vec::new(),
                    ),
                    UiNodeData::FooterKeys(footer_keys) => (
                        "footer_keys",
                        None,
                        Some(format!("{} entries", footer_keys.entries.len())),
                        Vec::new(),
                    ),
                    UiNodeData::Pinned(pinned) => (
                        "pinned",
                        None,
                        Some(format!("{:?}", pinned.edge).to_ascii_lowercase()),
                        vec![pinned.child.to_string()],
                    ),
                    UiNodeData::Spinner(spinner) => (
                        "spinner",
                        (!spinner.label.is_empty()).then(|| spinner.label.clone()),
                        None,
                        Vec::new(),
                    ),
                };
                SemanticPaneNode {
                    id: node.id.to_string(),
                    role: role.to_string(),
                    label,
                    value,
                    children,
                    bounds: None,
                    canvas_commands: Vec::new(),
                }
            })
            .collect();
        Self {
            schema_version: SEMANTIC_PANE_STATE_SCHEMA_VERSION,
            runtime_kind: "wasm".to_string(),
            roots: vec![tree.root.to_string()],
            nodes,
        }
    }

    pub(crate) fn expose_canvas_commands(&mut self, tree: &crate::host::wasm_view::UiTree) {
        use crate::host::wasm_view::UiNodeData;

        for (state_node, tree_node) in self.nodes.iter_mut().zip(&tree.nodes) {
            if let UiNodeData::Canvas(canvas) = &tree_node.data {
                state_node.canvas_commands = canvas
                    .commands
                    .iter()
                    .map(semantic_canvas_command)
                    .collect();
            }
        }
    }

    /// Resolve `node_id` (as reported by `plexi pane state`) against this
    /// cached snapshot and confirm it is an activatable role. Used by the
    /// `ClickPaneNode` host command to fail loudly — named error, no queued
    /// click — before a `PendingPaneClick` is ever inserted, instead of
    /// silently no-op'ing against a missing or non-interactive node.
    pub(crate) fn resolve_interactive_node(&self, node_id: &str) -> Result<u64, String> {
        const INTERACTIVE_ROLES: &[&str] = &["button", "text_input", "list_view", "checkbox"];

        let Some(node) = self.nodes.iter().find(|n| n.id == node_id) else {
            return Err(format!(
                "node {node_id} not found in this pane's current tree"
            ));
        };
        if !INTERACTIVE_ROLES.contains(&node.role.as_str()) {
            return Err(format!(
                "node {node_id} is not interactive (role: {})",
                node.role
            ));
        }
        node.id
            .parse::<u64>()
            .map_err(|_| format!("node {node_id} has a non-numeric id and cannot be targeted"))
    }
}

#[cfg(test)]
fn collect_process_semantics(
    value: &serde_json::Value,
    path: &str,
    nodes: &mut Vec<SemanticPaneNode>,
) -> Vec<String> {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .enumerate()
            .flat_map(|(index, value)| {
                collect_process_semantics(value, &format!("{path}.{index}"), nodes)
            })
            .collect(),
        serde_json::Value::Object(object) => {
            let children: Vec<String> = object
                .iter()
                .flat_map(|(key, value)| {
                    collect_process_semantics(value, &format!("{path}.{key}"), nodes)
                })
                .collect();
            let Some(role) = object.get("type").and_then(serde_json::Value::as_str) else {
                return children;
            };
            let label = object
                .get("text")
                .or_else(|| object.get("label"))
                .or_else(|| object.get("title"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let value = object.get("value").and_then(|value| match value {
                serde_json::Value::String(value) => Some(value.clone()),
                serde_json::Value::Number(value) => Some(value.to_string()),
                _ => None,
            });
            nodes.push(SemanticPaneNode {
                id: path.to_string(),
                role: role.to_string(),
                label,
                value,
                children,
                bounds: process_command_bounds(object),
                canvas_commands: Vec::new(),
            });
            vec![path.to_string()]
        }
        _ => Vec::new(),
    }
}

fn semantic_canvas_command(
    command: &crate::host::wasm_view::bindings::plexi::platform::types::CanvasCommand,
) -> serde_json::Value {
    use crate::host::wasm_view::bindings::plexi::platform::types::CanvasCommand;

    match command {
        CanvasCommand::Rect(rect) => serde_json::json!({
            "type": "rect",
            "x": rect.x,
            "y": rect.y,
            "width": rect.width,
            "height": rect.height,
            "fill": semantic_color(&rect.fill),
            "radius": rect.radius,
        }),
        CanvasCommand::Circle(circle) => serde_json::json!({
            "type": "circle",
            "x": circle.x,
            "y": circle.y,
            "radius": circle.radius,
            "fill": semantic_color(&circle.fill),
        }),
        CanvasCommand::Line(line) => serde_json::json!({
            "type": "line",
            "x1": line.x1,
            "y1": line.y1,
            "x2": line.x2,
            "y2": line.y2,
            "width": line.width,
            "color": semantic_color(&line.color),
        }),
        CanvasCommand::Text(text) => serde_json::json!({
            "type": "text",
            "x": text.x,
            "y": text.y,
            "text": text.text,
            "size": text.size,
            "color": semantic_color(&text.color),
            "bold": text.bold,
            "align": format!("{:?}", text.align).to_ascii_lowercase(),
        }),
    }
}

fn semantic_color(
    color: &crate::host::wasm_view::bindings::plexi::platform::types::Color,
) -> String {
    if color.a == u8::MAX {
        format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    } else {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            color.r, color.g, color.b, color.a
        )
    }
}

#[cfg(test)]
fn process_command_bounds(object: &serde_json::Map<String, serde_json::Value>) -> Option<[f64; 4]> {
    let x = object.get("x")?.as_f64()?;
    let y = object.get("y")?.as_f64()?;
    let width = object
        .get("w")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let height = object
        .get("h")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    Some([x, y, x + width, y + height])
}

// ---------------------------------------------------------------------------
// Pane ADT (spec §2) — Terminal | App | Portal.
// Issue #1374 added the Portal variant (formerly SubContext).
// ---------------------------------------------------------------------------

pub enum Pane {
    Terminal(Box<TerminalPane>),
    View(Box<ViewPane>),
    /// A tile that represents a child context nested inside this one.
    /// Renders a summary card with pane count, status, and per-pane summaries.
    /// Cmd+Enter zooms into the sub-context when this tile has focus.
    Portal(Box<PortalPane>),
}

/// A hook report owns the state unconditionally for this long. Afterward,
/// host-observed Working may corroborate activity that contradicts a stale
/// hook Idle; fresh hook reports and hook Working always remain authoritative.
pub(crate) const HOOK_AGENT_FRESHNESS: std::time::Duration = std::time::Duration::from_secs(3);

/// Quiet window after which a foreground agent TUI with no hook report yet is
/// considered idle by the host-observed detector. Measured against a real
/// Codex boot: the banner draws in bursts with gaps under ~300ms and goes
/// quiet within ~2.2s, so one full second of PTY silence cleanly separates
/// "still drawing" from "sitting at the prompt" while staying far inside the
/// 60s default boot window.
pub(crate) const OBSERVED_AGENT_SETTLE: std::time::Duration = std::time::Duration::from_secs(1);

/// PTY output this soon after a committed resize is the host's doing, not the
/// agent's (stint 1231): a TUI repaints on SIGWINCH, and that repaint must not
/// flip an idle agent to Working. A focus change writes nothing to the PTY
/// (egui_term sends no focus-in/out reports), so it opens no window (stint 1438).
pub(crate) const HOST_INDUCED_QUIET: std::time::Duration = std::time::Duration::from_millis(750);

fn select_terminal_agent<'a>(
    reported: Option<&'a crate::protocol::PaneAgentState>,
    hook: Option<&'a crate::protocol::PaneAgentState>,
    hook_reported_at: Option<std::time::Instant>,
    observed: Option<&'a crate::protocol::PaneAgentState>,
    now: std::time::Instant,
) -> Option<&'a crate::protocol::PaneAgentState> {
    if reported.is_some() {
        return reported;
    }
    let Some(hook) = hook else {
        return observed;
    };
    if hook.state != crate::protocol::AgentState::Idle {
        if hook_reported_at.is_none() && observed.is_some() {
            return observed;
        }
        return Some(hook);
    }
    let hook_is_stale = hook_reported_at
        .is_some_and(|reported| now.saturating_duration_since(reported) >= HOOK_AGENT_FRESHNESS);
    let contradicts_idle = observed.is_some_and(|observed| {
        observed.agent == hook.agent && observed.state == crate::protocol::AgentState::Working
    });
    if hook_is_stale && contradicts_idle {
        observed
    } else {
        Some(hook)
    }
}

/// A portal pane points at a child context and caches its rolled-up state.
pub struct PortalPane {
    pub pane_id: PaneId,
    pub target_context_id: u64,
    pub context_state: Option<crate::host::context_state::ContextState>,
    /// True when a close made this the sole portal in its parent.
    pub reveal_after_close: bool,
    /// When true, the pane is visually deprioritized (outline dot, dimmed tab title).
    pub hidden: bool,
}

impl Pane {
    pub fn id(&self) -> PaneId {
        match self {
            Pane::Terminal(t) => t.id,
            Pane::View(a) => a.id,
            Pane::Portal(p) => p.pane_id,
        }
    }

    pub fn is_hidden(&self) -> bool {
        match self {
            Pane::Terminal(t) => t.hidden,
            Pane::View(a) => a.hidden,
            Pane::Portal(p) => p.hidden,
        }
    }

    pub fn set_hidden(&mut self, val: bool) {
        match self {
            Pane::Terminal(t) => t.hidden = val,
            Pane::View(a) => a.hidden = val,
            Pane::Portal(p) => p.hidden = val,
        }
    }

    pub fn as_terminal(&self) -> Option<&TerminalPane> {
        match self {
            Pane::Terminal(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_terminal_mut(&mut self) -> Option<&mut TerminalPane> {
        match self {
            Pane::Terminal(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_view(&self) -> Option<&ViewPane> {
        match self {
            Pane::View(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_view_mut(&mut self) -> Option<&mut ViewPane> {
        match self {
            Pane::View(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_portal(&self) -> Option<&PortalPane> {
        match self {
            Pane::Portal(p) => Some(p),
            _ => None,
        }
    }

    pub fn as_portal_mut(&mut self) -> Option<&mut PortalPane> {
        match self {
            Pane::Portal(p) => Some(p),
            _ => None,
        }
    }

    /// True if this pane is an active agent pane (terminal agent or plugin agent).
    pub fn is_agent_pane(&self) -> bool {
        match self {
            Pane::Terminal(t) => t.is_agent_pane(),
            Pane::View(a) => a.agent.is_some(),
            Pane::Portal(_) => false,
        }
    }

    /// The pane's agent state, from the shared detector's two signal sources:
    /// hook-reported state (`SetAgentState`, authoritative while fresh and
    /// whenever Working) plus the host-observed synthesis from
    /// `tick_terminal_activity` (known agent process in the PTY foreground +
    /// output-settle state). A stale hook Idle may yield to corroborated
    /// observed Working until the output settles again.
    /// Every consumer — `pane list`, `pane state`, the `pane new --agent`
    /// boot predicate — reads through here, so they stay correct together.
    pub fn agent(&self) -> Option<&crate::protocol::PaneAgentState> {
        match self {
            Pane::Terminal(t) => t.selected_agent(std::time::Instant::now()),
            Pane::View(a) => a
                .agent
                .as_ref()
                .or_else(|| a.overlay_replaced.as_deref().and_then(Pane::agent)),
            Pane::Portal(_) => None,
        }
    }

    /// Agent report with terminal exit and completion precedence applied.
    pub(crate) fn effective_agent(&self) -> Option<crate::protocol::PaneAgentState> {
        match self {
            Pane::Terminal(t) => {
                let mut report = self.agent().cloned().or_else(|| {
                    t.is_agent_pane().then(|| crate::protocol::PaneAgentState {
                        pane_id: t.id,
                        state: crate::protocol::AgentState::Idle,
                        agent: t
                            .agent_harness
                            .clone()
                            .or_else(|| t.standby_session.as_ref().map(|s| s.harness.clone()))
                            .unwrap_or_default(),
                        detail: None,
                        session_id: t.agent_session_id.clone(),
                    })
                })?;
                (report.state, report.detail) = t.effective_agent_state();
                Some(report)
            }
            Pane::View(_) => self.agent().cloned(),
            Pane::Portal(_) => None,
        }
    }

    /// App-reported pip status, checking the outer plugin then any overlay-replaced
    /// pane underneath (mirrors `agent()`).
    pub fn pip_status(&self) -> Option<crate::protocol::PipStatus> {
        match self {
            Pane::View(a) => a
                .pip_status
                .or_else(|| a.overlay_replaced.as_deref().and_then(Pane::pip_status)),
            Pane::Terminal(_) | Pane::Portal(_) => None,
        }
    }

    /// An plugin's yellow health pip keeps its color without implying agent work.
    pub fn static_health_pip(&self) -> bool {
        self.pip_status() == Some(crate::protocol::PipStatus::Yellow)
            && !self
                .agent()
                .is_some_and(|agent| agent.state == crate::protocol::AgentState::Working)
    }

    /// Activity state for the unified activity dot. Generic terminal process
    /// liveness is tracked separately and does not imply agent activity.
    pub fn effective_activity(&self) -> Option<&crate::protocol::AgentState> {
        if let Some(pip) = self.pip_status() {
            return Some(pip.as_agent_state());
        }
        if let Pane::Terminal(t) = self {
            if t.is_agent_pane()
                && (t.exited || t.agent_completed || t.slots.contains_key("completed"))
            {
                use crate::protocol::AgentState;
                return Some(match t.effective_agent_state().0 {
                    AgentState::Working => &AgentState::Working,
                    AgentState::Idle => &AgentState::Idle,
                    AgentState::NeedsYou => &AgentState::NeedsYou,
                    AgentState::Blocked => &AgentState::Blocked,
                    AgentState::Failed => &AgentState::Failed,
                    AgentState::Interrupted => &AgentState::Interrupted,
                    AgentState::Completed => &AgentState::Completed,
                    AgentState::ExitedUnreported => &AgentState::ExitedUnreported,
                });
            }
        }
        if let Some(a) = self.agent() {
            return Some(&a.state);
        }
        match self {
            Pane::Terminal(_) => None,
            Pane::View(a) => a
                .overlay_replaced
                .as_deref()
                .and_then(Pane::effective_activity),
            Pane::Portal(_) => None,
        }
    }

    pub fn set_agent(&mut self, agent: Option<crate::protocol::PaneAgentState>) -> bool {
        self.set_agent_with_dirty(agent).0
    }

    /// Set the pane's agent state and return `(found, session_or_state_changed)`.
    /// Callers on `PlexiApp` use the second boolean to trigger `mark_workspace_dirty()`
    /// whenever `session_id` or `state` transitions so crashes never lose active
    /// session IDs or states.
    pub fn set_agent_with_dirty(
        &mut self,
        agent: Option<crate::protocol::PaneAgentState>,
    ) -> (bool, bool) {
        match self {
            Pane::Terminal(t) => {
                let prev_state = t.agent.as_ref().map(|a| a.state.clone());
                let prev_session = t
                    .agent_session_id
                    .clone()
                    .or_else(|| t.agent.as_ref().and_then(|a| a.session_id.clone()));
                let mut agent = agent;
                if let Some(ref mut a) = agent {
                    if let Some(ref sid) = a.session_id {
                        if !sid.trim().is_empty() {
                            t.agent_session_id = Some(sid.clone());
                        } else {
                            a.session_id = t.agent_session_id.clone();
                        }
                    } else if t.agent_session_id.is_some() {
                        a.session_id = t.agent_session_id.clone();
                    }
                    if t.agent_harness.is_none()
                        && crate::cli::AgentHarness::parse(&a.agent).is_some()
                    {
                        t.agent_harness = Some(a.agent.clone());
                    }
                    if let Some(ref mut standby) = t.standby_session {
                        standby.last_state = a.state.clone();
                        standby.last_detail = a.detail.clone();
                        if a.session_id
                            .as_deref()
                            .is_some_and(|s| !s.trim().is_empty())
                        {
                            standby.session_id = a.session_id.clone();
                        }
                    }
                }
                let new_state = agent.as_ref().map(|a| a.state.clone());
                let new_session = t
                    .agent_session_id
                    .clone()
                    .or_else(|| agent.as_ref().and_then(|a| a.session_id.clone()));
                let changed = prev_state != new_state || prev_session != new_session;
                t.update_fleet_transition(prev_state.as_ref(), new_state.as_ref());
                t.agent_reported_at = agent.as_ref().map(|_| std::time::Instant::now());
                t.agent = agent;
                (true, changed)
            }
            Pane::View(a) => {
                if let Some(replaced) = a.overlay_replaced.as_deref_mut() {
                    replaced.set_agent_with_dirty(agent)
                } else {
                    let prev_state = a.agent.as_ref().map(|s| s.state.clone());
                    let prev_session = a.agent.as_ref().and_then(|s| s.session_id.clone());
                    let new_state = agent.as_ref().map(|s| s.state.clone());
                    let new_session = agent.as_ref().and_then(|s| s.session_id.clone());
                    let changed = prev_state != new_state || prev_session != new_session;
                    a.agent = agent;
                    (true, changed)
                }
            }
            Pane::Portal(_) => (false, false),
        }
    }

    /// Set the plugin-reported pip status. View panes only (mirrors `set_agent`'s
    /// overlay delegation). Terminals and portals have no pip surface → false.
    pub fn set_pip_status(&mut self, status: Option<crate::protocol::PipStatus>) -> bool {
        match self {
            Pane::View(a) => {
                if let Some(replaced) = a.overlay_replaced.as_deref_mut() {
                    replaced.set_pip_status(status)
                } else {
                    a.pip_status = status;
                    true
                }
            }
            Pane::Terminal(_) | Pane::Portal(_) => false,
        }
    }

    pub fn slots(&self) -> Option<&HashMap<String, PathBuf>> {
        match self {
            Pane::Terminal(t) => Some(&t.slots),
            Pane::View(a) => Some(&a.slots),
            Pane::Portal(_) => None,
        }
    }

    pub fn slots_mut(&mut self) -> Option<&mut HashMap<String, PathBuf>> {
        match self {
            Pane::Terminal(t) => Some(&mut t.slots),
            Pane::View(a) => Some(&mut a.slots),
            Pane::Portal(_) => None,
        }
    }

    /// Returns the target context_id if this is a Portal pane.
    pub fn portal_target(&self) -> Option<u64> {
        match self {
            Pane::Portal(p) => Some(p.target_context_id),
            _ => None,
        }
    }

    /// Resume a Standby agent pane, returning `true` if the pane was in Standby.
    pub fn resume_from_standby(&mut self) -> bool {
        match self {
            Pane::Terminal(t) => t.resume_from_standby().is_some(),
            Pane::View(a) => a
                .overlay_replaced
                .as_deref_mut()
                .is_some_and(|r| r.resume_from_standby()),
            Pane::Portal(_) => false,
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalPane — PTY-only state
// ---------------------------------------------------------------------------

pub struct TerminalPane {
    _process_group: super::process_cleanup::PaneProcessGroup,
    pub fleet_result: Option<serde_json::Value>,
    pub fleet_seen: bool,
    pub fleet_dismissed: bool,
    pub fleet_done_unseen: bool,
    pub fleet_since: u64,
    /// Pane ID — matches the key used in HashMap<PaneId, Pane>.
    pub id: PaneId,
    pub backend: TerminalBackend,
    pub exited: bool,
    /// Observed PTY exit status (`Code`, `Signal`, or `Unknown`), recorded in
    /// `drain_pty_events` or `AgentDone` so `ContextState::compute` and crash
    /// retention can distinguish exit 0 from non-zero/signal failures.
    pub(crate) exit_status: Option<crate::host::pane_lifecycle::ExitStatus>,
    pub name: Option<String>,
    /// Context-derived placeholder title displayed before shell integration reports a title.
    pub placeholder_title: Option<String>,
    /// When true, the name was set explicitly by the user and OSC title sequences must not overwrite it.
    pub name_locked: bool,
    pub font_size: f32,
    /// When true, the pane closes automatically when its process exits cleanly with code 0.
    /// Set by `plexi pane new --ephemeral` or `plexi agent spawn --ephemeral`.
    pub ephemeral: bool,
    /// Parent master pane ID (`PLEXI_PARENT_PANE`), used for Tier 1 `Blocked` escalation.
    pub parent_pane_id: Option<u64>,
    /// Unspoofable spawner provenance (`"human"`, `"agent:<name>"`, `"team:<team>/<run_id>"`, `"system"`).
    pub spawned_by: Option<String>,
    /// Queued child `agent done` results waiting for this interactive parent pane to become Idle.
    pub pending_child_results: Vec<crate::workspace::QueuedChildResult>,
    /// True when the pane was spawned for or by an agent (including plain shells
    /// created by an agent via `plexi pane new`). Disqualifies the pane from
    /// human-owned user authority (`is_human_owned`).
    pub agent_spawned: bool,
    /// Configured or hook-detected CLI agent harness (`claude`, `codex`, `pi`, `agy`).
    pub agent_harness: Option<String>,
    /// Latest captured CLI agent session ID for resume (`claude --resume <id>`, etc.).
    pub agent_session_id: Option<String>,
    /// Optional agent role config name (`.plexi/agents/<name>.toml`).
    pub agent_role: Option<String>,
    /// Permission posture copied from the agent definition at spawn. Editing
    /// settings.toml cannot widen a live worker's authority.
    pub agent_permission_posture: Option<crate::broker::PermissionPosture>,
    /// Latest blocked reason reported by hook or `plexi agent ask`.
    pub last_blocked_reason: Option<crate::protocol::AgentBlockedReason>,
    /// Cached tail preview preserved across Standby / Interrupted restore.
    pub tail_preview: Option<String>,
    /// Set when the agent explicitly completed via `plexi agent done` or clean ephemeral exit.
    pub agent_completed: bool,
    /// When present, this background/sub-context agent pane was restored into lightweight
    /// `[Plexi Standby]` and has not yet launched its agent CLI process.
    pub standby_session: Option<crate::workspace::SavedAgentSession>,
    /// Resume command captured when entering Standby, so rendering never probes session files.
    pub standby_resume_command: Option<String>,
    /// Working directory captured when entering `[Plexi Standby]`.
    pub standby_cwd: Option<PathBuf>,
    /// Working directory configured when spawning the PTY backend.
    pub initial_cwd: Option<PathBuf>,
    /// Most recent CLI resume invocation dispatched by startup restore or Standby wake.
    pub last_resumed_command: Option<String>,
    /// Last OSC 2 title string the process wrote, tracked independently of `name` and `name_locked`.
    /// Used by FocusChanged events to record what was running in the pane.
    pub pty_title: Option<String>,
    /// Cached result for the workspace-scope badge. The actual probe hits the
    /// OS for the child process cwd, so the render path throttles it.
    pub(crate) outside_workspace_cached: bool,
    pub(crate) outside_workspace_checked_at: Option<std::time::Instant>,
    pub(crate) outside_workspace_root: Option<PathBuf>,
    /// When true, the pane is visually deprioritized (outline dot, dimmed tab title).
    pub hidden: bool,
    pub agent: Option<crate::protocol::PaneAgentState>,
    /// Receipt time of the latest lifecycle hook report. Used only to bound
    /// how long hook Idle suppresses contradictory host-observed Working.
    pub(crate) agent_reported_at: Option<std::time::Instant>,
    /// Host-observed agent state, synthesized in `tick_terminal_activity`
    /// when a known agent binary holds the PTY foreground: identity from the
    /// foreground process name or interpreter script argv, state from the
    /// agent-output settle (`observed_output_state`). Covers both the pre-hook boot
    /// window and stale hook-Idle gaps.
    pub observed_agent: Option<crate::protocol::PaneAgentState>,
    /// Last detected foreground identity until the shell is observed idle.
    pub(crate) last_observed_agent: Option<String>,
    /// Agent state mapped from OSC 7501 program-status records, refreshed in
    /// `tick_terminal_activity`. Highest-priority signal: when set it wins over
    /// hook and observed state; `None` leaves the old selection untouched.
    pub reported_7501: Option<crate::protocol::PaneAgentState>,
    /// Host-observed terminal activity (foreground process running, exited),
    /// polled via `tcgetpgrp` in `tick_terminal_activity`. Separate from
    /// `agent`, which is hook-reported; `agent` wins when both are present.
    pub activity: Option<crate::protocol::AgentState>,
    pub slots: HashMap<String, PathBuf>,
    /// When this pane last produced *any* PTY event, stamped in
    /// `drain_pty_events`. `pane send --submit` settles on the absence of these:
    /// a quiet window here means the program has finished redrawing and is ready
    /// for Enter. This is deliberately not the capture cursor — an in-place TUI
    /// redraw (Claude Code, Codex) never advances that counter, so it would
    /// report a busy pane as idle.
    pub last_pty_output_at: Option<std::time::Instant>,
    /// When the pane last produced PTY output the host did not provoke itself.
    /// Observed agent state settles on this (`observed_output_state`), so an
    /// idle agent's resize repaint cannot read as agent activity (stints 1231,
    /// 1438).
    pub(crate) last_agent_activity_at: Option<std::time::Instant>,
    /// End of the current host-induced quiet window (`HOST_INDUCED_QUIET`).
    pub(crate) host_induced_until: Option<std::time::Instant>,
    /// Whether output inside the current host-induced window was already traced.
    host_induced_output_logged: bool,
    /// `backend.resize_commits()` when last observed; a higher count means the
    /// host committed a resize since.
    seen_resize_commits: u64,
    /// OSC 7501 program-status records, fed by the PTY byte tap.
    pub program_status: crate::program_status::SharedProgramStatus,
    /// Test-only copy of the environment handed to the PTY constructor.
    #[cfg(test)]
    pub spawn_env: HashMap<String, String>,
}

impl TerminalPane {
    pub fn new(
        id: u64,
        ctx: egui::Context,
        tx: Sender<(u64, PtyEvent)>,
        mut settings: BackendSettings,
        default_font_size: f32,
    ) -> Option<Self> {
        settings.env.insert(
            super::process_cleanup::SESSION_ENV.into(),
            super::process_cleanup::session_marker().into(),
        );
        let ephemeral = settings
            .env
            .get("PLEXI_EPHEMERAL")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let parent_pane_id = settings
            .env
            .get("PLEXI_PARENT_PANE")
            .and_then(|v| v.parse::<u64>().ok());
        let spawned_by = settings
            .env
            .get("PLEXI_SPAWNED_BY")
            .cloned()
            .filter(|s| !s.trim().is_empty())
            // A pane nobody stamped (new window/split/first shell) was opened by the
            // person at the desktop; agent and system spawners stamp themselves.
            .or_else(|| Some("human".to_string()));
        let agent_harness = settings.env.get("PLEXI_AGENT_HARNESS").cloned();
        let agent_role = settings.env.get("PLEXI_AGENT_ROLE").cloned();
        let agent_spawned = settings
            .env
            .get("PLEXI_AGENT_SPAWNED")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            || parent_pane_id.is_some()
            || agent_harness.is_some()
            || agent_role.is_some();
        let last_resumed_command = settings.env.get("PLEXI_RESUME_COMMAND").cloned();
        let initial_cwd = settings.working_directory.clone();
        let placeholder_title = if settings.env.contains_key("PLEXI_CONTEXT_NAME") {
            let context_name = settings
                .env
                .get("PLEXI_CONTEXT_NAME")
                .cloned()
                .unwrap_or_default();
            let context_root = settings.env.get("PLEXI_CONTEXT_ROOT").map(PathBuf::from);
            crate::pane_ops::create::derive_context_placeholder_title(
                &context_name,
                settings.working_directory.as_deref(),
                context_root.as_deref(),
            )
        } else {
            None
        };
        if let Some(ref placeholder) = placeholder_title {
            log::info!(
                "terminal_pane: initialized stable pane chrome for pane_id={id} placeholder_title={placeholder:?} provenance={spawned_by:?}"
            );
        }
        #[cfg(test)]
        let spawn_env = settings.env.clone();
        let program_status = crate::program_status::SharedProgramStatus::default();
        settings.byte_tap = Some(egui_term::ByteTapHandle::new(
            crate::program_status::ProgramStatusTap::new(id, program_status.clone()),
        ));
        let backend = match TerminalBackend::new(id, ctx, tx, settings) {
            Ok(b) => b,
            Err(e) => {
                log::error!("Failed to create terminal backend {id}: {e}");
                return None;
            }
        };
        let process_group = super::process_cleanup::PaneProcessGroup::record(backend.child_pid());
        Some(Self {
            _process_group: process_group,
            id,
            backend,
            exited: false,
            exit_status: None,
            name: None,
            placeholder_title,
            name_locked: false,
            font_size: default_font_size,
            ephemeral,
            parent_pane_id,
            spawned_by,
            pending_child_results: Vec::new(),
            agent_spawned,
            agent_harness,
            agent_session_id: None,
            agent_role,
            agent_permission_posture: None,
            last_blocked_reason: None,
            tail_preview: None,
            agent_completed: false,
            standby_session: None,
            standby_resume_command: None,
            standby_cwd: None,
            initial_cwd,
            last_resumed_command,
            pty_title: None,
            outside_workspace_cached: false,
            outside_workspace_checked_at: None,
            outside_workspace_root: None,
            hidden: false,
            fleet_result: None,
            fleet_seen: false,
            fleet_dismissed: false,
            fleet_done_unseen: false,
            fleet_since: crate::platform::clock::now_secs(),
            agent: None,
            agent_reported_at: None,
            observed_agent: None,
            last_observed_agent: None,
            reported_7501: None,
            activity: None,
            slots: HashMap::new(),
            last_pty_output_at: None,
            last_agent_activity_at: None,
            host_induced_until: None,
            host_induced_output_logged: false,
            seen_resize_commits: 0,
            program_status,
            #[cfg(test)]
            spawn_env,
        })
    }

    /// Return the active display name for pane chrome: the explicit or shell-reported
    /// title when present, falling back to the context-derived placeholder title.
    pub fn display_name(&self) -> Option<&str> {
        self.name.as_deref().or(self.placeholder_title.as_deref())
    }

    /// True if this terminal pane is or was an agent pane (configured harness/role,
    /// hook-reported agent, host-observed agent, or restored Standby session).
    pub fn is_agent_pane(&self) -> bool {
        self.agent_harness.is_some()
            || self.agent_role.is_some()
            || self.agent.is_some()
            || self.reported_7501.is_some()
            || self.observed_agent.is_some()
            || self.standby_session.is_some()
    }

    /// True when this terminal pane is a human-owned user session: spawned
    /// directly by the host for the user, with no agent harness, agent role,
    /// hook/observed agent run, Standby session, parent pane, or agent-spawned
    /// lineage (`PLEXI_AGENT_SPAWNED`). Panes an agent creates (including via
    /// `plexi pane new`) never qualify.
    pub fn is_human_owned(&self) -> bool {
        !self.is_agent_pane() && !self.agent_spawned && self.parent_pane_id.is_none()
    }

    /// Human-readable provenance badge for GUI headers (`"spawned by you"`, `"spawned by researcher"`, etc.).
    pub fn spawned_by_badge(&self) -> Option<String> {
        format_spawned_by_badge(self.spawned_by.as_deref())
    }

    /// True when this terminal pane is running an idle shell (not exited, not an
    /// agent pane, and no foreground child process is running on the PTY).
    pub fn is_idle_shell(&self) -> bool {
        if self.exited || self.is_agent_pane() {
            return false;
        }
        if matches!(
            self.activity,
            Some(crate::protocol::AgentState::Working | crate::protocol::AgentState::Completed)
        ) {
            return false;
        }
        let shell_pid = self.backend.child_pid();
        let fg = self.backend.foreground_pid();
        !fg.is_some_and(|fg| fg > 0 && fg != shell_pid as i32)
    }
    /// True when this agent pane is currently in lightweight `[Plexi Standby]`.
    pub fn is_standby(&self) -> bool {
        self.standby_session.is_some()
    }

    /// Formatted `[Plexi Standby]` label and resume instruction when this pane is in Standby.
    pub fn standby_banner(&self) -> Option<String> {
        let s = self.standby_session.as_ref()?;
        let label = self
            .name
            .as_deref()
            .or(self.agent_role.as_deref())
            .unwrap_or(&s.harness);
        let cwd = self
            .standby_cwd
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "unknown directory".to_string());
        if crate::app::is_finished_script_session(s) {
            return Some(format!(
                "[Plexi] {label} · {cwd}\nscript · finished ({:?})\n[done]",
                s.last_state
            ));
        }
        let session_line = standby_session_line(s, self.standby_resume_command.as_deref());
        Some(format!(
            "[Plexi Standby] {label} · {cwd}\n{session_line}\n[Enter] resume"
        ))
    }

    /// Exit `[Plexi Standby]` and dispatch the Harness Adapter resume command
    /// into the terminal's PTY in its restored `cwd`.
    pub fn resume_from_standby(&mut self) -> Option<String> {
        if self
            .standby_session
            .as_ref()
            .is_some_and(crate::app::is_finished_script_session)
        {
            return None;
        }
        let standby = self.standby_session.take()?;
        let harness = crate::cli::AgentHarness::parse(&standby.harness);
        let cmd = self.standby_resume_command.take().or_else(|| {
            harness.map(|harness| {
                let flags = standby
                    .config_name
                    .as_deref()
                    .and_then(|role| {
                        self.standby_cwd
                            .as_deref()
                            .or(self.initial_cwd.as_deref())
                            .map(|cwd| {
                                crate::cli::agent::resolved_session_harness_flags_in(
                                    Some(role),
                                    harness,
                                    &crate::config::config_dir(),
                                    Some(cwd),
                                )
                            })
                    })
                    .unwrap_or_default();
                harness.restore_command_with_flags(standby.session_id.as_deref(), &flags)
            })
        });
        if let Some(ref resume_cmd) = cmd {
            if harness.is_some_and(|h| {
                standby
                    .session_id
                    .as_deref()
                    .is_some_and(|sid| !h.session_exists(sid))
            }) {
                self.agent_session_id = None;
                if let Some(ref mut a) = self.agent {
                    a.session_id = None;
                }
            }
            let line = if standby.ephemeral {
                format!("{resume_cmd}; exit $?\n")
            } else {
                format!("{resume_cmd}\n")
            };
            self.backend
                .process_command(egui_term::BackendCommand::Write(line.into_bytes()));
            self.last_resumed_command = Some(resume_cmd.clone());
        }
        log::info!(
            "standby: resumed agent pane {} harness={} session_id={:?}",
            self.id,
            standby.harness,
            standby.session_id
        );
        Some(cmd.unwrap_or(standby.harness))
    }

    /// Capture a compact tail preview from the terminal buffer, falling back to
    /// any previously preserved `tail_preview`.
    pub fn capture_tail_preview(&self) -> Option<String> {
        if self.is_standby() {
            return self.tail_preview.clone().or_else(|| {
                self.standby_session
                    .as_ref()
                    .and_then(|s| s.tail_preview.clone())
            });
        }
        let lines: Vec<String> = self
            .backend
            .capture_lines(8)
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.trim().is_empty() && !l.contains("[Plexi Standby]"))
            .collect();
        if lines.is_empty() {
            self.tail_preview.clone()
        } else {
            let start = lines.len().saturating_sub(4);
            Some(lines[start..].join("\n"))
        }
    }

    /// Capture the saved Standby view tail, retaining blank rows between content.
    pub fn capture_standby_scrollback_tail(&self, lines: usize) -> Option<Vec<String>> {
        if self.is_standby() {
            return self
                .standby_session
                .as_ref()
                .and_then(|session| session.standby_scrollback_tail.clone());
        }
        let tail = self.backend.capture_lines(lines);
        (!tail.is_empty()).then_some(tail)
    }

    fn update_fleet_transition(
        &mut self,
        previous: Option<&crate::protocol::AgentState>,
        next: Option<&crate::protocol::AgentState>,
    ) {
        use crate::protocol::AgentState;
        if previous == next {
            return;
        }
        self.fleet_since = crate::platform::clock::now_secs();
        if next == Some(&AgentState::Completed)
            || (previous == Some(&AgentState::Working) && next == Some(&AgentState::Idle))
        {
            self.fleet_done_unseen = true;
            self.fleet_seen = false;
            self.fleet_dismissed = false;
        } else if next == Some(&AgentState::Working) {
            self.fleet_done_unseen = false;
            self.fleet_seen = false;
            self.fleet_dismissed = false;
            self.agent_completed = false;
        } else {
            self.fleet_dismissed = false;
        }
    }

    pub(crate) fn set_reported_7501(&mut self, reported: Option<crate::protocol::PaneAgentState>) {
        let now = std::time::Instant::now();
        let before = self.selected_agent(now).map(|a| a.state.clone());
        self.reported_7501 = reported;
        let after = self.selected_agent(now).map(|a| a.state.clone());
        self.update_fleet_transition(before.as_ref(), after.as_ref());
    }

    pub(crate) fn set_observed_agent(&mut self, observed: Option<crate::protocol::PaneAgentState>) {
        let now = std::time::Instant::now();
        let before = self.selected_agent(now).map(|a| a.state.clone());
        self.observed_agent = observed;
        let after = self.selected_agent(now).map(|a| a.state.clone());
        self.update_fleet_transition(before.as_ref(), after.as_ref());
    }

    /// Hook, observed and OSC 7501 state merged by `select_terminal_agent`.
    fn selected_agent(&self, now: std::time::Instant) -> Option<&crate::protocol::PaneAgentState> {
        select_terminal_agent(
            self.reported_7501.as_ref(),
            self.agent.as_ref(),
            self.agent_reported_at,
            self.observed_agent.as_ref(),
            now,
        )
    }

    /// Open a host-induced quiet window: the host just did something that
    /// makes the program repaint (`cause`: a committed resize).
    pub(crate) fn mark_host_induced(&mut self, now: std::time::Instant, cause: &str) {
        self.host_induced_until = Some(now + HOST_INDUCED_QUIET);
        self.host_induced_output_logged = false;
        log::info!(
            "terminal {}: host-induced quiet window {}ms ({cause})",
            self.id,
            HOST_INDUCED_QUIET.as_millis()
        );
    }

    /// True while PTY output is attributed to the host, not the agent.
    pub(crate) fn host_induced_quiet(&self, now: std::time::Instant) -> bool {
        self.host_induced_until.is_some_and(|until| now < until)
    }

    /// Open a quiet window when the backend committed a resize (SIGWINCH to
    /// the child) since the last call. Called once per rendered frame.
    pub(crate) fn observe_resize_commits(&mut self, now: std::time::Instant) {
        let commits = self.backend.resize_commits();
        if commits > self.seen_resize_commits {
            self.seen_resize_commits = commits;
            self.mark_host_induced(now, "resize");
        }
    }

    /// Stamp one PTY event. `last_pty_output_at` always advances (`pane send
    /// --submit` settles on every event). Agent activity advances outside a
    /// host-induced quiet window, and inside one while the agent is already
    /// active: a working agent keeps working through a resize drag, so only an
    /// idle agent's SIGWINCH repaint is discounted.
    pub(crate) fn record_pty_event(&mut self, now: std::time::Instant) {
        self.last_pty_output_at = Some(now);
        let already_active = self
            .last_agent_activity_at
            .is_some_and(|at| now.saturating_duration_since(at) < OBSERVED_AGENT_SETTLE);
        if !self.host_induced_quiet(now) || already_active {
            self.last_agent_activity_at = Some(now);
            return;
        }
        if !self.host_induced_output_logged {
            self.host_induced_output_logged = true;
            log::info!(
                "terminal {}: PTY output inside host-induced window ignored for agent state",
                self.id
            );
        }
    }

    /// Observed agent state from the agent-output settle: Idle once the agent
    /// has produced no output of its own for `OBSERVED_AGENT_SETTLE`.
    ///
    /// Before the agent has any output of its own (a fresh boot whose every
    /// byte landed in host-induced windows: the first layout resize, the
    /// focus it took), there is nothing to discount against, so the settle
    /// runs on the PTY's own last output. Otherwise the boot never settles.
    pub(crate) fn observed_output_state(
        &self,
        now: std::time::Instant,
    ) -> crate::protocol::AgentState {
        let settled = self
            .last_agent_activity_at
            .or(self.last_pty_output_at)
            .is_some_and(|at| now.saturating_duration_since(at) >= OBSERVED_AGENT_SETTLE);
        if settled {
            crate::protocol::AgentState::Idle
        } else {
            crate::protocol::AgentState::Working
        }
    }

    /// Completion takes precedence over a failed process exit, then the report.
    pub(crate) fn effective_agent_state(&self) -> (crate::protocol::AgentState, Option<String>) {
        let report = self.selected_agent(std::time::Instant::now());
        let state = report
            .map(|a| a.state.clone())
            .or_else(|| self.standby_session.as_ref().map(|s| s.last_state.clone()))
            .unwrap_or(crate::protocol::AgentState::Idle);
        let detail = report.and_then(|a| a.detail.clone()).or_else(|| {
            self.standby_session
                .as_ref()
                .and_then(|s| s.last_detail.clone())
        });
        Self::resolve_agent_state(
            self.agent_completed || self.slots.contains_key("completed"),
            self.exited.then_some(self.exit_status).flatten(),
            state,
            detail,
        )
    }

    fn resolve_agent_state(
        completed: bool,
        exit: Option<crate::host::pane_lifecycle::ExitStatus>,
        state: crate::protocol::AgentState,
        detail: Option<String>,
    ) -> (crate::protocol::AgentState, Option<String>) {
        if completed || state == crate::protocol::AgentState::Completed {
            (crate::protocol::AgentState::Completed, detail)
        } else if let Some(status) = exit.filter(|s| s.is_failure()) {
            (crate::protocol::AgentState::Failed, Some(status.summary()))
        } else if exit.is_some() && state == crate::protocol::AgentState::Working {
            (crate::protocol::AgentState::Interrupted, detail)
        } else {
            (state, detail)
        }
    }

    /// Build the persisted [`crate::workspace::SavedAgentSession`] for this pane
    /// if it is an agent pane.
    pub fn build_saved_agent_session(&self) -> Option<crate::workspace::SavedAgentSession> {
        self.build_saved_agent_session_with_scrollback(20)
    }

    /// Build the persisted agent session with a bounded Standby scrollback tail.
    pub fn build_saved_agent_session_with_scrollback(
        &self,
        standby_scrollback_lines: usize,
    ) -> Option<crate::workspace::SavedAgentSession> {
        if !self.is_agent_pane() {
            return None;
        }
        if let Some(ref standby) = self.standby_session {
            let mut s = standby.clone();
            if let Some(ref sid) = self.agent_session_id {
                s.session_id = Some(sid.clone());
            }
            if let Some(ref role) = self.agent_role {
                s.config_name = Some(role.clone());
            }
            if self.agent_permission_posture.is_some() {
                s.permission_posture = self.agent_permission_posture.clone();
            }
            (s.last_state, s.last_detail) = self.effective_agent_state();
            if self.last_blocked_reason.is_some() {
                s.last_blocked_reason = self.last_blocked_reason;
            }
            if self.parent_pane_id.is_some() {
                s.parent_pane_id = self.parent_pane_id;
            }
            if self.spawned_by.is_some() {
                s.spawned_by = self.spawned_by.clone();
            }
            if !self.pending_child_results.is_empty() {
                s.pending_child_results = self.pending_child_results.clone();
            }
            s.fleet_result = self.fleet_result.clone();
            s.fleet_seen = self.fleet_seen;
            s.fleet_dismissed = self.fleet_dismissed;
            s.fleet_done_unseen = self.fleet_done_unseen;
            s.fleet_since = self.fleet_since;
            s.ephemeral = self.ephemeral;
            if s.tail_preview.is_none() {
                s.tail_preview = self.tail_preview.clone();
            }
            return Some(s);
        }

        let harness = self
            .agent_harness
            .clone()
            .or_else(|| self.agent.as_ref().map(|a| a.agent.clone()))
            .or_else(|| self.observed_agent.as_ref().map(|a| a.agent.clone()))
            .unwrap_or_else(|| "claude".to_string());
        let session_id = self
            .agent_session_id
            .clone()
            .or_else(|| self.agent.as_ref().and_then(|a| a.session_id.clone()));
        let (last_state, last_detail) = self.effective_agent_state();

        Some(crate::workspace::SavedAgentSession {
            fleet_result: self.fleet_result.clone(),
            fleet_seen: self.fleet_seen,
            fleet_dismissed: self.fleet_dismissed,
            fleet_done_unseen: self.fleet_done_unseen,
            fleet_since: self.fleet_since,
            harness,
            config_name: self.agent_role.clone(),
            permission_posture: self.agent_permission_posture.clone(),
            session_id,
            last_state,
            last_blocked_reason: self.last_blocked_reason,
            last_detail,
            parent_pane_id: self.parent_pane_id,
            spawned_by: self.spawned_by.clone(),
            pending_child_results: self.pending_child_results.clone(),
            ephemeral: self.ephemeral,
            tail_preview: self.capture_tail_preview(),
            standby_scrollback_tail: self.capture_standby_scrollback_tail(standby_scrollback_lines),
        })
    }

    /// Build a closed-run [`crate::workspace::AgentHistoryRecord`] for this pane
    /// if it is an agent pane.
    pub fn build_history_record(
        &self,
        context_id: u64,
        context_name: &str,
        cwd: PathBuf,
    ) -> Option<crate::workspace::AgentHistoryRecord> {
        let session = self.build_saved_agent_session()?;
        let summary = self
            .slots
            .get("summary")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| session.last_detail.clone());
        let result = self
            .slots
            .get("result")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .or_else(|| self.fleet_result.clone());
        let closed_at = crate::platform::clock::now_secs();
        let run_id = format!("run-{}-{closed_at}", self.id);
        Some(crate::workspace::AgentHistoryRecord {
            fleet_seen: self.fleet_seen,
            fleet_dismissed: self.fleet_dismissed,
            run_id,
            pane_id: self.id,
            context_id,
            context_name: context_name.to_string(),
            parent_pane_id: session.parent_pane_id,
            spawned_by: session.spawned_by,
            pane_name: self.name.clone(),
            harness: session.harness,
            config_name: session.config_name,
            permission_posture: session.permission_posture,
            session_id: session.session_id,
            cwd,
            ephemeral: session.ephemeral,
            last_state: session.last_state,
            last_blocked_reason: session.last_blocked_reason,
            last_detail: session.last_detail,
            summary,
            result,
            tail_preview: session.tail_preview,
            slots: self.slots.clone(),
            closed_at,
        })
    }

    /// Build the CLI resume invocation (`claude --resume <id>`, `codex resume <id>`,
    /// `pi --session <id>`, `agy --resume <id>`) when both a harness and session ID
    /// have been captured for this terminal pane.
    pub fn resume_command(&self) -> Option<String> {
        let harness_str = self
            .agent_harness
            .as_deref()
            .or_else(|| self.standby_session.as_ref().map(|s| s.harness.as_str()))
            .or_else(|| self.agent.as_ref().map(|a| a.agent.as_str()))
            .or_else(|| self.observed_agent.as_ref().map(|a| a.agent.as_str()))?;
        let harness = crate::cli::AgentHarness::parse(harness_str)?;
        let session_id = self
            .agent_session_id
            .as_deref()
            .or_else(|| {
                self.standby_session
                    .as_ref()
                    .and_then(|s| s.session_id.as_deref())
            })
            .or_else(|| self.agent.as_ref().and_then(|a| a.session_id.as_deref()))?;
        let flags = self
            .standby_session
            .as_ref()
            .map(|session| session.config_name.as_deref())
            .unwrap_or(self.agent_role.as_deref())
            .and_then(|role| {
                self.initial_cwd.as_deref().map(|cwd| {
                    crate::cli::agent::resolved_session_harness_flags_in(
                        Some(role),
                        harness,
                        &crate::config::config_dir(),
                        Some(cwd),
                    )
                })
            })
            .unwrap_or_default();
        Some(harness.resume_command_with_flags(session_id, &flags))
    }
}

/// Badge for a pane its own person opened. It labels an existing header bar but
/// never forces one: every plain shell is human-spawned.
pub const SPAWNED_BY_HUMAN_BADGE: &str = "spawned by you";

/// Format an unspoofable `spawned_by` provenance string into a concise GUI badge.
pub fn format_spawned_by_badge(raw: Option<&str>) -> Option<String> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }
    if s == "human" {
        return Some(SPAWNED_BY_HUMAN_BADGE.to_string());
    }
    if let Some(agent_name) = s.strip_prefix("agent:") {
        let name = agent_name.trim();
        if !name.is_empty() {
            return Some(format!("spawned by {name}"));
        }
    }
    if let Some(team_spec) = s.strip_prefix("team:") {
        let spec = team_spec.trim();
        if !spec.is_empty() {
            return Some(format!("spawned by team:{spec}"));
        }
    }
    if s == "system" {
        return Some("spawned by system".to_string());
    }
    Some(format!("spawned by {s}"))
}

#[cfg(test)]
mod agent_source_tests {
    use super::*;
    use crate::protocol::{AgentState, PaneAgentState};

    #[test]
    fn effective_agent_state_precedence_table() {
        use crate::host::pane_lifecycle::ExitStatus;
        for (completed, exit, reported, expected, detail) in [
            (
                true,
                Some(ExitStatus::Code(3)),
                AgentState::Working,
                AgentState::Completed,
                "report",
            ),
            (
                false,
                Some(ExitStatus::Code(3)),
                AgentState::Completed,
                AgentState::Completed,
                "report",
            ),
            (
                false,
                Some(ExitStatus::Code(3)),
                AgentState::Working,
                AgentState::Failed,
                "exited (3)",
            ),
            (
                false,
                Some(ExitStatus::Code(0)),
                AgentState::Working,
                AgentState::Interrupted,
                "report",
            ),
            (
                false,
                None,
                AgentState::Blocked,
                AgentState::Blocked,
                "report",
            ),
        ] {
            let actual = super::TerminalPane::resolve_agent_state(
                completed,
                exit,
                reported,
                Some("report".into()),
            );
            assert_eq!(actual, (expected, Some(detail.into())));
        }
    }

    use std::time::{Duration, Instant};

    fn saved_session(
        harness: &str,
        session_id: Option<&str>,
    ) -> crate::workspace::SavedAgentSession {
        crate::workspace::SavedAgentSession {
            fleet_result: None,
            fleet_seen: false,
            fleet_done_unseen: false,
            fleet_dismissed: false,
            fleet_since: 0,
            harness: harness.to_string(),
            config_name: None,
            permission_posture: None,
            session_id: session_id.map(str::to_string),
            last_state: AgentState::Blocked,
            last_blocked_reason: None,
            last_detail: None,
            parent_pane_id: None,
            spawned_by: None,
            pending_child_results: Vec::new(),
            ephemeral: false,
            tail_preview: None,
            standby_scrollback_tail: None,
        }
    }

    #[test]
    fn standby_banner_describes_resume_and_new_session_cases() {
        assert_eq!(
            standby_session_line(
                &saved_session("codex", Some("thread-0123456789")),
                Some("codex resume thread-0123456789"),
            ),
            "codex · resumes session thread-01234 (blocked)"
        );
        assert_eq!(
            standby_session_line(&saved_session("claude", None), Some("claude")),
            "claude · no saved session, Enter starts a new one"
        );

        assert_eq!(
            standby_session_line(&saved_session("pi", Some("missing-session")), Some("pi")),
            "pi · session missing-sess not found, Enter starts a new one"
        );
    }

    fn state(state: AgentState) -> PaneAgentState {
        PaneAgentState {
            pane_id: 7,
            state,
            agent: "codex".to_string(),
            detail: None,
            session_id: None,
        }
    }

    #[test]
    fn fresh_hook_idle_remains_authoritative_over_observed_working() {
        let now = Instant::now();
        let hook = state(AgentState::Idle);
        let observed = state(AgentState::Working);
        let selected = select_terminal_agent(
            None,
            Some(&hook),
            Some(now - Duration::from_millis(100)),
            Some(&observed),
            now,
        )
        .expect("agent");
        assert_eq!(selected.state, AgentState::Idle);
    }

    #[test]
    fn stale_hook_idle_yields_to_observed_working_then_returns_idle() {
        let now = Instant::now();
        let hook = state(AgentState::Idle);
        let working = state(AgentState::Working);
        let idle = state(AgentState::Idle);
        let stale_at = now - HOOK_AGENT_FRESHNESS - Duration::from_millis(1);

        let selected =
            select_terminal_agent(None, Some(&hook), Some(stale_at), Some(&working), now)
                .expect("agent");
        assert_eq!(selected.state, AgentState::Working);

        let selected = select_terminal_agent(None, Some(&hook), Some(stale_at), Some(&idle), now)
            .expect("agent");
        assert_eq!(selected.state, AgentState::Idle);
    }

    #[test]
    fn ps7501_reported_state_beats_hook_and_observed() {
        let now = Instant::now();
        let reported = state(AgentState::NeedsYou);
        let hook = state(AgentState::Working);
        let observed = state(AgentState::Idle);
        let selected = select_terminal_agent(
            Some(&reported),
            Some(&hook),
            Some(now),
            Some(&observed),
            now,
        )
        .expect("agent");
        assert_eq!(selected.state, AgentState::NeedsYou);
    }

    #[test]
    fn ps7501_no_record_leaves_old_selection() {
        let now = Instant::now();
        let hook = state(AgentState::Working);
        let observed = state(AgentState::Idle);
        let selected = select_terminal_agent(None, Some(&hook), Some(now), Some(&observed), now)
            .expect("agent");
        assert_eq!(selected.state, AgentState::Working);
        let selected =
            select_terminal_agent(None, None, None, Some(&observed), now).expect("agent");
        assert_eq!(selected.state, AgentState::Idle);
        assert!(select_terminal_agent(None, None, None, None, now).is_none());
    }

    #[test]
    fn hook_working_is_never_overridden() {
        let now = Instant::now();
        let hook = state(AgentState::Working);
        let observed = state(AgentState::Idle);
        let selected = select_terminal_agent(
            None,
            Some(&hook),
            Some(now - HOOK_AGENT_FRESHNESS - Duration::from_secs(1)),
            Some(&observed),
            now,
        )
        .expect("agent");
        assert_eq!(selected.state, AgentState::Working);
    }
}

// ---------------------------------------------------------------------------
// ViewPane — dedicated plugin runtime (process or in-process builtin)
// ---------------------------------------------------------------------------

/// What a `PendingPaneClick` is aimed at: a raw screen position (the pixel
/// `plexi pane click <x> <y>` path), or a specific WASM UI-tree arena node id
/// (the node-addressed `plexi pane click <pane_id> --node <node_id>` sibling,
/// stint 0414). `node_id` matches `SemanticPaneNode.id` — the same id
/// `plexi pane state` reports for every node in the tree.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PaneClickTarget {
    Pos(egui::Pos2),
    Node(u64),
}

/// A pointer click injected by `HostRequest::ClickPane`/`ClickPaneNode`
/// (`plexi pane click`, or `HostHarness::inject_click`/`inject_node_click`),
/// queued on `PlexiApp` and consumed by the canvas render pass for the
/// target pane on the next frame it renders.
///
/// egui's own click detection (`Response::clicked()`) resolves
/// `viewport.interact_widgets` once, inside `Context::begin_pass`, strictly
/// before host/plugin code runs — so a click cannot be faked by mutating
/// `ctx.input_mut()` mid-pass the way `KeyPane` replays keys (key checks
/// re-scan `InputState.events` live at widget-render time; pointer-click
/// detection does not). `PendingPaneClick` is delivered instead through the
/// same call chain a real click's hit-test result would reach —
/// `render_node`'s Canvas branch for `Pos`, or the matching interactive
/// node's own branch (Button/TextInput/ListView) for `Node` — so it still
/// exercises the pane's live rendering for the frame's actual widget, just
/// via an explicit id/rect-containment check instead of `Sense::click()`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingPaneClick {
    pub target: PaneClickTarget,
    pub button: &'static str,
    /// Which pointer transition this sample delivers. `Click` (the default,
    /// what `ClickPane`/`ClickPaneNode` queue) is a press+release pair;
    /// `Press`/`Move`/`Release` are the per-frame samples of an
    /// `HostRequest::DragPane` schedule (stint 0510) — one sample per frame,
    /// resolved against that frame's live canvas rect like any click.
    pub phase: PointerPhase,
}

/// Pointer transition carried by one [`PendingPaneClick`] sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerPhase {
    Click,
    Press,
    Move,
    Release,
}

impl PendingPaneClick {
    /// True when this pending click's target identity-matches `id` — the same
    /// honest node-id equality check `node_click_matches` (wasm_render.rs)
    /// uses on the WASM/Python path, applied to a Builtin plugin's own `egui::Id`
    /// instead of a WASM arena index. Drag-phase samples never activate a
    /// node — only a real `Click` does.
    pub(crate) fn targets_id(&self, id: egui::Id) -> bool {
        self.phase == PointerPhase::Click
            && matches!(self.target, PaneClickTarget::Node(n) if n == id.value())
    }
}

pub enum ViewRuntime {
    Builtin(Box<dyn View>),
    Python(Box<crate::host::wasm_python::LivePythonPane>),
    /// A sandboxed WASM component plugin driven by the synchronous effect loop.
    /// Neither a subprocess nor a native in-process builtin — it runs inside a
    /// per-pane `wasmtime::Store`.
    Wasm(Box<crate::host::wasm_pane::LiveWasmPane>),
}

impl ViewRuntime {
    pub fn submit_external_turn(
        &mut self,
        text: String,
        request_id: String,
        response_file: String,
        opts: crate::app::view_trait::ExternalTurnOpts,
    ) -> Result<(), String> {
        match self {
            ViewRuntime::Builtin(app) => {
                app.submit_external_turn(text, request_id, response_file, opts)
            }
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => {
                Err("this plugin does not accept external turns".to_string())
            }
        }
    }

    pub fn take_config_saved(&mut self) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.take_config_saved(),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => false,
        }
    }

    pub fn switch_commands_scope(&mut self) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.switch_commands_scope(),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => false,
        }
    }

    pub fn captures_editor_shortcuts(&self) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.captures_editor_shortcuts(),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => false,
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &ViewRenderContext<'_>,
        pending_click: Option<PendingPaneClick>,
    ) {
        match self {
            ViewRuntime::Builtin(app) => app.ui(ui, ctx, pending_click),
            ViewRuntime::Python(app) => app.ui(ui, ctx.colors, pending_click, ctx.pane_id),
            ViewRuntime::Wasm(app) => app.ui(ui, ctx.colors, pending_click, ctx.pane_id),
        }
    }

    pub fn handle_key(
        &mut self,
        input: &crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        match self {
            ViewRuntime::Builtin(app) => app.handle_key(input),
            ViewRuntime::Python(app) => app.handle_key(input),
            ViewRuntime::Wasm(app) => app.handle_key(input),
        }
    }

    /// Deliver the one bare Escape that just surrendered a declarative
    /// `TextInput`'s egui focus. Ordinary key routing reserves Escape for the
    /// host CloseView binding; this narrowly-scoped path lets the field's plugin
    /// react to its own focus-loss Escape before that binding is suppressed.
    pub fn handle_text_input_escape(
        &mut self,
        input: &crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        match self {
            // Preserve the existing builtin TextInput Escape behavior. The
            // caller consumes this one focus-loss Escape before CloseView runs.
            ViewRuntime::Builtin(app) => app.handle_key(input),
            ViewRuntime::Python(app) => app.handle_text_input_escape(input),
            ViewRuntime::Wasm(app) => app.handle_text_input_escape(input),
        }
    }

    pub fn take_pending_commands(&mut self) -> Vec<ViewCommand> {
        match self {
            ViewRuntime::Builtin(app) => app.take_pending_commands(),
            ViewRuntime::Python(app) => app.take_pending_commands(),
            ViewRuntime::Wasm(app) => app
                .take_host_effects()
                .into_iter()
                .map(|effect| ViewCommand::WasmHostEffect { pane_id: 0, effect })
                .collect(),
        }
    }

    pub fn keyboard_capture(&self) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.keyboard_capture(),
            ViewRuntime::Python(_) => false,
            ViewRuntime::Wasm(_) => false,
        }
    }

    /// Whether this runtime asks the host to route bare Enter to its action
    /// handler while a declarative text input has focus.
    pub fn submit_on_focused_text_input_enter(&self) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.submit_on_focused_text_input_enter(),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => false,
        }
    }

    pub fn wants_close(&self) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.wants_close(),
            ViewRuntime::Python(app) => app.wants_close(),
            ViewRuntime::Wasm(app) => app.wants_close(),
        }
    }

    /// Every state file this runtime persists to, resolved against its live
    /// context root. Only file-backed runtimes (Python today) have any.
    pub fn state_paths(&self) -> Vec<(crate::host::state_scope::StateScope, std::path::PathBuf)> {
        match self {
            ViewRuntime::Python(app) => app.state_paths(),
            ViewRuntime::Builtin(_) | ViewRuntime::Wasm(_) => Vec::new(),
        }
    }

    /// Re-read one scope's state file after an external change (see
    /// `LivePythonPane::apply_external_state`). No-op for runtimes without
    /// file-backed state.
    pub fn apply_external_state(&mut self, scope: crate::host::state_scope::StateScope) {
        match self {
            ViewRuntime::Python(app) => app.apply_external_state(scope),
            ViewRuntime::Builtin(_) | ViewRuntime::Wasm(_) => {}
        }
    }

    pub fn queue_outbound_event(&mut self, event: crate::protocol::PlexiEvent) {
        match self {
            ViewRuntime::Builtin(app) => app.queue_outbound_event(event),
            ViewRuntime::Python(app) => app.queue_outbound_event(event),
            ViewRuntime::Wasm(app) => app.queue_outbound_event(event),
        }
    }

    /// Returns None when this runtime does not support NotifyAction.
    pub(crate) fn try_queue_notify_action(
        &mut self,
        event: crate::protocol::PlexiEvent,
    ) -> Option<Result<(), crate::host::wasm_python::OutboundEventError>> {
        if !self.supports_notify_action() {
            return None;
        }
        match self {
            ViewRuntime::Python(app) => Some(app.try_queue_outbound_event(event)),
            ViewRuntime::Builtin(_) | ViewRuntime::Wasm(_) => None,
        }
    }

    pub(crate) fn supports_notify_action(&self) -> bool {
        matches!(self, ViewRuntime::Python(_))
            && matches!(self.lifecycle().0, "starting" | "running")
    }

    /// Register a directed pipe on this runtime's local pipe registry so
    /// `has_pipe_reader` reflects the connection on the target runtime.
    pub fn register_directed_pipe(&mut self, pipe_id: &str) -> bool {
        match self {
            ViewRuntime::Builtin(_) => false,
            ViewRuntime::Python(app) => app.register_directed_pipe(pipe_id),
            ViewRuntime::Wasm(app) => app.register_directed_pipe(pipe_id),
        }
    }

    /// True when `pipe_id` is open on this runtime's local pipe registry.
    pub fn has_pipe_reader(&self, pipe_id: &str) -> bool {
        match self {
            ViewRuntime::Builtin(_) => false,
            ViewRuntime::Python(app) => app.has_pipe_reader(pipe_id),
            ViewRuntime::Wasm(app) => app.has_pipe_reader(pipe_id),
        }
    }

    pub fn runtime_kind(&self) -> &'static str {
        match self {
            ViewRuntime::Builtin(_) => "builtin",
            ViewRuntime::Python(_) => "python",
            ViewRuntime::Wasm(_) => "wasm",
        }
    }

    /// Deliver the CLI/host semantic action contract to every plugin runtime.
    pub fn approval_request_id(&self) -> Option<String> {
        match self {
            ViewRuntime::Builtin(app) => app.approval_request_id(),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => None,
        }
    }

    pub fn send_plugin_action(&mut self, action: String, args: Vec<String>) -> Result<(), String> {
        match self {
            ViewRuntime::Wasm(app) => {
                if !args.is_empty() {
                    return Err("WASM UI actions do not accept positional arguments".to_string());
                }
                app.dispatch_ui_action(action)
            }
            ViewRuntime::Builtin(app) => {
                app.queue_outbound_event(crate::protocol::PlexiEvent::Action { action, args });
                Ok(())
            }
            ViewRuntime::Python(app) => {
                app.queue_outbound_event(crate::protocol::PlexiEvent::Action { action, args });
                Ok(())
            }
        }
    }

    pub(crate) fn tool_event_sender(
        &self,
        repaint: egui::Context,
    ) -> Option<crate::plexi_ai::tool_dispatch::PluginEventSender> {
        match self {
            ViewRuntime::Python(app) => Some(
                crate::plexi_ai::tool_dispatch::PluginEventSender::Python(app.tool_event_sender()),
            ),
            ViewRuntime::Wasm(app) => Some(
                crate::plexi_ai::tool_dispatch::PluginEventSender::Wasm(app.input_sender(repaint)),
            ),
            ViewRuntime::Builtin(_) => None,
        }
    }

    pub fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        match self {
            ViewRuntime::Builtin(app) => app.sync_cwd(new_cwd),
            ViewRuntime::Python(_) => {}
            ViewRuntime::Wasm(_) => {}
        }
    }

    pub(crate) fn semantic_details(&self) -> Option<serde_json::Value> {
        match self {
            ViewRuntime::Builtin(app) => app.semantic_state(),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => None,
        }
    }

    pub(crate) fn request_close(&mut self) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.request_close(),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => true,
        }
    }

    pub(crate) fn service_external(
        &mut self,
        wake: &std::sync::Arc<dyn crate::app::ui_mailbox::UiWake>,
    ) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.service_external(wake),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => false,
        }
    }

    pub(crate) fn sync_change_set(&mut self) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.sync_change_set(),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => false,
        }
    }

    pub(crate) fn editor_buffer(&self) -> Option<crate::host::changes::OpenEditorBuffer> {
        match self {
            ViewRuntime::Builtin(app) => app.editor_buffer(),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => None,
        }
    }

    /// Lifecycle is an observation of the runtime, not a pane-existence flag.
    pub(crate) fn lifecycle(&self) -> (&'static str, Option<&str>) {
        match self {
            ViewRuntime::Builtin(_) => ("running", None),
            ViewRuntime::Python(app) => app.lifecycle(),
            ViewRuntime::Wasm(app) if app.is_running() => ("running", None),
            ViewRuntime::Wasm(_) => ("exited", None),
        }
    }

    /// Poll state that must stay current even when eframe skips rendering an
    /// occluded host window.
    pub(crate) fn poll_runtime_state(&mut self) {
        if let ViewRuntime::Python(app) = self {
            app.poll_runtime_state();
        }
    }

    pub(crate) fn drop_file(&mut self, path_or_url: &str) -> Result<serde_json::Value, String> {
        match self {
            ViewRuntime::Builtin(app) => app.drop_file(path_or_url),
            ViewRuntime::Python(_) | ViewRuntime::Wasm(_) => {
                Err("target plugin does not accept file drops".to_string())
            }
        }
    }

    pub fn type_id(&self) -> &'static str {
        match self {
            ViewRuntime::Builtin(app) => app.type_id(),
            ViewRuntime::Python(_) => "python-wasm",
            ViewRuntime::Wasm(_) => "wasm",
        }
    }

    pub fn serialize_state(&self) -> Option<serde_json::Value> {
        match self {
            ViewRuntime::Builtin(app) => app.serialize_state(),
            ViewRuntime::Python(_) => None,
            // WASM plugin state is persisted host-side via the file-backed
            // StateStore, not through workspace JSON.
            ViewRuntime::Wasm(_) => None,
        }
    }

    /// Enough launch context to relaunch this exact plugin through the normal
    /// open path on a workspace restore (stint 0680). `None` for builtins —
    /// they restore through `restore_builtin_view_pane`'s `builtin_factory`
    /// lookup instead, which needs no relaunch context.
    pub fn launch_spec(&self) -> Option<crate::workspace::SavedViewLaunch> {
        match self {
            ViewRuntime::Builtin(_) => None,
            ViewRuntime::Python(app) => Some(app.launch_spec()),
            ViewRuntime::Wasm(app) => Some(app.launch_spec()),
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            ViewRuntime::Builtin(app) => app.display_name(),
            ViewRuntime::Python(app) => app.display_name(),
            ViewRuntime::Wasm(app) => app.display_name(),
        }
    }

    /// Seed text for the rename-pane overlay. See [`App::rename_seed`].
    pub fn rename_seed(&self) -> Option<String> {
        match self {
            ViewRuntime::Builtin(app) => app.rename_seed(),
            ViewRuntime::Python(_) => None,
            ViewRuntime::Wasm(_) => None,
        }
    }

    /// Notify the plugin that its pane was renamed. See [`App::on_pane_renamed`].
    pub fn on_pane_renamed(&mut self, name: &str) {
        match self {
            ViewRuntime::Builtin(app) => app.on_pane_renamed(name),
            ViewRuntime::Python(_) => {}
            ViewRuntime::Wasm(_) => {}
        }
    }

    /// Push the pane's current context root into runtimes that resolve
    /// state paths at call time. Builtins and WASM component panes keep
    /// their existing schemes and ignore it.
    pub fn refresh_context_root(&mut self, root: &std::path::Path) {
        match self {
            ViewRuntime::Builtin(_) => {}
            ViewRuntime::Python(app) => app.set_context_root(root),
            ViewRuntime::Wasm(_) => {}
        }
    }

    /// Pump event I/O for a pane not in the active context.
    pub fn background_tick(&mut self) {
        match self {
            ViewRuntime::Builtin(app) => app.background_tick(),
            ViewRuntime::Python(app) => app.background_tick(),
            ViewRuntime::Wasm(app) => app.background_tick(),
        }
    }

    /// Does this pane have pending background work that `background_tick`
    /// would make progress on?
    pub fn needs_background_tick(&self) -> bool {
        match self {
            ViewRuntime::Builtin(app) => app.needs_background_tick(),
            ViewRuntime::Python(app) => app.needs_background_tick(),
            ViewRuntime::Wasm(app) => app.needs_background_tick(),
        }
    }

    /// Current nav stack depth as reported by the plugin via `PushNav`/`PopNav`.
    /// Always 0 for builtin plugins — they manage their own internal navigation.
    pub fn nav_stack_depth(&self) -> usize {
        match self {
            ViewRuntime::Builtin(_) => 0,
            ViewRuntime::Python(_) => 0,
            ViewRuntime::Wasm(_) => 0,
        }
    }

    /// Title of the current top-of-stack view for pane chrome display.
    /// `None` when the stack is empty (root view — no back arrow shown).
    pub fn nav_top_title(&self) -> Option<&str> {
        match self {
            ViewRuntime::Builtin(_) => None,
            ViewRuntime::Python(_) => None,
            ViewRuntime::Wasm(_) => None,
        }
    }

    /// The `view_id` the plugin should navigate back to (the entry below current
    /// top, or empty string for root). Used to populate `NavBack { view_id }`.
    pub fn nav_back_view_id(&self) -> String {
        match self {
            ViewRuntime::Builtin(_) => String::new(),
            ViewRuntime::Python(_) => String::new(),
            ViewRuntime::Wasm(_) => String::new(),
        }
    }

    pub(crate) fn set_pending_notification_count(&mut self, count: usize) {
        let _ = count;
    }

    /// Serialize the last-rendered frame (Vec<RenderCommand>) as a JSON array.
    /// Returns `None` for builtin plugins (no accessible frame).
    pub(crate) fn frame_json(&self) -> Option<serde_json::Value> {
        match self {
            ViewRuntime::Builtin(_) => None,
            ViewRuntime::Python(_) => None,
            ViewRuntime::Wasm(_) => None,
        }
    }
}

pub struct ViewPane {
    pub id: PaneId,
    pub runtime: ViewRuntime,
    pub workspace_root: PathBuf,
    pub permissions: PluginPermissions,
    pub manifest_id: String,
    pub name: String,
    /// Pane group this plugin joined at spawn. Only `sync_plugin_cwd` reads it, and
    /// only builtin panes are ever assigned one — see `LaunchSection::join_group`.
    pub pane_group: Option<String>,
    /// The terminal pane this plugin was spawned alongside. CdRequest routes here
    /// directly — no tile-tree walk needed.
    pub linked_pane_id: Option<PaneId>,
    /// Pane hidden by an overlay plugin. Closing the plugin restores this pane instead
    /// of deleting the tile.
    pub overlay_replaced: Option<Box<Pane>>,
    /// When true, the pane is visually deprioritized (outline dot, dimmed tab title).
    pub hidden: bool,
    pub agent: Option<crate::protocol::PaneAgentState>,
    /// App-reported pip status (red/yellow/green). Takes priority over derived
    /// activity for the activity dot. `None` = fall back to agent()/host-observed.
    pub pip_status: Option<crate::protocol::PipStatus>,
    pub slots: HashMap<String, PathBuf>,
    /// Last semantics committed by the production render path for native plugins.
    pub(crate) semantic_state: SemanticPaneState,
}

impl ViewPane {
    pub(crate) fn semantic_state(&self) -> SemanticPaneState {
        match &self.runtime {
            ViewRuntime::Builtin(_) => self.semantic_state.clone(),
            ViewRuntime::Python(app) => app.semantic_state(),
            ViewRuntime::Wasm(app) => app.semantic_state().clone(),
        }
    }
}

impl Drop for ViewPane {
    fn drop(&mut self) {
        // Overlay plugins reuse the replaced pane's id. Restoring the boxed pane
        // must preserve that pane's connector and subscription registrations.
        if self.overlay_replaced.is_some() {
            log::info!(
                "view pane: dropped overlay pane={} without clearing restored pane registrations",
                self.id
            );
            return;
        }
        crate::plexi_ai::tool_dispatch::unregister(self.id);
        let (subscriptions, deliveries) = crate::host::plugin_timeline::global()
            .lock()
            .unwrap()
            .clear_subscriber(
                crate::broker::ActorType::Plugin,
                &crate::host::event_subscriptions::plugin_subscriber_id(self.id),
            );
        log::info!(
            "view pane: dropped pane={} and cleared connector tools, {} event subscription(s), {} queued delivery(ies)",
            self.id,
            subscriptions,
            deliveries
        );
    }
}

#[cfg(test)]
mod semantic_state_tests {
    use super::*;

    #[test]
    fn process_frame_normalization_preserves_text_and_runtime_kind() {
        let frame = serde_json::json!([
            {"type": "text", "text": "process label", "x": 1.0, "y": 2.0},
            {"type": "rect", "hit_region": "open-settings"}
        ]);

        let state = SemanticPaneState::from_process_frame(&frame);

        assert_eq!(state.schema_version, SEMANTIC_PANE_STATE_SCHEMA_VERSION);
        assert_eq!(state.runtime_kind, "process");
        assert!(state
            .nodes
            .iter()
            .any(|node| { node.role == "text" && node.label.as_deref() == Some("process label") }));
    }

    #[test]
    fn default_builtin_state_has_valid_versioned_metadata() {
        let state = SemanticPaneState::default();

        assert_eq!(state.schema_version, SEMANTIC_PANE_STATE_SCHEMA_VERSION);
        assert_eq!(state.runtime_kind, "builtin");
    }

    #[test]
    fn accesskit_normalization_removes_children_outside_pane() {
        let parent_id = egui::Id::new("parent");
        let inside_id = egui::Id::new("inside");
        let outside_id = egui::Id::new("outside");
        let mut parent = egui::accesskit::Node::new(egui::accesskit::Role::Group);
        parent.set_bounds(egui::accesskit::Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 100.0,
            y1: 100.0,
        });
        parent.push_child(inside_id.value().into());
        parent.push_child(outside_id.value().into());
        let node_at = |x0, x1| {
            let mut node = egui::accesskit::Node::new(egui::accesskit::Role::Label);
            node.set_bounds(egui::accesskit::Rect {
                x0,
                y0: 10.0,
                x1,
                y1: 20.0,
            });
            node
        };
        let mut nodes = egui::IdMap::default();
        nodes.insert(parent_id, parent);
        nodes.insert(inside_id, node_at(10.0, 20.0));
        nodes.insert(outside_id, node_at(200.0, 220.0));

        let state = SemanticPaneState::from_accesskit(
            &nodes,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(100.0, 100.0)),
        );
        let parent = state
            .nodes
            .iter()
            .find(|node| node.id == parent_id.value().to_string())
            .expect("parent retained");

        assert_eq!(parent.children, vec![inside_id.value().to_string()]);
    }

    fn node(id: &str, role: &str) -> SemanticPaneNode {
        SemanticPaneNode {
            id: id.to_string(),
            role: role.to_string(),
            label: None,
            value: None,
            children: Vec::new(),
            bounds: None,
            canvas_commands: Vec::new(),
        }
    }

    #[test]
    fn resolve_interactive_node_accepts_button_and_returns_arena_id() {
        let mut state = SemanticPaneState::empty("wasm");
        state.nodes = vec![node("0", "column"), node("5", "button")];

        assert_eq!(state.resolve_interactive_node("5"), Ok(5));
    }

    #[test]
    fn resolve_interactive_node_rejects_missing_node_id() {
        let state = SemanticPaneState::empty("wasm");

        let err = state
            .resolve_interactive_node("9999")
            .expect_err("missing node_id must fail loudly");
        assert!(err.contains("not found"), "unexpected error: {err}");
    }

    #[test]
    fn resolve_interactive_node_rejects_non_interactive_role() {
        let mut state = SemanticPaneState::empty("wasm");
        state.nodes = vec![node("1", "text")];

        let err = state
            .resolve_interactive_node("1")
            .expect_err("non-interactive role must fail loudly");
        assert!(err.contains("not interactive"), "unexpected error: {err}");
    }
}
