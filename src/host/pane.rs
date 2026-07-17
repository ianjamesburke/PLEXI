use crate::app::app_trait::{App, AppCommand, AppRenderContext};
use crate::app::permissions::AppPermissions;
use crate::spatial::tiling::PaneId;
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

pub(crate) const SEMANTIC_PANE_STATE_SCHEMA_VERSION: u32 = 2;

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

    pub(crate) fn from_wasm_tree(tree: &crate::host::wasm_app::UiTree) -> Self {
        use crate::host::wasm_app::UiNodeData;

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
                        Some(format!("{}{}", space.size, if space.grow { "+grow" } else { "" })),
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
                    UiNodeData::AppBar(app_bar) => (
                        "app_bar",
                        Some(app_bar.title.clone()),
                        (!app_bar.subtitle.is_empty()).then(|| app_bar.subtitle.clone()),
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

    pub(crate) fn expose_canvas_commands(&mut self, tree: &crate::host::wasm_app::UiTree) {
        use crate::host::wasm_app::UiNodeData;

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
    pub(crate) fn resolve_interactive_node(&self, node_id: &str) -> Result<u32, String> {
        const INTERACTIVE_ROLES: &[&str] = &["button", "text_input", "list_view"];

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
            .parse::<u32>()
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
    command: &crate::host::wasm_app::bindings::plexi::platform::types::CanvasCommand,
) -> serde_json::Value {
    use crate::host::wasm_app::bindings::plexi::platform::types::CanvasCommand;

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
    color: &crate::host::wasm_app::bindings::plexi::platform::types::Color,
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
    App(Box<AppPane>),
    /// A tile that represents a child context nested inside this one.
    /// Renders a summary card with pane count, status, and per-pane summaries.
    /// Cmd+Enter zooms into the sub-context when this tile has focus.
    Portal(Box<PortalPane>),
}

/// A portal pane points at a child context and caches its rolled-up state.
pub struct PortalPane {
    pub pane_id: PaneId,
    pub target_context_id: u64,
    pub context_state: Option<crate::host::context_state::ContextState>,
    /// When true, the pane is visually deprioritized (outline dot, dimmed tab title).
    pub hidden: bool,
}

impl Pane {
    pub fn id(&self) -> PaneId {
        match self {
            Pane::Terminal(t) => t.id,
            Pane::App(a) => a.id,
            Pane::Portal(p) => p.pane_id,
        }
    }

    pub fn is_hidden(&self) -> bool {
        match self {
            Pane::Terminal(t) => t.hidden,
            Pane::App(a) => a.hidden,
            Pane::Portal(p) => p.hidden,
        }
    }

    pub fn set_hidden(&mut self, val: bool) {
        match self {
            Pane::Terminal(t) => t.hidden = val,
            Pane::App(a) => a.hidden = val,
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

    pub fn as_app(&self) -> Option<&AppPane> {
        match self {
            Pane::App(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_app_mut(&mut self) -> Option<&mut AppPane> {
        match self {
            Pane::App(a) => Some(a),
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

    pub fn agent(&self) -> Option<&crate::app_protocol::PaneAgentState> {
        match self {
            Pane::Terminal(t) => t.agent.as_ref(),
            Pane::App(a) => a
                .agent
                .as_ref()
                .or_else(|| a.overlay_replaced.as_deref().and_then(Pane::agent)),
            Pane::Portal(_) => None,
        }
    }

    /// App-reported pip status, checking the outer app then any overlay-replaced
    /// pane underneath (mirrors `agent()`).
    pub fn pip_status(&self) -> Option<crate::app_protocol::PipStatus> {
        match self {
            Pane::App(a) => a
                .pip_status
                .or_else(|| a.overlay_replaced.as_deref().and_then(Pane::pip_status)),
            Pane::Terminal(_) | Pane::Portal(_) => None,
        }
    }

    /// Activity state for the unified activity dot. App-reported pip status wins;
    /// then hook-reported agent state; otherwise falls back to host-observed
    /// terminal activity (foreground process running / exited). Portals have no
    /// host-observed fallback yet.
    pub fn effective_activity(&self) -> Option<&crate::app_protocol::AgentState> {
        if let Some(pip) = self.pip_status() {
            return Some(pip.as_agent_state());
        }
        if let Some(a) = self.agent() {
            return Some(&a.state);
        }
        match self {
            Pane::Terminal(t) => t.activity.as_ref(),
            Pane::App(a) => a
                .overlay_replaced
                .as_deref()
                .and_then(Pane::effective_activity),
            Pane::Portal(_) => None,
        }
    }

    pub fn set_agent(&mut self, agent: Option<crate::app_protocol::PaneAgentState>) -> bool {
        match self {
            Pane::Terminal(t) => {
                t.agent = agent;
                true
            }
            Pane::App(a) => {
                if let Some(replaced) = a.overlay_replaced.as_deref_mut() {
                    replaced.set_agent(agent)
                } else {
                    a.agent = agent;
                    true
                }
            }
            Pane::Portal(_) => false,
        }
    }

    /// Set the app-reported pip status. App panes only (mirrors `set_agent`'s
    /// overlay delegation). Terminals and portals have no pip surface → false.
    pub fn set_pip_status(&mut self, status: Option<crate::app_protocol::PipStatus>) -> bool {
        match self {
            Pane::App(a) => {
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
            Pane::App(a) => Some(&a.slots),
            Pane::Portal(_) => None,
        }
    }

    pub fn slots_mut(&mut self) -> Option<&mut HashMap<String, PathBuf>> {
        match self {
            Pane::Terminal(t) => Some(&mut t.slots),
            Pane::App(a) => Some(&mut a.slots),
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
}

// ---------------------------------------------------------------------------
// TerminalPane — PTY-only state
// ---------------------------------------------------------------------------

pub struct TerminalPane {
    /// Pane ID — matches the key used in HashMap<PaneId, Pane>.
    pub id: PaneId,
    pub backend: TerminalBackend,
    pub exited: bool,
    pub name: Option<String>,
    /// When true, the name was set explicitly by the user and OSC title sequences must not overwrite it.
    pub name_locked: bool,
    pub font_size: f32,
    /// When true, the pane closes automatically when its process exits (no "[process exited]" prompt).
    /// Set by `plexi pane new --ephemeral`.
    pub ephemeral: bool,
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
    pub agent: Option<crate::app_protocol::PaneAgentState>,
    /// Host-observed terminal activity (foreground process running, exited),
    /// polled via `tcgetpgrp` in `tick_terminal_activity`. Separate from
    /// `agent`, which is hook-reported; `agent` wins when both are present.
    pub activity: Option<crate::app_protocol::AgentState>,
    pub slots: HashMap<String, PathBuf>,
}

impl TerminalPane {
    pub fn new(
        id: u64,
        ctx: egui::Context,
        tx: Sender<(u64, PtyEvent)>,
        settings: BackendSettings,
        default_font_size: f32,
    ) -> Option<Self> {
        let backend = match TerminalBackend::new(id, ctx, tx, settings) {
            Ok(b) => b,
            Err(e) => {
                log::error!("Failed to create terminal backend {id}: {e}");
                return None;
            }
        };
        Some(Self {
            id,
            backend,
            exited: false,
            name: None,
            name_locked: false,
            font_size: default_font_size,
            ephemeral: false,
            pty_title: None,
            outside_workspace_cached: false,
            outside_workspace_checked_at: None,
            outside_workspace_root: None,
            hidden: false,
            agent: None,
            activity: None,
            slots: HashMap::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// AppPane — dedicated app runtime (process or in-process builtin)
// ---------------------------------------------------------------------------

/// What a `PendingPaneClick` is aimed at: a raw screen position (the pixel
/// `plexi pane click <x> <y>` path), or a specific WASM UI-tree arena node id
/// (the node-addressed `plexi pane click <pane_id> --node <node_id>` sibling,
/// stint 0414). `node_id` matches `SemanticPaneNode.id` — the same id
/// `plexi pane state` reports for every node in the tree.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PaneClickTarget {
    Pos(egui::Pos2),
    Node(u32),
}

/// A pointer click injected by `AppRequest::ClickPane`/`ClickPaneNode`
/// (`plexi pane click`, or `HostHarness::inject_click`/`inject_node_click`),
/// queued on `PlexiApp` and consumed by the canvas render pass for the
/// target pane on the next frame it renders.
///
/// egui's own click detection (`Response::clicked()`) resolves
/// `viewport.interact_widgets` once, inside `Context::begin_pass`, strictly
/// before host/app code runs — so a click cannot be faked by mutating
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
}

pub enum AppRuntime {
    Builtin(Box<dyn App>),
    Python(Box<crate::host::wasm_python::LivePythonPane>),
    /// A sandboxed WASM component app driven by the synchronous effect loop.
    /// Neither a subprocess nor a native in-process builtin — it runs inside a
    /// per-pane `wasmtime::Store`.
    Wasm(Box<crate::host::wasm_pane::LiveWasmPane>),
}

impl AppRuntime {
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &AppRenderContext<'_>,
        pending_click: Option<PendingPaneClick>,
    ) {
        match self {
            AppRuntime::Builtin(app) => app.ui(ui, ctx),
            AppRuntime::Python(app) => app.ui(ui, ctx.colors, pending_click),
            AppRuntime::Wasm(app) => app.ui(ui, ctx.colors, pending_click),
        }
    }

    pub fn handle_key(
        &mut self,
        input: &crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        match self {
            AppRuntime::Builtin(app) => app.handle_key(input),
            AppRuntime::Python(app) => app.handle_key(input),
            AppRuntime::Wasm(app) => app.handle_key(input),
        }
    }

    pub fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        match self {
            AppRuntime::Builtin(app) => app.take_pending_commands(),
            AppRuntime::Python(app) => app.take_pending_commands(),
            AppRuntime::Wasm(app) => app
                .take_host_effects()
                .into_iter()
                .map(|effect| AppCommand::WasmHostEffect { pane_id: 0, effect })
                .collect(),
        }
    }

    pub fn keyboard_capture(&self) -> bool {
        match self {
            AppRuntime::Builtin(app) => app.keyboard_capture(),
            AppRuntime::Python(_) => false,
            AppRuntime::Wasm(_) => false,
        }
    }

    pub fn wants_close(&self) -> bool {
        match self {
            AppRuntime::Builtin(app) => app.wants_close(),
            AppRuntime::Python(app) => app.wants_close(),
            AppRuntime::Wasm(app) => app.wants_close(),
        }
    }

    pub fn queue_outbound_event(&mut self, event: crate::app_protocol::PlexiEvent) {
        match self {
            AppRuntime::Builtin(app) => app.queue_outbound_event(event),
            AppRuntime::Python(app) => app.queue_outbound_event(event),
            AppRuntime::Wasm(app) => app.queue_outbound_event(event),
        }
    }

    /// Deliver the CLI/host semantic action contract to every app runtime.
    pub fn send_app_action(&mut self, action: String, args: Vec<String>) -> Result<(), String> {
        match self {
            AppRuntime::Wasm(app) => {
                if !args.is_empty() {
                    return Err("WASM UI actions do not accept positional arguments".to_string());
                }
                app.dispatch_ui_action(action)
            }
            AppRuntime::Builtin(app) => {
                app.queue_outbound_event(crate::app_protocol::PlexiEvent::Action { action, args });
                Ok(())
            }
            AppRuntime::Python(app) => {
                app.queue_outbound_event(crate::app_protocol::PlexiEvent::Action { action, args });
                Ok(())
            }
        }
    }

    pub(crate) fn tool_event_sender(
        &self,
        repaint: egui::Context,
    ) -> Option<crate::plexi_ai::tool_dispatch::AppEventSender> {
        match self {
            AppRuntime::Python(app) => Some(
                crate::plexi_ai::tool_dispatch::AppEventSender::Python(app.tool_event_sender()),
            ),
            AppRuntime::Wasm(app) => Some(crate::plexi_ai::tool_dispatch::AppEventSender::Wasm(
                app.input_sender(repaint),
            )),
            AppRuntime::Builtin(_) => None,
        }
    }

    pub fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        match self {
            AppRuntime::Builtin(app) => app.sync_cwd(new_cwd),
            AppRuntime::Python(_) => {}
            AppRuntime::Wasm(_) => {}
        }
    }

    pub fn type_id(&self) -> &'static str {
        match self {
            AppRuntime::Builtin(app) => app.type_id(),
            AppRuntime::Python(_) => "python-wasm",
            AppRuntime::Wasm(_) => "wasm",
        }
    }

    pub fn serialize_state(&self) -> Option<serde_json::Value> {
        match self {
            AppRuntime::Builtin(app) => app.serialize_state(),
            AppRuntime::Python(_) => None,
            // WASM app state is persisted host-side via the file-backed
            // StateStore, not through workspace JSON.
            AppRuntime::Wasm(_) => None,
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            AppRuntime::Builtin(app) => app.display_name(),
            AppRuntime::Python(app) => app.display_name(),
            AppRuntime::Wasm(app) => app.display_name(),
        }
    }

    /// Seed text for the rename-pane overlay. See [`App::rename_seed`].
    pub fn rename_seed(&self) -> Option<String> {
        match self {
            AppRuntime::Builtin(app) => app.rename_seed(),
            AppRuntime::Python(_) => None,
            AppRuntime::Wasm(_) => None,
        }
    }

    /// Notify the app that its pane was renamed. See [`App::on_pane_renamed`].
    pub fn on_pane_renamed(&mut self, name: &str) {
        match self {
            AppRuntime::Builtin(app) => app.on_pane_renamed(name),
            AppRuntime::Python(_) => {}
            AppRuntime::Wasm(_) => {}
        }
    }

    /// Pump event I/O for a pane not in the active context.
    pub fn background_tick(&mut self) {
        match self {
            AppRuntime::Builtin(app) => app.background_tick(),
            AppRuntime::Python(_) => {}
            AppRuntime::Wasm(app) => app.background_tick(),
        }
    }

    /// Does this pane have pending background work that `background_tick`
    /// would make progress on?
    pub fn needs_background_tick(&self) -> bool {
        match self {
            AppRuntime::Builtin(app) => app.needs_background_tick(),
            AppRuntime::Python(_) => false,
            AppRuntime::Wasm(app) => app.needs_background_tick(),
        }
    }

    /// Current nav stack depth as reported by the app via `PushNav`/`PopNav`.
    /// Always 0 for builtin apps — they manage their own internal navigation.
    pub fn nav_stack_depth(&self) -> usize {
        match self {
            AppRuntime::Builtin(_) => 0,
            AppRuntime::Python(_) => 0,
            AppRuntime::Wasm(_) => 0,
        }
    }

    /// Title of the current top-of-stack view for pane chrome display.
    /// `None` when the stack is empty (root view — no back arrow shown).
    pub fn nav_top_title(&self) -> Option<&str> {
        match self {
            AppRuntime::Builtin(_) => None,
            AppRuntime::Python(_) => None,
            AppRuntime::Wasm(_) => None,
        }
    }

    /// The `view_id` the app should navigate back to (the entry below current
    /// top, or empty string for root). Used to populate `NavBack { view_id }`.
    pub fn nav_back_view_id(&self) -> String {
        match self {
            AppRuntime::Builtin(_) => String::new(),
            AppRuntime::Python(_) => String::new(),
            AppRuntime::Wasm(_) => String::new(),
        }
    }

    pub(crate) fn set_pending_notification_count(&mut self, count: usize) {
        let _ = count;
    }

    /// Serialize the last-rendered frame (Vec<RenderCommand>) as a JSON array.
    /// Returns `None` for builtin apps (no accessible frame).
    pub(crate) fn frame_json(&self) -> Option<serde_json::Value> {
        match self {
            AppRuntime::Builtin(_) => None,
            AppRuntime::Python(_) => None,
            AppRuntime::Wasm(_) => None,
        }
    }

    pub(crate) fn runtime_kind(&self) -> &'static str {
        match self {
            AppRuntime::Builtin(_) => "builtin",
            AppRuntime::Python(_) => "python-wasm",
            AppRuntime::Wasm(_) => "wasm",
        }
    }
}

#[allow(dead_code)]
pub struct AppPane {
    pub id: PaneId,
    pub runtime: AppRuntime,
    pub workspace_root: PathBuf,
    pub permissions: AppPermissions,
    pub manifest_id: String,
    pub name: String,
    /// Pane group this app joined at spawn (for PathChanged routing).
    pub pane_group: Option<String>,
    /// The terminal pane this app was spawned alongside. CdRequest routes here
    /// directly — no tile-tree walk needed.
    pub linked_pane_id: Option<PaneId>,
    /// Pane hidden by an overlay app. Closing the app restores this pane instead
    /// of deleting the tile.
    pub overlay_replaced: Option<Box<Pane>>,
    /// When true, the pane is visually deprioritized (outline dot, dimmed tab title).
    pub hidden: bool,
    pub agent: Option<crate::app_protocol::PaneAgentState>,
    /// App-reported pip status (red/yellow/green). Takes priority over derived
    /// activity for the activity dot. `None` = fall back to agent()/host-observed.
    pub pip_status: Option<crate::app_protocol::PipStatus>,
    pub slots: HashMap<String, PathBuf>,
    /// Last semantics committed by the production render path for native apps.
    pub(crate) semantic_state: SemanticPaneState,
}

impl AppPane {
    pub(crate) fn semantic_state(&self) -> SemanticPaneState {
        match &self.runtime {
            AppRuntime::Builtin(_) => self.semantic_state.clone(),
            AppRuntime::Python(app) => app.semantic_state(),
            AppRuntime::Wasm(app) => app.semantic_state().clone(),
        }
    }
}

impl Drop for AppPane {
    fn drop(&mut self) {
        // Overlay apps reuse the replaced pane's id. Restoring the boxed pane
        // must preserve that pane's connector and subscription registrations.
        if self.overlay_replaced.is_some() {
            log::info!(
                "app pane: dropped overlay pane={} without clearing restored pane registrations",
                self.id
            );
            return;
        }
        crate::plexi_ai::tool_dispatch::unregister(self.id);
        let (subscriptions, deliveries) = crate::host::app_timeline::global()
            .lock()
            .unwrap()
            .clear_subscriber(
                crate::broker::ActorType::App,
                &crate::host::event_subscriptions::app_subscriber_id(self.id),
            );
        log::info!(
            "app pane: dropped pane={} and cleared connector tools, {} event subscription(s), {} queued delivery(ies)",
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
