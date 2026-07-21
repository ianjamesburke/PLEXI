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
use crate::spatial::tiling::{paint_tab_bar, PaneId, TabBarAction, TabGroupInfo, TAB_BAR_HEIGHT};
use crate::ui::theme::{self, Colors};
use egui::Vec2;
use egui_term::{TerminalTheme, TerminalView};
use egui_tiles::TileId;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Once;
use std::time::{Duration, Instant};

const TAB_PIP_RADIUS: f32 = 4.0;
const TAB_PIP_MARGIN: f32 = 7.0;

/// Host-supplied, frame-scoped keyboard input for one terminal pane
/// (stint 0387). Built in `PlexiApp::update` by taking the frame's
/// `PlexiInput` ownership buffer *before* the render pass and threaded down to
/// the focused terminal so egui's render-time widget machinery can't swallow a
/// key (e.g. Cmd+A) first. The focused pane receives the taken events;
/// unfocused panes receive an empty `keyboard_events` list (they only need
/// pointer/wheel, which stays on the ctx path).
#[derive(Default)]
pub struct TerminalInput {
    pub keyboard_events: Vec<egui::Event>,
    pub modifiers: egui::Modifiers,
}

const OUTSIDE_WORKSPACE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const TERMINAL_GLYPH_PADDING_X: f32 = 2.0;
static LOG_TERMINAL_GLYPH_PADDING: Once = Once::new();

/// Render one frame of a terminal pane. Returns `true` if the process has
/// exited and the user pressed a key (the caller should close the tile).
/// Returns (close_exited, tab_action).
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
    tab_activities: &HashMap<PaneId, AgentState>,
    workspace_root: Option<&Path>,
    pane_title_font_size: f32,
    input: TerminalInput,
) -> (bool, Option<(TileId, TabBarAction)>) {
    if terminal.exited {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.colored_label(colors.text_dim, "[process exited]");
            });
        });
        // Auto-close on any keypress. Keyboard events were taken out of `ctx`
        // before the render pass (stint 0387), so read them from the
        // host-supplied buffer rather than `ui.input()`.
        let close = is_focused
            && input
                .keyboard_events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }));
        return (close, None);
    }

    let outside_workspace = is_terminal_outside_workspace(terminal, workspace_root);
    let tab_action = render_name_bar_and_tabs(
        ui,
        tile_id,
        pane_id,
        tab_info,
        tab_labels,
        tab_activities,
        pane_names,
        colors,
        outside_workspace,
        pane_title_font_size,
    );

    let font_size = terminal.font_size;
    LOG_TERMINAL_GLYPH_PADDING.call_once(|| {
        log::info!(
            "terminal_renderer: glyph padding enabled x={}px",
            TERMINAL_GLYPH_PADDING_X
        );
    });
    // The terminal's deterministic widget id is this pane's default text
    // surface (stint 0429): the reconciler grants it egui focus while the
    // pane owns input, replacing the view's old per-frame request/surrender.
    crate::ui::focus::register_default_text_surface(
        ui.ctx(),
        crate::ui::focus::SurfaceKey::Pane(*pane_id),
        egui_term::terminal_widget_id(*pane_id),
    );
    let view = TerminalView::new(ui, &mut terminal.backend)
        .set_focus(is_focused)
        .set_theme(theme.clone())
        .set_font(theme::terminal_font(font_size))
        .set_padding(Vec2::new(TERMINAL_GLYPH_PADDING_X, 0.0))
        .set_size(Vec2::new(ui.available_width(), ui.available_height()))
        .with_input(input.keyboard_events, input.modifiers);
    ui.add(view);

    (false, tab_action)
}

fn render_name_bar_and_tabs(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_id: &PaneId,
    tab_info: &HashMap<TileId, TabGroupInfo>,
    tab_labels: &HashMap<PaneId, String>,
    tab_activities: &HashMap<PaneId, AgentState>,
    pane_names: &HashMap<PaneId, String>,
    colors: &Colors,
    outside_workspace: bool,
    name_font_size: f32,
) -> Option<(TileId, TabBarAction)> {
    let has_name = pane_names.contains_key(pane_id);
    let tab_group = tab_info.get(&tile_id);
    let mut tab_action = None;

    if let Some(group) = tab_group {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), TAB_BAR_HEIGHT),
        );
        ui.advance_cursor_after_rect(bar_rect);
        if let Some(action) = paint_tab_bar(
            ui.ctx(),
            ui.painter(),
            bar_rect,
            group,
            tab_labels,
            tab_activities,
            colors,
            name_font_size,
            false,
        ) {
            tab_action = Some((group.container_tile, action));
        }

        if outside_workspace {
            paint_outside_workspace_badge(ui.painter(), bar_rect);
        }
    } else if has_name || outside_workspace {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), TAB_BAR_HEIGHT),
        );
        ui.advance_cursor_after_rect(bar_rect);
        ui.painter()
            .rect_filled(bar_rect, 0.0, colors.pane_header_bg());

        let agent_state = tab_activities.get(pane_id);
        let pip_space = if agent_state.is_some() {
            TAB_PIP_MARGIN + TAB_PIP_RADIUS * 2.0
        } else {
            0.0
        };

        if let Some(state) = agent_state {
            paint_activity_dot(ui, bar_rect, state, colors);
        }

        if has_name {
            let name = &pane_names[pane_id];
            let center_x = bar_rect.center().x + pip_space / 2.0;
            ui.painter().text(
                egui::pos2(center_x, bar_rect.center().y),
                egui::Align2::CENTER_CENTER,
                name,
                egui::FontId::proportional(name_font_size),
                colors.text_dim,
            );
        }

        if outside_workspace {
            paint_outside_workspace_badge(ui.painter(), bar_rect);
        }
    }

    tab_action
}

fn paint_activity_dot(
    ui: &mut egui::Ui,
    bar_rect: egui::Rect,
    state: &AgentState,
    colors: &Colors,
) {
    let t = ui.input(|i| i.time);
    let color = crate::ui::activity::dot_color_from_time(state, colors, t, 0);
    let cx = bar_rect.left() + TAB_PIP_MARGIN + TAB_PIP_RADIUS;
    ui.painter()
        .circle_filled(egui::pos2(cx, bar_rect.center().y), TAB_PIP_RADIUS, color);
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
