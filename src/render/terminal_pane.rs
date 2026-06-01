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
use std::path::Path;
use std::time::{Duration, Instant};

const OUTSIDE_WORKSPACE_CHECK_INTERVAL: Duration = Duration::from_secs(1);

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
    workspace_root: Option<&Path>,
    pane_title_font_size: f32,
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

    let outside_workspace = is_terminal_outside_workspace(terminal, workspace_root);
    render_name_bar_and_dots(
        ui,
        tile_id,
        pane_id,
        tab_info,
        pane_names,
        colors,
        outside_workspace,
        pane_title_font_size,
    );

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
/// terminal in full-pane mode. When `outside_workspace` is true, paint a
/// small right-aligned "↗ outside workspace" badge so the user can see the
/// scope drift without it being intrusive.
fn render_name_bar_and_dots(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_id: &PaneId,
    tab_info: &HashMap<TileId, (usize, usize)>,
    pane_names: &HashMap<PaneId, String>,
    colors: &Colors,
    outside_workspace: bool,
    name_font_size: f32,
) {
    let name_bar_height = 20.0;
    let has_name = pane_names.contains_key(pane_id);
    let has_tabs = tab_info.contains_key(&tile_id);

    // The full name-bar strip is allocated when there's a name to print or
    // an outside-workspace badge to draw. Without one, the slim tab-dot
    // reservation (or no reservation at all) is enough.
    let needs_full_bar = has_name || outside_workspace;

    if needs_full_bar {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), name_bar_height),
        );
        ui.advance_cursor_after_rect(bar_rect);

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

        if has_name {
            let name = &pane_names[pane_id];
            ui.painter().text(
                bar_rect.center(),
                egui::Align2::CENTER_CENTER,
                name,
                egui::FontId::proportional(name_font_size),
                colors.text_dim,
            );
        }

        if outside_workspace {
            // Right-aligned amber badge. Mirrors the lifecycle-pill pattern
            // — minimal, glanceable, never intrusive.
            let label = "↗ outside workspace";
            let amber = egui::Color32::from_rgb(0xff, 0xb8, 0x6b);
            let font = egui::FontId::proportional(10.0);
            let galley =
                ui.painter().layout_no_wrap(label.to_string(), font.clone(), amber);
            let pad_x = 6.0;
            let badge_w = galley.size().x + pad_x * 2.0;
            let badge_h = 14.0;
            let badge_rect = egui::Rect::from_min_size(
                egui::pos2(
                    bar_rect.right() - badge_w - 4.0,
                    bar_rect.center().y - badge_h / 2.0,
                ),
                egui::vec2(badge_w, badge_h),
            );
            ui.painter().rect_filled(
                badge_rect,
                egui::CornerRadius::same(3),
                egui::Color32::from_rgba_unmultiplied(0xff, 0xb8, 0x6b, 28),
            );
            ui.painter().text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                font,
                amber,
            );
        }
    } else if has_tabs {
        ui.add_space(TAB_DOT_RESERVED_HEIGHT);
    }
}

/// True when the terminal's child PID has a CWD that is not an ancestor of
/// the workspace root. Returns false when there is no workspace root, when
/// the CWD can't be probed, or when the terminal has exited.
fn is_terminal_outside_workspace(
    terminal: &mut TerminalPane,
    workspace_root: Option<&Path>,
) -> bool {
    let Some(root) = workspace_root else {
        terminal.outside_workspace_cached = false;
        terminal.outside_workspace_checked_at = None;
        terminal.outside_workspace_root = None;
        return false;
    };
    if terminal.exited {
        return false;
    }
    if terminal.outside_workspace_root.as_deref() == Some(root)
        && terminal
            .outside_workspace_checked_at
            .map_or(false, |checked_at| checked_at.elapsed() < OUTSIDE_WORKSPACE_CHECK_INTERVAL)
    {
        return terminal.outside_workspace_cached;
    }

    let pid = terminal.backend.child_pid();
    let outside_workspace = if let Some(cwd) = crate::shell::get_pid_cwd(pid) {
        // Canonicalize both sides so /var → /private/var on macOS doesn't
        // produce a false-positive "outside" badge.
        let cwd_canon = cwd.canonicalize().unwrap_or(cwd);
        let root_canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        !cwd_canon.starts_with(&root_canon)
    } else {
        false
    };

    terminal.outside_workspace_cached = outside_workspace;
    terminal.outside_workspace_checked_at = Some(Instant::now());
    terminal.outside_workspace_root = Some(root.to_path_buf());
    outside_workspace
}
