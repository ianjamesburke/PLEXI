use crate::keys::{self, Action, Direction};
use crate::pane::TerminalPane;
use crate::shell;
use crate::theme::{self, Colors};
use crate::tiling::{PaneId, PlexiBehavior};
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Stroke, StrokeKind,
    Vec2,
};
use egui_term::{BackendSettings, PtyEvent, TerminalFont, TerminalTheme};
use egui_tiles::{Container, SimplificationOptions, Tile, TileId, Tree};
use std::collections::HashMap;
use std::sync::mpsc;

struct Context {
    name: String,
    path: String,
}

pub struct PlexiApp {
    tree: Tree<PaneId>,
    panes: HashMap<PaneId, TerminalPane>,
    focused_pane: Option<TileId>,
    pty_event_rx: mpsc::Receiver<(u64, PtyEvent)>,
    pty_event_tx: mpsc::Sender<(u64, PtyEvent)>,
    theme: TerminalTheme,
    font: TerminalFont,
    next_pane_id: u64,
    ctx: egui::Context,
    contexts: Vec<Context>,
    active_context: usize,
    sidebar_visible: bool,
    show_shortcuts: bool,
    quitting: bool,
}

impl PlexiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(target_os = "macos")]
        crate::macos_menu::remove_hide_menu_item();

        theme::setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        theme::setup_style(&cc.egui_ctx);

        let (tx, rx) = mpsc::channel();
        let settings = Self::make_backend_settings(None);

        let mut panes = HashMap::new();
        let pane = TerminalPane::new(0, cc.egui_ctx.clone(), tx.clone(), settings)
            .expect("failed to create initial terminal");
        panes.insert(0u64, pane);

        let mut tiles = egui_tiles::Tiles::default();
        let root_tile = tiles.insert_pane(0u64);
        let tree = Tree::new("plexi", root_tile, tiles);

        Self {
            tree,
            panes,
            focused_pane: Some(root_tile),
            pty_event_rx: rx,
            pty_event_tx: tx,
            theme: theme::catppuccin_mocha(),
            font: theme::terminal_font(),
            next_pane_id: 1,
            ctx: cc.egui_ctx.clone(),
            contexts: vec![
                Context { name: "Plexi".into(), path: "~/Documents/GitHub/plexi".into() },
            ],
            active_context: 0,
            sidebar_visible: true,
            show_shortcuts: false,
            quitting: false,
        }
    }

    fn make_backend_settings(working_directory: Option<std::path::PathBuf>) -> BackendSettings {
        BackendSettings {
            shell: shell::detect_shell(),
            args: vec!["-l".to_string()],
            env: shell::build_env(),
            working_directory,
            ..Default::default()
        }
    }

    fn drain_pty_events(&mut self) {
        while let Ok((id, event)) = self.pty_event_rx.try_recv() {
            if matches!(event, PtyEvent::Exit) {
                if let Some(pane) = self.panes.get_mut(&id) {
                    pane.exited = true;
                }
            }
        }
    }

    fn split_focused(&mut self, vertical: bool) {
        let Some(focused) = self.focused_pane else {
            return;
        };

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

        // Inherit cwd from focused pane's shell process
        let cwd = self.get_focused_pane_cwd(focused);
        let settings = Self::make_backend_settings(cwd);
        let Some(pane) =
            TerminalPane::new(new_id, self.ctx.clone(), self.pty_event_tx.clone(), settings)
        else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        self.panes.insert(new_id, pane);

        let parent = self.tree.tiles.parent_of(focused);
        let new_tile = self.tree.tiles.insert_pane(new_id);

        let container_tile = if vertical {
            self.tree
                .tiles
                .insert_vertical_tile(vec![focused, new_tile])
        } else {
            self.tree
                .tiles
                .insert_horizontal_tile(vec![focused, new_tile])
        };

        if let Some(parent_id) = parent {
            if let Some(Tile::Container(parent)) = self.tree.tiles.get_mut(parent_id) {
                replace_child(parent, focused, container_tile);
            }
        } else {
            self.tree.root = Some(container_tile);
        }

        self.focused_pane = Some(new_tile);
    }

    fn new_tab(&mut self) {
        let Some(focused) = self.focused_pane else {
            return;
        };

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

        let cwd = self.get_focused_pane_cwd(focused);
        let settings = Self::make_backend_settings(cwd);
        let Some(pane) =
            TerminalPane::new(new_id, self.ctx.clone(), self.pty_event_tx.clone(), settings)
        else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        self.panes.insert(new_id, pane);

        let new_tile = self.tree.tiles.insert_pane(new_id);

        // Check if focused pane is already inside a Tabs container (possibly nested via splits)
        if let Some((tabs_id, _)) = self.find_ancestor_tabs(focused) {
            if let Some(Tile::Container(Container::Tabs(tabs))) =
                self.tree.tiles.get_mut(tabs_id)
            {
                tabs.add_child(new_tile);
                tabs.set_active(new_tile);
            }
            self.focused_pane = Some(new_tile);
            return;
        }

        // Wrap focused + new in a Tabs container
        let parent = self.tree.tiles.parent_of(focused);
        let tab_tile = self.tree.tiles.insert_tab_tile(vec![focused, new_tile]);

        // Set the new tab as active
        if let Some(Tile::Container(Container::Tabs(tabs))) = self.tree.tiles.get_mut(tab_tile) {
            tabs.set_active(new_tile);
        }

        if let Some(parent_id) = parent {
            if let Some(Tile::Container(parent_container)) = self.tree.tiles.get_mut(parent_id) {
                replace_child(parent_container, focused, tab_tile);
            }
        } else {
            self.tree.root = Some(tab_tile);
        }

        self.focused_pane = Some(new_tile);
    }

    /// Walk up the tree from `tile_id` to find the nearest ancestor Tabs container.
    /// Returns (tabs_tile_id, child_of_tabs) where child_of_tabs is the direct
    /// child of the Tabs container that contains `tile_id`.
    fn find_ancestor_tabs(&self, tile_id: TileId) -> Option<(TileId, TileId)> {
        let mut current = tile_id;
        loop {
            let parent_id = self.tree.tiles.parent_of(current)?;
            if matches!(
                self.tree.tiles.get(parent_id),
                Some(Tile::Container(Container::Tabs(_)))
            ) {
                return Some((parent_id, current));
            }
            current = parent_id;
        }
    }

    /// Find the first pane tile inside `tile_id` (depth-first).
    /// If `tile_id` is itself a pane, returns it directly.
    fn find_first_pane_in(&self, tile_id: TileId) -> Option<TileId> {
        match self.tree.tiles.get(tile_id)? {
            Tile::Pane(_) => Some(tile_id),
            Tile::Container(container) => {
                for &child in container.children() {
                    if let Some(pane) = self.find_first_pane_in(child) {
                        return Some(pane);
                    }
                }
                None
            }
        }
    }

    fn cycle_tab(&mut self, forward: bool) {
        let Some(focused) = self.focused_pane else {
            return;
        };

        let Some((tabs_id, current_tab_child)) = self.find_ancestor_tabs(focused) else {
            return;
        };

        let Some(Tile::Container(Container::Tabs(tabs))) = self.tree.tiles.get(tabs_id) else {
            return;
        };

        let children = &tabs.children;
        if children.len() < 2 {
            return;
        }

        let Some(pos) = children.iter().position(|&c| c == current_tab_child) else {
            return;
        };

        let next_pos = if forward {
            (pos + 1) % children.len()
        } else {
            (pos + children.len() - 1) % children.len()
        };
        let next_tile = children[next_pos];

        // Set the new tab as active
        if let Some(Tile::Container(Container::Tabs(tabs))) = self.tree.tiles.get_mut(tabs_id) {
            tabs.set_active(next_tile);
        }

        // Focus a pane inside the new tab (it might be a container with splits)
        if let Some(pane_tile) = self.find_first_pane_in(next_tile) {
            self.focused_pane = Some(pane_tile);
        }
    }

    fn close_focused(&mut self) {
        let Some(focused) = self.focused_pane else {
            return;
        };

        let next = self.find_next_focus(focused);

        // Remove from parent's children
        if let Some(parent_id) = self.tree.tiles.parent_of(focused) {
            if let Some(Tile::Container(parent)) = self.tree.tiles.get_mut(parent_id) {
                parent.remove_child(focused);
            }
        }

        // Remove tile and pane data
        if let Some(Tile::Pane(pane_id)) = self.tree.tiles.remove(focused) {
            self.panes.remove(&pane_id);
        }

        self.tree.simplify(&SimplificationOptions::default());
        self.focused_pane = next;
    }

    fn find_next_focus(&self, excluding: TileId) -> Option<TileId> {
        // Try spatial directions in priority order: Left, Up, Right, Down
        for dir in [Direction::Left, Direction::Up, Direction::Right, Direction::Down] {
            if let Some(target) = self.find_pane_in_direction_from(excluding, dir) {
                return Some(target);
            }
        }
        // Fallback: any remaining pane
        self.tree
            .active_tiles()
            .into_iter()
            .find(|&id| id != excluding && matches!(self.tree.tiles.get(id), Some(Tile::Pane(_))))
    }

    fn navigate(&mut self, dir: Direction) {
        if let Some(focused) = self.focused_pane {
            if let Some(target) = self.find_pane_in_direction_from(focused, dir) {
                self.focused_pane = Some(target);
            }
        }
    }

    fn find_pane_in_direction_from(&self, from: TileId, dir: Direction) -> Option<TileId> {
        let from_rect = self.tree.tiles.rect(from)?;
        let center = from_rect.center();

        let mut best: Option<(TileId, f32)> = None;

        for tile_id in self.tree.active_tiles() {
            if tile_id == from {
                continue;
            }
            if !matches!(self.tree.tiles.get(tile_id), Some(Tile::Pane(_))) {
                continue;
            }
            let Some(rect) = self.tree.tiles.rect(tile_id) else {
                continue;
            };
            let other = rect.center();

            let valid = match dir {
                Direction::Left => other.x < center.x,
                Direction::Right => other.x > center.x,
                Direction::Up => other.y < center.y,
                Direction::Down => other.y > center.y,
            };

            if valid {
                let dist = center.distance(other);
                if best.map_or(true, |(_, d)| dist < d) {
                    best = Some((tile_id, dist));
                }
            }
        }

        best.map(|(id, _)| id)
    }

    fn get_focused_pane_cwd(&self, tile_id: TileId) -> Option<std::path::PathBuf> {
        let pane_id = match self.tree.tiles.get(tile_id)? {
            Tile::Pane(id) => *id,
            _ => return None,
        };
        let pane = self.panes.get(&pane_id)?;
        shell::get_pid_cwd(pane.backend.child_pid())
    }
}

