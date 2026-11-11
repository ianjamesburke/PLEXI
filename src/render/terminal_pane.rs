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
/// Header pip radius: with its 0.7pt stroke extent the ink nearly fills the glyph slot, like the icon.
const HEADER_PIP_RADIUS: f32 = 6.0;
/// Left pad of a tab member's pip; mirrors `TAB_PIP_LEFT_PAD` in `paint_tab_bar`.
const TAB_PIP_MARGIN: f32 = 7.0;

/// The header pip's center: its glyph slot starts at the shared content inset, like the icon's.
fn header_pip_center_x(bar: egui::Rect) -> f32 {
    bar.left() + crate::ui::style::PANE_CONTENT_INSET + crate::ui::style::PANE_HEADER_GLYPH / 2.0
}

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

/// Single source for the terminal grid padding: one inset token on both axes. Normal and
/// zoomed views both use it, so the PTY grid and the drawn grid agree.
pub(crate) fn terminal_content_padding() -> Vec2 {
    Vec2::splat(crate::ui::style::PANE_CONTENT_INSET)
}
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
    tab_activities: &HashMap<PaneId, (AgentState, bool)>,
    workspace_root: Option<&Path>,
    pane_title_font_size: f32,
    input: TerminalInput,
    resize_immediately: bool,
) -> (bool, Option<(TileId, TabBarAction)>) {
    let pane_rect = ui.max_rect();
    let is_error = terminal.exited
        && terminal
            .exit_status
            .as_ref()
            .is_some_and(|s| s.is_failure());
    let pane_agent_state = (terminal.agent.is_some()
        || terminal.reported_7501.is_some()
        || terminal.observed_agent.is_some())
        .then(|| terminal.effective_agent_state().0)
        .or_else(|| {
            terminal
                .standby_session
                .as_ref()
                .map(|_| AgentState::NeedsYou)
        })
        .or_else(|| tab_activities.get(pane_id).map(|(state, _)| state.clone()));
    let needs_you = matches!(pane_agent_state, Some(AgentState::NeedsYou));
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
                    overtake_hint: false,
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
        crate::ui::activity::paint_attention_effect(ui.painter(), pane_rect, colors.state_needs_you, egui::Id::new(("terminal_attention", pane_id)), needs_you, false);
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
            overtake_hint: false,
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
                ui.set_clip_rect(banner_rect.intersect(ui.clip_rect()));
                ui.label(
                    egui::RichText::new(banner)
                        .monospace()
                        .size(terminal.font_size)
                        .color(colors.accent),
                );
            },
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(hint_rect), |ui| {
            ui.set_clip_rect(hint_rect.intersect(ui.clip_rect()));
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
                ui.set_clip_rect(tail_rect.intersect(ui.clip_rect()));
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
        crate::ui::activity::paint_attention_effect(ui.painter(), pane_rect, colors.state_needs_you, egui::Id::new(("terminal_attention", pane_id)), needs_you, false);
        return (false, tab_action);
    }

    let font_size = terminal.font_size;
    LOG_TERMINAL_GLYPH_PADDING.call_once(|| {
        log::info!(
            "terminal_renderer: content padding enabled {} points on both axes",
            crate::ui::style::PANE_CONTENT_INSET
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
        .set_padding(terminal_content_padding())
        .set_size(Vec2::new(ui.available_width(), ui.available_height()))
        .resize_immediately(resize_immediately)
        .with_input(input.keyboard_events, input.modifiers);
    ui.add(view);

    crate::ui::activity::paint_attention_effect(ui.painter(), pane_rect, colors.state_needs_you, egui::Id::new(("terminal_attention", pane_id)), needs_you, false);

    (false, tab_action)
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

static LOG_WORKING_NO_ESCALATION: Once = Once::new();

pub(crate) struct NameBarParams<'a> {
    pub(crate) tile_id: TileId,
    pub(crate) pane_id: &'a PaneId,
    pub(crate) tab_info: &'a HashMap<TileId, TabGroupInfo>,
    pub(crate) tab_labels: &'a HashMap<PaneId, String>,
    pub(crate) tab_activities: &'a HashMap<PaneId, (AgentState, bool)>,
    pub(crate) pane_names: &'a HashMap<PaneId, String>,
    pub(crate) pane_agent_state: Option<&'a AgentState>,
    pub(crate) is_error: bool,
    pub(crate) colors: &'a Colors,
    pub(crate) outside_workspace: bool,
    pub(crate) name_font_size: f32,
    pub(crate) spawned_by_badge: Option<&'a str>,
    pub(crate) waiting_results_badge: Option<&'a str>,
    pub(crate) overtake_hint: bool,
}

/// Place the cursor flush under the header. egui's `advance_cursor_after_rect` adds item
/// spacing, which left a pane-colored strip between header and body that only showed
/// (as a translucent band) when the pane was ghosted.
fn advance_past_header(ui: &mut egui::Ui, bar_rect: egui::Rect) {
    let spacing = std::mem::take(&mut ui.spacing_mut().item_spacing.y);
    ui.advance_cursor_after_rect(bar_rect);
    ui.spacing_mut().item_spacing.y = spacing;
}

fn header_pip_extent(ctx: &egui::Context, state: Option<&AgentState>) -> f32 {
    if matches!(state, Some(AgentState::NeedsYou)) {
        let visual = crate::ui::activity::attention(ctx);
        if visual.alpha > 0.0 {
            return TAB_PIP_RADIUS * visual.radius_scale + 1.5 + visual.stroke / 2.0;
        }
    }
    TAB_PIP_RADIUS + 0.7
}

pub(crate) fn render_name_bar_and_tabs(
    ui: &mut egui::Ui,
    params: NameBarParams<'_>,
) -> Option<(TileId, TabBarAction)> {
    // Ghosting belongs to pane content. Header state and control labels must
    // retain their composited contrast even when the pane body is dimmed.
    let body_opacity = ui.painter().opacity();
    ui.set_opacity(1.0);
    if body_opacity < 1.0 {
        static LEGIBLE_HEADER: std::sync::Once = std::sync::Once::new();
        LEGIBLE_HEADER.call_once(|| log::info!("ui: preserving header contrast while dimming pane content"));
    }
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
        overtake_hint,
    } = params;
    let has_name = pane_names.contains_key(pane_id);
    let tab_group = tab_info.get(&tile_id);
    let mut tab_action = None;
    let agent_state = pane_agent_state.or_else(|| tab_activities.get(pane_id).map(|(state, _)| state));
    let escalation_col = crate::ui::activity::escalation_color(agent_state, is_error, colors)
        .filter(|_| !matches!(agent_state, Some(AgentState::NeedsYou)) || crate::ui::activity::attention(ui.ctx()).alpha > 0.0);
    let escalation_col = escalation_col.map(|color| colors.status_badge_color(color));
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
        let mut capture = crate::ui::layout_report::Capture::begin(ui.painter(), "pane_header");
        advance_past_header(ui, bar_rect);
        let extent = header_pip_extent(ui.ctx(), agent_state);
        let leading = if agent_state.is_some() || is_error { TAB_PIP_MARGIN + TAB_PIP_RADIUS + extent } else { 0.0 };
        let labels = crate::ui::pane_header::Labels {
            outside: outside_workspace, waiting: waiting_results_badge, provenance: spawned_by_badge,
            state: escalation_lbl.zip(escalation_col),
        };
        let layout = crate::ui::pane_header::PaneHeader::measure(ui, bar_rect, leading, &labels, colors);
        ui.painter()
            .rect_filled(bar_rect, 0.0, colors.pane_header_bg());

        layout.record_slots(&mut capture);
        let outside_badge_rect = layout.outside;
        let tabs_right = layout.text.right().max(bar_rect.left());
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
            capture.text_slot(format!("tab-{member_id}"), &galley, egui::Rect::from_min_max(egui::pos2(content_left, tab_rect.top()), tab_rect.max));
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
            &ui.painter().with_clip_rect(tabs_bar_rect),
            TabBarPaintParams {
                bar_rect: tabs_bar_rect,
                group,
                tab_labels: &elided_tab_labels,
                tab_activities,
                colors,
                font_size: name_font_size,
                overtake_hint,
            },
        ) {
            tab_action = Some((group.container_tile, action));
        }

        layout.paint(ui, &labels, *pane_id, colors.pane_header_bg());

        capture.finish(ui.painter(), bar_rect, colors.pane_header_bg(), vec![], vec![]);
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
        let mut capture = crate::ui::layout_report::Capture::begin(ui.painter(), "pane_header");
        advance_past_header(ui, bar_rect);
                let leading = if agent_state.is_some() || is_error { crate::ui::style::PANE_CONTENT_INSET + crate::ui::style::PANE_HEADER_GLYPH } else { 0.0 };
        let labels = crate::ui::pane_header::Labels {
            outside: outside_workspace, waiting: waiting_results_badge, provenance: spawned_by_badge,
            state: escalation_lbl.zip(escalation_col),
        };
        let layout = crate::ui::pane_header::PaneHeader::measure(ui, bar_rect, leading, &labels, colors);
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

        let pip_space = leading;

        if is_error && bar_rect.width() >= pip_space {
            let cx = header_pip_center_x(bar_rect);
            crate::ui::activity::paint_pip(ui.painter(), egui::pos2(cx, bar_rect.center().y), HEADER_PIP_RADIUS, Some(&AgentState::Failed), colors.state_error, false);
        } else if bar_rect.width() >= pip_space {
            if let Some(state) = agent_state { paint_activity_dot(ui, bar_rect, state, colors); }
        }

        layout.record_slots(&mut capture);
        let outside_badge_rect = layout.outside;
        layout.paint(ui, &labels, *pane_id, colors.pane_header_bg().blend(escalation_col.map(|c| egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 28)).unwrap_or(egui::Color32::TRANSPARENT)));

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
            let left_edge = layout.text.left();
            let avail_width = layout.text.width();
            let galley = crate::ui::text::elided_galley(
                ui,
                name,
                egui::FontId::proportional(name_font_size),
                avail_width,
                crate::ui::text::Side::Trailing,
            );
            capture.text_slot("title", &galley, layout.text);
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

        capture.finish(ui.painter(), bar_rect, colors.pane_header_bg(), vec![], vec![]);
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

    ui.set_opacity(body_opacity);
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
    let cx = header_pip_center_x(bar_rect);
    crate::ui::activity::paint_pip(ui.painter(), egui::pos2(cx, bar_rect.center().y), HEADER_PIP_RADIUS, Some(state), color, false);
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
    fn pane_chrome_header_degrades_without_overlap() {
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let state = AgentState::NeedsYou;
        let mut names = HashMap::new();
        names.insert(7, "A long agent pane title".to_owned());
        let status_color = colors.status_badge_color(colors.state_needs_you);
        let fill = egui::Color32::from_rgba_unmultiplied(status_color.r(), status_color.g(), status_color.b(), 36);
        for width in [30.0, 50.0, 80.0, 120.0, 200.0, 400.0] {
            let ctx = egui::Context::default();
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, TAB_BAR_HEIGHT))), |ui| {
                    render_name_bar_and_tabs(ui, NameBarParams {
                        tile_id: TileId::from_u64(1), pane_id: &7, tab_info: &HashMap::new(), tab_labels: &HashMap::new(), tab_activities: &HashMap::new(), pane_names: &names,
                        pane_agent_state: Some(&state), is_error: false, colors: &colors, outside_workspace: false, name_font_size: 12.0, spawned_by_badge: Some("human"), waiting_results_badge: None, overtake_hint: false,
                    });
                });
            });
            let badge = output.shapes.iter().find_map(|s| match &s.shape { egui::Shape::Rect(r) if r.fill == fill => Some(r.rect), _ => None });
            if let Some(rect) = badge {
                assert!(rect.left() >= TAB_PIP_MARGIN + TAB_PIP_RADIUS * 2.0 && rect.right() <= width, "status badge overlaps the state icon or escapes header at width {width}: {rect:?}");
            }
            let title = last_rendered_tab_bar_layout(&ctx, 7).and_then(|layout| layout.label_rect);
            let provenance: Option<egui::Rect> = ctx.data(|d| d.get_temp(egui::Id::new(("provenance_icon_rect", 7_u64))));
            if let (Some(title), Some(badge)) = (title, badge) {
                assert!(!title.intersects(badge), "title and state chip overlap at width {width}");
            }
            if let (Some(title), Some(icon)) = (title, provenance) {
                assert!(!title.intersects(icon), "title and provenance icon overlap at width {width}");
            }
            if let (Some(badge), Some(icon)) = (badge, provenance) {
                assert!(!badge.intersects(icon), "state chip and provenance icon overlap at width {width}");
            }
            if width == 400.0 { assert!(badge.is_some(), "wide header retains the full chip"); }
            if width == 30.0 { assert!(badge.is_none(), "minimal header hides the chip"); }
        }

    }

    #[test]
    fn pane_header_provenance_icon_clears_content_inset() {
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        for width in [120.0, 200.0, 400.0, 1200.0] {
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, TAB_BAR_HEIGHT))), |ui| {
                    render_name_bar_and_tabs(ui, NameBarParams {
                        tile_id: TileId::from_u64(1), pane_id: &7, tab_info: &HashMap::new(), tab_labels: &HashMap::new(), tab_activities: &HashMap::new(), pane_names: &HashMap::new(),
                        pane_agent_state: None, is_error: false, colors: &colors, outside_workspace: false, name_font_size: 12.0, spawned_by_badge: Some("human"), waiting_results_badge: None, overtake_hint: false,
                    });
                });
            });
            let icon: egui::Rect = ctx.data(|d| d.get_temp(egui::Id::new(("provenance_icon_rect", 7_u64)))).expect("header paints provenance icon");
            assert!(icon.right() <= width - crate::ui::style::PANE_CONTENT_INSET, "provenance icon {icon:?} crosses the content inset at width {width}");
        }
    }

    /// Ink bounds of the shapes whose center satisfies `pick`, ignoring bar-wide fills.
    fn header_ink(shapes: &[egui::epaint::ClippedShape], pick: impl Fn(egui::Pos2) -> bool) -> egui::Rect {
        shapes.iter().map(|s| s.shape.visual_bounding_rect())
            .filter(|r| r.is_positive() && r.width() <= 20.0 && pick(r.center()))
            .fold(egui::Rect::NOTHING, |a, r| a.union(r))
    }

    /// The pip and the provenance icon keep one inset from the side rings, at least the gap
    /// above and below the glyph, and that vertical gap (pane top edge to ink vs ink to bar bottom) is equal, at every scale, at narrow widths and for every icon kind.
    /// The icon's ink fills its slot; the pip is a hollow ring (ink is 0.75 of its radius).
    #[test]
    fn pane_header_glyphs_have_symmetric_insets() {
        use crate::ui::style::{PANE_HEADER_GLYPH, PANE_HEADER_HEIGHT};
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let mut names = HashMap::new();
        names.insert(7, "title".to_owned());
        for badge in ["spawned by you", "spawned by researcher", "spawned by system"] {
            for scale in [1.0, 1.5] {
                for width in [140.0, 400.0] {
                    let ctx = egui::Context::default();
                    ctx.set_zoom_factor(scale);
                    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                        ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, TAB_BAR_HEIGHT))), |ui| {
                            render_name_bar_and_tabs(ui, NameBarParams {
                                tile_id: TileId::from_u64(1), pane_id: &7, tab_info: &HashMap::new(), tab_labels: &HashMap::new(), tab_activities: &HashMap::new(), pane_names: &names,
                                pane_agent_state: Some(&AgentState::Idle), is_error: false, colors: &colors, outside_workspace: false, name_font_size: 12.0, spawned_by_badge: Some(badge), waiting_results_badge: None, overtake_hint: false,
                            });
                        });
                    });
                    let ctx_msg = format!("{badge}, scale {scale}, width {width}");
                    let pip = header_ink(&output.shapes, |c| c.x < 30.0);
                    let icon = header_ink(&output.shapes, |c| c.x > width - 30.0);
                    assert!(pip.is_positive() && icon.is_positive(), "pip and icon must paint: {ctx_msg}");
                    // Slot-to-ring gap on each side: pip slot is centered on the pip, icon slot on the icon.
                    let pip_slot = pip.center().x - PANE_HEADER_GLYPH / 2.0 - PANE_RING_WIDTH;
                    let icon_slot = (width - PANE_RING_WIDTH) - (icon.center().x + PANE_HEADER_GLYPH / 2.0);
                    assert!((pip_slot - icon_slot).abs() <= 0.5, "pip slot gap {pip_slot} vs icon slot gap {icon_slot}: {ctx_msg}");
                    let side = width - icon.right();
                    let top = icon.top();
                    let bottom = PANE_HEADER_HEIGHT - icon.bottom();
                    assert!((top - bottom).abs() <= 0.5 && side >= top - 0.5, "icon gaps from outer edge: side {side}, above {top}, below {bottom}: {ctx_msg}");
                    let (ptop, pbottom) = (pip.top(), PANE_HEADER_HEIGHT - pip.bottom());
                    assert!((ptop - pbottom).abs() <= 0.5, "pip vertical gaps {ptop} vs {pbottom}: {ctx_msg}");
                    let pside = pip.left();
                    assert!(pside >= ptop - 0.5, "pip side gap {pside} must be at least vertical {ptop}: {ctx_msg}");
                    println!("{ctx_msg}: icon side {side:.2} top {top:.2} bottom {bottom:.2}; pip side {pside:.2} top {ptop:.2} bottom {pbottom:.2}; slot gaps {pip_slot:.2}/{icon_slot:.2}");
                }
            }
        }
    }

    /// The outside-workspace chip sits inside the band with equal margins above and below, its
    /// arrow is centered on the text, and its side padding is equal.
    #[test]
    fn outside_chip_is_centered_in_band_with_equal_padding() {
        use crate::ui::style::{CHIP_ICON_GAP, PANE_HEADER_CHIP_HEIGHT, PANE_HEADER_HEIGHT};
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let mut names = HashMap::new();
        names.insert(7, "title".to_owned());
        for scale in [1.0, 1.5] {
            let ctx = egui::Context::default();
            ctx.set_zoom_factor(scale);
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, TAB_BAR_HEIGHT))), |ui| {
                    render_name_bar_and_tabs(ui, NameBarParams {
                        tile_id: TileId::from_u64(1), pane_id: &7, tab_info: &HashMap::new(), tab_labels: &HashMap::new(), tab_activities: &HashMap::new(), pane_names: &names,
                        pane_agent_state: None, is_error: false, colors: &colors, outside_workspace: true, name_font_size: 12.0, spawned_by_badge: None, waiting_results_badge: None, overtake_hint: false,
                    });
                });
            });
            let chip = ctx.data(|d| d.get_temp::<TerminalTabBarLayout>(egui::Id::new(("terminal_tab_bar_layout", 7_u64)))).and_then(|l| l.outside_workspace_badge_rect).expect("chip rect");
            assert!((chip.height() - PANE_HEADER_CHIP_HEIGHT).abs() < 0.01, "chip height {}", chip.height());
            let (top, bottom) = (chip.top(), PANE_HEADER_HEIGHT - chip.bottom());
            assert!((top - bottom).abs() <= 0.5 && top >= 2.0, "chip margins top {top} bottom {bottom} at scale {scale}");
            let text = output.shapes.iter().filter_map(|s| match &s.shape { egui::Shape::Text(_) if chip.contains(s.shape.visual_bounding_rect().center()) => Some(s.shape.visual_bounding_rect()), _ => None }).next().expect("chip text");
            let pts: Vec<egui::Pos2> = output.shapes.iter().filter_map(|s| match &s.shape { egui::Shape::LineSegment { points, .. } if chip.contains(points[0]) => Some(points.to_vec()), _ => None }).flatten().collect();
            assert!(pts.len() >= 6, "arrow is three painted segments");
            let arrow = egui::Rect::from_points(&pts);
            assert!((arrow.center().y - text.center().y).abs() <= 0.5, "arrow centre {} vs text centre {}", arrow.center().y, text.center().y);
            assert!((arrow.left() - chip.left() - (chip.right() - text.right())).abs() <= 0.5, "left pad {} vs right pad {}", arrow.left() - chip.left(), chip.right() - text.right());
            assert!((text.left() - arrow.right() - CHIP_ICON_GAP).abs() <= 0.5, "arrow-to-text gap {}", text.left() - arrow.right());
            println!("chip scale {scale}: margins {top:.2}/{bottom:.2}, arrow-text dy {:.2}, pads {:.2}/{:.2}", arrow.center().y - text.center().y, arrow.left() - chip.left(), chip.right() - text.right());
        }
    }

    #[test]
    fn terminal_padding_is_one_token_clearing_pane_rings() {
        use crate::ui::style::{PANE_CONTENT_INSET, PANE_FOCUS_WIDTH, PANE_STATE_WIDTH};
        let pad = terminal_content_padding();
        assert_eq!(pad.x, pad.y, "x and y share one inset token");
        assert_eq!(pad.x, PANE_CONTENT_INSET);
        assert_eq!(pad.x, PANE_FOCUS_WIDTH + PANE_STATE_WIDTH + crate::ui::style::PANE_CONTENT_GAP);
        assert_eq!(crate::ui::style::PANE_CONTENT_GAP, 2.0, "split the difference: 2pt of air beyond the rings");
        assert!(pad.x > PANE_FOCUS_WIDTH + PANE_STATE_WIDTH, "inset leaves a gap beyond both rings");
        let frame = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 200.0));
        let grid = egui::Rect::from_min_max(frame.min + pad, frame.max - pad);
        let inside_rings = frame.shrink(PANE_FOCUS_WIDTH + PANE_STATE_WIDTH);
        assert!(inside_rings.contains_rect(grid), "grid {grid:?} overlaps rings; frame {frame:?}");
    }

    #[test]
    fn pane_chrome_error_chip_collapses_to_failed_glyph() {
        let ctx = egui::Context::default();
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(55.0, TAB_BAR_HEIGHT))), |ui| {
                render_name_bar_and_tabs(ui, NameBarParams {
                    tile_id: TileId::from_u64(1), pane_id: &7, tab_info: &HashMap::new(), tab_labels: &HashMap::new(), tab_activities: &HashMap::new(), pane_names: &HashMap::new(),
                    pane_agent_state: None, is_error: true, colors: &colors, outside_workspace: false, name_font_size: 12.0, spawned_by_badge: None, waiting_results_badge: None, overtake_hint: false,
                });
            });
        });
        let lines = output.shapes.iter().filter(|s| matches!(s.shape, egui::Shape::LineSegment { .. })).count();
        assert_eq!(lines, 3, "one leading failed cross plus the divider; no duplicate state glyph");
    }

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
