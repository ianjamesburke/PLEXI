use egui::{Align2, Color32, CornerRadius, RichText, Stroke, Vec2};

use crate::app::PlexiApp;
use crate::app_protocol::{StackDirection, UiNode, UiPadding};
use crate::overlays::MODAL_WIDTH;
use crate::theme::Colors;
use crate::widgets::selectable_row;

enum PaletteEntry {
    Context {
        ctx_idx: usize,
        context_id: u64,
        name: String,
        workspace_name: String,
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
}

impl PlexiApp {
    pub(crate) fn draw_command_palette(&mut self, ctx: &egui::Context) {
        let query = self.palette_query.to_lowercase();

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
            // For each context we emit exactly ONE primary entry — either the
            // context name (active pane is unnamed) or "context › pane-name"
            // (active pane is named). Additional named panes beyond the active
            // one appear as secondary entries. This keeps the ⌘P → ↓ → Enter
            // flow clean: the top entry for each context always represents the
            // most-recently-active pane.
            let mut seen_contexts: std::collections::HashSet<u64> =
                std::collections::HashSet::new();
            let mut ctx_entries: Vec<PaletteEntry> = Vec::new();

            // Resolve the active pane name for a context (following
            // context_active_window → focused_pane → terminal name).
            let active_pane_for_context = |ctx_id: u64| -> Option<(usize, u64, u64, String)> {
                let active_win_id = self.context_active_window.get(&ctx_id).copied()?;
                let (win_ci, win) = self
                    .windows
                    .iter()
                    .enumerate()
                    .find(|(_, w)| w.window_id == active_win_id)?;
                let tile_id = win.focused_pane?;
                let tile = win.tree.tiles.get(tile_id)?;
                let pane_id = match tile {
                    egui_tiles::Tile::Pane(pid) => *pid,
                    _ => return None,
                };
                let pane_name = win
                    .panes
                    .get(&pane_id)
                    .and_then(|p| p.as_terminal())
                    .and_then(|t| t.name.clone())?;
                Some((win_ci, active_win_id, pane_id, pane_name))
            };

            for (ci, w) in self.windows.iter().enumerate() {
                let ctx_name = self
                    .router
                    .iter()
                    .find(|c| c.context_id == w.context_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();

                // Primary entry — one per context.
                if !seen_contexts.contains(&w.context_id) {
                    seen_contexts.insert(w.context_id);

                    // Fallback window if context_active_window is absent/stale.
                    let fallback_win_id = self
                        .context_active_window
                        .get(&w.context_id)
                        .copied()
                        .and_then(|id| {
                            self.windows.iter().enumerate().find(|(_, w2)| w2.window_id == id)
                        })
                        .map(|(i, w2)| (i, w2.window_id))
                        .unwrap_or((ci, w.window_id));

                    if let Some((win_ci, win_id, pane_id, pane_name)) =
                        active_pane_for_context(w.context_id)
                    {
                        // Active pane is named — it IS the primary entry.
                        if query.is_empty()
                            || pane_name.to_lowercase().contains(&query)
                            || ctx_name.to_lowercase().contains(&query)
                        {
                            ctx_entries.push(PaletteEntry::Context {
                                ctx_idx: win_ci,
                                context_id: win_id,
                                name: pane_name,
                                workspace_name: ctx_name.clone(),
                                pane_id: Some(pane_id),
                            });
                        }
                    } else {
                        // Active pane is unnamed — show the context by name.
                        if query.is_empty() || ctx_name.to_lowercase().contains(&query) {
                            ctx_entries.push(PaletteEntry::Context {
                                ctx_idx: fallback_win_id.0,
                                context_id: fallback_win_id.1,
                                name: ctx_name.clone(),
                                workspace_name: String::new(),
                                pane_id: None,
                            });
                        }
                    }
                }

                // Secondary entries — named panes that are NOT the active pane.
                let active_win_id = self
                    .context_active_window
                    .get(&w.context_id)
                    .copied()
                    .unwrap_or(0);
                let active_pane_id = if w.window_id == active_win_id {
                    w.focused_pane.and_then(|tid| {
                        w.tree.tiles.get(tid).and_then(|tile| match tile {
                            egui_tiles::Tile::Pane(pid) => Some(*pid),
                            _ => None,
                        })
                    })
                } else {
                    None
                };

                for (&pane_id, pane) in &w.panes {
                    if let Some(t) = pane.as_terminal() {
                        if let Some(pane_name) = &t.name {
                            // Skip if this is already the primary entry.
                            if Some(pane_id) == active_pane_id && w.window_id == active_win_id {
                                continue;
                            }
                            if query.is_empty()
                                || pane_name.to_lowercase().contains(&query)
                                || ctx_name.to_lowercase().contains(&query)
                            {
                                ctx_entries.push(PaletteEntry::Context {
                                    ctx_idx: ci,
                                    context_id: w.window_id,
                                    name: pane_name.clone(),
                                    workspace_name: ctx_name.clone(),
                                    pane_id: Some(pane_id),
                                });
                            }
                        }
                    }
                }
            }

            ctx_entries.sort_by_key(|e| match e {
                PaletteEntry::Context { context_id, .. } => rank_of(*context_id),
                _ => (usize::MAX, usize::MAX),
            });

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
            self.registry = crate::app_registry::AppRegistry::load(rescan_cwd);
        }

        let app_entries: Vec<(String, String, String, bool)> = self
            .registry
            .list()
            .into_iter()
            .filter(|app| {
                // Local apps are visible only when the focused pane is in their workspace.
                // Use the same explicit predicate here as in the badge below — no wildcard.
                let workspace_visible = match app.source {
                    crate::app_registry::RegistrySource::Global => true,
                    crate::app_registry::RegistrySource::LocalApp
                    | crate::app_registry::RegistrySource::LocalAgent => {
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
                    crate::app_registry::RegistrySource::LocalApp
                        | crate::app_registry::RegistrySource::LocalAgent
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

        let total = entries.len();

        if self.palette_selected >= total && total > 0 {
            self.palette_selected = total - 1;
        }

        // ── Keyboard nav ───────────────────────────────────────────────────
        #[derive(Clone)]
        enum Action {
            JumpContext(usize, u64, Option<u64>),
            LaunchApp(String),
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
            None => {}
        }

        if !self.show_command_palette {
            return;
        }

        // ── Render ─────────────────────────────────────────────────────────
        let screen_rect = ctx.screen_rect();
        let palette_max_list_h = (screen_rect.height() - 80.0 - 120.0).max(200.0);

        egui::Area::new(egui::Id::new("palette_scrim"))
            .fixed_pos(screen_rect.min)
            .show(ctx, |ui| {
                ui.painter()
                    .rect_filled(screen_rect, 0.0, Color32::from_black_alpha(120));
                let scrim_response = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_response.clicked() {
                    self.show_command_palette = false;
                }
            });

        egui::Area::new(egui::Id::new("command_palette"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);

                        let te_id = egui::Id::new("palette_search");
                        let te = ui.scope(|ui| {
                            ui.visuals_mut().text_cursor.stroke.width = 1.5;
                            ui.visuals_mut().text_cursor.stroke.color = self.colors.accent;
                            ui.visuals_mut().extreme_bg_color = self.colors.bg_active;
                            ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, self.colors.accent);
                            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, self.colors.border);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.palette_query)
                                    .id(te_id)
                                    .desired_width(MODAL_WIDTH)
                                    .hint_text("Jump to context or launch app...")
                                    .font(egui::TextStyle::Body)
                                    .margin(egui::Margin::symmetric(8, 5)),
                            )
                        }).inner;
                        if !te.has_focus() {
                            te.request_focus();
                        }
                        if te.changed() {
                            self.palette_selected = 0;
                        }

                        ui.add_space(6.0);

                        if entries.is_empty() {
                            ui.label(
                                RichText::new("No matching contexts or apps")
                                    .size(11.0)
                                    .color(self.colors.text_dim),
                            );
                            return;
                        }

                        let mut shown_apps_header = false;
                        let mut click_action: Option<Action> = None;
                        let mut hover_select: Option<usize> = None;
                        let colors = self.colors;
                        let mouse_moved = ctx.input(|i| i.pointer.delta().length_sq() > 0.5);
                        let should_scroll = self.palette_selected != prev_selected;

                        egui::ScrollArea::vertical()
                            .max_height(palette_max_list_h)
                            .min_scrolled_height(palette_max_list_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(MODAL_WIDTH);

                                for (i, entry) in entries.iter().enumerate() {
                                    let is_selected = i == self.palette_selected;

                                    match entry {
                                        PaletteEntry::Context {
                                            ctx_idx,
                                            context_id,
                                            name,
                                            workspace_name,
                                            pane_id,
                                        } => {
                                            let is_active = *context_id == active_win_id;
                                            let name_color = if is_active {
                                                colors.accent
                                            } else {
                                                colors.text_primary
                                            };

                                            let (r, _) = selectable_row(
                                                ui,
                                                is_selected,
                                                &colors,
                                                |ui| {
                                                    ui.horizontal(|ui| {
                                                        crate::widgets::key_chip(ui, "ctx", &colors);
                                                        if !workspace_name.is_empty() {
                                                            ui.label(
                                                                RichText::new(workspace_name.as_str())
                                                                    .size(10.0)
                                                                    .color(colors.text_dim),
                                                            );
                                                            ui.label(
                                                                RichText::new("\u{203A}")
                                                                    .size(10.0)
                                                                    .color(colors.text_dim),
                                                            );
                                                        }
                                                        ui.label(
                                                            RichText::new(name.as_str())
                                                                .size(12.0)
                                                                .color(name_color),
                                                        );
                                                    });
                                                },
                                            );

                                            if is_selected && should_scroll {
                                                r.scroll_to_me(None);
                                            }
                                            if r.clicked() {
                                                click_action = Some(Action::JumpContext(
                                                    *ctx_idx,
                                                    *context_id,
                                                    *pane_id,
                                                ));
                                            }
                                            if r.hovered() {
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
                                                ui.add_space(4.0);
                                                ui.label(
                                                    RichText::new("APPS")
                                                        .size(9.0)
                                                        .color(colors.text_dim),
                                                );
                                                ui.add_space(2.0);
                                            }

                                            let row_node = build_app_row_node(
                                                name,
                                                description,
                                                *running_in_background,
                                                *is_workspace_local,
                                                &colors,
                                            );
                                            let (r, _) = selectable_row(
                                                ui,
                                                is_selected,
                                                &colors,
                                                |ui| {
                                                    ui.horizontal(|ui| {
                                                        crate::widgets::key_chip(ui, "app", &colors);
                                                        crate::render_components::render_component_tree(
                                                            ui,
                                                            &row_node,
                                                            &colors,
                                                        );
                                                    });
                                                },
                                            );

                                            if is_selected && should_scroll {
                                                r.scroll_to_me(None);
                                            }
                                            if r.clicked() {
                                                click_action =
                                                    Some(Action::LaunchApp(id.clone()));
                                            }
                                            if r.hovered() {
                                                hover_select = Some(i);
                                            }
                                        }

                                    }
                                }
                            });

                        ui.separator();
                        ui.horizontal(|ui| {
                            crate::widgets::key_combo_list(ui, &[&["↑↓"]], Some("navigate"), &colors);
                            ui.add_space(crate::style::SPACE_MD);
                            crate::widgets::key_combo_list(ui, &[&["↵"]], Some("select"), &colors);
                            ui.add_space(crate::style::SPACE_MD);
                            crate::widgets::key_combo_list(ui, &[&["esc"]], Some("dismiss"), &colors);
                        });

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
                            }
                        }
                    });
            });
    }

    /// Jump to a window by index, switching context if necessary.
    /// If `pane_id` is provided, also focuses that specific pane in the window.

    fn jump_to_context(&mut self, ctx_idx: usize, win_id: u64, pane_id: Option<u64>) {
        let target_ctx_id = self.windows[ctx_idx].context_id;
        if let Some(ctx_idx_sidebar) = self
            .router
            .position(|c| c.context_id == target_ctx_id)
        {
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
            if let Some(tile_id) = win
                .tree
                .tiles
                .iter()
                .find_map(|(tid, tile)| {
                    if matches!(tile, egui_tiles::Tile::Pane(p) if *p == pid) {
                        Some(*tid)
                    } else {
                        None
                    }
                })
            {
                win.focused_pane = Some(tile_id);
            }
        }
    }

}

