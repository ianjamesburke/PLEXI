use egui::RichText;

use crate::app::PlexiApp;
use crate::ui::{
    hints::{HintBar, HintGroup},
    labels::description_label,
    list::{ListRow, ListRowPips},
    overlay::ModalShell,
    style,
    text_field::TextField,
};

enum PaletteEntry {
    Command {
        command: PaletteCommand,
        name: &'static str,
        description: &'static str,
        search_text: &'static str,
    },
    Context {
        target: PaletteFocusTarget,
        name: String,
        workspace_name: String,
        metadata_chips: Vec<&'static str>,
        pane_pips: Option<ListRowPips>,
        search_text: String,
    },
    /// A context persists without a live window. It remains discoverable, but
    /// cannot be restored until something recreates a window for it.
    UnavailableContext { name: String, search_text: String },
    App {
        id: String,
        name: String,
        description: String,
        running_in_background: bool,
        is_workspace_local: bool,
        search_text: String,
    },
    /// Host-native builtin app (no registry manifest), launched by id.
    Builtin {
        id: &'static str,
        name: &'static str,
        description: &'static str,
        search_text: String,
    },
    Note {
        path: std::path::PathBuf,
        title: String,
        preview: String,
        search_text: String,
    },
    /// A pane running an agent, anywhere in the host. Enter focuses it.
    Agent {
        context_id: u64,
        window_id: u64,
        pane_id: u64,
        /// Hook-reported agent name (`PaneAgentState::agent`).
        agent_name: String,
        /// Owning context, live state, and the active tool when reported.
        secondary: String,
        state: crate::protocol::AgentState,
        /// True when this pane is the focused pane of its own window — drives
        /// the state dot's focused/dim rendering, same as every other pip.
        focused: bool,
        search_text: String,
    },
    /// A user-authored command from `commands.toml` or a global script,
    /// executed via `plexi run <name>` in a new terminal pane.
    UserCommand {
        name: String,
        /// Secondary text: description, run snippet, or "global script".
        secondary: String,
        scope: crate::cli::UserCommandScope,
        search_text: String,
    },
}

impl PaletteEntry {
    /// Stable identity for a result while the palette is live.  Display order
    /// is deliberately not an identity: agents can update their state and
    /// panes can close while the user is deciding what to open.
    fn selection_key(&self) -> String {
        match self {
            PaletteEntry::Command { command, .. } => format!("command:{command:?}"),
            PaletteEntry::Context { target, .. } => format!(
                "context:{}:{}:{}",
                target.context_id,
                target.window_id,
                target.pane_id.unwrap_or_default()
            ),
            PaletteEntry::UnavailableContext { name, .. } => format!("unavailable:{name}"),
            PaletteEntry::Agent {
                window_id, pane_id, ..
            } => format!("agent:{window_id}:{pane_id}"),
            PaletteEntry::App { id, .. } => format!("app:{id}"),
            PaletteEntry::Builtin { id, .. } => format!("builtin:{id}"),
            PaletteEntry::Note { path, .. } => format!("note:{}", path.display()),
            PaletteEntry::UserCommand { name, scope, .. } => format!("run:{scope:?}:{name}"),
        }
    }

    fn group_rank(&self) -> usize {
        match self {
            PaletteEntry::Context { .. } | PaletteEntry::UnavailableContext { .. } => 0,
            PaletteEntry::Agent { .. } => 1,
            PaletteEntry::Note { .. } => 2,
            PaletteEntry::App { .. } | PaletteEntry::Builtin { .. } => 3,
            PaletteEntry::Command { .. } => 4,
            PaletteEntry::UserCommand { .. } => 5,
        }
    }

    /// Tie-break within an equal (query, group) rank: an agent waiting on a
    /// human surfaces above an equally-scoring one that is not. This is the
    /// only state-aware ordering in the palette.
    fn state_rank(&self) -> usize {
        match self {
            PaletteEntry::Agent { state, .. } if *state == crate::protocol::AgentState::Blocked => {
                0
            }
            _ => 1,
        }
    }

    fn query_rank(&self, query: &str) -> usize {
        if query.is_empty() {
            return self.group_rank();
        }
        let search_text = match self {
            PaletteEntry::Command { search_text, .. } => *search_text,
            PaletteEntry::Context { search_text, .. }
            | PaletteEntry::UnavailableContext { search_text, .. }
            | PaletteEntry::Agent { search_text, .. }
            | PaletteEntry::App { search_text, .. }
            | PaletteEntry::Builtin { search_text, .. }
            | PaletteEntry::Note { search_text, .. }
            | PaletteEntry::UserCommand { search_text, .. } => search_text.as_str(),
        };
        if search_text.starts_with(query) {
            return 0;
        }
        if search_text
            .split_whitespace()
            .any(|part| part.starts_with(query))
        {
            return 1;
        }
        search_text
            .find(query)
            .map(|idx| idx + 2)
            .unwrap_or(usize::MAX)
    }

    fn matches_query(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        match self {
            PaletteEntry::Command { search_text, .. } => search_text.contains(query),
            PaletteEntry::Context { search_text, .. }
            | PaletteEntry::UnavailableContext { search_text, .. }
            | PaletteEntry::Agent { search_text, .. }
            | PaletteEntry::App { search_text, .. }
            | PaletteEntry::Builtin { search_text, .. }
            | PaletteEntry::Note { search_text, .. }
            | PaletteEntry::UserCommand { search_text, .. } => search_text.contains(query),
        }
    }
}

/// Stable navigation identity carried from collection through execution. Window
/// ordering is display policy, never an address: a row must survive a sort or
/// another window closing while the palette is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PaletteFocusTarget {
    context_id: u64,
    window_id: u64,
    pane_id: Option<u64>,
}

/// Palette commands either dispatch the canonical host key action or perform
/// the palette-only workspace-scoped config handoff.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PaletteCommand {
    Host(crate::host::keys::Action),
    OpenWorkspaceConfig,
}

struct PaletteCommandEntry {
    command: PaletteCommand,
    name: &'static str,
    description: &'static str,
    search_text: &'static str,
}

const PALETTE_COMMANDS: &[PaletteCommandEntry] = &[
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::SplitRight),
        name: "Split right",
        description: "Open a new terminal beside the focused pane",
        search_text: "split right new terminal shell console pane vsplit vertical",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::SplitDown),
        name: "Split down",
        description: "Open a new terminal below the focused pane",
        search_text: "split down below new terminal shell console pane hsplit horizontal",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenConfig),
        name: "Open global config",
        description: "Edit the channel-wide Plexi config file",
        search_text: "open global config settings preferences config.toml configuration channel",
    },
    PaletteCommandEntry {
        command: PaletteCommand::OpenWorkspaceConfig,
        name: "Open workspace config",
        description: "Edit config for the active workspace scope",
        search_text: "open workspace config settings preferences config.toml configuration scoped project local",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::ReloadConfig),
        name: "Reload configuration",
        description: "Reload global and active-workspace configuration",
        search_text: "reload config configuration settings global workspace scoped refresh",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenNotesPicker),
        name: "Browse notes",
        description: "Open the notes picker for global and context notes",
        search_text: "browse notes note picker open find global context",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::ToggleNotificationModal),
        name: "Review notifications",
        description: "Open the pending notification review queue",
        search_text: "review notifications notification queue alerts inbox pending",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenQuickNote),
        name: "Quick Note",
        description: "Capture a note in the current context",
        search_text: "quick note capture inbox memo",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenScratchpad),
        name: "Scratch Pad",
        description: "Open a fresh scratch note editor",
        search_text: "scratch pad scratchpad note editor inbox memo",
    },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::NewContext), name: "New context", description: "Create a new context", search_text: "new create open context workspace" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::RenameContext), name: "Rename context", description: "Rename the selected context", search_text: "rename context workspace" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::CloseContext), name: "Close context", description: "Close the selected context (confirmation required)", search_text: "close delete context workspace" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::ParkContext), name: "Park or unpark context", description: "Park or restore the selected context", search_text: "park unpark restore context workspace" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::ContextZoomOut), name: "Go to parent context", description: "Return to the parent context", search_text: "parent context zoom out back" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::NewTab), name: "New tab", description: "Create a new tab in the selected context", search_text: "new tab pane terminal" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::NewPageRight), name: "New pane", description: "Create a new pane beside the selected pane", search_text: "new pane page window right" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::RenamePane), name: "Rename pane", description: "Rename the selected pane", search_text: "rename pane tab" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::ClosePane), name: "Close pane", description: "Close the selected pane", search_text: "close pane tab" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::HidePane), name: "Hide or reveal pane", description: "Hide or reveal the selected pane", search_text: "hide reveal show pane" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::ToggleZoom), name: "Zoom pane", description: "Zoom or restore the selected pane", search_text: "zoom restore pane fullscreen" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::NavBackApp), name: "Back", description: "Go back in pane navigation or focus history", search_text: "back previous history pane" },
];

fn searchable_text(parts: &[&str]) -> String {
    parts.join(" ").to_lowercase()
}

fn normalized_palette_query(query: &str) -> String {
    query.trim().to_lowercase()
}

fn shortcut_label((modifiers, key): (egui::Modifiers, egui::Key)) -> String {
    let mut parts = Vec::new();
    if modifiers.command {
        parts.push(if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" });
    }
    if modifiers.ctrl && !modifiers.command {
        parts.push(if cfg!(target_os = "macos") { "⌃" } else { "Ctrl" });
    }
    if modifiers.alt {
        parts.push(if cfg!(target_os = "macos") { "⌥" } else { "Alt" });
    }
    if modifiers.shift {
        parts.push(if cfg!(target_os = "macos") { "⇧" } else { "Shift" });
    }
    let separator = if cfg!(target_os = "macos") { "" } else { "+" };
    let prefix = parts.join(separator);
    let key_separator = (!prefix.is_empty() && !cfg!(target_os = "macos")).then_some("+");
    format!("{prefix}{}{}", key_separator.unwrap_or(""), key.name())
}

fn inactive_pane_matches_query(pane_name: &str, query: &str) -> bool {
    searchable_text(&[pane_name]).contains(query)
}

/// POSIX single-quote a shell argument: wrap in `'…'` and escape embedded
/// single quotes as `'\''`. Command names are usually bare identifiers, but
/// global script filenames may contain spaces.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn sort_palette_entries(entries: &mut [PaletteEntry], query: &str) {
    entries.sort_by_key(|entry| {
        (
            entry.group_rank(),
            entry.query_rank(query),
            entry.state_rank(),
        )
    });
}

