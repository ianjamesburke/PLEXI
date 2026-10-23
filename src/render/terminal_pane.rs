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
use crate::spatial::tiling::{
    paint_tab_bar, PaneId, TabBarAction, TabBarPaintParams, TabGroupInfo, TAB_BAR_HEIGHT,
};
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

fn standby_view_rects(
    content_rect: egui::Rect,
    banner_height: f32,
    hint_height: f32,
) -> (egui::Rect, egui::Rect, egui::Rect) {
    let hint_height = hint_height.min(content_rect.height());
    let available_banner_height = (content_rect.height() - hint_height).max(0.0);
    let banner_height = banner_height.min(available_banner_height);
    let banner_rect = egui::Rect::from_min_size(
        content_rect.min,
        Vec2::new(content_rect.width(), banner_height),
    );
    let tail_rect = egui::Rect::from_min_max(
        egui::pos2(content_rect.min.x, banner_rect.max.y),
        egui::pos2(content_rect.max.x, content_rect.max.y - hint_height),
    );
    let hint_rect = egui::Rect::from_min_size(
        egui::pos2(content_rect.min.x, content_rect.max.y - hint_height),
        Vec2::new(content_rect.width(), hint_height),
    );
    (banner_rect, tail_rect, hint_rect)
}

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
    let needs_you = matches!(pane_agent_state, Some(AgentState::NeedsYou));
    let border_escalation_color =
        crate::ui::activity::escalation_color(pane_agent_state.as_ref(), is_error, colors);
    let spawned_by_badge = terminal.spawned_by_badge();
    let waiting_results_badge = (!terminal.pending_child_results.is_empty()).then(|| {
        let count = terminal.pending_child_results.len();
        if count == 1 {
            "1 result waiting".to_string()
        } else {
            format!("{count} results waiting")
        }
    });

    if terminal.exited {
        ui.painter().rect_filled(pane_rect, 0.0, colors.terminal_bg);
        let tab_action = if is_error {
            render_name_bar_and_tabs(
                ui,
                NameBarParams {
                    tile_id,
                    pane_id,
                    tab_info,
                    tab_labels,
                    tab_activities,
                    pane_names,
                    pane_agent_state: pane_agent_state.as_ref(),
                    is_error: true,
                    colors,
                    outside_workspace: false,
                    name_font_size: pane_title_font_size,
                    spawned_by_badge: spawned_by_badge.as_deref(),
                    waiting_results_badge: waiting_results_badge.as_deref(),
                },
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
        if !needs_you { paint_escalation_border(ui.painter(), pane_rect, border_escalation_color); }
    crate::ui::activity::paint_attention_chrome(ui.painter(), pane_rect, colors.state_needs_you, egui::Id::new(("terminal_attention", pane_id)), needs_you);
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
        NameBarParams {
            tile_id,
            pane_id,
            tab_info,
            tab_labels,
            tab_activities,
            pane_names,
            pane_agent_state: pane_agent_state.as_ref(),
            is_error,
            colors,
            outside_workspace,
            name_font_size: pane_title_font_size,
            spawned_by_badge: spawned_by_badge.as_deref(),
            waiting_results_badge: waiting_results_badge.as_deref(),
        },
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
        let tail = terminal
            .standby_session
            .as_ref()
            .and_then(|session| session.standby_scrollback_tail.clone())
            .unwrap_or_default()
            .join("\n");
        let (banner, hint) = banner.rsplit_once('\n').unwrap_or((&banner, "[Enter] resume"));
        let content_rect = rect.shrink2(Vec2::new(16.0, 12.0));
        let font = egui::FontId::monospace(terminal.font_size);
        let banner_height = ui.fonts_mut(|fonts| {
            fonts
                .layout(banner.to_owned(), font.clone(), colors.accent, content_rect.width())
                .size()
                .y
        });
        let hint_height = ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(hint.to_owned(), font.clone(), colors.accent)
                .size()
                .y
        });
        let (banner_rect, tail_rect, hint_rect) =
            standby_view_rects(content_rect, banner_height, hint_height);
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(banner_rect),
            |ui| {
                ui.set_clip_rect(banner_rect);
                ui.label(
                    egui::RichText::new(banner)
                        .monospace()
                        .size(terminal.font_size)
                        .color(colors.accent),
                );
            },
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(hint_rect), |ui| {
            ui.set_clip_rect(hint_rect);
            ui.label(
                egui::RichText::new(hint)
                    .monospace()
                    .size(terminal.font_size)
                    .color(colors.accent),
            );
        });
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(tail_rect),
            |ui| {
                ui.set_clip_rect(tail_rect);
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    if !tail.is_empty() {
                        ui.label(
                            egui::RichText::new(&tail)
                                .monospace()
                                .size((terminal.font_size - 1.0).max(9.0))
                                .color(colors.text_dim),
                        );
                    }
                });
            },
        );
        if !needs_you { paint_escalation_border(ui.painter(), pane_rect, border_escalation_color); }
    crate::ui::activity::paint_attention_chrome(ui.painter(), pane_rect, colors.state_needs_you, egui::Id::new(("terminal_attention", pane_id)), needs_you);
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

    if !needs_you { paint_escalation_border(ui.painter(), pane_rect, border_escalation_color); }
    crate::ui::activity::paint_attention_chrome(ui.painter(), pane_rect, colors.state_needs_you, egui::Id::new(("terminal_attention", pane_id)), needs_you);

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