// ── Free helpers — component-tree builders ────────────────────────────────────

fn color_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c.r(), c.g(), c.b(), c.a())
}

/// Build a `UiNode` tree for an app palette row.
///
/// Returns a vertical `Stack` with:
/// - a horizontal name row: hex icon + app name + optional "bg" / "ws" badges
/// - an optional description line (dim, small)
///
/// Extracted so the node construction is testable without a live egui context.
/// Used by `draw_command_palette` to render `PaletteEntry::App` rows.
pub(crate) fn build_app_row_node(
    name: &str,
    description: &str,
    running_in_background: bool,
    is_workspace_local: bool,
    colors: &Colors,
) -> UiNode {
    let mut name_children: Vec<UiNode> = vec![
        UiNode::Text {
            text: "⬡".to_string(),
            size: 10.0,
            color: color_hex(colors.accent),
            bold: false,
            monospace: false,
        },
        UiNode::Text {
            text: name.to_string(),
            size: 12.0,
            color: color_hex(colors.text_primary),
            bold: false,
            monospace: false,
        },
    ];
    if running_in_background {
        name_children.push(UiNode::Text {
            text: "bg".to_string(),
            size: 9.0,
            color: color_hex(colors.text_dim),
            bold: false,
            monospace: false,
        });
    }
    if is_workspace_local {
        name_children.push(UiNode::Text {
            text: "ws".to_string(),
            size: 9.0,
            color: color_hex(colors.accent),
            bold: false,
            monospace: false,
        });
    }

    let name_row = UiNode::Stack {
        direction: StackDirection::Horizontal,
        children: name_children,
        gap: 4.0,
        padding: UiPadding::default(),
    };

    if description.is_empty() {
        name_row
    } else {
        UiNode::Stack {
            direction: StackDirection::Vertical,
            children: vec![
                name_row,
                UiNode::Text {
                    text: description.to_string(),
                    size: 9.0,
                    color: color_hex(colors.text_dim),
                    bold: false,
                    monospace: false,
                },
            ],
            gap: 2.0,
            padding: UiPadding::default(),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod command_palette_node_tests {
    use super::*;
    use crate::config::ThemeConfig;
    use crate::theme::Colors;

    fn test_colors() -> Colors {
        Colors::from_config(&ThemeConfig::default())
    }

    /// The app row node must always include the app name as a `UiNode::Text`.
    #[test]
    fn command_row_node_includes_label() {
        let colors = test_colors();
        let node = build_app_row_node("MyApp", "", false, false, &colors);

        // Without description the top-level node is the name row (horizontal Stack).
        let children = match &node {
            UiNode::Stack { children, direction, .. } => {
                assert!(matches!(direction, StackDirection::Horizontal));
                children
            }
            _ => panic!("expected UiNode::Stack for name-only row"),
        };

        // Second child is the app name text.
        assert!(children.len() >= 2);
        if let UiNode::Text { text, .. } = &children[1] {
            assert_eq!(text, "MyApp");
        } else {
            panic!("expected UiNode::Text at index 1");
        }
    }

    /// When `running_in_background` is true the name row contains a "bg" badge node.
    #[test]
    fn command_row_node_includes_bg_badge() {
        let colors = test_colors();
        let node = build_app_row_node("Foo", "", true, false, &colors);
        let children = match &node {
            UiNode::Stack { children, .. } => children,
            _ => panic!("expected Stack"),
        };
        let has_bg = children.iter().any(|n| {
            matches!(n, UiNode::Text { text, .. } if text == "bg")
        });
        assert!(has_bg, "expected 'bg' badge node");
    }

    /// When `is_workspace_local` is true the name row contains a "ws" badge node.
    #[test]
    fn command_row_node_includes_ws_badge() {
        let colors = test_colors();
        let node = build_app_row_node("Bar", "", false, true, &colors);
        let children = match &node {
            UiNode::Stack { children, .. } => children,
            _ => panic!("expected Stack"),
        };
        let has_ws = children.iter().any(|n| {
            matches!(n, UiNode::Text { text, .. } if text == "ws")
        });
        assert!(has_ws, "expected 'ws' badge node");
    }

    /// When a non-empty description is provided the top node is a vertical Stack
    /// whose second child carries the description text.
    #[test]
    fn command_row_node_description_vertical_stack() {
        let colors = test_colors();
        let node = build_app_row_node("MyApp", "does stuff", false, false, &colors);
        match &node {
            UiNode::Stack { direction, children, .. } => {
                assert!(matches!(direction, StackDirection::Vertical));
                assert_eq!(children.len(), 2);
                if let UiNode::Text { text, .. } = &children[1] {
                    assert_eq!(text, "does stuff");
                } else {
                    panic!("expected description UiNode::Text at index 1");
                }
            }
            _ => panic!("expected vertical Stack when description is non-empty"),
        }
    }
}