/// Preserve the user's target across a live rebuild.  If that target was
/// removed, deliberately leave no selection; clamping its former index would
/// make Enter operate on a different row without the user's input.
fn reconcile_palette_selection(
    entries: &[PaletteEntry],
    selected: &mut usize,
    selected_key: &mut Option<String>,
) {
    match selected_key {
        Some(key) => {
            if let Some(index) = entries.iter().position(|entry| entry.selection_key() == *key) {
                *selected = index;
            } else {
                // An empty key means a live result was removed. Keep that
                // explicit no-selection state until the user navigates or
                // changes the query, rather than selecting a replacement.
                *selected_key = Some(String::new());
            }
        }
        None if !entries.is_empty() => {
            *selected = (*selected).min(entries.len() - 1);
            *selected_key = Some(entries[*selected].selection_key());
        }
        None => {}
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PaletteAction {
    RunCommand(PaletteCommand),
    Focus(PaletteFocusTarget),
    LaunchApp(String),
    LaunchBuiltin(&'static str),
    OpenNote(std::path::PathBuf),
    RunUserCommand {
        name: String,
        scope: crate::cli::UserCommandScope,
    },
}

fn action_for_palette_entry(entry: &PaletteEntry) -> Option<PaletteAction> {
    match entry {
        PaletteEntry::Command { command, .. } => Some(PaletteAction::RunCommand(command.clone())),
        PaletteEntry::Context { target, .. } => Some(PaletteAction::Focus(*target)),
        PaletteEntry::UnavailableContext { name, .. } => {
            log::warn!("palette: parked context '{name}' has no live window to restore");
            None
        }
        PaletteEntry::Agent {
            context_id,
            window_id,
            pane_id,
            ..
        } => Some(PaletteAction::Focus(PaletteFocusTarget {
            context_id: *context_id,
            window_id: *window_id,
            pane_id: Some(*pane_id),
        })),
        PaletteEntry::App { id, .. } => Some(PaletteAction::LaunchApp(id.clone())),
        PaletteEntry::Builtin { id, .. } => Some(PaletteAction::LaunchBuiltin(id)),
        PaletteEntry::Note { path, .. } => Some(PaletteAction::OpenNote(path.clone())),
        PaletteEntry::UserCommand { name, scope, .. } => Some(PaletteAction::RunUserCommand {
            name: name.clone(),
            scope: *scope,
        }),
    }
}

#[cfg(test)]
fn palette_command_matches(query: &str) -> Vec<PaletteCommand> {
    PALETTE_COMMANDS
        .iter()
        .filter(|entry| query.is_empty() || entry.search_text.contains(query))
        .map(|entry| entry.command.clone())
        .collect()
}

struct BuiltinPaletteApp {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    gate: crate::release::ReleaseFeature,
}

/// Builtin host apps surfaced in the palette alongside registry apps.
const BUILTIN_APPS: &[BuiltinPaletteApp] = &[BuiltinPaletteApp {
    id: "assistant",
    name: "Assistant",
    description: "Host-native AI chat for this workspace",
    gate: crate::release::ReleaseFeature::Assistant,
}];

pub(crate) fn app_metadata_chips(
    running_in_background: bool,
    is_workspace_local: bool,
) -> &'static [&'static str] {
    match (running_in_background, is_workspace_local) {
        (false, false) => &["app"],
        (true, false) => &["app", "bg"],
        (false, true) => &["app", "ws"],
        (true, true) => &["app", "bg", "ws"],
    }
}

fn palette_pips_for_context(
    windows: &[crate::host::context::Window],
    context_active_window: &std::collections::HashMap<u64, u64>,
    ctx_id: u64,
    fallback_active_window_id: Option<u64>,
) -> Option<ListRowPips> {
    let mut ctx_windows: Vec<usize> = windows
        .iter()
        .enumerate()
        .filter(|(_, w)| w.context_id == ctx_id)
        .map(|(idx, _)| idx)
        .collect();
    ctx_windows.sort_by_key(|&idx| {
        let w = &windows[idx];
        (w.grid_y, w.grid_x)
    });

    let mut pane_ids = Vec::new();
    for &win_idx in &ctx_windows {
        let win = &windows[win_idx];
        if let Some(root) = win.tree.root() {
            pane_ids.extend(crate::spatial::tiling::collect_pane_ids_spatial(
                &win.tree.tiles,
                root,
            ));
        }
    }
    if pane_ids.is_empty() {
        return None;
    }

    let active_window_id = context_active_window
        .get(&ctx_id)
        .copied()
        .or_else(|| {
            fallback_active_window_id.filter(|win_id| {
                windows
                    .iter()
                    .any(|w| w.window_id == *win_id && w.context_id == ctx_id)
            })
        })
        .or_else(|| ctx_windows.first().map(|idx| windows[*idx].window_id));

    let focused_pane_id = active_window_id
        .and_then(|active_win_id| windows.iter().find(|w| w.window_id == active_win_id))
        .and_then(|win| {
            win.focused_pane
                .and_then(|tile_id| match win.tree.tiles.get(tile_id) {
                    Some(egui_tiles::Tile::Pane(pid)) => Some(*pid),
                    _ => None,
                })
        });
    let focused_idx = focused_pane_id.and_then(|pid| pane_ids.iter().position(|&p| p == pid));
    let hidden_indices = pane_ids
        .iter()
        .enumerate()
        .filter_map(|(idx, pane_id)| {
            windows
                .iter()
                .filter(|w| w.context_id == ctx_id)
                .find_map(|w| w.panes.get(pane_id))
                .is_some_and(|pane| pane.is_hidden())
                .then_some(idx)
        })
        .collect();

    let activities = pane_ids
        .iter()
        .map(|pane_id| {
            windows
                .iter()
                .filter(|w| w.context_id == ctx_id)
                .find_map(|w| w.panes.get(pane_id))
                .and_then(|pane| pane.effective_activity())
                .cloned()
        })
        .collect();

    Some(ListRowPips {
        count: pane_ids.len(),
        focused_idx,
        hidden_indices,
        activities,
    })
}

/// One palette row per pane — the unpacked view shown for the active context
/// (and for query-pierced matches in collapsed contexts). Spatial order across
/// the context's windows, matching the pip strip on the collapsed `ctx` row.
struct PaneRow {
    window_id: u64,
    pane_id: u64,
    name: String,
    chip: &'static str,
    pips: ListRowPips,
    recency_rank: usize,
    spatial_idx: usize,
}

fn pane_recency_rank(
    active_window_id: u64,
    active_focused_tile: Option<egui_tiles::TileId>,
    focus_history: &[(u64, egui_tiles::TileId)],
    window_id: u64,
    tile_id: egui_tiles::TileId,
) -> usize {
    if window_id == active_window_id && active_focused_tile == Some(tile_id) {
        return 0;
    }

    focus_history
        .iter()
        .rev()
        .position(|(history_window_id, history_tile_id)| {
            *history_window_id == window_id && *history_tile_id == tile_id
        })
        .map(|idx| idx + 1)
        .unwrap_or(usize::MAX)
}

fn palette_pane_rows_for_context(
    windows: &[crate::host::context::Window],
    context_active_window: &std::collections::HashMap<u64, u64>,
    context_names: &[(u64, String)],
    ctx_id: u64,
    host_active_window_id: u64,
    active_focused_tile: Option<egui_tiles::TileId>,
    focus_history: &[(u64, egui_tiles::TileId)],
) -> Vec<PaneRow> {
    let mut ctx_windows: Vec<usize> = windows
        .iter()
        .enumerate()
        .filter(|(_, w)| w.context_id == ctx_id)
        .map(|(idx, _)| idx)
        .collect();
    ctx_windows.sort_by_key(|&idx| {
        let w = &windows[idx];
        (w.grid_y, w.grid_x)
    });

    let context_focused_window_id = context_active_window
        .get(&ctx_id)
        .copied()
        .filter(|id| {
            windows
                .iter()
                .any(|w| w.window_id == *id && w.context_id == ctx_id)
        })
        .or_else(|| ctx_windows.first().map(|idx| windows[*idx].window_id));
    let focused_pane_id = context_focused_window_id
        .and_then(|active_win_id| windows.iter().find(|w| w.window_id == active_win_id))
        .and_then(|win| {
            win.focused_pane
                .and_then(|tile_id| match win.tree.tiles.get(tile_id) {
                    Some(egui_tiles::Tile::Pane(pid)) => Some(*pid),
                    _ => None,
                })
        });

    let mut rows = Vec::new();
    let mut spatial_idx = 0;
    for &win_idx in &ctx_windows {
        let win = &windows[win_idx];
        let Some(root) = win.tree.root() else {
            continue;
        };
        for pane_id in crate::spatial::tiling::collect_pane_ids_spatial(&win.tree.tiles, root) {
            let Some(pane) = win.panes.get(&pane_id) else {
                continue;
            };
            let Some(tile_id) = win.tree.tiles.find_pane(&pane_id) else {
                continue;
            };
            let (name, chip) = pane_row_identity(pane, context_names);
            rows.push(PaneRow {
                window_id: win.window_id,
                pane_id,
                name,
                chip,
                pips: ListRowPips {
                    count: 1,
                    focused_idx: (Some(pane_id) == focused_pane_id).then_some(0),
                    hidden_indices: if pane.is_hidden() {
                        vec![0]
                    } else {
                        Vec::new()
                    },
                    activities: vec![pane.effective_activity().cloned()],
                },
                recency_rank: pane_recency_rank(
                    host_active_window_id,
                    active_focused_tile,
                    focus_history,
                    win.window_id,
                    tile_id,
                ),
                spatial_idx,
            });
            spatial_idx += 1;
        }
    }
    rows.sort_by_key(|row| (row.recency_rank, row.spatial_idx));
    rows
}

/// One palette row per agent-bearing pane. Unlike the context rows, these are
/// sourced from every window of every context — the fleet is addressable from
/// wherever you happen to be standing.
struct AgentRow {
    context_id: u64,
    window_id: u64,
    pane_id: u64,
    /// One-based position among agent panes in the owning context. This keeps
    /// same-name squad members distinguishable without exposing tree ids.
    context_agent_index: usize,
    agent_name: String,
    pane_title: String,
    context_name: String,
    state: crate::protocol::AgentState,
    detail: Option<String>,
    focused: bool,
}

fn agent_state_label(state: &crate::protocol::AgentState) -> &'static str {
    match state {
        crate::protocol::AgentState::Working => "working",
        crate::protocol::AgentState::Blocked => "blocked",
        crate::protocol::AgentState::Idle => "idle",
    }
}

/// Secondary line for an agent row: owning context, live state, and the
/// hook-reported detail (the active tool) when there is one.
fn agent_row_secondary(
    context_name: &str,
    context_agent_index: usize,
    state: &crate::protocol::AgentState,
    detail: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if !context_name.is_empty() {
        parts.push(context_name.to_string());
    }
    parts.push(format!("agent {context_agent_index}"));
    parts.push(agent_state_label(state).to_string());
    if let Some(detail) = detail.filter(|d| !d.is_empty()) {
        parts.push(detail.to_string());
    }
    parts.join(" · ")
}

fn palette_agent_rows(
    windows: &[crate::host::context::Window],
    context_names: &[(u64, String)],
) -> Vec<AgentRow> {
    let mut rows = Vec::new();
    let mut context_agent_counts = std::collections::HashMap::<u64, usize>::new();
    for win in windows {
        let Some(root) = win.tree.root() else {
            continue;
        };
        let context_name = context_names
            .iter()
            .find(|(id, _)| *id == win.context_id)
            .map(|(_, name)| name.clone())
            .unwrap_or_default();
        let focused_pane_id =
            win.focused_pane
                .and_then(|tile_id| match win.tree.tiles.get(tile_id) {
                    Some(egui_tiles::Tile::Pane(pid)) => Some(*pid),
                    _ => None,
                });
        for pane_id in crate::spatial::tiling::collect_pane_ids_spatial(&win.tree.tiles, root) {
            let Some(pane) = win.panes.get(&pane_id) else {
                continue;
            };
            let Some(agent) = pane.agent() else {
                continue;
            };
            let context_agent_index = context_agent_counts.entry(win.context_id).or_default();
            *context_agent_index += 1;
            let (pane_title, _) = pane_row_identity(pane, context_names);
            // A hook that reports no agent name still gets a nameable row.
            let agent_name = if agent.agent.is_empty() {
                pane_title.clone()
            } else {
                agent.agent.clone()
            };
            rows.push(AgentRow {
                context_id: win.context_id,
                window_id: win.window_id,
                pane_id,
                context_agent_index: *context_agent_index,
                agent_name,
                pane_title,
                context_name: context_name.clone(),
                state: agent.state.clone(),
                detail: agent.detail.clone(),
                focused: Some(pane_id) == focused_pane_id,
            });
        }
    }
    rows
}

/// Display name + type chip for a palette pane row.
fn pane_row_identity(
    pane: &crate::host::pane::Pane,
    context_names: &[(u64, String)],
) -> (String, &'static str) {
    match pane {
        crate::host::pane::Pane::Terminal(t) => (
            t.name
                .clone()
                .or_else(|| t.pty_title.clone())
                .unwrap_or_else(|| "terminal".to_string()),
            "term",
        ),
        crate::host::pane::Pane::App(a) => {
            let chip = if a.manifest_id == "text-editor" {
                "text"
            } else {
                "app"
            };
            let name = if a.name.is_empty() {
                a.runtime.display_name()
            } else {
                a.name.clone()
            };
            (name, chip)
        }
        crate::host::pane::Pane::Portal(p) => (
            context_names
                .iter()
                .find(|(id, _)| *id == p.target_context_id)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "sub-context".to_string()),
            "ctx",
        ),
    }
}