/// Recorded geometry from the most recent `render_name_bar_and_tabs` pass for a
/// terminal pane, used by UI tests to assert that the elided tab/name label and
/// the outside-workspace badge never intersect.
#[derive(Clone, Debug, Default)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TerminalTabBarLayout {
    pub label_rect: Option<egui::Rect>,
    pub elided_label: Option<String>,
    pub outside_workspace_badge_rect: Option<egui::Rect>,
}

#[cfg(test)]
pub(crate) fn last_rendered_tab_bar_layout(
    ctx: &egui::Context,
    pane_id: PaneId,
) -> Option<TerminalTabBarLayout> {
    ctx.data(|d| d.get_temp::<TerminalTabBarLayout>(egui::Id::new(("terminal_tab_bar_layout", pane_id))))
}

static LOG_OUTSIDE_WORKSPACE_BADGE_RESERVE: Once = Once::new();
static LOG_WORKING_NO_ESCALATION: Once = Once::new();

struct NameBarParams<'a> {
    tile_id: TileId,
    pane_id: &'a PaneId,
    tab_info: &'a HashMap<TileId, TabGroupInfo>,
    tab_labels: &'a HashMap<PaneId, String>,
    tab_activities: &'a HashMap<PaneId, AgentState>,
    pane_names: &'a HashMap<PaneId, String>,
    pane_agent_state: Option<&'a AgentState>,
    is_error: bool,
    colors: &'a Colors,
    outside_workspace: bool,
    name_font_size: f32,
    spawned_by_badge: Option<&'a str>,
    waiting_results_badge: Option<&'a str>,
}