impl eframe::App for PlexiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("Plexi".into()));
        self.drain_pty_events();

        // Handle keyboard shortcuts
        for action in keys::poll_actions(ctx) {
            match action {
                Action::SplitHorizontal => self.split_focused(false),
                Action::SplitVertical => self.split_focused(true),
                Action::Navigate(dir) => self.navigate(dir),
                Action::ClosePane => {
                    if self.panes.len() > 1 {
                        self.close_focused();
                    } else {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Action::NewTab => self.new_tab(),
                Action::NextTab => self.cycle_tab(true),
                Action::PrevTab => self.cycle_tab(false),
                Action::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Action::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
                Action::ToggleShortcuts => self.show_shortcuts = !self.show_shortcuts,
            }
        }

        // Handle window close request (X button, Cmd+W on macOS)
        // Skip the cancel-close guard when quitting the entire app
        if ctx.input(|i| i.viewport().close_requested()) && self.panes.len() > 1 && !self.quitting
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_focused();
        }

        // All panes exited
        if self.panes.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Toolbar
        egui::TopBottomPanel::top("toolbar")
            .exact_height(28.0)
            .frame(
                egui::Frame::new()
                    .fill(Colors::BG_TOOLBAR)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 4,
                        bottom: 4,
                    }),
            )
            .show(ctx, |ui| {
                self.draw_toolbar(ui);
            });

        // Separator line under toolbar
        egui::TopBottomPanel::top("toolbar_sep")
            .exact_height(1.0)
            .frame(egui::Frame::new().fill(Colors::BORDER))
            .show(ctx, |_ui| {});

        // Sidebar
        if self.sidebar_visible {
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .frame(
                    egui::Frame::new()
                        .fill(Colors::BG_SIDEBAR)
                        .inner_margin(egui::Margin::same(0)),
                )
                .show(ctx, |ui| {
                    self.draw_sidebar(ui);
                });
        }

        // Central panel — terminal tiles
        egui::CentralPanel::default()
            .frame(egui::Frame {
                fill: Colors::BG_DARKEST,
                inner_margin: egui::Margin::same(4),
                outer_margin: egui::Margin::ZERO,
                ..Default::default()
            })
            .show(ctx, |ui| {
                let mut behavior = PlexiBehavior {
                    panes: &mut self.panes,
                    focused_tile: self.focused_pane,
                    theme: self.theme.clone(),
                    font: self.font.clone(),
                    new_focused: None,
                    close_exited: None,
                };
                self.tree.ui(&mut behavior, ui);

                if let Some(new) = behavior.new_focused {
                    self.focused_pane = Some(new);
                }

                if behavior.close_exited.is_some() {
                    self.close_focused();
                }
            });

        // Shortcuts overlay
        if self.show_shortcuts {
            self.draw_shortcuts_overlay(ctx);
        }
    }
}