impl PlexiApp {
    fn palette_shortcut(&self, command: &PaletteCommand) -> Option<String> {
        let binding = match command {
            PaletteCommand::Host(crate::host::keys::Action::OpenConfig) => self.key_bindings.open_config,
            PaletteCommand::Host(crate::host::keys::Action::ReloadConfig) => self.key_bindings.reload_config,
            PaletteCommand::Host(crate::host::keys::Action::OpenNotesPicker) => self.key_bindings.open_notes_picker,
            PaletteCommand::Host(crate::host::keys::Action::ToggleNotificationModal) => self.key_bindings.toggle_notification_modal,
            PaletteCommand::Host(crate::host::keys::Action::SplitRight) => self.key_bindings.split_right,
            PaletteCommand::Host(crate::host::keys::Action::SplitDown) => self.key_bindings.split_down,
            PaletteCommand::Host(crate::host::keys::Action::OpenQuickNote) => self.key_bindings.open_quick_note,
            PaletteCommand::Host(crate::host::keys::Action::OpenScratchpad) => self.key_bindings.open_scratchpad,
            PaletteCommand::OpenWorkspaceConfig => return None,
            PaletteCommand::Host(other) => {
                log::warn!("palette: no shortcut label for host binding {other:?}");
                return None;
            }
        };
        Some(shortcut_label(binding))
    }

