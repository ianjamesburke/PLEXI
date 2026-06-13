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

use crate::app_protocol::AgentState;
use crate::host::pane::TerminalPane;
use crate::spatial::tiling::{paint_tab_bar, PaneId, TabGroupInfo, TAB_BAR_HEIGHT};
use crate::ui::theme::{self, Colors};
use egui::Vec2;
use egui_term::{TerminalTheme, TerminalView};
use egui_tiles::TileId;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Once;
use std::time::{Duration, Instant};

const OUTSIDE_WORKSPACE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const TERMINAL_GLYPH_PADDING_X: f32 = 2.0;
static LOG_TERMINAL_GLYPH_PADDING: Once = Once::new();

/// Render one frame of a terminal pane. Returns `true` if the process has
/// exited and the user pressed a key (the caller should close the tile).
/// Returns (close_exited, tab_click).
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
    tab_info: &HashMap<TileId, TabGroupInfo>,
    tab_labels: &HashMap<PaneId, String>,
    workspace_root: Option<&Path>,
    pane_title_font_size: f32,
) -> (bool, Option<(TileId, usize)>) {
    if terminal.exited {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.colored_label(colors.text_dim, "[process exited]");
            });
        });
        let close = is_focused
            && ui.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
            });
        return (close, None);
    }

    let outside_workspace = is_terminal_outside_workspace(terminal, workspace_root);
    let tab_click = render_name_bar_and_tabs(
        ui,
        tile_id,
        pane_id,
        tab_info,
        tab_labels,
        pane_names,
        colors,
        outside_workspace,
        pane_title_font_size,
        terminal
            .agent
            .as_ref()
            .map(|a| &a.state)
            .or(terminal.activity.as_ref()),
    );

    let font_size = terminal.font_size;
    LOG_TERMINAL_GLYPH_PADDING.call_once(|| {
        log::info!(
            "terminal_renderer: glyph padding enabled x={}px",
            TERMINAL_GLYPH_PADDING_X
        );
    });
    let view = TerminalView::new(ui, &mut terminal.backend)
        .set_focus(is_focused)
        .set_theme(theme.clone())
        .set_font(theme::terminal_font(font_size))
        .set_padding(Vec2::new(TERMINAL_GLYPH_PADDING_X, 0.0))
        .set_size(Vec2::new(ui.available_width(), ui.available_height()));
    ui.add(view);

    (false, tab_click)
}

fn render_name_bar_and_tabs(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_id: &PaneId,
    tab_info: &HashMap<TileId, TabGroupInfo>,
    tab_labels: &HashMap<PaneId, String>,
    pane_names: &HashMap<PaneId, String>,
    colors: &Colors,
    outside_workspace: bool,
    name_font_size: f32,
    agent_state: Option<&AgentState>,
) -> Option<(TileId, usize)> {
    let has_name = pane_names.contains_key(pane_id);
    let tab_group = tab_info.get(&tile_id);
    let mut tab_click = None;

    if let Some(group) = tab_group {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), TAB_BAR_HEIGHT),
        );
        ui.advance_cursor_after_rect(bar_rect);
        if let Some(idx) = paint_tab_bar(ui.ctx(), ui.painter(), bar_rect, group, tab_labels, colors, name_font_size) {
            tab_click = Some((group.container_tile, idx));
        }

        if outside_workspace {
            paint_outside_workspace_badge(ui.painter(), bar_rect);
        }

        if let Some(state) = agent_state {
            paint_activity_dot(ui, bar_rect, state, colors);
        }
    } else if has_name || outside_workspace {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), TAB_BAR_HEIGHT),
        );
        ui.advance_cursor_after_rect(bar_rect);
        ui.painter().rect_filled(bar_rect, 0.0, colors.pane_header_bg());

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

        if let Some(state) = agent_state {
            paint_activity_dot(ui, bar_rect, state, colors);
        }

        if outside_workspace {
            paint_outside_workspace_badge(ui.painter(), bar_rect);
        }
    }

    tab_click
}

fn paint_activity_dot(
    ui: &mut egui::Ui,
    bar_rect: egui::Rect,
    state: &AgentState,
    colors: &Colors,
) {
    const ACTIVITY_DOT_RADIUS: f32 = 3.0;
    const ACTIVITY_DOT_MARGIN: f32 = 6.0;
    let t = ui.input(|i| i.time);
    let color = crate::ui::activity::dot_color_from_time(state, colors, t, 0);
    let cx = bar_rect.left() + ACTIVITY_DOT_MARGIN + ACTIVITY_DOT_RADIUS;
    ui.painter().circle_filled(
        egui::pos2(cx, bar_rect.center().y),
        ACTIVITY_DOT_RADIUS,
        color,
    );
    if matches!(state, AgentState::Working) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}

fn paint_outside_workspace_badge(painter: &egui::Painter, bar_rect: egui::Rect) {
    let label = "↗ outside workspace";
    let amber = egui::Color32::from_rgb(0xff, 0xb8, 0x6b);
    let font = egui::FontId::proportional(10.0);
    let galley = painter.layout_no_wrap(label.to_string(), font.clone(), amber);
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
    painter.rect_filled(
        badge_rect,
        egui::CornerRadius::same(3),
        egui::Color32::from_rgba_unmultiplied(0xff, 0xb8, 0x6b, 28),
    );
    painter.text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        amber,
    );
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
            .map_or(false, |checked_at| {
                checked_at.elapsed() < OUTSIDE_WORKSPACE_CHECK_INTERVAL
            })
    {
        return terminal.outside_workspace_cached;
    }

    let pid = terminal.backend.child_pid();
    let outside_workspace = if let Some(cwd) = crate::host::shell::get_pid_cwd(pid) {
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