fn render_name_bar_and_tabs(
    ui: &mut egui::Ui,
    params: NameBarParams<'_>,
) -> Option<(TileId, TabBarAction)> {
    let NameBarParams {
        tile_id,
        pane_id,
        tab_info,
        tab_labels,
        tab_activities,
        pane_names,
        pane_agent_state,
        is_error,
        colors,
        outside_workspace,
        name_font_size,
        spawned_by_badge,
        waiting_results_badge,
    } = params;
    let has_name = pane_names.contains_key(pane_id);
    let tab_group = tab_info.get(&tile_id);
    let mut tab_action = None;
    let agent_state = pane_agent_state.or_else(|| tab_activities.get(pane_id));
    let escalation_col = crate::ui::activity::escalation_color(agent_state, is_error, colors)
        .filter(|_| !matches!(agent_state, Some(AgentState::NeedsYou)) || crate::ui::activity::attention(ui.ctx()).alpha > 0.0);
    let escalation_lbl = crate::ui::activity::escalation_label(agent_state, is_error);
    let has_escalation = is_error
        || matches!(
            agent_state,
            Some(AgentState::Blocked | AgentState::NeedsYou | AgentState::Interrupted | AgentState::Failed | AgentState::ExitedUnreported)
        );
    if matches!(agent_state, Some(AgentState::Working)) && !is_error {
        LOG_WORKING_NO_ESCALATION.call_once(|| {
            log::info!(
                "terminal_pane: Working state skips escalation chrome (has_name={has_name}, tabbed={})",
                tab_group.is_some()
            );
        });
    }

    if let Some(group) = tab_group {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), TAB_BAR_HEIGHT),
        );
        ui.advance_cursor_after_rect(bar_rect);
        ui.painter()
            .rect_filled(bar_rect, 0.0, colors.pane_header_bg());

        // Measure and reserve right-aligned badge space before laying out and
        // eliding the tabs so the active tab label and highlight never run
        // under the outside-workspace or status badges (stint 0755).
        let mut right_edge = bar_rect.right();
        let outside_badge_rect = if outside_workspace {
            let rect = outside_workspace_badge_rect(ui.painter(), bar_rect, right_edge);
            right_edge = rect.left();
            LOG_OUTSIDE_WORKSPACE_BADGE_RESERVE.call_once(|| {
                log::info!(
                    "terminal_pane: reserved outside-workspace badge width={:.1}px (plus SPACE_XS={:.1}px) in tab bar",
                    rect.width(),
                    crate::ui::style::SPACE_XS,
                );
            });
            Some(rect)
        } else {
            None
        };
        let status_badge_x = if let (Some(lbl), Some(col)) = (escalation_lbl, escalation_col) {
            let galley =
                ui.painter()
                    .layout_no_wrap(lbl.to_string(), crate::ui::badge::badge_font(), col);
            let size = crate::ui::badge::badge_size(galley.size());
            let x = right_edge - size.x - crate::ui::style::SPACE_XS;
            right_edge = x;
            Some((lbl, col))
        } else {
            None
        };
        let waiting_badge_x = if let Some(lbl) = waiting_results_badge {
            let col = colors.accent;
            let galley =
                ui.painter()
                    .layout_no_wrap(lbl.to_string(), crate::ui::badge::badge_font(), col);
            let size = crate::ui::badge::badge_size(galley.size());
            let x = right_edge - size.x - crate::ui::style::SPACE_XS;
            right_edge = x;
            Some((lbl, col))
        } else {
            None
        };
        let spawned_badge_x = if let Some(lbl) = spawned_by_badge {
            let col = colors.text_secondary(colors.bg_toolbar);
            let galley =
                ui.painter()
                    .layout_no_wrap(lbl.to_string(), crate::ui::badge::badge_font(), col);
            let size = crate::ui::badge::badge_size(galley.size());
            let x = right_edge - size.x - crate::ui::style::SPACE_XS;
            right_edge = x;
            Some((lbl, col))
        } else {
            None
        };

        let tabs_right = if right_edge < bar_rect.right() {
            (right_edge - crate::ui::style::SPACE_XS).max(bar_rect.left())
        } else {
            bar_rect.right()
        };
        let tabs_bar_rect = egui::Rect::from_min_max(
            bar_rect.min,
            egui::pos2(tabs_right, bar_rect.bottom()),
        );

        let tab_count = group.members.len().max(1);
        let tab_width = tabs_bar_rect.width() / tab_count as f32;
        let font = egui::FontId::proportional(name_font_size);
        let mut elided_tab_labels = HashMap::with_capacity(group.members.len());
        let mut active_label_rect = None;
        let mut active_elided_label = None;

        for (idx, &member_id) in group.members.iter().enumerate() {
            let tab_rect = egui::Rect::from_min_size(
                egui::pos2(tabs_bar_rect.left() + idx as f32 * tab_width, tabs_bar_rect.top()),
                egui::vec2(tab_width, TAB_BAR_HEIGHT),
            );
            let pip_space = if tab_activities.contains_key(&member_id) {
                TAB_PIP_MARGIN + TAB_PIP_RADIUS * 2.0
            } else {
                0.0
            };
            let content_left = tab_rect.left() + pip_space;
            let max_text_width = (tab_rect.right() - content_left - 8.0).max(0.0);
            let raw_label = tab_labels
                .get(&member_id)
                .map(String::as_str)
                .or_else(|| pane_names.get(&member_id).map(String::as_str))
                .unwrap_or("Tab");
            let galley = crate::ui::text::elided_galley(
                ui,
                raw_label,
                font.clone(),
                max_text_width,
                crate::ui::text::Side::Trailing,
            );
            let elided_str = galley.job.text.clone();
            if idx == group.active_idx {
                let text_pos = egui::pos2(
                    content_left + (tab_rect.right() - content_left) / 2.0 - galley.size().x / 2.0,
                    tab_rect.center().y - galley.size().y / 2.0,
                );
                active_label_rect = Some(egui::Rect::from_min_size(text_pos, galley.size()));
                active_elided_label = Some(elided_str.clone());
            }
            elided_tab_labels.insert(member_id, elided_str);
        }

        if let Some(action) = paint_tab_bar(
            ui.ctx(),
            ui.painter(),
            TabBarPaintParams {
                bar_rect: tabs_bar_rect,
                group,
                tab_labels: &elided_tab_labels,
                tab_activities,
                colors,
                font_size: name_font_size,
                overtake_hint: false,
            },
        ) {
            tab_action = Some((group.container_tile, action));
        }

        let mut paint_right = bar_rect.right();
        if outside_workspace {
            let rect = paint_outside_workspace_badge(ui.painter(), bar_rect, paint_right, colors);
            paint_right = rect.left();
        }
        if let Some((lbl, col)) = status_badge_x {
            paint_right = paint_status_badge(ui.painter(), bar_rect, paint_right, lbl, col);
        }
        if let Some((lbl, col)) = waiting_badge_x {
            paint_right = paint_status_badge(ui.painter(), bar_rect, paint_right, lbl, col);
        }
        if let Some((lbl, col)) = spawned_badge_x {
            paint_status_badge(ui.painter(), bar_rect, paint_right, lbl, col);
        }

        ui.ctx().data_mut(|d| {
            d.insert_temp(
                egui::Id::new(("terminal_tab_bar_layout", *pane_id)),
                TerminalTabBarLayout {
                    label_rect: active_label_rect,
                    elided_label: active_elided_label,
                    outside_workspace_badge_rect: outside_badge_rect,
                },
            );
        });
    } else if has_name
        || outside_workspace
        || has_escalation
        || spawned_by_badge.is_some_and(|b| b != crate::host::pane::SPAWNED_BY_HUMAN_BADGE)
        || waiting_results_badge.is_some()
    {
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
            crate::ui::activity::paint_pip(ui.painter(), egui::pos2(cx, bar_rect.center().y), TAB_PIP_RADIUS, Some(&AgentState::Failed), colors.state_error, false);
        } else if let Some(state) = agent_state {
            paint_activity_dot(ui, bar_rect, state, colors);
        }

        let mut right_edge = bar_rect.right();
        let outside_badge_rect = if outside_workspace {
            let rect = paint_outside_workspace_badge(ui.painter(), bar_rect, right_edge, colors);
            right_edge = rect.left();
            LOG_OUTSIDE_WORKSPACE_BADGE_RESERVE.call_once(|| {
                log::info!(
                    "terminal_pane: reserved outside-workspace badge width={:.1}px (plus SPACE_XS={:.1}px) in tab bar",
                    rect.width(),
                    crate::ui::style::SPACE_XS,
                );
            });
            Some(rect)
        } else {
            None
        };
        if let (Some(lbl), Some(col)) = (escalation_lbl, escalation_col) {
            right_edge = paint_status_badge(ui.painter(), bar_rect, right_edge, lbl, col);
        }
        if let Some(lbl) = waiting_results_badge {
            right_edge = paint_status_badge(ui.painter(), bar_rect, right_edge, lbl, colors.accent);
        }
        if let Some(lbl) = spawned_by_badge {
            right_edge = paint_status_badge(
                ui.painter(),
                bar_rect,
                right_edge,
                lbl,
                colors.text_secondary(colors.bg_toolbar),
            );
        }

        let title_text = if has_name {
            Some(pane_names[pane_id].as_str())
        } else if let Some(lbl) = tab_labels
            .get(pane_id)
            .map(String::as_str)
            .filter(|s| *s != "Terminal")
        {
            Some(lbl)
        } else if has_escalation
            || spawned_by_badge.is_some_and(|b| b != crate::host::pane::SPAWNED_BY_HUMAN_BADGE)
            || waiting_results_badge.is_some()
        {
            Some("Terminal")
        } else {
            None
        };
        let mut active_label_rect = None;
        let mut active_elided_label = None;
        if let Some(name) = title_text {
            let left_edge = bar_rect.left() + pip_space + crate::ui::style::SPACE_SM;
            let max_right = (right_edge - crate::ui::style::SPACE_XS).max(left_edge);
            let avail_width = (max_right - left_edge).max(0.0);
            let galley = crate::ui::text::elided_galley(
                ui,
                name,
                egui::FontId::proportional(name_font_size),
                avail_width,
                crate::ui::text::Side::Trailing,
            );
            if !galley.is_empty() {
                let title_col = if matches!(
                    agent_state,
                    Some(AgentState::Blocked | AgentState::NeedsYou | AgentState::Interrupted | AgentState::Failed | AgentState::ExitedUnreported)
                ) || is_error
                {
                    colors.text_primary
                } else {
                    colors.text_secondary(colors.bg_toolbar)
                };
                let text_pos = egui::pos2(
                    left_edge + (avail_width - galley.size().x).max(0.0) / 2.0,
                    bar_rect.center().y - galley.size().y / 2.0,
                );
                active_label_rect = Some(egui::Rect::from_min_size(text_pos, galley.size()));
                active_elided_label = Some(galley.job.text.clone());
                crate::ui::snap::galley_snapped(ui.painter(), text_pos, galley, title_col);
            }
        }

        ui.ctx().data_mut(|d| {
            d.insert_temp(
                egui::Id::new(("terminal_tab_bar_layout", *pane_id)),
                TerminalTabBarLayout {
                    label_rect: active_label_rect,
                    elided_label: active_elided_label,
                    outside_workspace_badge_rect: outside_badge_rect,
                },
            );
        });
    } else {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<TerminalTabBarLayout>(egui::Id::new((
                "terminal_tab_bar_layout",
                *pane_id,
            )));
        });
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
    crate::ui::activity::paint_pip(ui.painter(), egui::pos2(cx, bar_rect.center().y), TAB_PIP_RADIUS, Some(state), color, false);
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

