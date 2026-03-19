use crate::keys::{self, Action, Direction};
use crate::pane::TerminalPane;
use crate::shell;
use crate::theme;
use crate::tiling::{PaneId, PlexiBehavior};
use egui_term::{BackendSettings, PtyEvent, TerminalFont, TerminalTheme};
use egui_tiles::{Container, SimplificationOptions, Tile, TileId, Tree};
use std::collections::HashMap;
use std::sync::mpsc;

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
}

impl PlexiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let (tx, rx) = mpsc::channel();
        let settings = Self::make_backend_settings();

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
        }
    }

    fn make_backend_settings() -> BackendSettings {
        BackendSettings {
            shell: shell::detect_shell(),
            args: vec!["-l".to_string()],
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
            log::warn!("split_focused: no focused pane");
            return;
        };
        log::info!(
            "split_focused: vertical={vertical}, focused={focused:?}, root={:?}, pane_count={}",
            self.tree.root,
            self.panes.len()
        );

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

        let settings = Self::make_backend_settings();
        let Some(pane) =
            TerminalPane::new(new_id, self.ctx.clone(), self.pty_event_tx.clone(), settings)
        else {
            log::error!("split_focused: failed to create new terminal pane");
            return;
        };
        self.panes.insert(new_id, pane);

        // Query parent BEFORE creating the container (otherwise parent_of finds the new container)
        let parent = self.tree.tiles.parent_of(focused);

        let new_tile = self.tree.tiles.insert_pane(new_id);

        // Create container holding [focused, new]
        let container_tile = if vertical {
            self.tree
                .tiles
                .insert_vertical_tile(vec![focused, new_tile])
        } else {
            self.tree
                .tiles
                .insert_horizontal_tile(vec![focused, new_tile])
        };
        log::info!(
            "split_focused: new_tile={new_tile:?}, container={container_tile:?}, parent={parent:?}"
        );

        // Replace focused with container in its parent (or set as root)
        if let Some(parent_id) = parent {
            if let Some(Tile::Container(parent)) = self.tree.tiles.get_mut(parent_id) {
                replace_child(parent, focused, container_tile);
            }
        } else {
            self.tree.root = Some(container_tile);
        }

        self.focused_pane = Some(new_tile);
        log::info!(
            "split_focused: done. root={:?}, pane_count={}",
            self.tree.root,
            self.panes.len()
        );
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
        self.tree
            .active_tiles()
            .into_iter()
            .find(|&id| id != excluding && matches!(self.tree.tiles.get(id), Some(Tile::Pane(_))))
    }

    fn navigate(&mut self, dir: Direction) {
        if let Some(target) = self.find_pane_in_direction(dir) {
            self.focused_pane = Some(target);
        }
    }

    fn find_pane_in_direction(&self, dir: Direction) -> Option<TileId> {
        let focused = self.focused_pane?;
        let focused_rect = self.tree.tiles.rect(focused)?;
        let center = focused_rect.center();

        let mut best: Option<(TileId, f32)> = None;

        for tile_id in self.tree.active_tiles() {
            if tile_id == focused {
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
}

impl eframe::App for PlexiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
            }
        }

        // Handle window close request (X button, Cmd+W on macOS)
        if ctx.input(|i| i.viewport().close_requested()) && self.panes.len() > 1 {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_focused();
        }

        // All panes exited
        if self.panes.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Render tiled layout
        let panel_bg = egui::Color32::from_rgb(0x11, 0x11, 0x1b); // Catppuccin Mocha crust
        egui::CentralPanel::default()
            .frame(egui::Frame {
                fill: panel_bg,
                ..Default::default()
            })
            .show(ctx, |ui| {
                let mut behavior = PlexiBehavior {
                    panes: &mut self.panes,
                    focused_tile: self.focused_pane,
                    theme: self.theme.clone(),
                    font: self.font.clone(),
                    new_focused: None,
                };
                self.tree.ui(&mut behavior, ui);

                if let Some(new) = behavior.new_focused {
                    self.focused_pane = Some(new);
                }
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
