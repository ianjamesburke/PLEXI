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

use crate::protocol::AgentState;
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
    let pane_rect = ui.max_rect();
    let is_error = terminal.exited
        && terminal
            .exit_status
            .as_ref()
            .is_some_and(|s| s.is_failure());
    let pane_agent_state = terminal
        .agent
        .as_ref()
        .map(|a| a.state.clone())
        .or_else(|| terminal.observed_agent.as_ref().map(|a| a.state.clone()))
        .or_else(|| terminal.activity.clone())
        .or_else(|| {
            terminal
                .standby_session
                .as_ref()
                .map(|_| AgentState::NeedsYou)
        })
        .or_else(|| tab_activities.get(pane_id).cloned());
    let border_escalation_color =
        crate::ui::activity::escalation_color(pane_agent_state.as_ref(), is_error, colors);

    if terminal.exited {
        ui.painter().rect_filled(pane_rect, 0.0, colors.terminal_bg);
        let tab_action = if is_error {
            render_name_bar_and_tabs(
                ui,
                tile_id,
                pane_id,
                tab_info,
                tab_labels,
                tab_activities,
                pane_names,
                pane_agent_state.as_ref(),
                true,
                colors,
                false,
                pane_title_font_size,
            )
        } else {
            None
        };
        let body_rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(body_rect, 0.0, colors.terminal_bg);
        let exit_color = if is_error {
            colors.danger
        } else {
            colors.text_dim
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.colored_label(exit_color, "[process exited]");
            });
        });
        paint_escalation_border(ui.painter(), pane_rect, border_escalation_color);
        // Auto-close on any keypress. Keyboard events were taken out of `ctx`
        // before the render pass (stint 0387), so read them from the
        // host-supplied buffer rather than `ui.input()`.
        let close = is_focused
            && input
                .keyboard_events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }));
        return (close, tab_action);
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
        pane_agent_state.as_ref(),
        is_error,
        colors,
        outside_workspace,
        pane_title_font_size,
    );

    if terminal.is_standby() {
        let enter_pressed = is_focused
            && input.keyboard_events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        ..
                    }
                )
            });
        if enter_pressed {
            terminal.resume_from_standby();
        }
    }

    if let Some(banner) = terminal.standby_banner() {
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);
        let tail = terminal.tail_preview.clone();
        let response = ui.interact(
            rect,
            egui::Id::new(("plexi_standby_banner", *pane_id)),
            egui::Sense::click(),
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(16.0, 12.0))),
            |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(&banner)
                            .monospace()
                            .size(terminal.font_size)
                            .color(colors.accent),
                    );
                    if let Some(tail) = tail.as_deref().filter(|s| !s.trim().is_empty()) {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(tail)
                                .monospace()
                                .size((terminal.font_size - 1.0).max(9.0))
                                .color(colors.text_dim),
                        );
                    }
                });
            },
        );
        if response.clicked() {
            terminal.resume_from_standby();
        }
        paint_escalation_border(ui.painter(), pane_rect, border_escalation_color);
        return (false, tab_action);
    }

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

    paint_escalation_border(ui.painter(), pane_rect, border_escalation_color);

    (false, tab_action)
}

fn paint_escalation_border(
    painter: &egui::Painter,
    pane_rect: egui::Rect,
    escalation_color: Option<egui::Color32>,
) {
    if let Some(col) = escalation_color {
        let stroke_col = egui::Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 200);
        painter.rect_stroke(
            pane_rect,
            0.0,
            egui::Stroke::new(1.5_f32, stroke_col),
            egui::StrokeKind::Inside,
        );
    }
}