const OUTSIDE_WORKSPACE_BADGE_LABEL: &str = "↗ outside workspace";

fn outside_workspace_badge_rect(
    painter: &egui::Painter,
    bar_rect: egui::Rect,
    right_edge: f32,
) -> egui::Rect {
    let galley = painter.layout_no_wrap(
        OUTSIDE_WORKSPACE_BADGE_LABEL.to_string(),
        crate::ui::badge::badge_font(),
        egui::Color32::PLACEHOLDER,
    );
    let size = crate::ui::badge::badge_size(galley.size());
    let badge_x = right_edge - size.x - crate::ui::style::SPACE_XS;
    egui::Rect::from_min_size(
        egui::pos2(badge_x, bar_rect.center().y - size.y / 2.0),
        size,
    )
}

fn paint_outside_workspace_badge(
    painter: &egui::Painter,
    bar_rect: egui::Rect,
    right_edge: f32,
    colors: &Colors,
) -> egui::Rect {
    let fg = colors.warning;
    let galley = painter.layout_no_wrap(
        OUTSIDE_WORKSPACE_BADGE_LABEL.to_string(),
        crate::ui::badge::badge_font(),
        fg,
    );
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
    badge_rect
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_wrapped_standby_banner_keeps_enter_hint_visible() {
        let content = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(320.0, 88.0));
        // A realistic long pane name + cwd wraps to four terminal lines at this width.
        let (banner, tail, hint) = standby_view_rects(content, 56.0, 14.0);

        assert_eq!(banner.min, content.min);
        assert_eq!(banner.height(), 56.0);
        assert_eq!(tail.min.y, banner.max.y);
        assert_eq!(tail.max.y, hint.min.y);
        assert_eq!(hint.max, content.max, "Enter hint has its own reserved line");
    }
}
