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
    Context {
        ctx_idx: usize,
        context_id: u64,
        name: String,
        workspace_name: String,
        metadata_chip: &'static str,
        pane_pips: Option<ListRowPips>,
        /// If set, focus this specific pane after navigating to the window.
        pane_id: Option<u64>,
    },
    App {
        id: String,
        name: String,
        description: String,
        running_in_background: bool,
        is_workspace_local: bool,
    },
    Note {
        path: std::path::PathBuf,
        title: String,
        preview: String,
    },
}

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
}

fn palette_pane_rows_for_context(
    windows: &[crate::host::context::Window],
    context_active_window: &std::collections::HashMap<u64, u64>,
    context_names: &[(u64, String)],
    ctx_id: u64,
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

    let active_window_id = context_active_window
        .get(&ctx_id)
        .copied()
        .filter(|id| {
            windows
                .iter()
                .any(|w| w.window_id == *id && w.context_id == ctx_id)
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

    let mut rows = Vec::new();
    for &win_idx in &ctx_windows {
        let win = &windows[win_idx];
        let Some(root) = win.tree.root() else {
            continue;
        };
        for pane_id in crate::spatial::tiling::collect_pane_ids_spatial(&win.tree.tiles, root) {
            let Some(pane) = win.panes.get(&pane_id) else {
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
                    hidden_indices: if pane.is_hidden() { vec![0] } else { Vec::new() },
                    activities: vec![pane.effective_activity().cloned()],
                },
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
    pub(crate) fn draw_command_palette(&mut self, ctx: &egui::Context) {
        let query = self.palette_query.to_lowercase();
        let colors = self.colors;

        // ── Window entries (active context first, then by visit recency) ──
        // Two-tier sort: panes whose window belongs to the active context float
        // above panes in other contexts. Within each tier, recency wins.
        // Mirrors macOS Cmd+Tab — current app's windows first.
        let active_win_id = self.windows[self.active_window].window_id;
        let active_ctx_id = self.windows[self.active_window].context_id;

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

        let mut entries: Vec<PaletteEntry> = {
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
                let resolved = self
                    .context_active_window
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
                    ) {
                        if query.is_empty()
                            || row.name.to_lowercase().contains(&query)
                            || c.name.to_lowercase().contains(&query)
                        {
                            ctx_entries.push(PaletteEntry::Context {
                                ctx_idx: row.win_idx,
                                context_id: row.window_id,
                                name: row.name,
                                workspace_name: c.name.clone(),
                                metadata_chip: row.chip,
                                pane_pips: Some(row.pips),
                                pane_id: Some(row.pane_id),
                            });
                        }
                    }
                } else {
                    // Inactive context — one collapsed row, full pip strip.
                    if query.is_empty() || c.name.to_lowercase().contains(&query) {
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
                        ) {
                            if row.name.to_lowercase().contains(&query) {
                                ctx_entries.push(PaletteEntry::Context {
                                    ctx_idx: row.win_idx,
                                    context_id: row.window_id,
                                    name: row.name,
                                    workspace_name: c.name.clone(),
                                    metadata_chip: row.chip,
                                    pane_pips: Some(row.pips),
                                    pane_id: Some(row.pane_id),
                                });
                            }
                        }
                    }
                }
            }

            ctx_entries
        };

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

        let app_entries: Vec<(String, String, String, bool)> = self
            .registry
            .list()
            .into_iter()
            .filter(|app| {
                // Local apps are visible only when the focused pane is in their workspace.
                // Use the same explicit predicate here as in the badge below — no wildcard.
                let workspace_visible = match app.source {
                    crate::app::registry::RegistrySource::Global => true,
                    crate::app::registry::RegistrySource::LocalApp
                    | crate::app::registry::RegistrySource::LocalAgent => {
                        app.workspace_root.as_ref() == focused_workspace_root
                    }
                };
                workspace_visible
                    && (query.is_empty()
                        || app.manifest.name.to_lowercase().contains(&query)
                        || app.manifest.id.to_lowercase().contains(&query)
                        || app.manifest.description.to_lowercase().contains(&query))
            })
            .map(|app| {
                let is_local = matches!(
                    app.source,
                    crate::app::registry::RegistrySource::LocalApp
                        | crate::app::registry::RegistrySource::LocalAgent
                );
                (
                    app.manifest.id.clone(),
                    app.manifest.name.clone(),
                    app.manifest.description.clone(),
                    is_local,
                )
            })
            .collect();

        for (id, name, description, is_workspace_local) in app_entries {
            let running_in_background = self.background_apps.contains_key(&id);
            entries.push(PaletteEntry::App {
                id,
                name,
                description,
                running_in_background,
                is_workspace_local,
            });
        }

        // ── Note entries ────────────────────────────────────────────────────
        for note in &self.palette_notes {
            let matches = query.is_empty()
                || note.title.to_lowercase().contains(&query)
                || note.search_text.contains(&query);
            if matches {
                entries.push(PaletteEntry::Note {
                    path: note.path.clone(),
                    title: note.title.clone(),
                    preview: note.preview.clone(),
                });
            }
        }

        let total = entries.len();

        if self.palette_selected >= total && total > 0 {
            self.palette_selected = total - 1;
        }

        // ── Keyboard nav ───────────────────────────────────────────────────
        #[derive(Clone)]
        enum Action {
            JumpContext(usize, u64, Option<u64>),
            LaunchApp(String),
            OpenNote(std::path::PathBuf),
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
                    Some(PaletteEntry::Note { path, .. }) => {
                        action = Some(Action::OpenNote(path.clone()));
                    }
                    None => {}
                }
            }
        });

        match action {
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
            Some(Action::OpenNote(path)) => {
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
                let te = TextField::singleline(te_id, "Jump to context or launch app...")
                    .focused(true)
                    .log_name("command_palette")
                    .show(ui, &mut self.palette_query, &colors);
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
                                PaletteEntry::Context {
                                    ctx_idx,
                                    context_id,
                                    name,
                                    workspace_name,
                                    metadata_chip,
                                    pane_pips,
                                    pane_id,
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
                                PaletteEntry::Note { path, title, preview } => {
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
                                        .chip("text")
                                        .secondary(preview.as_str())
                                        .selected(is_selected);
                                    let row_response = row.show(ui, &colors);
                                    if is_selected {
                                        row_response.scroll_into_view(ui, should_scroll);
                                    }
                                    if row_response.row_clicked() {
                                        click_action =
                                            Some(Action::OpenNote(path.clone()));
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
                        Action::OpenNote(path) => {
                            self.show_command_palette = false;
                            self.palette_query.clear();
                            let active = self.active_window;
                            let path_str = path.display().to_string();
                            if let Some((existing_tile_id, _)) =
                                self.find_open_text_editor_tile(active, &path)
                            {
                                log::info!(
                                    "palette: note already open, focusing pane"
                                );
                                self.set_window_focused_pane(active, existing_tile_id);
                            } else {
                                log::info!(
                                    "palette: opening note {:?} in new pane",
                                    path
                                );
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

    /// Jump to a window by index, switching context if necessary.
    /// If `pane_id` is provided, also focuses that specific pane in the window.
    fn jump_to_context(&mut self, ctx_idx: usize, win_id: u64, pane_id: Option<u64>) {
        let target_ctx_id = self.windows[ctx_idx].context_id;
        log::info!(
            "palette: jump to context {target_ctx_id} (window {win_id}, pane {pane_id:?})"
        );
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

        let rows = palette_pane_rows_for_context(&windows, &context_active_window, &names, 1);

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

        let rows = palette_pane_rows_for_context(&windows, &context_active_window, &[], 1);

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
    fn pane_row_identity_chips_apps_and_text_editors() {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};
        use crate::process_app::ProcessApp;

        let make_app = |manifest_id: &str, name: &str| {
            let (process_app, _draw_tx) = ProcessApp::new_for_test(99, AppPermissions::builtin());
            Pane::App(Box::new(AppPane {
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