// Arg-struct refactor is a design change tracked in stint 0661.
#[allow(clippy::too_many_arguments)]
fn render_name_bar_and_tabs(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_id: &PaneId,
    tab_info: &HashMap<TileId, TabGroupInfo>,
    tab_labels: &HashMap<PaneId, String>,
    tab_activities: &HashMap<PaneId, AgentState>,
    pane_names: &HashMap<PaneId, String>,
    pane_agent_state: Option<&AgentState>,
    is_error: bool,
    colors: &Colors,
    outside_workspace: bool,
    name_font_size: f32,
) -> Option<(TileId, TabBarAction)> {
    let has_name = pane_names.contains_key(pane_id);
    let tab_group = tab_info.get(&tile_id);
    let mut tab_action = None;
    let agent_state = pane_agent_state.or_else(|| tab_activities.get(pane_id));
    let escalation_col = crate::ui::activity::escalation_color(agent_state, is_error, colors);
    let escalation_lbl = crate::ui::activity::escalation_label(agent_state, is_error);
    let has_escalation = is_error
        || matches!(
            agent_state,
            Some(
                AgentState::Working
                    | AgentState::Blocked
                    | AgentState::NeedsYou
                    | AgentState::Interrupted
            )
        );

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

        let mut right_edge = bar_rect.right();
        if outside_workspace {
            right_edge = paint_outside_workspace_badge(ui.painter(), bar_rect, right_edge, colors);
        }
        if let (Some(lbl), Some(col)) = (escalation_lbl, escalation_col) {
            paint_status_badge(ui.painter(), bar_rect, right_edge, lbl, col);
        }
    } else if has_name || outside_workspace || has_escalation {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), TAB_BAR_HEIGHT),
        );
        ui.advance_cursor_after_rect(bar_rect);
        ui.painter()
            .rect_filled(bar_rect, 0.0, colors.pane_header_bg());
        if let Some(esc_col) = escalation_col {
            let header_wash =
                egui::Color32::from_rgba_unmultiplied(esc_col.r(), esc_col.g(), esc_col.b(), 28);
            ui.painter().rect_filled(bar_rect, 0.0, header_wash);
            let divider_col =
                egui::Color32::from_rgba_unmultiplied(esc_col.r(), esc_col.g(), esc_col.b(), 96);
            ui.painter().line_segment(
                [bar_rect.left_bottom(), bar_rect.right_bottom()],
                egui::Stroke::new(1.0_f32, divider_col),
            );
        }

        let pip_space = if agent_state.is_some() || is_error {
            TAB_PIP_MARGIN + TAB_PIP_RADIUS * 2.0
        } else {
            0.0
        };

        if is_error {
            let cx = bar_rect.left() + TAB_PIP_MARGIN + TAB_PIP_RADIUS;
            ui.painter().circle_filled(
                egui::pos2(cx, bar_rect.center().y),
                TAB_PIP_RADIUS,
                colors.danger,
            );
        } else if let Some(state) = agent_state {
            paint_activity_dot(ui, bar_rect, state, colors);
        }

        let mut right_edge = bar_rect.right();
        if outside_workspace {
            right_edge = paint_outside_workspace_badge(ui.painter(), bar_rect, right_edge, colors);
        }
        if let (Some(lbl), Some(col)) = (escalation_lbl, escalation_col) {
            right_edge = paint_status_badge(ui.painter(), bar_rect, right_edge, lbl, col);
        }

        let title_text = if has_name {
            Some(pane_names[pane_id].as_str())
        } else if has_escalation {
            Some("Terminal")
        } else {
            None
        };
        if let Some(name) = title_text {
            let left_edge = bar_rect.left() + pip_space + 6.0;
            let max_right = (right_edge - 6.0).max(left_edge);
            if max_right > left_edge + 8.0 {
                let title_clip = egui::Rect::from_min_max(
                    egui::pos2(left_edge, bar_rect.top()),
                    egui::pos2(max_right, bar_rect.bottom()),
                );
                let clipped_painter = ui.painter().with_clip_rect(title_clip);
                let title_col = if matches!(
                    agent_state,
                    Some(AgentState::Blocked | AgentState::NeedsYou | AgentState::Interrupted)
                ) || is_error
                {
                    colors.text_primary
                } else {
                    colors.text_secondary(colors.bg_toolbar)
                };
                let has_right_badge = right_edge < bar_rect.right() - 1.0;
                if has_right_badge {
                    crate::ui::snap::text_snapped(
                        &clipped_painter,
                        egui::pos2(left_edge, bar_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(name_font_size),
                        title_col,
                    );
                } else {
                    let center_x = bar_rect.center().x + pip_space / 2.0;
                    crate::ui::snap::text_snapped(
                        &clipped_painter,
                        egui::pos2(center_x, bar_rect.center().y),
                        egui::Align2::CENTER_CENTER,
                        name,
                        egui::FontId::proportional(name_font_size),
                        title_col,
                    );
                }
            }
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

fn paint_status_badge(
    painter: &egui::Painter,
    bar_rect: egui::Rect,
    right_edge: f32,
    label: &str,
    fg: egui::Color32,
) -> f32 {
    let galley = painter.layout_no_wrap(label.to_string(), crate::ui::badge::badge_font(), fg);
    let size = crate::ui::badge::badge_size(galley.size());
    let badge_x = right_edge - size.x - crate::ui::style::SPACE_XS;
    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(badge_x, bar_rect.center().y - size.y / 2.0),
        size,
    );
    crate::ui::badge::paint_badge(
        painter,
        badge_rect,
        galley,
        egui::Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), 36),
        fg,
    );
    badge_x
}

fn paint_outside_workspace_badge(
    painter: &egui::Painter,
    bar_rect: egui::Rect,
    right_edge: f32,
    colors: &Colors,
) -> f32 {
    let label = "↗ outside workspace";
    let fg = colors.warning;
    let galley = painter.layout_no_wrap(label.to_string(), crate::ui::badge::badge_font(), fg);
    let size = crate::ui::badge::badge_size(galley.size());
    let badge_x = right_edge - size.x - crate::ui::style::SPACE_XS;
    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(badge_x, bar_rect.center().y - size.y / 2.0),
        size,
    );
    // A tint, not a fill: the tag annotates the tab bar rather than
    // competing with the pane title beside it.
    crate::ui::badge::paint_badge(
        painter,
        badge_rect,
        galley,
        egui::Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), 28),
        fg,
    );
    badge_x
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
            .is_some_and(|checked_at| {
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