const R2: CornerRadius = CornerRadius::same(2);
const R3: CornerRadius = CornerRadius::same(3);
const R6: CornerRadius = CornerRadius::same(6);

impl PlexiApp {
    fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let ctx_info = &self.contexts[self.active_context];

            // Sidebar toggle
            let toggle_text = if self.sidebar_visible { "\u{25C0}" } else { "\u{25B6}" };
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(toggle_text).size(11.0).color(Colors::TEXT_DIM),
                    )
                    .frame(false),
                )
                .on_hover_text("Toggle sidebar (\u{2318}B)")
                .clicked()
            {
                self.sidebar_visible = !self.sidebar_visible;
            }

            ui.add_space(8.0);

            // Context info
            ui.label(
                RichText::new(&ctx_info.name)
                    .size(12.0)
                    .color(Colors::TEXT_PRIMARY)
                    .strong(),
            );
            ui.label(
                RichText::new(&ctx_info.path)
                    .size(11.0)
                    .color(Colors::TEXT_DIM)
                    .family(egui::FontFamily::Monospace),
            );
            let pane_count = self.panes.len();
            ui.label(
                RichText::new(format!(
                    "{} pane{}",
                    pane_count,
                    if pane_count == 1 { "" } else { "s" }
                ))
                .size(11.0)
                .color(Colors::TEXT_SECTION),
            );

            // Right side — help button
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("?").size(12.0).color(Colors::TEXT_DIM),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Keyboard shortcuts (\u{2318}/)")
                    .clicked()
                {
                    self.show_shortcuts = !self.show_shortcuts;
                }
            });
        });
    }

    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let sidebar_width = ui.available_width();

        // Branding
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("PLEXI")
                    .size(16.0)
                    .color(Colors::TEXT_PRIMARY)
                    .strong(),
            );
        });
        ui.add_space(12.0);

        // Divider
        let rect = ui.cursor();
        ui.painter().line_segment(
            [
                egui::pos2(rect.min.x, rect.min.y),
                egui::pos2(rect.min.x + sidebar_width, rect.min.y),
            ],
            Stroke::new(1.0, Colors::BORDER),
        );
        ui.add_space(4.0);

        // Contexts section header
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("Contexts")
                    .size(10.0)
                    .color(Colors::TEXT_SECTION),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                ui.add(
                    egui::Button::new(
                        RichText::new("+").size(12.0).color(Colors::TEXT_DIM),
                    )
                    .frame(false),
                )
                .on_hover_text("New context");
            });
        });
        ui.add_space(4.0);

        // Context list
        let active = self.active_context;
        for (i, context) in self.contexts.iter().enumerate() {
            let is_active = i == active;

            let response = ui.allocate_ui_with_layout(
                Vec2::new(sidebar_width, 26.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    let rect = ui.max_rect();
                    let hover = ui.rect_contains_pointer(rect);

                    let fill = if is_active {
                        Colors::BG_ACTIVE
                    } else if hover {
                        Colors::BG_HOVER
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter()
                        .rect_filled(rect, CornerRadius::ZERO, fill);

                    if is_active {
                        ui.painter().rect_filled(
                            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                            CornerRadius::ZERO,
                            Colors::ACCENT,
                        );
                    }

                    ui.add_space(20.0);
                    let text_color = if is_active {
                        Colors::TEXT_PRIMARY
                    } else {
                        Colors::TEXT_DIM
                    };
                    ui.label(RichText::new(&context.name).size(12.0).color(text_color));
                },
            );

            if response.response.clicked() {
                self.active_context = i;
            }
        }

        ui.add_space(16.0);

        // Map section divider
        let rect = ui.cursor();
        ui.painter().line_segment(
            [
                egui::pos2(rect.min.x, rect.min.y),
                egui::pos2(rect.min.x + sidebar_width, rect.min.y),
            ],
            Stroke::new(1.0, Colors::BORDER),
        );
        ui.add_space(4.0);

        // Map section
        ui.add_space(8.0);
        let pane_count = self.panes.len();
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("Map")
                    .size(10.0)
                    .color(Colors::TEXT_SECTION),
            );
            ui.label(
                RichText::new(format!(
                    "{} node{}",
                    pane_count,
                    if pane_count == 1 { "" } else { "s" }
                ))
                .size(10.0)
                .color(Colors::TEXT_SECTION),
            );
        });
        ui.add_space(8.0);

        // Minimap
        self.draw_minimap(ui, sidebar_width, pane_count);
    }

    fn draw_minimap(&self, ui: &mut egui::Ui, sidebar_width: f32, pane_count: usize) {
        let map_width = sidebar_width - 32.0;
        let map_height = 60.0;
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(sidebar_width, map_height), egui::Sense::hover());

        let map_rect = Rect::from_min_size(
            egui::pos2(rect.min.x + 16.0, rect.min.y),
            Vec2::new(map_width, map_height),
        );

        // Background
        ui.painter().rect(
            map_rect,
            R3,
            Colors::BG_DARKEST,
            Stroke::new(1.0, Colors::MINIMAP_BORDER),
            StrokeKind::Inside,
        );

        let gap = 3.0;
        let inner = map_rect.shrink(4.0);

        match pane_count {
            1 => {
                ui.painter().rect_filled(inner, R2, Colors::MINIMAP_PANE);
            }
            2 => {
                let half = (inner.width() - gap) / 2.0;
                let left = Rect::from_min_size(inner.min, Vec2::new(half, inner.height()));
                let right = Rect::from_min_size(
                    egui::pos2(inner.min.x + half + gap, inner.min.y),
                    Vec2::new(half, inner.height()),
                );
                ui.painter().rect_filled(left, R2, Colors::MINIMAP_PANE);
                ui.painter().rect_filled(right, R2, Colors::MINIMAP_PANE);
                ui.painter().rect_stroke(
                    left,
                    R2,
                    Stroke::new(1.0, Colors::ACCENT_DIM),
                    StrokeKind::Inside,
                );
            }
            3 => {
                let half_w = (inner.width() - gap) / 2.0;
                let half_h = (inner.height() - gap) / 2.0;
                let left =
                    Rect::from_min_size(inner.min, Vec2::new(half_w, inner.height()));
                let top_right = Rect::from_min_size(
                    egui::pos2(inner.min.x + half_w + gap, inner.min.y),
                    Vec2::new(half_w, half_h),
                );
                let bot_right = Rect::from_min_size(
                    egui::pos2(
                        inner.min.x + half_w + gap,
                        inner.min.y + half_h + gap,
                    ),
                    Vec2::new(half_w, half_h),
                );
                ui.painter().rect_filled(left, R2, Colors::MINIMAP_PANE);
                ui.painter()
                    .rect_filled(top_right, R2, Colors::MINIMAP_PANE);
                ui.painter()
                    .rect_filled(bot_right, R2, Colors::MINIMAP_PANE);
                ui.painter().rect_stroke(
                    left,
                    R2,
                    Stroke::new(1.0, Colors::ACCENT_DIM),
                    StrokeKind::Inside,
                );
            }
            _ => {
                ui.painter().rect_filled(inner, R2, Colors::MINIMAP_PANE);
            }
        }
    }

    fn draw_shortcuts_overlay(&self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("shortcuts_overlay"))
            .anchor(Align2::RIGHT_TOP, Vec2::new(-16.0, 44.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Colors::BG_SIDEBAR)
                    .stroke(Stroke::new(1.0, Colors::BORDER))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(240.0);
                        ui.label(
                            RichText::new("Keyboard Shortcuts")
                                .size(13.0)
                                .color(Colors::TEXT_PRIMARY)
                                .strong(),
                        );
                        ui.add_space(8.0);

                        let shortcuts = [
                            ("\u{2318}T", "New tab"),
                            ("\u{2318}]/[", "Next/prev tab"),
                            ("\u{2318}D", "Split right"),
                            ("\u{2318}\u{21E7}D", "Split down"),
                            ("\u{2318}W", "Close pane"),
                            ("\u{2318}B", "Toggle sidebar"),
                            ("\u{2318}H/J/K/L", "Focus pane"),
                            ("\u{2318}/", "This help"),
                            ("\u{2318}Q", "Quit"),
                        ];

                        for (key, desc) in shortcuts {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(key)
                                        .size(11.0)
                                        .color(Colors::ACCENT)
                                        .family(egui::FontFamily::Monospace),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(desc)
                                        .size(11.0)
                                        .color(Colors::TEXT_DIM),
                                );
                            });
                        }
                    });
            });
    }
}

fn replace_child(container: &mut Container, old: TileId, new: TileId) {
    match container {
        Container::Linear(linear) => {
            if let Some(pos) = linear.children.iter().position(|&c| c == old) {
                linear.children[pos] = new;
            }
        }
        Container::Tabs(tabs) => {
            if let Some(pos) = tabs.children.iter().position(|&c| c == old) {
                tabs.children[pos] = new;
            }
        }
        Container::Grid(_) => {
            container.remove_child(old);
            container.add_child(new);
        }
    }
}
