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
        ctx_idx: usize,
        context_id: u64,
        name: String,
        workspace_name: String,
        metadata_chip: &'static str,
        pane_pips: Option<ListRowPips>,
        /// If set, focus this specific pane after navigating to the window.
        pane_id: Option<u64>,
        search_text: String,
    },
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
    fn group_rank(&self) -> usize {
        match self {
            PaletteEntry::Context { .. } => 0,
            PaletteEntry::Note { .. } => 1,
            PaletteEntry::App { .. } | PaletteEntry::Builtin { .. } => 2,
            PaletteEntry::Command { .. } => 3,
            PaletteEntry::UserCommand { .. } => 4,
        }
    }

    fn query_rank(&self, query: &str) -> usize {
        if query.is_empty() {
            return self.group_rank();
        }
        let search_text = match self {
            PaletteEntry::Command { search_text, .. } => *search_text,
            PaletteEntry::Context { search_text, .. }
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
            | PaletteEntry::App { search_text, .. }
            | PaletteEntry::Builtin { search_text, .. }
            | PaletteEntry::Note { search_text, .. }
            | PaletteEntry::UserCommand { search_text, .. } => search_text.contains(query),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaletteCommand {
    SplitRight,
    SplitDown,
    OpenConfig,
    OpenQuickNote,
    OpenScratchpad,
}

struct PaletteCommandEntry {
    command: PaletteCommand,
    name: &'static str,
    description: &'static str,
    search_text: &'static str,
}

const PALETTE_COMMANDS: &[PaletteCommandEntry] = &[
    PaletteCommandEntry {
        command: PaletteCommand::SplitRight,
        name: "Split right",
        description: "Open a new terminal beside the focused pane",
        search_text: "split right new terminal shell console pane vsplit vertical",
    },
    PaletteCommandEntry {
        command: PaletteCommand::SplitDown,
        name: "Split down",
        description: "Open a new terminal below the focused pane",
        search_text: "split down below new terminal shell console pane hsplit horizontal",
    },
    PaletteCommandEntry {
        command: PaletteCommand::OpenConfig,
        name: "Open config",
        description: "Edit the active Plexi config file",
        search_text: "open config settings preferences config.toml configuration",
    },
    PaletteCommandEntry {
        command: PaletteCommand::OpenQuickNote,
        name: "Quick Note",
        description: "Capture a note in the current context",
        search_text: "quick note capture inbox memo",
    },
    PaletteCommandEntry {
        command: PaletteCommand::OpenScratchpad,
        name: "Scratch Pad",
        description: "Open a fresh scratch note editor",
        search_text: "scratch pad scratchpad note editor inbox memo",
    },
];

fn searchable_text(parts: &[&str]) -> String {
    parts.join(" ").to_lowercase()
}

/// POSIX single-quote a shell argument: wrap in `'…'` and escape embedded
/// single quotes as `'\''`. Command names are usually bare identifiers, but
/// global script filenames may contain spaces.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn sort_palette_entries(entries: &mut [PaletteEntry], query: &str) {
    entries.sort_by_key(|entry| (entry.query_rank(query), entry.group_rank()));
}

#[cfg(test)]
fn palette_command_matches(query: &str) -> Vec<PaletteCommand> {
    PALETTE_COMMANDS
        .iter()
        .filter(|entry| query.is_empty() || entry.search_text.contains(query))
        .map(|entry| entry.command)
        .collect()
}

struct BuiltinPaletteApp {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    gate: crate::release::ReleaseFeature,
}

/// Builtin host apps surfaced in the palette alongside registry apps.
const BUILTIN_APPS: &[BuiltinPaletteApp] = &[
    BuiltinPaletteApp {
        id: "assistant",
        name: "Assistant",
        description: "Host-native AI chat for this workspace",
        gate: crate::release::ReleaseFeature::Assistant,
    },
];

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
    win_idx: usize,
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
                win_idx,
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
    pub(crate) fn draw_command_palette(&mut self, ctx: &egui::Context) {
        let query = self.palette_query.to_lowercase();
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

        let ctx_entries: Vec<PaletteEntry> = {
            // Context-scoped unpacking: the ACTIVE context expands to one row
            // per pane (each carrying a single status pip), while every OTHER
            // context collapses to one `ctx` row carrying its full pip strip.
            // A non-empty query pierces the collapse — matching pane names
            // surface from inactive contexts so the palette stays a global
            // jump tool.
            let context_names: Vec<(u64, String)> = self
                .router
                .iter()
                .map(|c| (c.context_id, c.name.clone()))
                .collect();

            // Non-parked contexts that own at least one window. The jump
            // target is context_active_window when it still belongs to the
            // context, else the context's first window.
            struct CtxInfo {
                ctx_id: u64,
                name: String,
                win_idx: usize,
                window_id: u64,
            }
            let mut contexts: Vec<CtxInfo> = Vec::new();
            for ctx_meta in self.router.iter() {
                if ctx_meta.parked {
                    continue;
                }
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
                let Some((win_idx, win)) = resolved else {
                    continue;
                };
                contexts.push(CtxInfo {
                    ctx_id: ctx_meta.context_id,
                    name: ctx_meta.name.clone(),
                    win_idx,
                    window_id: win.window_id,
                });
            }
            // rank_of already tiers active-context windows first, then recency.
            contexts.sort_by_key(|c| rank_of(c.window_id));

            let mut ctx_entries: Vec<PaletteEntry> = Vec::new();
            for c in &contexts {
                if c.ctx_id == active_ctx_id {
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
                        let search_text = searchable_text(&[row.name.as_str(), c.name.as_str()]);
                        if query.is_empty() || search_text.contains(&query) {
                            ctx_entries.push(PaletteEntry::Context {
                                ctx_idx: row.win_idx,
                                context_id: row.window_id,
                                name: row.name,
                                workspace_name: c.name.clone(),
                                metadata_chip: row.chip,
                                pane_pips: Some(row.pips),
                                pane_id: Some(row.pane_id),
                                search_text,
                            });
                        }
                    }
                } else {
                    // Inactive context — one collapsed row, full pip strip.
                    let search_text = searchable_text(&[c.name.as_str()]);
                    if query.is_empty() || search_text.contains(&query) {
                        ctx_entries.push(PaletteEntry::Context {
                            ctx_idx: c.win_idx,
                            context_id: c.window_id,
                            name: c.name.clone(),
                            workspace_name: String::new(),
                            metadata_chip: "ctx",
                            pane_pips: palette_pips_for_context(
                                &self.windows,
                                &self.context_active_window,
                                c.ctx_id,
                                Some(active_win_id),
                            ),
                            pane_id: None,
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
                            let search_text =
                                searchable_text(&[row.name.as_str(), c.name.as_str()]);
                            if search_text.contains(&query) {
                                ctx_entries.push(PaletteEntry::Context {
                                    ctx_idx: row.win_idx,
                                    context_id: row.window_id,
                                    name: row.name,
                                    workspace_name: c.name.clone(),
                                    metadata_chip: row.chip,
                                    pane_pips: Some(row.pips),
                                    pane_id: Some(row.pane_id),
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
        let focused_workspace_root = self.palette_workspace_root.as_ref();

        // If the cached workspace differs from what the registry was last loaded
        // for, rescan once now so local apps for this workspace appear.
        if focused_workspace_root != self.registry.loaded_workspace.as_ref() {
            let home = dirs::home_dir();
            let rescan_cwd = focused_workspace_root
                .map(|p| p.as_path())
                .or_else(|| home.as_deref())
                .unwrap_or(std::path::Path::new("/"));
            log::info!(
                "palette: registry workspace ({:?}) differs from palette workspace ({:?}), rescanning",
                self.registry.loaded_workspace,
                focused_workspace_root,
            );
            self.registry = crate::app::registry::AppRegistry::load(rescan_cwd);
        }

        let app_entries: Vec<(String, String, String, bool, String)> = self
            .registry
            .list()
            .into_iter()
            .filter_map(|app| {
                // Local apps are visible only when the focused pane is in their workspace.
                // Use the same explicit predicate here as in the badge below — no wildcard.
                let workspace_visible = match app.source {
                    crate::app::registry::RegistrySource::Global => true,
                    crate::app::registry::RegistrySource::LocalApp
                    | crate::app::registry::RegistrySource::LocalAgent => {
                        app.workspace_root.as_ref() == focused_workspace_root
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
                    command: entry.command,
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

        if self.palette_selected >= total && total > 0 {
            self.palette_selected = total - 1;
        }

        // ── Keyboard nav ───────────────────────────────────────────────────
        #[derive(Clone)]
        enum Action {
            RunCommand(PaletteCommand),
            JumpContext(usize, u64, Option<u64>),
            LaunchApp(String),
            LaunchBuiltin(&'static str),
            OpenNote(std::path::PathBuf),
            RunUserCommand {
                name: String,
                scope: crate::cli::UserCommandScope,
            },
        }
        let mut action: Option<Action> = None;
        let prev_selected = self.palette_selected;

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.show_command_palette = false;
            }
            if input.consume_key(egui::Modifiers::COMMAND, egui::Key::P) {
                self.show_command_palette = false;
            }
            if (input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                || input.consume_key(egui::Modifiers::COMMAND, egui::Key::J))
                && total > 0
                && self.palette_selected < total - 1
            {
                self.palette_selected += 1;
            }
            if (input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                || input.consume_key(egui::Modifiers::COMMAND, egui::Key::K))
                && self.palette_selected > 0
            {
                self.palette_selected -= 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                match entries.get(self.palette_selected) {
                    Some(PaletteEntry::Command { command, .. }) => {
                        action = Some(Action::RunCommand(*command));
                    }
                    Some(PaletteEntry::Context {
                        ctx_idx,
                        context_id,
                        pane_id,
                        ..
                    }) => {
                        action = Some(Action::JumpContext(*ctx_idx, *context_id, *pane_id));
                    }
                    Some(PaletteEntry::App { id, .. }) => {
                        action = Some(Action::LaunchApp(id.clone()));
                    }
                    Some(PaletteEntry::Builtin { id, .. }) => {
                        action = Some(Action::LaunchBuiltin(id));
                    }
                    Some(PaletteEntry::Note { path, .. }) => {
                        action = Some(Action::OpenNote(path.clone()));
                    }
                    Some(PaletteEntry::UserCommand { name, scope, .. }) => {
                        action = Some(Action::RunUserCommand {
                            name: name.clone(),
                            scope: *scope,
                        });
                    }
                    None => {}
                }
            }
        });

        match action {
            Some(Action::RunCommand(command)) => {
                self.show_command_palette = false;
                self.palette_query.clear();
                self.run_palette_command(command);
                return;
            }
            Some(Action::RunUserCommand { name, scope }) => {
                self.show_command_palette = false;
                self.palette_query.clear();
                self.run_user_command(&name, scope);
                return;
            }
            Some(Action::JumpContext(ctx_idx, context_id, pane_id)) => {
                self.jump_to_context(ctx_idx, context_id, pane_id);
                self.show_command_palette = false;
                self.palette_query.clear();
                return;
            }
            Some(Action::LaunchApp(id)) => {
                self.show_command_palette = false;
                self.palette_query.clear();
                self.launch_app_by_id(&id);
                return;
            }
            Some(Action::LaunchBuiltin(id)) => {
                self.show_command_palette = false;
                self.palette_query.clear();
                self.launch_builtin_by_id(id);
                return;
            }
            Some(Action::OpenNote(path)) => {
                self.show_command_palette = false;
                self.palette_query.clear();
                let active = self.active_window;
                let path_str = path.display().to_string();
                if let Some((existing_tile_id, _)) = self.find_open_text_editor_tile(active, &path)
                {
                    log::info!("palette: note already open, focusing pane");
                    self.set_window_focused_pane(active, existing_tile_id);
                } else {
                    log::info!("palette: opening note {:?} in new pane", path);
                    let _ =
                        self.launch_app_by_id_with_layout("text-editor", None, &[path_str], None);
                }
                return;
            }
            None => {}
        }

        if !self.show_command_palette {
            return;
        }

        // ── Render ─────────────────────────────────────────────────────────
        // 0.8: the palette is a launcher, not a workspace — filling nearly
        // the whole screen height read as massive.
        let palette_max_list_h = ((ctx.screen_rect().height() - 80.0 - 120.0) * 0.8).max(200.0);
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
                                .focused(true)
                                .log_name("command_palette")
                                .show(ui, &mut self.palette_query, &colors)
                        },
                    )
                    .inner;
                if te.changed() {
                    self.palette_selected = 0;
                }

                ui.add_space(style::SPACE_SM);

                if entries.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(ui.available_width());
                        description_label(ui, "No matching contexts or apps", &colors);
                    });
                    return;
                }

                let mut click_action: Option<Action> = None;
                let mut hover_select: Option<usize> = None;
                let mouse_moved = ctx.input(|i| i.pointer.delta().length_sq() > 0.5);
                let should_scroll = self.palette_selected != prev_selected;
                let mut shown_apps_header = false;
                let mut shown_notes_header = false;
                let mut shown_commands_header = false;
                let mut shown_run_header = false;

                egui::ScrollArea::vertical()
                    // animated(false): required by scroll_row_into_view — see src/ui/list.rs.
                    .animated(false)
                    .id_salt("palette_list")
                    .max_height(palette_max_list_h)
                    .min_scrolled_height(palette_max_list_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // available_width — the scroll area reserves a
                        // scrollbar gutter; forcing the modal width would
                        // push rows back under the bar.
                        ui.set_width(ui.available_width());

                        for (i, entry) in entries.iter().enumerate() {
                            let is_selected = i == self.palette_selected;

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
                                    let row_response = ListRow::new(name)
                                        .metadata_chips(&["cmd"])
                                        .secondary(description)
                                        .selected(is_selected)
                                        .show(ui, &colors);
                                    if is_selected {
                                        row_response.scroll_into_view(ui, should_scroll);
                                    }
                                    if row_response.row_clicked() {
                                        click_action = Some(Action::RunCommand(*command));
                                    }
                                    if row_response.row_hovered() {
                                        hover_select = Some(i);
                                    }
                                }
                                PaletteEntry::Context {
                                    ctx_idx,
                                    context_id,
                                    name,
                                    workspace_name,
                                    metadata_chip,
                                    pane_pips,
                                    pane_id,
                                    ..
                                } => {
                                    let mut row = ListRow::new(name.as_str())
                                        .metadata_chips(std::slice::from_ref(metadata_chip))
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
                                        click_action = Some(Action::JumpContext(
                                            *ctx_idx,
                                            *context_id,
                                            *pane_id,
                                        ));
                                    }
                                    if row_response.row_hovered() {
                                        hover_select = Some(i);
                                    }
                                }
                                PaletteEntry::App {
                                    id,
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
                                        click_action = Some(Action::LaunchApp(id.clone()));
                                    }
                                    if row_response.row_hovered() {
                                        hover_select = Some(i);
                                    }
                                }
                                PaletteEntry::Builtin {
                                    id,
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
                                        click_action = Some(Action::LaunchBuiltin(id));
                                    }
                                    if row_response.row_hovered() {
                                        hover_select = Some(i);
                                    }
                                }
                                PaletteEntry::Note {
                                    path,
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
                                        click_action = Some(Action::OpenNote(path.clone()));
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
                                        click_action = Some(Action::RunUserCommand {
                                            name: name.clone(),
                                            scope: *scope,
                                        });
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
                    }
                }

                if let Some(act) = click_action {
                    match act {
                        Action::RunCommand(command) => {
                            self.show_command_palette = false;
                            self.palette_query.clear();
                            self.run_palette_command(command);
                        }
                        Action::RunUserCommand { name, scope } => {
                            self.show_command_palette = false;
                            self.palette_query.clear();
                            self.run_user_command(&name, scope);
                        }
                        Action::JumpContext(ctx_idx, context_id, pane_id) => {
                            self.jump_to_context(ctx_idx, context_id, pane_id);
                            self.show_command_palette = false;
                            self.palette_query.clear();
                        }
                        Action::LaunchApp(id) => {
                            self.show_command_palette = false;
                            self.palette_query.clear();
                            self.launch_app_by_id(&id);
                        }
                        Action::LaunchBuiltin(id) => {
                            self.show_command_palette = false;
                            self.palette_query.clear();
                            self.launch_builtin_by_id(id);
                        }
                        Action::OpenNote(path) => {
                            self.show_command_palette = false;
                            self.palette_query.clear();
                            let active = self.active_window;
                            let path_str = path.display().to_string();
                            if let Some((existing_tile_id, _)) =
                                self.find_open_text_editor_tile(active, &path)
                            {
                                log::info!("palette: note already open, focusing pane");
                                self.set_window_focused_pane(active, existing_tile_id);
                            } else {
                                log::info!("palette: opening note {:?} in new pane", path);
                                let _ = self.launch_app_by_id_with_layout(
                                    "text-editor",
                                    None,
                                    &[path_str],
                                    None,
                                );
                            }
                        }
                    }
                }
            });

        if modal_response.dismissed {
            self.show_command_palette = false;
            self.palette_query.clear();
            self.palette_selected = 0;
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

    fn run_palette_command(&mut self, command: PaletteCommand) {
        log::info!("palette: running host command {command:?}");
        match command {
            PaletteCommand::SplitRight => {
                self.windows[self.active_window].clear_zoom();
                self.ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                self.split_focused(false, None, false, false, None);
                self.save_workspace();
            }
            PaletteCommand::SplitDown => {
                self.windows[self.active_window].clear_zoom();
                self.ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                self.split_focused(true, None, false, false, None);
                self.save_workspace();
            }
            PaletteCommand::OpenConfig => {
                crate::config::open_config_file();
            }
            PaletteCommand::OpenQuickNote => {
                self.open_quick_note_modal();
            }
            PaletteCommand::OpenScratchpad => {
                self.open_scratchpad();
            }
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
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        let initial_cmd = format!("plexi run {}", shell_single_quote(name));
        self.split_focused(false, Some(&initial_cmd), false, false, cwd);
        self.save_workspace();
    }

    /// Jump to a window by index, switching context if necessary.
    /// If `pane_id` is provided, also focuses that specific pane in the window.
    fn jump_to_context(&mut self, ctx_idx: usize, win_id: u64, pane_id: Option<u64>) {
        let target_ctx_id = self.windows[ctx_idx].context_id;
        log::info!("palette: jump to context {target_ctx_id} (window {win_id}, pane {pane_id:?})");
        if let Some(ctx_idx_sidebar) = self.router.position(|c| c.context_id == target_ctx_id) {
            if ctx_idx_sidebar != self.router.active_idx() {
                // switch_workspace → pick_active_context_from_workspace sets
                // active_window based on context_active_window. We override it
                // immediately below.
                self.switch_workspace(ctx_idx_sidebar);
            }
        }
        self.active_window = ctx_idx;
        self.windows[ctx_idx].zoomed_pane = None;
        let ctx_id = self.router.active().context_id;
        self.context_active_window.insert(ctx_id, win_id);
        self.record_context_visit(win_id);

        // Focus the specific pane if requested — find its TileId in the tree.
        if let Some(pid) = pane_id {
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
            vec![PaletteCommand::SplitRight, PaletteCommand::SplitDown]
        );
        assert_eq!(
            palette_command_matches("console"),
            vec![PaletteCommand::SplitRight, PaletteCommand::SplitDown]
        );
        assert_eq!(
            palette_command_matches("hsplit"),
            vec![PaletteCommand::SplitDown]
        );
        assert_eq!(
            palette_command_matches("config"),
            vec![PaletteCommand::OpenConfig]
        );
        assert_eq!(
            palette_command_matches("scratchpad"),
            vec![PaletteCommand::OpenScratchpad]
        );
        assert_eq!(
            palette_command_matches("scratch pad"),
            vec![PaletteCommand::OpenScratchpad]
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
    fn empty_palette_puts_commands_after_apps() {
        let mut entries = vec![
            PaletteEntry::Command {
                command: PaletteCommand::OpenConfig,
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
                ctx_idx: 0,
                context_id: 1,
                name: "Workspace".to_string(),
                workspace_name: String::new(),
                metadata_chip: "ctx",
                pane_pips: None,
                pane_id: None,
                search_text: "workspace".to_string(),
            },
        ];

        sort_palette_entries(&mut entries, "");

        assert!(matches!(entries[0], PaletteEntry::Context { .. }));
        assert!(matches!(entries[1], PaletteEntry::App { .. }));
        assert!(matches!(entries[2], PaletteEntry::Command { .. }));
    }

    #[test]
    fn typed_palette_query_lets_commands_cut_through_group_order() {
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
                command: PaletteCommand::OpenConfig,
                name: "Open config",
                description: "Edit config",
                search_text: "open config settings preferences",
            },
        ];

        sort_palette_entries(&mut entries, "open");

        assert!(matches!(entries[0], PaletteEntry::Command { .. }));
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
                command: PaletteCommand::OpenConfig,
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

        // Prefix match on the command name ranks it ahead of the substring app match.
        sort_palette_entries(&mut entries, "test");
        assert!(matches!(entries[0], PaletteEntry::UserCommand { .. }));
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
        use crate::process_app::ProcessApp;

        let make_app = |manifest_id: &str, name: &str| {
            let (process_app, _draw_tx) = ProcessApp::new_for_test(99, AppPermissions::builtin());
            Pane::App(Box::new(AppPane {
                pip_status: None,
                id: 99,
                runtime: AppRuntime::Process(Box::new(process_app)),
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
            }))
        };

        let (name, chip) = pane_row_identity(&make_app("notes", "My Notes"), &[]);
        assert_eq!((name.as_str(), chip), ("My Notes", "app"));

        let (name, chip) = pane_row_identity(&make_app("text-editor", "todo.md"), &[]);
        assert_eq!((name.as_str(), chip), ("todo.md", "text"));

        // Empty pane name falls back to the runtime display name.
        let (name, _) = pane_row_identity(&make_app("notes", ""), &[]);
        assert_eq!(name, "Test App");
    }
}
