//! Terminal pane render path — extracted from `tiling::PlexiBehavior::pane_ui`.
//!
//! Covers three cases:
//! 1. Exited process — centered "[process exited]" label, auto-close on keypress.
//! 2. Live terminal — `TerminalView` with name bar / tab dots overlay.
//!
//! The outer `pane_ui` path already painted the pane background and shrunk
//! into the inner UI, so this renderer does not repaint the full pane
//! background — only the exit-message rect, which gets its own fill to cover
//! any stale terminal glyphs underneath.

use crate::pane::TerminalPane;
use crate::theme::{self, Colors};
use crate::tiling::{paint_tab_dots, PaneId, DOT_RADIUS, TAB_DOT_RESERVED_HEIGHT};
use egui::Vec2;
use egui_term::{TerminalTheme, TerminalView};
use egui_tiles::TileId;
use std::collections::HashMap;

/// Render one frame of a terminal pane. Returns `true` if the process has
/// exited and the user pressed a key (the caller should close the tile).
#[allow(clippy::too_many_arguments)]
pub fn render(
    ui: &mut egui::Ui,
    terminal: &mut TerminalPane,
    tile_id: TileId,
    pane_id: &PaneId,
    is_focused: bool,
    theme: &TerminalTheme,
    colors: &Colors,
    pane_names: &HashMap<PaneId, String>,
    tab_info: &HashMap<TileId, (usize, usize)>,
) -> bool {
    if terminal.exited {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.colored_label(colors.text_dim, "[process exited]");
            });
        });
        return is_focused
            && ui.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
            });
    }

    render_name_bar_and_dots(ui, tile_id, pane_id, tab_info, pane_names, colors);

    let font_size = terminal.font_size;
    let view = TerminalView::new(ui, &mut terminal.backend)
        .set_focus(is_focused)
        .set_theme(theme.clone())
        .set_font(theme::terminal_font(font_size))
        .set_size(Vec2::new(ui.available_width(), ui.available_height()));
    ui.add(view);

    // Draw tab indicator dots (top-left) when 2+ tabs and NO name bar.
    if !pane_names.contains_key(pane_id) {
        if let Some(&(active_idx, count)) = tab_info.get(&tile_id) {
            let rect = ui.max_rect();
            paint_tab_dots(
                ui.painter(),
                rect.left(),
                rect.top() + 2.0 + DOT_RADIUS,
                active_idx,
                count,
                colors.accent,
                colors.bg_active,
            );
        }
    }

    false
}

/// Render the pane name bar (if named) and reserve tab-dot space for a
/// terminal in full-pane mode.
fn render_name_bar_and_dots(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_id: &PaneId,
    tab_info: &HashMap<TileId, (usize, usize)>,
    pane_names: &HashMap<PaneId, String>,
    colors: &Colors,
) {
    let name_bar_height = 20.0;
    let has_name = pane_names.contains_key(pane_id);
    let has_tabs = tab_info.contains_key(&tile_id);

    if has_name {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), name_bar_height),
        );
        ui.advance_cursor_after_rect(bar_rect);

        let name = &pane_names[pane_id];

        if let Some(&(active_idx, count)) = tab_info.get(&tile_id) {
            paint_tab_dots(
                ui.painter(),
                bar_rect.left(),
                bar_rect.center().y,
                active_idx,
                count,
                colors.accent,
                colors.bg_active,
            );
        }

        ui.painter().text(
            bar_rect.center(),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(11.0),
            colors.text_dim,
        );
    } else if has_tabs {
        ui.add_space(TAB_DOT_RESERVED_HEIGHT);
    }
}