    pub(crate) fn draw_command_palette(&mut self, ctx: &egui::Context) {
        let query = normalized_palette_query(&self.palette_query);
        let colors = self.colors;

        // ── Window entries (active context first, then by visit recency) ──
        // Two-tier sort: panes whose window belongs to the active context float
        // above panes in other contexts. Within each tier, recency wins.
        // Mirrors macOS Cmd+Tab — current app's windows first.
        let active_win_id = self.windows[self.active_window].window_id;
        let active_ctx_id = self.windows[self.active_window].context_id;
        let active_focused_tile = self.windows[self.active_window].focused_pane;

        let rank_of = |win_id: u64| -> (usize, usize) {
            let in_active_ctx = self
                .windows
                .iter()
                .find(|w| w.window_id == win_id)
                .map(|w| w.context_id == active_ctx_id)
                .unwrap_or(false);
            let tier = if in_active_ctx { 0 } else { 1 };
            let recency = if win_id == active_win_id {
                0
            } else {
                self.context_visit_history
                    .iter()
                    .position(|&id| id == win_id)
                    .map(|p| p + 1)
                    .unwrap_or(usize::MAX)
            };
            (tier, recency)
        };

        let mut entries: Vec<PaletteEntry> = Vec::new();

        let context_names: Vec<(u64, String)> = self
            .router
            .iter()
            .map(|c| (c.context_id, c.name.to_string()))
            .collect();

        // ── Agent panes (every window, every context) ───────────────────────
        // Collected fresh each frame, never snapshotted at open time: agent
        // state arrives over pane IPC (`set_agent_state`) while the palette is
        // up, and the rows must show it live.
        let agent_rows = palette_agent_rows(&self.windows, &context_names);
        if self.palette_agent_count_logged != Some(agent_rows.len()) {
            self.palette_agent_count_logged = Some(agent_rows.len());
            log::info!("palette: collected {} agent panes", agent_rows.len());
        }
        // An agent pane is represented once — by its agent row, which carries
        // strictly more (agent name, live state, active tool) than the plain
        // pane row would.
        let agent_pane_ids: std::collections::HashSet<u64> =
            agent_rows.iter().map(|row| row.pane_id).collect();

        let ctx_entries: Vec<PaletteEntry> = {
            // Context-scoped unpacking: the ACTIVE context expands to one row
            // per pane (each carrying a single status pip), while every OTHER
            // context collapses to one `ctx` row carrying its full pip strip.
            // A non-empty query pierces the collapse — matching pane names
            // surface from inactive contexts so the palette stays a global
            // jump tool.
            // Every context stays addressable here, including parked contexts
            // and metadata that outlived its last live window. A parked
            // context is always collapsed, even in the all-parked case where
            // it happens to be the active router context.
            struct CtxInfo {
                ctx_id: u64,
                name: String,
                parked: bool,
                resolved: Option<(usize, u64)>,
            }
            let mut contexts: Vec<CtxInfo> = Vec::new();
            for ctx_meta in self.router.iter() {
                let resolved =
                    self.context_active_window
                        .get(&ctx_meta.context_id)
                        .copied()
                        .and_then(|id| {
                            self.windows.iter().enumerate().find(|(_, w)| {
                                w.window_id == id && w.context_id == ctx_meta.context_id
                            })
                        })
                        .or_else(|| {
                            self.windows
                                .iter()
                                .enumerate()
                                .find(|(_, w)| w.context_id == ctx_meta.context_id)
                        });
                contexts.push(CtxInfo {
                    ctx_id: ctx_meta.context_id,
                    name: ctx_meta.name.to_string(),
                    parked: ctx_meta.parked,
                    resolved: resolved.map(|(win_idx, win)| (win_idx, win.window_id)),
                });
            }
            // rank_of already tiers active-context windows first, then recency.
            contexts.sort_by_key(|c| {
                c.resolved
                    .map(|(_, window_id)| rank_of(window_id))
                    .unwrap_or((2, usize::MAX))
            });

            let mut ctx_entries: Vec<PaletteEntry> = Vec::new();
            for c in &contexts {
                let Some((_win_idx, window_id)) = c.resolved else {
                    let search_text = searchable_text(&[c.name.as_str(), "parked unavailable"]);
                    if query.is_empty() || search_text.contains(&query) {
                        ctx_entries.push(PaletteEntry::UnavailableContext {
                            name: c.name.clone(),
                            search_text,
                        });
                    }
                    continue;
                };
                if c.ctx_id == active_ctx_id && !c.parked {
                    // Active context — unpack every pane, one pip each.
                    for row in palette_pane_rows_for_context(
                        &self.windows,
                        &self.context_active_window,
                        &context_names,
                        c.ctx_id,
                        active_win_id,
                        active_focused_tile,
                        &self.pane_focus_history,
                    ) {
                        if agent_pane_ids.contains(&row.pane_id) {
                            continue;
                        }
                        let search_text = searchable_text(&[row.name.as_str(), c.name.as_str()]);
                        if query.is_empty() || search_text.contains(&query) {
                            ctx_entries.push(PaletteEntry::Context {
                                target: PaletteFocusTarget {
                                    context_id: c.ctx_id,
                                    window_id: row.window_id,
                                    pane_id: Some(row.pane_id),
                                },
                                name: row.name,
                                workspace_name: c.name.clone(),
                                metadata_chips: vec![row.chip],
                                pane_pips: Some(row.pips),
                                search_text,
                            });
                        }
                    }
                } else {
                    // Inactive and parked contexts — one collapsed row, full
                    // pip strip. Search still pierces the collapse below.
                    let search_text = searchable_text(&[c.name.as_str()]);
                    if query.is_empty() || search_text.contains(&query) {
                        ctx_entries.push(PaletteEntry::Context {
                            target: PaletteFocusTarget {
                                context_id: c.ctx_id,
                                window_id,
                                pane_id: None,
                            },
                            name: c.name.clone(),
                            workspace_name: if c.parked {
                                "Parked".to_string()
                            } else {
                                String::new()
                            },
                            metadata_chips: if c.parked {
                                vec!["ctx", "parked"]
                            } else {
                                vec!["ctx"]
                            },
                            pane_pips: palette_pips_for_context(
                                &self.windows,
                                &self.context_active_window,
                                c.ctx_id,
                                Some(active_win_id),
                            ),
                            search_text,
                        });
                    }
                    if !query.is_empty() {
                        // Search pierces the collapse — match on pane name only,
                        // so typing a context name keeps the single ctx row.
                        for row in palette_pane_rows_for_context(
                            &self.windows,
                            &self.context_active_window,
                            &context_names,
                            c.ctx_id,
                            active_win_id,
                            active_focused_tile,
                            &self.pane_focus_history,
                        ) {
                            if agent_pane_ids.contains(&row.pane_id) {
                                continue;
                            }
                            // A context-name match keeps this context collapsed.
                            // Only the pane's own title pierces the collapse.
                            let search_text = searchable_text(&[row.name.as_str()]);
                            if inactive_pane_matches_query(&row.name, &query) {
                                ctx_entries.push(PaletteEntry::Context {
                                    target: PaletteFocusTarget {
                                        context_id: c.ctx_id,
                                        window_id: row.window_id,
                                        pane_id: Some(row.pane_id),
                                    },
                                    name: row.name,
                                    workspace_name: if c.parked {
                                        format!("Parked · {}", c.name)
                                    } else {
                                        c.name.clone()
                                    },
                                    metadata_chips: if c.parked {
                                        vec![row.chip, "parked"]
                                    } else {
                                        vec![row.chip]
                                    },
                                    pane_pips: Some(row.pips),
                                    search_text,
                                });
                            }
                        }
                    }
                }
            }

            ctx_entries
        };
        entries.extend(ctx_entries);

        // Agent rows match on agent name, pane title, and context name through
        // the same substring matcher every other result type uses.
        for row in agent_rows {
            let search_text = searchable_text(&[
                row.agent_name.as_str(),
                row.pane_title.as_str(),
                row.context_name.as_str(),
                agent_state_label(&row.state),
                row.detail.as_deref().unwrap_or_default(),
            ]);
            if !query.is_empty() && !search_text.contains(&query) {
                continue;
            }
            entries.push(PaletteEntry::Agent {
                context_id: row.context_id,
                window_id: row.window_id,
                pane_id: row.pane_id,
                secondary: agent_row_secondary(
                    &row.context_name,
                    row.context_agent_index,
                    &row.state,
                    row.detail.as_deref(),
                ),
                agent_name: row.agent_name,
                state: row.state,
                focused: row.focused,
                search_text,
            });
        }

        // ── Note entries ────────────────────────────────────────────────────
        for note in &self.palette_notes {
            let search_text = searchable_text(&[note.title.as_str(), note.search_text.as_str()]);
            let matches = query.is_empty() || search_text.contains(&query);
            if matches {
                entries.push(PaletteEntry::Note {
                    path: note.path.clone(),
                    title: note.title.clone(),
                    preview: note.preview.clone(),
                    search_text,
                });
            }
        }

        // ── Workspace-aware app entries ────────────────────────────────────
        // Use the workspace root cached at palette-open time (not re-resolved
        // per frame) to avoid filesystem traversal in the egui draw loop.
        //
        // The palette is a viewer surface for the focused window, not a
        // resource owner (stint 0724 Phase B) — it resolves its view by the
        // focused pane's own resolved workspace root via
        // `RegistryViews::view_for_root`, a synchronous per-root cache. No
        // staleness comparison is needed the way the old single shared
        // `AppRegistry` required: a root already seen is a cache hit, and a
        // never-seen root loads on this first access.
        let focused_workspace_root = self.palette_workspace_root.clone();
        let home = dirs::home_dir();
        let rescan_cwd = focused_workspace_root
            .as_deref()
            .or(home.as_deref())
            .unwrap_or(std::path::Path::new("/"));

        let app_entries: Vec<(String, String, String, bool, String)> = self
            .registries
            .view_for_root(rescan_cwd)
            .list()
            .into_iter()
            .filter_map(|app| {
                // Local apps are visible only when the focused pane is in their workspace.
                // Use the same explicit predicate here as in the badge below — no wildcard.
                let workspace_visible = match app.source {
                    crate::app::registry::RegistrySource::Global => true,
                    crate::app::registry::RegistrySource::LocalApp
                    | crate::app::registry::RegistrySource::LocalAgent => {
                        app.workspace_root.as_ref() == focused_workspace_root.as_ref()
                    }
                };
                if !workspace_visible {
                    return None;
                }
                let search_text = searchable_text(&[
                    app.manifest.name.as_str(),
                    app.manifest.id.as_str(),
                    app.manifest.description.as_str(),
                ]);
                if !query.is_empty() && !search_text.contains(&query) {
                    return None;
                }
                let is_local = matches!(
                    app.source,
                    crate::app::registry::RegistrySource::LocalApp
                        | crate::app::registry::RegistrySource::LocalAgent
                );
                Some((
                    app.manifest.id.clone(),
                    app.manifest.name.clone(),
                    app.manifest.description.clone(),
                    is_local,
                    search_text,
                ))
            })
            .collect();

        for app in BUILTIN_APPS {
            if !crate::release::feature_enabled(app.gate) {
                continue;
            }
            let search_text = searchable_text(&[app.name, app.id, app.description]);
            if query.is_empty() || search_text.contains(&query) {
                entries.push(PaletteEntry::Builtin {
                    id: app.id,
                    name: app.name,
                    description: app.description,
                    search_text,
                });
            }
        }

        for (id, name, description, is_workspace_local, search_text) in app_entries {
            let running_in_background = self.background_apps.contains_key(&id);
            entries.push(PaletteEntry::App {
                id,
                name,
                description,
                running_in_background,
                is_workspace_local,
                search_text,
            });
        }

        entries.extend(
            PALETTE_COMMANDS
                .iter()
                .map(|entry| PaletteEntry::Command {
                    command: entry.command.clone(),
                    name: entry.name,
                    description: entry.description,
                    search_text: entry.search_text,
                })
                .filter(|entry| entry.matches_query(&query)),
        );

        // ── User commands (commands.toml + global scripts) ──────────────────
        // Resolved at palette-open time into self.palette_commands; mirrors
        // `plexi run` precedence. Executed via `plexi run <name>` on select.
        for cmd in &self.palette_commands {
            let scope_word = match cmd.scope {
                crate::cli::UserCommandScope::Workspace => "workspace ws",
                crate::cli::UserCommandScope::Global => "global script",
            };
            let secondary = match (&cmd.description, &cmd.run) {
                (Some(desc), _) => desc.clone(),
                (None, Some(run)) => run.clone(),
                (None, None) => "global script".to_string(),
            };
            let search_text =
                searchable_text(&[cmd.name.as_str(), secondary.as_str(), "run", scope_word]);
            if query.is_empty() || search_text.contains(&query) {
                entries.push(PaletteEntry::UserCommand {
                    name: cmd.name.clone(),
                    secondary,
                    scope: cmd.scope,
                    search_text,
                });
            }
        }

        sort_palette_entries(&mut entries, &query);

        let total = entries.len();
        reconcile_palette_selection(
            &entries,
            &mut self.palette_selected,
            &mut self.palette_selected_key,
        );

        // ── Keyboard nav ───────────────────────────────────────────────────
        let mut action: Option<PaletteAction> = None;
        let prev_selected = self.palette_selected;

        // The toggle that opened the palette also dismisses it, so it is read
        // from the (config-overridable) binding table rather than hardcoded —
        // a user who rebinds it can close with the same key they opened with.
        let toggle = self.key_bindings.toggle_command_palette;

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.show_command_palette = false;
            }
            if input.consume_key(toggle.0, toggle.1) {
                self.show_command_palette = false;
            }
            if (input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                || input.consume_key(egui::Modifiers::COMMAND, egui::Key::J))
                && total > 0
                && self.palette_selected < total - 1
            {
                self.palette_selected += 1;
                self.palette_selected_key = Some(entries[self.palette_selected].selection_key());
            }
            if (input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                || input.consume_key(egui::Modifiers::COMMAND, egui::Key::K))
                && self.palette_selected > 0
            {
                self.palette_selected -= 1;
                self.palette_selected_key = Some(entries[self.palette_selected].selection_key());
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                action = self
                    .palette_selected_key
                    .as_ref()
                    .and_then(|key| entries.iter().find(|entry| entry.selection_key() == *key))
                    .and_then(action_for_palette_entry);
            }
        });

        if let Some(action) = action {
            self.run_palette_selection(action);
            return;
        }

        if !self.show_command_palette {
            return;
        }

        // ── Render ─────────────────────────────────────────────────────────
        // 0.8: the palette is a launcher, not a workspace — filling nearly
        // the whole screen height read as massive.
        let palette_max_list_h = ((ctx.content_rect().height() - 80.0 - 120.0) * 0.8).max(200.0);
        let modal_response = ModalShell::centered("command_palette")
            .width(style::MODAL_WIDTH_PALETTE)
            .escape(true)
            .show(ctx, &colors, |ui| {
                let te_id = egui::Id::new("palette_search");
                let te = ui
                    .scope_builder(
                        egui::UiBuilder::new().layer_id(egui::LayerId::new(
                            egui::Order::Tooltip,
                            te_id.with("paint_layer"),
                        )),
                        |ui| {
                            TextField::singleline(te_id, "Jump to context or launch app...")
                                .surface(crate::ui::focus::SurfaceKey::Overlay(
                                    crate::app::input_owner::OverlaySurface::Layer(
                                        crate::app::FocusKind::CommandPalette,
                                    ),
                                ))
                                .log_name("command_palette")
                                .show(ui, &mut self.palette_query, &colors)
                        },
                    )
                    .inner;
                if te.changed() {
                    self.palette_selected = 0;
                    self.palette_selected_key = None;
                }

                ui.add_space(style::SPACE_SM);

                if entries.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(ui.available_width());
                        description_label(ui, "No matching contexts or apps", &colors);
                    });
                    return;
                }

                let mut click_action: Option<PaletteAction> = None;
                let mut hover_select: Option<usize> = None;
                let mouse_moved = ctx.input(|i| i.pointer.delta().length_sq() > 0.5);
                let should_scroll = self.palette_selected != prev_selected;
                let mut shown_contexts_header = false;
                let mut shown_agents_header = false;
                let mut shown_apps_header = false;
                let mut shown_notes_header = false;
                let mut shown_commands_header = false;
                let mut shown_run_header = false;

                let scroll_reset = std::mem::take(&mut self.palette_scroll_reset);
                let mut scroll_area = egui::ScrollArea::vertical()
                    // animated(false): required by scroll_row_into_view — see src/ui/list.rs.
                    .animated(false)
                    .id_salt("palette_list")
                    .max_height(palette_max_list_h)
                    .min_scrolled_height(palette_max_list_h)
                    .auto_shrink([false, false]);
                if scroll_reset {
                    scroll_area = scroll_area.vertical_scroll_offset(0.0);
                }
                scroll_area.show(ui, |ui| {
                    // available_width — the scroll area reserves a
                    // scrollbar gutter; forcing the modal width would
                    // push rows back under the bar.
                    ui.set_width(ui.available_width());

                    for (i, entry) in entries.iter().enumerate() {
                        let is_selected = self.palette_selected_key.as_deref()
                            == Some(entries[i].selection_key().as_str());

                        match entry {
                            PaletteEntry::Command {
                                command,
                                name,
                                description,
                                ..
                            } => {
                                if !shown_commands_header {
                                    shown_commands_header = true;
                                    ui.add_space(style::SPACE_XS);
                                    ui.label(
                                        RichText::new("COMMANDS")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.add_space(style::SPACE_XS);
                                }
                                let description = match self.palette_shortcut(command) {
                                    Some(shortcut) => format!("{description} · {shortcut}"),
                                    None => (*description).to_string(),
                                };
                                let row_response = ListRow::new(name)
                                    .metadata_chips(&["cmd"])
                                    .secondary(&description)
                                    .selected(is_selected)
                                    .show(ui, &colors);
                                if is_selected {
                                    row_response.scroll_into_view(ui, should_scroll);
                                }
                                if row_response.row_clicked() {
                                    click_action = action_for_palette_entry(entry);
                                }
                                if row_response.row_hovered() {
                                    hover_select = Some(i);
                                }
                            }
                            PaletteEntry::Context {
                                name,
                                workspace_name,
                                metadata_chips,
                                pane_pips,
                                ..
                            } => {
                                if !shown_contexts_header {
                                    shown_contexts_header = true;
                                    ui.add_space(style::SPACE_XS);
                                    ui.label(
                                        RichText::new("CONTEXTS")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.add_space(style::SPACE_XS);
                                }
                                let mut row = ListRow::new(name.as_str())
                                    .metadata_chips(metadata_chips)
                                    .secondary(workspace_name.as_str())
                                    .selected(is_selected);
                                if let Some(pips) = pane_pips.clone() {
                                    row = row.pane_pips(pips);
                                }
                                let row_response = row.show(ui, &colors);
                                if is_selected {
                                    row_response.scroll_into_view(ui, should_scroll);
                                }

                                if row_response.row_clicked() {
                                    click_action = action_for_palette_entry(entry);
                                }
                                if row_response.row_hovered() {
                                    hover_select = Some(i);
                                }
                            }
                            PaletteEntry::UnavailableContext { name, .. } => {
                                if !shown_contexts_header {
                                    shown_contexts_header = true;
                                    ui.add_space(style::SPACE_XS);
                                    ui.label(
                                        RichText::new("CONTEXTS")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.add_space(style::SPACE_XS);
                                }
                                // This is deliberately not a click target: there
                                // is no window to focus, and switching the
                                // router alone would leave it out of sync with
                                // active_window. The row makes that state
                                // explicit instead of silently losing it.
                                ListRow::new(name.as_str())
                                    .metadata_chips(&["ctx", "parked", "unavailable"])
                                    .secondary("Parked · no live window to restore")
                                    .selected(is_selected)
                                    .show(ui, &colors);
                            }
                            PaletteEntry::Agent {
                                agent_name,
                                secondary,
                                state,
                                focused,
                                ..
                            } => {
                                if !shown_agents_header {
                                    shown_agents_header = true;
                                    ui.add_space(style::SPACE_XS);
                                    ui.label(
                                        RichText::new("AGENTS")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.add_space(style::SPACE_XS);
                                }
                                // The dot goes through the shared pip lane, so
                                // palette, sidebar, and pane chrome can never
                                // disagree about what a state looks like.
                                let row_response = ListRow::new(agent_name.as_str())
                                    .metadata_chips(&["agent"])
                                    .secondary(secondary.as_str())
                                    .pane_pips(ListRowPips {
                                        count: 1,
                                        focused_idx: focused.then_some(0),
                                        hidden_indices: Vec::new(),
                                        activities: vec![Some(state.clone())],
                                    })
                                    .selected(is_selected)
                                    .show(ui, &colors);
                                if is_selected {
                                    row_response.scroll_into_view(ui, should_scroll);
                                }
                                if row_response.row_clicked() {
                                    click_action = action_for_palette_entry(entry);
                                }
                                if row_response.row_hovered() {
                                    hover_select = Some(i);
                                }
                            }
                            PaletteEntry::App {
                                name,
                                description,
                                running_in_background,
                                is_workspace_local,
                                ..
                            } => {
                                if !shown_apps_header {
                                    shown_apps_header = true;
                                    ui.add_space(style::SPACE_XS);
                                    ui.label(
                                        RichText::new("APPS")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.add_space(style::SPACE_XS);
                                }

                                let row = ListRow::new(name.as_str())
                                    .metadata_chips(app_metadata_chips(
                                        *running_in_background,
                                        *is_workspace_local,
                                    ))
                                    .secondary(description)
                                    .selected(is_selected);

                                let row_response = row.show(ui, &colors);
                                if is_selected {
                                    row_response.scroll_into_view(ui, should_scroll);
                                }

                                if row_response.row_clicked() {
                                    click_action = action_for_palette_entry(entry);
                                }
                                if row_response.row_hovered() {
                                    hover_select = Some(i);
                                }
                            }
                            PaletteEntry::Builtin {
                                name,
                                description,
                                ..
                            } => {
                                if !shown_apps_header {
                                    shown_apps_header = true;
                                    ui.add_space(style::SPACE_XS);
                                    ui.label(
                                        RichText::new("APPS")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.add_space(style::SPACE_XS);
                                }

                                let row_response = ListRow::new(name)
                                    .metadata_chips(&["app", "host"])
                                    .secondary(description)
                                    .selected(is_selected)
                                    .show(ui, &colors);

                                if is_selected {
                                    row_response.scroll_into_view(ui, should_scroll);
                                }
                                if row_response.row_clicked() {
                                    click_action = action_for_palette_entry(entry);
                                }
                                if row_response.row_hovered() {
                                    hover_select = Some(i);
                                }
                            }
                            PaletteEntry::Note {
                                title,
                                preview,
                                ..
                            } => {
                                if !shown_notes_header {
                                    shown_notes_header = true;
                                    ui.add_space(style::SPACE_XS);
                                    ui.label(
                                        RichText::new("NOTES")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.add_space(style::SPACE_XS);
                                }
                                let row = ListRow::new(title.as_str())
                                    .metadata_chips(&["text"])
                                    .secondary(preview.as_str())
                                    .selected(is_selected);
                                let row_response = row.show(ui, &colors);
                                if is_selected {
                                    row_response.scroll_into_view(ui, should_scroll);
                                }
                                if row_response.row_clicked() {
                                    click_action = action_for_palette_entry(entry);
                                }
                                if row_response.row_hovered() {
                                    hover_select = Some(i);
                                }
                            }
                            PaletteEntry::UserCommand {
                                name,
                                secondary,
                                scope,
                                ..
                            } => {
                                if !shown_run_header {
                                    shown_run_header = true;
                                    ui.add_space(style::SPACE_XS);
                                    ui.label(
                                        RichText::new("RUN")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.add_space(style::SPACE_XS);
                                }
                                let chips: &[&str] = match scope {
                                    crate::cli::UserCommandScope::Workspace => &["run", "ws"],
                                    crate::cli::UserCommandScope::Global => &["run", "global"],
                                };
                                let row_response = ListRow::new(name.as_str())
                                    .metadata_chips(chips)
                                    .secondary(secondary.as_str())
                                    .selected(is_selected)
                                    .show(ui, &colors);
                                if is_selected {
                                    row_response.scroll_into_view(ui, should_scroll);
                                }
                                if row_response.row_clicked() {
                                    click_action = action_for_palette_entry(entry);
                                }
                                if row_response.row_hovered() {
                                    hover_select = Some(i);
                                }
                            }
                        }
                    }
                });

                ui.add_space(style::SPACE_SM);
                let hints = [
                    HintGroup::alternatives(&[&["\u{2318}", "j"], &["\u{2318}", "k"]], "navigate"),
                    HintGroup::new(&["\u{21b5}"], "open"),
                    HintGroup::new(&["esc"], "dismiss"),
                ];
                HintBar::new(&hints).show(ui, &colors);

                if let Some(i) = hover_select {
                    if mouse_moved {
                        self.palette_selected = i;
                        self.palette_selected_key = Some(entries[i].selection_key());
                    }
                }

                if let Some(act) = click_action {
                    self.run_palette_selection(act);
                }
            });

        if modal_response.dismissed {
            self.show_command_palette = false;
            self.palette_query.clear();
            self.palette_selected = 0;
            self.palette_selected_key = None;
        }
    }

    /// Execute a palette interaction after keyboard and mouse have resolved
    /// the same stable entry action. Host-wide action unification remains
    /// deliberately outside PAL-05.
    fn run_palette_selection(&mut self, action: PaletteAction) {
        self.show_command_palette = false;
        self.palette_query.clear();
        self.palette_selected_key = None;
        match action {
            PaletteAction::RunCommand(command) => self.run_palette_command(command),
            PaletteAction::RunUserCommand { name, scope } => self.run_user_command(&name, scope),
            PaletteAction::Focus(target) => self.focus_palette_target(target),
            PaletteAction::LaunchApp(id) => self.launch_app_by_id(&id),
            PaletteAction::LaunchBuiltin(id) => self.launch_builtin_by_id(id),
            PaletteAction::OpenNote(path) => {
                let path_str = path.display().to_string();
                if let Some(pane_id) = self.find_open_text_editor_pane_any_window(&path) {
                    log::info!("palette: note already open in pane {pane_id}, navigating");
                    self.pane_navigate(pane_id);
                } else {
                    log::info!("palette: opening note {:?} in new pane", path);
                    let _ = self.launch_app_by_id_with_layout("text-editor", None, &[path_str], None);
                }
            }
        }
    }

    /// Launch a host-native builtin palette entry by its id.
    pub(crate) fn launch_builtin_by_id(&mut self, id: &str) {
        log::info!("palette: launching builtin app '{id}'");
        match id {
            "assistant" => {
                if crate::release::feature_enabled(crate::release::ReleaseFeature::Assistant) {
                    self.open_assistant_pane();
                } else {
                    log::info!("assistant: palette launch blocked by stable release gate");
                }
            }
            other => log::warn!("palette: unknown builtin app id '{other}'"),
        }
    }

    /// Execute the subset of keyboard bindings intentionally surfaced by the
    /// palette. The descriptor is the host binding `Action`, so labels/search
    /// metadata cannot drift onto a private palette-only command enum.
    fn run_palette_command(&mut self, command: PaletteCommand) {
        match command {
            PaletteCommand::Host(action) => self.run_palette_host_binding(action),
            PaletteCommand::OpenWorkspaceConfig => {
                let root = self.router.active().root.clone();
                log::info!("palette: opening workspace config at {}", root.display());
                crate::config::open_workspace_config_file(&root);
            }
        }
    }

    fn run_palette_host_binding(&mut self, action: crate::host::keys::Action) {
        log::info!("palette: executing host binding {action:?}");
        match action {
            crate::host::keys::Action::SplitRight => {
                self.windows[self.active_window].clear_zoom();
                self.split_focused(false, None, false, false, None);
                self.mark_workspace_dirty();
            }
            crate::host::keys::Action::SplitDown => {
                self.windows[self.active_window].clear_zoom();
                self.split_focused(true, None, false, false, None);
                self.mark_workspace_dirty();
            }
            crate::host::keys::Action::OpenConfig => {
                crate::config::open_config_file();
            }
            crate::host::keys::Action::ReloadConfig => self.reload_config(),
            crate::host::keys::Action::OpenNotesPicker => self.open_notes_picker(),
            crate::host::keys::Action::ToggleNotificationModal => {
                self.show_notification_modal = true;
                if self.current_notify_id.is_none() {
                    self.current_notify_id = self.select_next_notification();
                }
            }
            crate::host::keys::Action::OpenQuickNote => {
                self.open_quick_note_modal();
            }
            crate::host::keys::Action::OpenScratchpad => {
                self.open_scratchpad();
            }
            crate::host::keys::Action::NewContext => {
                self.new_context();
                self.mark_workspace_dirty();
            }
            crate::host::keys::Action::RenameContext => self.open_context_rename(self.router.active_idx()),
            crate::host::keys::Action::ParkContext => self.toggle_park_active_context(),
            crate::host::keys::Action::ContextZoomOut => {
                self.zoom_out_of_context();
            }
            crate::host::keys::Action::NewTab => {
                self.new_tab(self.active_window, None, false, None);
                self.mark_workspace_dirty();
            }
            crate::host::keys::Action::NewPageRight => {
                if self.windows[self.active_window].panes.is_empty()
                    || self.windows[self.active_window].tree.root.is_none()
                {
                    self.reset_active_context();
                } else {
                    self.new_page_right();
                }
                self.mark_workspace_dirty();
            }
            crate::host::keys::Action::RenamePane => self.open_rename_for_focused(),
            crate::host::keys::Action::HidePane => self.toggle_palette_focused_pane_hidden(),
            crate::host::keys::Action::ToggleZoom => self.toggle_palette_zoom(),
            crate::host::keys::Action::NavBackApp => {
                if !self.try_nav_back_focused() {
                    self.step_focus_history_back();
                }
            }
            crate::host::keys::Action::ClosePane => self.close_palette_focused_pane(),
            crate::host::keys::Action::CloseContext => self.close_palette_active_context(),
            other => log::warn!("palette: unsupported host binding {other:?}"),
        }
    }

    fn toggle_palette_focused_pane_hidden(&mut self) {
        let win = &mut self.windows[self.active_window];
        if let Some(tile) = win.focused_pane {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = win.tree.tiles.get(tile) {
                if let Some(pane) = win.panes.get_mut(pane_id) {
                    let hidden = !pane.is_hidden();
                    pane.set_hidden(hidden);
                    log::info!("palette: pane_hide pane={pane_id} hidden={hidden}");
                    self.mark_workspace_dirty();
                }
            }
        }
    }

    fn toggle_palette_zoom(&mut self) {
        let (tile, portal_target) = {
            let win = &self.windows[self.active_window];
            let target = win
                .focused_pane
                .and_then(|tile_id| win.tree.tiles.get(tile_id))
                .and_then(|entry| match entry {
                    egui_tiles::Tile::Pane(pane_id) => Some(*pane_id),
                    _ => None,
                })
                .and_then(|pane_id| win.panes.get(&pane_id))
                .and_then(crate::host::pane::Pane::portal_target);
            (win.focused_pane, target)
        };
        if let Some(child_context_id) = portal_target {
            if let Some(context_idx) = self.router.position(|context| context.context_id == child_context_id) {
                let current_context_id = self.router.active().context_id;
                let current_window_id = self.windows[self.active_window].window_id;
                self.router.push_depth(current_context_id, current_window_id, tile);
                self.switch_workspace(context_idx);
            }
        } else if let Some(tile) = tile {
            let win = &mut self.windows[self.active_window];
            if win.zoomed_pane == Some(tile) {
                win.clear_zoom();
            } else {
                win.zoom_to(tile);
            }
            log::info!("palette: zoom toggled window={} tile={tile:?}", win.window_id);
        }
    }

    fn close_palette_focused_pane(&mut self) {
        if self.confirm_close() {
            self.pending_close = true;
        } else if !self.execute_close_pane() {
            self.mark_workspace_dirty();
        }
    }

    fn close_palette_active_context(&mut self) {
        let ctx_id = self.router.active().context_id;
        if self.router.len() <= 1 {
            log::info!("palette: refusing to close only context");
            return;
        }
        let state = self.build_context_close_state(ctx_id);
        if state.items.is_empty() || !self.config.confirm_context_close.unwrap_or(true) {
            self.delete_context(self.router.active_idx());
            self.mark_workspace_dirty();
        } else {
            log::info!("palette: close context={ctx_id} requires confirmation");
            self.pending_context_close = Some(state);
        }
    }

    /// Execute a user-authored command by opening a terminal pane that runs
    /// `plexi run <name>` in the focused workspace cwd. Routing through the CLI
    /// keeps secret injection, workspace scoping, and the security inventory
    /// true — the host never re-implements command execution.
    fn run_user_command(&mut self, name: &str, scope: crate::cli::UserCommandScope) {
        let cwd = self.palette_workspace_root.clone();
        log::info!("palette: running user command '{name}' scope={scope:?} cwd={cwd:?}");
        self.windows[self.active_window].clear_zoom();
        let initial_cmd = format!("plexi run {}", shell_single_quote(name));
        self.split_focused(false, Some(&initial_cmd), false, false, cwd);
        self.mark_workspace_dirty();
    }

    /// Resolve a stable palette target against current host state, then focus
    /// it. Collection never leaks a mutable window index into execution.
    /// Sanctioned cross-context focus path (also used by the `[launch] on_launch`
    /// resolver, #0336) — it switches the sidebar context via `switch_workspace`
    /// rather than mutating `active_window` mid-spawn.
    fn focus_palette_target(&mut self, target: PaletteFocusTarget) {
        let Some(ctx_idx) = self.windows.iter().position(|window| {
            window.window_id == target.window_id && window.context_id == target.context_id
        })
        else {
            log::warn!(
                "palette: target context={} window={} disappeared before focus",
                target.context_id,
                target.window_id
            );
            return;
        };
        log::info!(
            "palette: focus target context={} window={} pane={:?}",
            target.context_id,
            target.window_id,
            target.pane_id
        );
        if let Some(ctx_idx_sidebar) = self.router.position(|c| c.context_id == target.context_id) {
            if self.router.get(ctx_idx_sidebar).parked {
                // Parking keeps windows alive. Reuse the established unpark
                // transition before targeting this row's specific window/pane
                // so agent and ordinary-pane results restore consistently.
                self.unpark_context(ctx_idx_sidebar);
            } else if ctx_idx_sidebar != self.router.active_idx() {
                // switch_workspace → pick_active_context_from_workspace sets
                // active_window based on context_active_window. We override it
                // immediately below.
                self.switch_workspace(ctx_idx_sidebar);
            }
        }
        self.active_window = ctx_idx;
        self.windows[ctx_idx].zoomed_pane = None;
        let ctx_id = self.router.active().context_id;
        self.context_active_window.insert(ctx_id, target.window_id);
        self.record_context_visit(target.window_id);

        // Focus the specific pane if requested — find its TileId in the tree.
        if let Some(pid) = target.pane_id {
            let win = &mut self.windows[ctx_idx];
            if let Some(tile_id) = win.tree.tiles.iter().find_map(|(tid, tile)| {
                if matches!(tile, egui_tiles::Tile::Pane(p) if *p == pid) {
                    Some(*tid)
                } else {
                    None
                }
            }) {
                win.focused_pane = Some(tile_id);
            }
        }
    }

    /// Compatibility adapter for non-palette callers that still resolve a
    /// window by index. Palette rows must use `focus_palette_target` instead.
    pub(crate) fn jump_to_context(&mut self, window_index: usize, window_id: u64, pane_id: Option<u64>) {
        let Some(context_id) = self.windows.get(window_index).map(|window| window.context_id) else {
            log::warn!("palette: window index {window_index} disappeared before context focus");
            return;
        };
        self.focus_palette_target(PaletteFocusTarget {
            context_id,
            window_id,
            pane_id,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::pane::{Pane, PortalPane};
    use crate::ui::list::ListRowPips;

    #[test]
    fn app_metadata_chips_add_workspace_scope_without_secondary_text() {
        assert_eq!(app_metadata_chips(false, false), &["app"]);
        assert_eq!(app_metadata_chips(false, true), &["app", "ws"]);
        assert_eq!(app_metadata_chips(true, true), &["app", "bg", "ws"]);
    }

    #[test]
    fn palette_command_aliases_match_starter_synonyms() {
        assert_eq!(
            palette_command_matches("shell"),
            vec![
                PaletteCommand::Host(crate::host::keys::Action::SplitRight),
                PaletteCommand::Host(crate::host::keys::Action::SplitDown),
            ]
        );
        assert_eq!(
            palette_command_matches("console"),
            vec![
                PaletteCommand::Host(crate::host::keys::Action::SplitRight),
                PaletteCommand::Host(crate::host::keys::Action::SplitDown),
            ]
        );
        assert_eq!(
            palette_command_matches("hsplit"),
            vec![PaletteCommand::Host(crate::host::keys::Action::SplitDown)]
        );
        assert_eq!(
            palette_command_matches("config"),
            vec![
                PaletteCommand::Host(crate::host::keys::Action::OpenConfig),
                PaletteCommand::OpenWorkspaceConfig,
                PaletteCommand::Host(crate::host::keys::Action::ReloadConfig),
            ]
        );
        assert_eq!(
            palette_command_matches("notification"),
            vec![PaletteCommand::Host(crate::host::keys::Action::ToggleNotificationModal)]
        );
        assert_eq!(
            palette_command_matches("picker"),
            vec![PaletteCommand::Host(crate::host::keys::Action::OpenNotesPicker)]
        );
        assert_eq!(
            palette_command_matches("scratchpad"),
            vec![PaletteCommand::Host(crate::host::keys::Action::OpenScratchpad)]
        );
        assert_eq!(
            palette_command_matches("scratch pad"),
            vec![PaletteCommand::Host(crate::host::keys::Action::OpenScratchpad)]
        );
    }

    #[test]
    fn palette_searchable_text_is_cached_lowercase() {
        assert_eq!(
            searchable_text(&["Open Config", "Settings", "CONFIG.toml"]),
            "open config settings config.toml"
        );
        for entry in PALETTE_COMMANDS {
            assert_eq!(
                entry.search_text,
                entry.search_text.to_lowercase(),
                "palette command search text must be pre-lowercased"
            );
        }
    }

    #[test]
    fn palette_shortcut_label_uses_platform_command_name_and_configured_key() {
        let configured = (egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::F);
        let expected = if cfg!(target_os = "macos") {
            "⌘⇧F"
        } else {
            "Ctrl+Shift+F"
        };
        assert_eq!(shortcut_label(configured), expected);
    }

    #[test]
    fn empty_palette_puts_commands_after_apps() {
        let mut entries = vec![
            PaletteEntry::Command {
                command: PaletteCommand::Host(crate::host::keys::Action::OpenConfig),
                name: "Open config",
                description: "Edit config",
                search_text: "open config",
            },
            PaletteEntry::App {
                id: "balls".to_string(),
                name: "Balls".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "balls demo".to_string(),
            },
            PaletteEntry::Context {
                target: PaletteFocusTarget { context_id: 1, window_id: 1, pane_id: None },
                name: "Workspace".to_string(),
                workspace_name: String::new(),
                metadata_chips: vec!["ctx"],
                pane_pips: None,
                search_text: "workspace".to_string(),
            },
        ];

        sort_palette_entries(&mut entries, "");

        assert!(matches!(entries[0], PaletteEntry::Context { .. }));
        assert!(matches!(entries[1], PaletteEntry::App { .. }));
        assert!(matches!(entries[2], PaletteEntry::Command { .. }));
    }

    #[test]
    fn parked_and_windowless_context_entries_remain_searchable() {
        let parked = PaletteEntry::Context {
            target: PaletteFocusTarget { context_id: 7, window_id: 70, pane_id: None },
            name: "parked release work".to_string(),
            workspace_name: "Parked".to_string(),
            metadata_chips: vec!["ctx", "parked"],
            pane_pips: None,
            search_text: "parked release work".to_string(),
        };
        let unavailable = PaletteEntry::UnavailableContext {
            name: "parked without window".to_string(),
            search_text: "parked without window unavailable".to_string(),
        };

        assert!(parked.matches_query("release"));
        assert!(unavailable.matches_query("window"));
        assert_eq!(parked.group_rank(), 0);
        assert_eq!(unavailable.group_rank(), 0);
    }

    /// The render loop's `shown_contexts_header` flag prints CONTEXTS the
    /// first time it sees a `PaletteEntry::Context` row and never again;
    /// that's gated on group_rank() putting every Context row in one
    /// contiguous leading block on an empty query, and on there being no
    /// Context row at all to key off of when the group is empty.
    #[test]
    fn contexts_header_shows_once_for_a_leading_context_block_and_never_without_one() {
        let mut with_contexts = vec![
            PaletteEntry::App {
                id: "balls".to_string(),
                name: "Balls".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "balls demo".to_string(),
            },
            PaletteEntry::Context {
                target: PaletteFocusTarget { context_id: 1, window_id: 1, pane_id: None },
                name: "Workspace A".to_string(),
                workspace_name: String::new(),
                metadata_chips: vec!["ctx"],
                pane_pips: None,
                search_text: "workspace a".to_string(),
            },
            PaletteEntry::Context {
                target: PaletteFocusTarget { context_id: 2, window_id: 2, pane_id: None },
                name: "Workspace B".to_string(),
                workspace_name: String::new(),
                metadata_chips: vec!["ctx"],
                pane_pips: None,
                search_text: "workspace b".to_string(),
            },
        ];
        sort_palette_entries(&mut with_contexts, "");

        // Both Context rows land in one leading block, so the render loop's
        // shown_contexts_header flag flips true on the first and stays true
        // (never fires again) through the second.
        assert!(matches!(with_contexts[0], PaletteEntry::Context { .. }));
        assert!(matches!(with_contexts[1], PaletteEntry::Context { .. }));
        assert!(matches!(with_contexts[2], PaletteEntry::App { .. }));

        let without_contexts = vec![PaletteEntry::App {
            id: "balls".to_string(),
            name: "Balls".to_string(),
            description: "Demo".to_string(),
            running_in_background: false,
            is_workspace_local: false,
            search_text: "balls demo".to_string(),
        }];
        assert!(
            !without_contexts
                .iter()
                .any(|e| matches!(e, PaletteEntry::Context { .. })),
            "no Context row means shown_contexts_header never flips true, so CONTEXTS never renders"
        );
    }

    #[test]
    fn typed_palette_query_keeps_groups_contiguous_for_their_headers() {
        let mut entries = vec![
            PaletteEntry::App {
                id: "config-viewer".to_string(),
                name: "Config Viewer".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "demo config viewer".to_string(),
            },
            PaletteEntry::Command {
                command: PaletteCommand::Host(crate::host::keys::Action::OpenConfig),
                name: "Open config",
                description: "Edit config",
                search_text: "open config settings preferences",
            },
        ];

        sort_palette_entries(&mut entries, "open");

        assert!(matches!(entries[0], PaletteEntry::App { .. }));
        assert!(matches!(entries[1], PaletteEntry::Command { .. }));
    }

    #[test]
    fn empty_palette_puts_user_commands_below_host_commands() {
        let mut entries = vec![
            PaletteEntry::UserCommand {
                name: "build".to_string(),
                secondary: "cargo build".to_string(),
                scope: crate::cli::UserCommandScope::Workspace,
                search_text: "build cargo build run workspace ws".to_string(),
            },
            PaletteEntry::Command {
                command: PaletteCommand::Host(crate::host::keys::Action::OpenConfig),
                name: "Open config",
                description: "Edit config",
                search_text: "open config",
            },
            PaletteEntry::App {
                id: "balls".to_string(),
                name: "Balls".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "balls demo".to_string(),
            },
        ];

        sort_palette_entries(&mut entries, "");

        assert!(matches!(entries[0], PaletteEntry::App { .. }));
        assert!(matches!(entries[1], PaletteEntry::Command { .. }));
        assert!(matches!(entries[2], PaletteEntry::UserCommand { .. }));
    }

    #[test]
    fn typed_query_surfaces_user_command_by_name() {
        let mut entries = vec![
            PaletteEntry::App {
                // Only a substring match for "test" (inside "fastest").
                id: "fastest".to_string(),
                name: "Fastest".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "fastest demo loader".to_string(),
            },
            PaletteEntry::UserCommand {
                name: "test".to_string(),
                secondary: "cargo test".to_string(),
                scope: crate::cli::UserCommandScope::Workspace,
                search_text: "test cargo test run workspace ws".to_string(),
            },
        ];

        // Groups stay contiguous; relevance ranks within each group.
        sort_palette_entries(&mut entries, "test");
        assert!(matches!(entries[0], PaletteEntry::App { .. }));
        assert!(matches!(entries[1], PaletteEntry::UserCommand { .. }));
    }

    #[test]
    fn inactive_context_name_query_stays_collapsed_but_pane_name_pierces() {
        assert!(
            !inactive_pane_matches_query("build log", "squad-alpha"),
            "a context-name match must leave its inactive context collapsed"
        );
        assert!(inactive_pane_matches_query("build log", "build log"));
    }

    #[test]
    fn palette_query_trims_outer_whitespace() {
        assert_eq!(normalized_palette_query("  Blocked  "), "blocked");
    }

    #[test]
    fn agent_search_includes_state_and_active_tool() {
        use crate::protocol::AgentState;

        let search = searchable_text(&[
            "implementer",
            "build pane",
            "squad-alpha",
            agent_state_label(&AgentState::Blocked),
            "Bash",
        ]);
        assert!(search.contains("blocked"));
        assert!(search.contains("bash"));
    }

    #[test]
    fn palette_selection_survives_reorder_and_clears_on_removal() {
        let app = PaletteEntry::App {
            id: "alpha".to_string(),
            name: "Alpha".to_string(),
            description: String::new(),
            running_in_background: false,
            is_workspace_local: false,
            search_text: "alpha".to_string(),
        };
        let command = PaletteEntry::Command {
            command: PaletteCommand::Host(crate::host::keys::Action::OpenConfig),
            name: "Open config",
            description: "Edit config",
            search_text: "open config",
        };
        let mut selected = 1;
        let mut key = Some(command.selection_key());
        let reordered = vec![command, app];

        reconcile_palette_selection(&reordered, &mut selected, &mut key);
        assert_eq!(selected, 0);
        assert_eq!(key.as_deref(), Some("command:Host(OpenConfig)"));

        reconcile_palette_selection(&reordered[1..], &mut selected, &mut key);
        assert_eq!(key.as_deref(), Some(""));
        assert!(key
            .as_ref()
            .and_then(|key| reordered[1..]
                .iter()
                .find(|entry| entry.selection_key() == *key))
            .and_then(action_for_palette_entry)
            .is_none());
    }

    #[test]
    fn palette_mouse_and_enter_resolve_the_same_action() {
        let entry = PaletteEntry::Agent {
            context_id: 7,
            window_id: 7,
            pane_id: 42,
            agent_name: "reviewer".to_string(),
            secondary: String::new(),
            state: crate::protocol::AgentState::Blocked,
            focused: false,
            search_text: "reviewer blocked".to_string(),
        };
        let click = action_for_palette_entry(&entry);
        let enter = action_for_palette_entry(&entry);
        assert_eq!(
            click,
            Some(PaletteAction::Focus(PaletteFocusTarget {
                context_id: 7,
                window_id: 7,
                pane_id: Some(42),
            }))
        );
        assert_eq!(click, enter);
    }

    #[test]
    fn shell_single_quote_escapes_embedded_quotes() {
        assert_eq!(shell_single_quote("test"), "'test'");
        assert_eq!(shell_single_quote("my command"), "'my command'");
        assert_eq!(shell_single_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn palette_pips_include_hidden_and_focused_panes() {
        let win = test_window(1, 1, 0, 0, &[(10, false), (20, false), (30, true)], 1);
        let context_active_window = std::collections::HashMap::from([(1, 1)]);
        let windows = vec![win];
        let pips = palette_pips_for_context(&windows, &context_active_window, 1, None);

        assert_eq!(
            pips,
            Some(ListRowPips {
                count: 3,
                focused_idx: Some(1),
                hidden_indices: vec![2],
                activities: vec![None, None, None],
            })
        );
    }

    #[test]
    fn palette_pips_cover_every_window_in_the_context() {
        let first = test_window(1, 1, 0, 0, &[(10, false), (20, true), (30, false)], 0);
        let second = test_window(1, 2, 1, 0, &[(40, false), (50, true), (60, false)], 2);
        let context_active_window = std::collections::HashMap::from([(1, 2)]);
        let windows = vec![first, second];
        let pips = palette_pips_for_context(&windows, &context_active_window, 1, None);

        assert_eq!(
            pips,
            Some(ListRowPips {
                count: 6,
                focused_idx: Some(5),
                hidden_indices: vec![1, 4],
                activities: vec![None, None, None, None, None, None],
            })
        );
    }

    fn test_window(
        context_id: u64,
        window_id: u64,
        grid_x: u32,
        grid_y: u32,
        panes_spec: &[(u64, bool)],
        focused_idx: usize,
    ) -> crate::host::context::Window {
        let mut panes = std::collections::HashMap::new();
        let mut tiles = egui_tiles::Tiles::default();
        let mut tile_ids = Vec::new();
        for &(pane_id, hidden) in panes_spec {
            panes.insert(pane_id, test_pane(pane_id, hidden));
            tile_ids.push(tiles.insert_pane(pane_id));
        }
        let focused_pane = tile_ids.get(focused_idx).copied();
        let root = match tile_ids.as_slice() {
            [only] => *only,
            _ => tiles.insert_horizontal_tile(tile_ids),
        };
        let tree = egui_tiles::Tree::new("test", root, tiles);
        crate::host::context::Window {
            name: "test".to_string(),
            path: std::path::PathBuf::from("/tmp"),
            tree,
            panes,
            focused_pane,
            zoomed_pane: None,
            grid_x,
            grid_y,
            window_id,
            context_id,
        }
    }

    /// An app pane carrying hook-reported agent state.
    fn test_agent_pane(
        id: u64,
        pane_name: &str,
        agent: &str,
        state: crate::protocol::AgentState,
        detail: Option<&str>,
    ) -> Pane {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};

        Pane::App(Box::new(AppPane {
            pip_status: None,
            id,
            runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
                std::env::temp_dir(),
            ))),
            workspace_root: std::env::temp_dir(),
            permissions: AppPermissions::builtin(),
            manifest_id: "terminal".to_string(),
            name: pane_name.to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: Some(crate::protocol::PaneAgentState {
                pane_id: id,
                state,
                agent: agent.to_string(),
                detail: detail.map(str::to_string),
                session_id: None,
            }),
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        }))
    }

    /// A window built from explicit panes, so a test can mix agent-bearing and
    /// plain panes in one tree.
    fn window_of(
        context_id: u64,
        window_id: u64,
        panes_spec: Vec<(u64, Pane)>,
        focused_idx: usize,
    ) -> crate::host::context::Window {
        let mut panes = std::collections::HashMap::new();
        let mut tiles = egui_tiles::Tiles::default();
        let mut tile_ids = Vec::new();
        for (pane_id, pane) in panes_spec {
            panes.insert(pane_id, pane);
            tile_ids.push(tiles.insert_pane(pane_id));
        }
        let focused_pane = tile_ids.get(focused_idx).copied();
        let root = match tile_ids.as_slice() {
            [only] => *only,
            _ => tiles.insert_horizontal_tile(tile_ids),
        };
        crate::host::context::Window {
            name: "test".to_string(),
            path: std::path::PathBuf::from("/tmp"),
            tree: egui_tiles::Tree::new("test", root, tiles),
            panes,
            focused_pane,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id,
            context_id,
        }
    }

    #[test]
    fn agent_rows_collect_from_every_window_and_context() {
        use crate::protocol::AgentState;

        let squad = window_of(
            7,
            70,
            vec![
                (
                    10,
                    test_agent_pane(10, "claude", "impl-a", AgentState::Working, Some("Bash")),
                ),
                (
                    11,
                    test_agent_pane(11, "claude", "tester", AgentState::Blocked, None),
                ),
                (12, test_pane(12, false)),
            ],
            1,
        );
        let other_ctx = window_of(
            8,
            80,
            vec![(
                20,
                test_agent_pane(20, "codex", "reviewer", AgentState::Idle, None),
            )],
            0,
        );
        let names = vec![(7, "squad-alpha".to_string()), (8, "plexi".to_string())];

        let rows = palette_agent_rows(&[squad, other_ctx], &names);

        // The non-agent pane is skipped; both contexts contribute.
        assert_eq!(
            rows.iter().map(|r| r.pane_id).collect::<Vec<_>>(),
            vec![10, 11, 20]
        );
        assert_eq!(
            rows.iter()
                .map(|r| r.agent_name.as_str())
                .collect::<Vec<_>>(),
            vec!["impl-a", "tester", "reviewer"]
        );
        assert_eq!(
            rows.iter()
                .map(|r| r.context_name.as_str())
                .collect::<Vec<_>>(),
            vec!["squad-alpha", "squad-alpha", "plexi"]
        );
        // The stable window id must address the owning window, not the active one.
        assert_eq!(rows[2].window_id, 80);
        assert_eq!(
            rows.iter()
                .map(|r| r.context_agent_index)
                .collect::<Vec<_>>(),
            vec![1, 2, 1]
        );
        // Focus is per-window: pane 11 in the squad, pane 20 in the other context.
        assert_eq!(
            rows.iter().map(|r| r.focused).collect::<Vec<_>>(),
            vec![false, true, true]
        );
        assert_eq!(rows[0].detail.as_deref(), Some("Bash"));
    }

    #[test]
    fn squad_panes_sharing_a_title_and_agent_name_stay_distinguishable() {
        use crate::protocol::AgentState;

        // stint 0568 commonly launches the same agent command N times. Those
        // panes can share both title and reported agent name.
        let squad = window_of(
            7,
            70,
            vec![
                (
                    10,
                    test_agent_pane(10, "claude", "claude-code", AgentState::Working, None),
                ),
                (
                    11,
                    test_agent_pane(11, "claude", "claude-code", AgentState::Idle, None),
                ),
                (
                    12,
                    test_agent_pane(12, "claude", "claude-code", AgentState::Blocked, None),
                ),
            ],
            0,
        );
        let names = vec![(7, "squad-alpha".to_string())];
        let rows = palette_agent_rows(&[squad], &names);

        let search: Vec<String> = rows
            .iter()
            .map(|r| {
                searchable_text(&[
                    r.agent_name.as_str(),
                    r.pane_title.as_str(),
                    r.context_name.as_str(),
                ])
            })
            .collect();

        // Search still reaches the whole squad, while the visible one-based
        // positions distinguish otherwise identical rows.
        assert_eq!(
            search.iter().filter(|s| s.contains("claude")).count(),
            3,
            "the shared pane title must reach every squad member"
        );
        assert_eq!(
            rows.iter()
                .map(|r| r.context_agent_index)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        // The context name reaches the whole squad — jump by squad, then pick.
        assert_eq!(
            search.iter().filter(|s| s.contains("squad-alpha")).count(),
            3
        );
    }

    #[test]
    fn blocked_agent_outranks_equally_scoring_agent() {
        use crate::protocol::AgentState;

        let agent = |agent_name: &str, state: AgentState| PaletteEntry::Agent {
            context_id: 1,
            window_id: 1,
            pane_id: 10,
            agent_name: agent_name.to_string(),
            secondary: String::new(),
            state,
            focused: false,
            search_text: "claude squad-alpha".to_string(),
        };

        let mut entries = vec![
            agent("idle-one", AgentState::Idle),
            agent("working-one", AgentState::Working),
            agent("blocked-one", AgentState::Blocked),
        ];
        sort_palette_entries(&mut entries, "claude");

        match &entries[0] {
            PaletteEntry::Agent { agent_name, .. } => assert_eq!(agent_name, "blocked-one"),
            other => panic!("expected an agent row first, got {:?}", other.group_rank()),
        }
        // Non-blocked rows keep their relative order — the sort is stable and
        // state only breaks exact ties.
        match (&entries[1], &entries[2]) {
            (
                PaletteEntry::Agent {
                    agent_name: second, ..
                },
                PaletteEntry::Agent {
                    agent_name: third, ..
                },
            ) => {
                assert_eq!(second, "idle-one");
                assert_eq!(third, "working-one");
            }
            _ => panic!("expected agent rows"),
        }
    }

    #[test]
    fn agent_state_never_reorders_other_result_types() {
        use crate::protocol::AgentState;

        // A blocked agent must not jump ahead of a context row that scores the
        // same — state ordering is scoped to the agent group.
        let mut entries = vec![
            PaletteEntry::Context {
                target: PaletteFocusTarget { context_id: 1, window_id: 1, pane_id: None },
                name: "claude-ctx".to_string(),
                workspace_name: String::new(),
                metadata_chips: vec!["ctx"],
                pane_pips: None,
                search_text: "claude-ctx".to_string(),
            },
            PaletteEntry::Agent {
                context_id: 1,
                window_id: 1,
                pane_id: 10,
                agent_name: "blocked-one".to_string(),
                secondary: String::new(),
                state: AgentState::Blocked,
                focused: false,
                search_text: "claude-ctx".to_string(),
            },
        ];
        sort_palette_entries(&mut entries, "claude");

        assert!(matches!(entries[0], PaletteEntry::Context { .. }));
        assert!(matches!(entries[1], PaletteEntry::Agent { .. }));
    }

    #[test]
    fn agent_secondary_reads_context_state_and_active_tool() {
        use crate::protocol::AgentState;

        assert_eq!(
            agent_row_secondary("squad-alpha", 2, &AgentState::Working, Some("Bash")),
            "squad-alpha · agent 2 · working · Bash"
        );
        assert_eq!(
            agent_row_secondary("squad-alpha", 1, &AgentState::Blocked, None),
            "squad-alpha · agent 1 · blocked"
        );
        // An empty detail string is not a detail.
        assert_eq!(
            agent_row_secondary("plexi", 3, &AgentState::Idle, Some("")),
            "plexi · agent 3 · idle"
        );
        // A context with no resolvable name still yields a usable line.
        assert_eq!(
            agent_row_secondary("", 1, &AgentState::Idle, None),
            "agent 1 · idle"
        );
    }

    #[test]
    fn agent_row_falls_back_to_pane_title_when_the_hook_reports_no_name() {
        use crate::protocol::AgentState;

        let win = window_of(
            7,
            70,
            vec![(
                10,
                test_agent_pane(10, "claude", "", AgentState::Idle, None),
            )],
            0,
        );
        let rows = palette_agent_rows(&[win], &[(7, "squad-alpha".to_string())]);
        assert_eq!(rows[0].agent_name, "claude");
    }

    #[test]
    fn agent_jump_switches_window_context_and_focused_pane_together() {
        use crate::protocol::AgentState;
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let target = window_of(
            7,
            70,
            vec![(
                10,
                test_agent_pane(10, "claude", "claude-code", AgentState::Working, None),
            )],
            0,
        );
        h.app.windows.push(target);
        h.app.router.push(crate::host::context::Context {
            name: "squad-alpha".to_string().into(),
            root: std::env::temp_dir(),
            description: None,
            context_id: 7,
            parent_id: None,
            depth: 0,
            parked: true,
        });

        h.app.focus_palette_target(PaletteFocusTarget {
            context_id: 7,
            window_id: 70,
            pane_id: Some(10),
        });

        assert_eq!(h.app.active_window, 1);
        assert_eq!(h.app.windows[h.app.active_window].context_id, 7);
        assert_eq!(h.app.router.active().context_id, 7);
        let focused = h.app.windows[1]
            .focused_pane
            .and_then(|tile| h.app.windows[1].tree.tiles.get(tile));
        assert!(matches!(focused, Some(egui_tiles::Tile::Pane(10))));
        assert_eq!(h.app.context_active_window.get(&7), Some(&70));
        assert!(
            !h.app.router.get(1).parked,
            "agent selection restores parked work"
        );
    }

    #[test]
    fn palette_jump_unparks_and_focuses_a_parked_context_pane() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let target = test_window(7, 70, 1, 0, &[(10, false), (11, false)], 0);
        h.app.windows.push(target);
        h.app.router.push(crate::host::context::Context {
            name: "parked-alpha".to_string().into(),
            root: std::env::temp_dir(),
            description: None,
            context_id: 7,
            parent_id: None,
            depth: 0,
            parked: true,
        });

        h.app.focus_palette_target(PaletteFocusTarget {
            context_id: 7,
            window_id: 70,
            pane_id: Some(11),
        });

        assert!(
            !h.app.router.get(1).parked,
            "selection restores the context"
        );
        assert_eq!(h.app.router.active().context_id, 7);
        assert_eq!(h.app.active_window, 1);
        let focused = h.app.windows[1]
            .focused_pane
            .and_then(|tile| h.app.windows[1].tree.tiles.get(tile));
        assert!(matches!(focused, Some(egui_tiles::Tile::Pane(11))));
    }

    #[test]
    fn palette_jump_restores_the_last_parked_context() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let idx = h.app.router.active_idx();
        let window_id = h.app.windows[h.app.active_window].window_id;
        h.app.router.get_mut(idx).parked = true;

        let context_id = h.app.router.active().context_id;
        h.app.focus_palette_target(PaletteFocusTarget {
            context_id,
            window_id,
            pane_id: None,
        });

        assert!(!h.app.router.get(idx).parked);
        assert_eq!(h.app.router.active_idx(), idx);
        assert_eq!(h.app.active_window, 0);
    }

    #[test]
    fn palette_jump_restores_the_selected_context_when_all_are_parked() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let first_idx = h.app.router.active_idx();
        h.app.router.get_mut(first_idx).parked = true;
        h.app
            .windows
            .push(test_window(7, 70, 1, 0, &[(10, false)], 0));
        h.app.router.push(crate::host::context::Context {
            name: "second-parked".to_string().into(),
            root: std::env::temp_dir(),
            description: None,
            context_id: 7,
            parent_id: None,
            depth: 0,
            parked: true,
        });

        h.app.focus_palette_target(PaletteFocusTarget {
            context_id: 7,
            window_id: 70,
            pane_id: Some(10),
        });

        assert!(h.app.router.get(0).parked, "other parked work stays parked");
        assert!(!h.app.router.get(1).parked, "selected work is restored");
        assert_eq!(h.app.router.active().context_id, 7);
        assert_eq!(h.app.active_window, 1);
    }

    #[test]
    fn stale_agent_jump_leaves_focus_unchanged() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let active_window = h.app.active_window;
        let active_context = h.app.router.active().context_id;

        h.app.focus_palette_target(PaletteFocusTarget {
            context_id: 999,
            window_id: 999,
            pane_id: Some(123),
        });

        assert_eq!(h.app.active_window, active_window);
        assert_eq!(h.app.router.active().context_id, active_context);
    }

    fn test_pane(id: u64, hidden: bool) -> Pane {
        Pane::Portal(Box::new(PortalPane {
            pane_id: id,
            target_context_id: id + 1000,
            context_state: None,
            hidden,
        }))
    }

    #[test]
    fn pane_rows_unpack_each_pane_with_a_single_pip() {
        let win = test_window(1, 1, 0, 0, &[(10, false), (20, false), (30, true)], 1);
        let context_active_window = std::collections::HashMap::from([(1, 1)]);
        let windows = vec![win];
        let names = vec![(1010, "child-a".to_string())];

        let rows = palette_pane_rows_for_context(
            &windows,
            &context_active_window,
            &names,
            1,
            999,
            None,
            &[],
        );

        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert_eq!(row.pips.count, 1);
            assert_eq!(row.chip, "ctx"); // test panes are portals
        }
        // Portal target 1010 resolves to its context name; unknown targets fall back.
        assert_eq!(rows[0].name, "child-a");
        assert_eq!(rows[1].name, "sub-context");
        // Focused pane (idx 1) carries the focus pip; hidden pane (idx 2) the hidden pip.
        assert_eq!(rows[0].pips.focused_idx, None);
        assert_eq!(rows[1].pips.focused_idx, Some(0));
        assert!(rows[0].pips.hidden_indices.is_empty());
        assert_eq!(rows[2].pips.hidden_indices, vec![0]);
    }

    #[test]
    fn pane_rows_cover_every_window_in_the_context() {
        // Regression: panes must unpack from ALL windows of a context, not
        // just the first one.
        let first = test_window(1, 1, 0, 0, &[(10, false)], 0);
        let second = test_window(1, 2, 1, 0, &[(40, false), (50, false)], 1);
        let context_active_window = std::collections::HashMap::from([(1, 2)]);
        let windows = vec![first, second];

        let rows =
            palette_pane_rows_for_context(&windows, &context_active_window, &[], 1, 999, None, &[]);

        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter().map(|r| r.window_id).collect::<Vec<_>>(),
            vec![1, 2, 2]
        );
        assert_eq!(
            rows.iter().map(|r| r.pane_id).collect::<Vec<_>>(),
            vec![10, 40, 50]
        );
        // Focus follows the context's active window (window 2, pane 50).
        assert_eq!(rows[2].pips.focused_idx, Some(0));
        assert_eq!(rows[0].pips.focused_idx, None);
    }

    #[test]
    fn pane_rows_sort_current_focus_then_reverse_history_before_spatial_fallback() {
        let win = test_window(1, 1, 0, 0, &[(10, false), (20, false), (30, false)], 0);
        let current_tile = win.focused_pane.expect("focused tile");
        let older_tile = win.tree.tiles.find_pane(&20).expect("older tile");
        let newer_tile = win.tree.tiles.find_pane(&30).expect("newer tile");
        let context_active_window = std::collections::HashMap::from([(1, 1)]);
        let windows = vec![win];

        let rows = palette_pane_rows_for_context(
            &windows,
            &context_active_window,
            &[],
            1,
            1,
            Some(current_tile),
            &[(1, older_tile), (1, newer_tile)],
        );

        assert_eq!(
            rows.iter().map(|r| r.pane_id).collect::<Vec<_>>(),
            vec![10, 30, 20]
        );
    }

    #[test]
    fn pane_row_identity_chips_apps_and_text_editors() {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};

        let make_app = |manifest_id: &str, name: &str| {
            Pane::App(Box::new(AppPane {
                pip_status: None,
                id: 99,
                runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
                    std::env::temp_dir(),
                ))),
                workspace_root: std::env::temp_dir(),
                permissions: AppPermissions::builtin(),
                manifest_id: manifest_id.to_string(),
                name: name.to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            }))
        };

        let (name, chip) = pane_row_identity(&make_app("notes", "My Notes"), &[]);
        assert_eq!((name.as_str(), chip), ("My Notes", "app"));

        let (name, chip) = pane_row_identity(&make_app("text-editor", "todo.md"), &[]);
        assert_eq!((name.as_str(), chip), ("todo.md", "text"));

        // Empty pane name falls back to the runtime display name.
        let (name, _) = pane_row_identity(&make_app("notes", ""), &[]);
        assert!(!name.is_empty());

        // User-renamed app pane uses the assigned name in palette search.
        let (name, chip) = pane_row_identity(&make_app("assistant", "chad"), &[]);
        assert_eq!((name.as_str(), chip), ("chad", "app"));
        let search = searchable_text(&[name.as_str(), "work"]);
        assert!(
            search.contains("chad"),
            "renamed pane title must appear in palette search_text; got: {search:?}"
        );
    }
}
