//! Activity dot widget and semantic escalation colors for agent panes,
//! `PortalPane` summary cards, and context sidebar indicators.

use crate::host::context_state::ContextStatus;
use crate::protocol::AgentState;
use crate::ui::theme::Colors;

pub fn blocked_color(colors: &Colors) -> egui::Color32 {
    colors.state_blocked
}
pub fn needs_you_color(colors: &Colors) -> egui::Color32 {
    colors.state_needs_you
}

/// Semantic accent color for a rolled-up [`ContextStatus`] on `PortalPane`
/// summary cards and context sidebar indicators.
pub fn context_status_color(status: &ContextStatus, colors: &Colors) -> Option<egui::Color32> {
    match status {
        ContextStatus::NeedsYou => Some(needs_you_color(colors)),
        ContextStatus::Blocked => Some(blocked_color(colors)),
        ContextStatus::Error => Some(colors.state_error),
        ContextStatus::Working => Some(colors.state_working),
        ContextStatus::Done => Some(colors.state_waiting),
        ContextStatus::Idle => None,
    }
}

/// Semantic border/header escalation color for a terminal or agent pane.
pub fn escalation_color(
    state: Option<&AgentState>,
    is_error: bool,
    colors: &Colors,
) -> Option<egui::Color32> {
    if is_error {
        return Some(colors.state_error);
    }
    match state {
        Some(AgentState::NeedsYou) => Some(needs_you_color(colors)),
        Some(AgentState::Blocked) => Some(blocked_color(colors)),
        Some(AgentState::Interrupted) => Some(colors.state_interrupted),
        Some(AgentState::Failed) => Some(colors.state_error),
        Some(AgentState::ExitedUnreported) => Some(colors.warning),
        Some(AgentState::Working | AgentState::Idle | AgentState::Completed) | None => None,
    }
}

/// State color shared by terminal and app pane outlines.
pub fn pane_state_color(pane: &crate::host::pane::Pane, colors: &Colors) -> Option<egui::Color32> {
    // Quiet states (Idle, Completed) paint no ring: a gray ring just inside an unfocused
    // pane's edge reads as a stray line, and the header glyph already carries the state.
    let ring = |state: &AgentState| (!matches!(state, AgentState::Idle | AgentState::Completed)).then(|| dot_color_from_time(state, colors, 0.0, 0));
    if let Some(terminal) = pane.as_terminal() {
        if terminal.exited && terminal.exit_status.as_ref().is_some_and(|status| status.is_failure()) {
            return Some(colors.state_error);
        }
        if terminal.standby_session.is_some() {
            return Some(colors.state_needs_you);
        }
        if terminal.agent.is_some() || terminal.reported_7501.is_some() || terminal.observed_agent.is_some() {
            return ring(&terminal.effective_agent_state().0);
        }
    }
    pane.effective_activity().and_then(ring)
}

/// Compact uppercase status badge label for a pane's escalation state.
pub fn escalation_label(state: Option<&AgentState>, is_error: bool) -> Option<&'static str> {
    if is_error {
        return Some("ERROR");
    }
    match state {
        Some(AgentState::NeedsYou) => Some("NEEDS YOU"),
        Some(AgentState::Blocked) => Some("BLOCKED"),
        Some(AgentState::Interrupted) => Some("INTERRUPTED"),
        Some(AgentState::Failed) => Some("FAILED"),
        Some(AgentState::ExitedUnreported) => Some("EXITED WITHOUT REPORT"),
        Some(AgentState::Working | AgentState::Idle | AgentState::Completed) | None => None,
    }
}

/// Single source of truth for pip coloring across every pip surface
/// (sidebar rows, palette rows, portal rows). Handles both the activity
/// color and the focused/unfocused dim uniformly: focused pips render at
/// full strength, unfocused neutral pips at `colors.pip_dim`.
/// Neutral pips dim; semantic state tokens retain their background contrast.
pub fn pip_color(
    state: Option<&AgentState>,
    focused: bool,
    colors: &Colors,
    time: f64,
    pip_index: usize,
) -> egui::Color32 {
    let base = match state {
        Some(s) => dot_color_from_time(s, colors, time, pip_index),
        None if focused => colors.text_primary,
        None => colors.text_dim,
    };
    if focused || state.is_some() {
        base
    } else {
        base.gamma_multiply(colors.pip_dim.max(0.72))
    }
}

/// Returns the color for a given agent state given an explicit `time` value.
/// State colors stay constant; motion is handled by the glyph renderer.
pub fn dot_color_from_time(
    state: &AgentState,
    colors: &Colors,
    _time: f64,
    _pip_index: usize,
) -> egui::Color32 {
    match state {
        AgentState::Working => colors.state_working,
        AgentState::Idle | AgentState::Completed => colors.state_waiting,
        AgentState::Interrupted => colors.state_interrupted,
        AgentState::Failed => colors.state_error,
        AgentState::ExitedUnreported => colors.warning,
        AgentState::Blocked => blocked_color(colors),
        AgentState::NeedsYou => needs_you_color(colors),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateGlyph {
    Arc,
    Hollow,
    Square,
    Target,
    Diamond,
    Cross,
}

pub fn state_glyph(state: &AgentState) -> StateGlyph {
    match state {
        AgentState::Working => StateGlyph::Arc,
        AgentState::Idle | AgentState::Completed => StateGlyph::Hollow,
        AgentState::Blocked => StateGlyph::Square,
        AgentState::NeedsYou => StateGlyph::Target,
        AgentState::Interrupted => StateGlyph::Diamond,
        AgentState::Failed => StateGlyph::Cross,
        AgentState::ExitedUnreported => StateGlyph::Diamond,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttentionVisual {
    pub radius_scale: f32,
    pub alpha: f32,
    pub stroke: f32,
    pub animate: bool,
    pub loud: bool,
}

pub fn attention_visual(
    config: crate::config::AttentionConfig,
    reduce_motion: bool,
    time: f64,
) -> AttentionVisual {
    use crate::config::AttentionStyle;
    let style = config.style.unwrap_or_default();
    let intensity = config.intensity.unwrap_or(1.0).clamp(0.0, 1.0);
    let animate = !reduce_motion
        && intensity > 0.0
        && matches!(style, AttentionStyle::Normal | AttentionStyle::Loud);
    let pulse = if animate {
        0.8 + 0.2 * (time * std::f64::consts::PI).sin() as f32
    } else {
        1.0
    };
    AttentionVisual {
        radius_scale: if matches!(style, AttentionStyle::Normal | AttentionStyle::Loud) {
            1.5
        } else {
            1.0
        },
        alpha: match style {
            AttentionStyle::Off => 0.0,
            AttentionStyle::Subtle => 0.63 * intensity,
            _ => pulse * intensity,
        },
        stroke: if matches!(style, AttentionStyle::Normal | AttentionStyle::Loud) {
            2.0
        } else {
            1.0
        },
        animate,
        loud: style == AttentionStyle::Loud,
    }
}

/// Host-local presentation settings are projected once in logic, including hidden passes.
pub fn set_attention(
    ctx: &egui::Context,
    config: crate::config::AttentionConfig,
    reduce_motion: bool,
) {
    ctx.data_mut(|d| {
        d.insert_temp(
            egui::Id::new("agent_attention_style"),
            (config, reduce_motion),
        )
    });
}

fn motion_settings(ctx: &egui::Context) -> (crate::config::AttentionConfig, bool) {
    ctx.data(|d| {
        d.get_temp::<(crate::config::AttentionConfig, bool)>(egui::Id::new("agent_attention_style"))
    })
    .unwrap_or_default()
}

pub fn attention(ctx: &egui::Context) -> AttentionVisual {
    let (config, reduce) = motion_settings(ctx);
    attention_visual(config, reduce, ctx.input(|i| i.time))
}

fn schedule_attention(ctx: &egui::Context, visual: AttentionVisual) {
    if visual.animate {
        // egui subtracts predicted_dt; compensate so wakeups stay at most 10 Hz.
        let predicted = ctx.input(|i| i.predicted_dt).max(0.0);
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(0.1 + predicted));
    }
}

pub fn paint_pip(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    state: Option<&AgentState>,
    color: egui::Color32,
    hidden: bool,
) {
    use egui::{Rect, Stroke, StrokeKind, Vec2};
    let Some(state) = state else {
        if hidden {
            painter.circle_stroke(center, radius, Stroke::new(1.0_f32, color));
        } else {
            painter.circle_filled(center, radius, color);
        }
        return;
    };
    let stroke = Stroke::new(1.4_f32, color);
    match state_glyph(state) {
        StateGlyph::Arc => {
            let ctx = painter.ctx();
            let reduced = motion_settings(ctx).1;
            let angle_offset = if reduced {
                0.0
            } else {
                if painter.clip_rect().intersects(Rect::from_center_size(
                    center,
                    Vec2::splat(radius * 2.0 + stroke.width),
                )) {
                    let predicted = ctx.input(|i| i.predicted_dt).max(0.0);
                    ctx.request_repaint_after(std::time::Duration::from_secs_f32(1.0 / 30.0 + predicted));
                }
                (ctx.input(|i| i.time).rem_euclid(2.0) * std::f64::consts::PI) as f32
            };
            let points = (0..=16)
                .map(|i| {
                    let angle = angle_offset + i as f32 / 16.0 * std::f32::consts::TAU * 0.75;
                    center + Vec2::angled(angle) * radius
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
        }
        StateGlyph::Hollow => {
            painter.circle_stroke(center, radius * 0.75, stroke);
        }
        StateGlyph::Square => {
            painter.rect_stroke(
                Rect::from_center_size(center, Vec2::splat(radius * 1.6)),
                0.0,
                stroke,
                StrokeKind::Inside,
            );
        }
        StateGlyph::Target => {
            let visual = attention(painter.ctx());
            schedule_attention(painter.ctx(), visual);
            painter.circle_filled(center, radius * 0.6, color);
            // The shape outline survives style=off; only extra attention chrome is disabled.
            painter.circle_stroke(center, radius, stroke);
            if visual.alpha > 0.0 {
                painter.circle_stroke(
                    center,
                    radius * visual.radius_scale + 1.5,
                    Stroke::new(visual.stroke, color.gamma_multiply(visual.alpha)),
                );
            }
        }
        StateGlyph::Diamond => {
            painter.add(egui::Shape::closed_line(
                vec![
                    center + Vec2::new(0.0, -radius),
                    center + Vec2::new(radius, 0.0),
                    center + Vec2::new(0.0, radius),
                    center + Vec2::new(-radius, 0.0),
                ],
                stroke,
            ));
        }
        StateGlyph::Cross => {
            painter.line_segment(
                [
                    center - Vec2::splat(radius * 0.75),
                    center + Vec2::splat(radius * 0.75),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + Vec2::new(-radius, radius) * 0.75,
                    center + Vec2::new(radius, -radius) * 0.75,
                ],
                stroke,
            );
        }
    }
    if hidden {
        painter.line_segment(
            [
                center + Vec2::new(-radius, radius + 2.0),
                center + Vec2::new(radius, radius + 2.0),
            ],
            stroke,
        );
    }
}

pub fn paint_attention_chrome(
    painter: &egui::Painter,
    rect: egui::Rect,
    color: egui::Color32,
    identity: egui::Id,
    needs_you: bool,
) {
    paint_attention_effect(painter, rect, color, identity, needs_you, true);
}

pub fn paint_attention_effect(
    painter: &egui::Painter, rect: egui::Rect, color: egui::Color32, identity: egui::Id, needs_you: bool, ring: bool,
) {
    let key = identity.with("attention_entered");
    let time = painter.ctx().input(|i| i.time);
    if !needs_you {
        painter.ctx().data_mut(|d| d.remove::<f64>(key));
        return;
    }
    let entered = painter
        .ctx()
        .data_mut(|d| *d.get_temp_mut_or_insert_with(key, || time));
    let visual = attention(painter.ctx());
    if visual.alpha == 0.0 {
        return;
    }
    schedule_attention(painter.ctx(), visual);
    if ring {
        painter.rect_stroke(
            rect.shrink(1.0),
            crate::ui::style::radius_fit(rect.shrink(1.0)),
            egui::Stroke::new(visual.stroke, color.gamma_multiply(visual.alpha)),
            egui::StrokeKind::Inside,
        );
    }
    if visual.loud {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "NEEDS YOU",
            crate::ui::theme::font_medium(
                (crate::ui::style::TEXT_TITLE_XL * 1.5)
                    .min(rect.width() / 6.5)
                    .min(rect.height() / 3.0),
            ),
            color.gamma_multiply(0.35 * visual.alpha),
        );
        if visual.animate && time - entered < 0.3 {
            painter.rect_filled(rect, 0.0, color.gamma_multiply(0.15 * visual.alpha));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;

    #[test]
    fn static_health_pip_keeps_color_without_animation() {
        let ctx = egui::Context::default();
        let frame = |time| ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| {
            paint_pip(ui.painter(), egui::pos2(30.0, 30.0), 4.0, None, egui::Color32::YELLOW, false);
        });
        for n in 0..4 { frame(n as f64); }
        let first = frame(4.0);
        let second = frame(4.5);
        assert_eq!(format!("{:?}", first.shapes), format!("{:?}", second.shapes));
        assert_eq!(first.viewport_output[&egui::ViewportId::ROOT].repaint_delay, std::time::Duration::MAX);
    }

    #[test]
    fn working_arc_rotates_and_repaints_only_when_visible_and_motion_enabled() {
        let ctx = egui::Context::default();
        let frame = |time, state: &AgentState, reduced, visible| {
            set_attention(&ctx, Default::default(), reduced);
            let output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    let painter = if visible {
                        ui.painter().clone()
                    } else {
                        ui.painter().with_clip_rect(egui::Rect::NOTHING)
                    };
                    paint_pip(
                        &painter,
                        egui::pos2(30.0, 30.0),
                        4.0,
                        Some(state),
                        egui::Color32::WHITE,
                        false,
                    );
                },
            );
            (
                format!("{:?}", output.shapes),
                output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            )
        };
        for n in 0..4 {
            frame(n as f64, &AgentState::Working, false, true);
        }
        let (first, delay) = frame(4.0, &AgentState::Working, false, true);
        let (second, _) = frame(4.5, &AgentState::Working, false, true);
        assert_ne!(first, second, "working geometry must rotate");
        assert!(
            delay >= std::time::Duration::from_millis(32)
                && delay < std::time::Duration::from_millis(40),
            "{delay:?}"
        );
        for n in 5..9 {
            frame(n as f64, &AgentState::Idle, false, true);
        }
        assert_eq!(
            frame(9.0, &AgentState::Idle, false, true).1,
            std::time::Duration::MAX
        );
        for n in 10..14 {
            frame(n as f64, &AgentState::Working, true, true);
        }
        let (reduced, delay) = frame(14.0, &AgentState::Working, true, true);
        assert_eq!(reduced, frame(14.5, &AgentState::Working, true, true).0);
        assert_eq!(delay, std::time::Duration::MAX);
        for n in 20..24 {
            frame(n as f64, &AgentState::NeedsYou, false, true);
        }
        let delay = frame(24.0, &AgentState::NeedsYou, false, true).1;
        assert!(
            delay >= std::time::Duration::from_millis(99)
                && delay < std::time::Duration::from_millis(200)
        );
        for n in 25..29 {
            frame(n as f64, &AgentState::NeedsYou, true, true);
        }
        assert_eq!(
            frame(29.0, &AgentState::NeedsYou, true, true).1,
            std::time::Duration::MAX
        );
        for n in 15..19 {
            frame(n as f64, &AgentState::Working, false, false);
        }
        assert_eq!(
            frame(19.0, &AgentState::Working, false, false).1,
            std::time::Duration::MAX
        );
    }

    #[test]
    fn state_shapes_are_distinct_even_without_color() {
        let states = [
            AgentState::Working,
            AgentState::Idle,
            AgentState::Blocked,
            AgentState::NeedsYou,
            AgentState::Interrupted,
            AgentState::Failed,
        ];
        for (i, a) in states.iter().enumerate() {
            for b in &states[i + 1..] {
                assert_ne!(state_glyph(a), state_glyph(b));
            }
        }
        assert_eq!(
            state_glyph(&AgentState::Idle),
            state_glyph(&AgentState::Completed)
        );
    }

    #[test]
    fn attention_styles_intensity_and_reduce_motion() {
        use crate::config::{AttentionConfig, AttentionStyle};
        let cfg = |style, intensity| AttentionConfig {
            style: Some(style),
            intensity: Some(intensity),
            ..Default::default()
        };
        let off = attention_visual(cfg(AttentionStyle::Off, 1.0), false, 0.0);
        let subtle = attention_visual(cfg(AttentionStyle::Subtle, 1.0), false, 0.0);
        let normal = attention_visual(cfg(AttentionStyle::Normal, 1.0), false, 0.0);
        let loud = attention_visual(cfg(AttentionStyle::Loud, 1.0), false, 0.0);
        assert_eq!(off.alpha, 0.0);
        assert!(!off.animate);
        assert!(!subtle.animate);
        assert!(normal.animate);
        assert!(loud.loud);
        assert_ne!(off, subtle);
        assert_ne!(subtle, normal);
        assert_ne!(normal, loud);
        let half = attention_visual(cfg(AttentionStyle::Normal, 0.5), false, 0.0);
        assert_eq!(half.alpha, normal.alpha * 0.5);
        let reduced = attention_visual(cfg(AttentionStyle::Normal, 1.0), true, 0.0);
        assert!(!reduced.animate);
        assert_eq!(
            reduced,
            attention_visual(cfg(AttentionStyle::Normal, 1.0), true, 0.5)
        );
        assert!(!attention_visual(cfg(AttentionStyle::Normal, 0.0), false, 0.0).animate);
    }

    #[test]
    fn attention_config_rejects_invalid_values() {
        for raw in [
            "[attention]\nstyle='unknown'",
            "[attention]\nintensity=1.1",
            "[attention]\nintensity=nan",
            "[attention]\ndock='forever'",
            "[theme]\nstate_preset='unknown'",
        ] {
            assert!(
                toml::from_str::<crate::config::PlexiConfig>(raw).is_err(),
                "{raw}"
            );
        }
    }

    #[test]
    fn working_color_is_static() {
        let colors = Colors::from_config(&ThemeConfig::default());
        assert_eq!(
            dot_color_from_time(&AgentState::Working, &colors, 0.0, 0),
            dot_color_from_time(&AgentState::Working, &colors, 0.5, 0)
        );
    }

    #[test]
    fn pip_color_focused_without_agent_uses_text_primary_not_accent() {
        let colors = Colors::from_config(&ThemeConfig::default());
        assert_ne!(colors.text_primary, colors.accent);
        let focused_neutral = pip_color(None, true, &colors, 0.0, 0);
        assert_eq!(focused_neutral, colors.text_primary);
        assert_ne!(focused_neutral, colors.accent);

        let unfocused_neutral = pip_color(None, false, &colors, 0.0, 0);
        assert_eq!(
            unfocused_neutral,
            colors.text_dim.gamma_multiply(colors.pip_dim.max(0.72))
        );
    }

    #[test]
    fn blocked_and_needs_you_render_distinct_semantic_colors() {
        let colors = Colors::from_config(&ThemeConfig::default());
        let blocked = dot_color_from_time(&AgentState::Blocked, &colors, 0.0, 0);
        let needs_you = dot_color_from_time(&AgentState::NeedsYou, &colors, 0.0, 0);
        assert_eq!(blocked, colors.state_blocked);
        assert_eq!(needs_you, colors.state_needs_you);
        assert_ne!(blocked, needs_you);
        assert_eq!(
            context_status_color(&ContextStatus::Blocked, &colors),
            Some(colors.state_blocked)
        );
        assert_eq!(
            context_status_color(&ContextStatus::NeedsYou, &colors),
            Some(colors.state_needs_you)
        );
        assert_eq!(
            context_status_color(&ContextStatus::Error, &colors),
            Some(colors.state_error)
        );
    }

    #[test]
    fn working_does_not_escalate_pane_color_or_label() {
        let colors = Colors::from_config(&ThemeConfig::default());
        assert_eq!(
            escalation_color(Some(&AgentState::Working), false, &colors),
            None
        );
        assert_eq!(escalation_label(Some(&AgentState::Working), false), None);

        // Context-level Working status and activity dot colors remain active.
        assert_eq!(
            context_status_color(&ContextStatus::Working, &colors),
            Some(colors.state_working)
        );

        // Actual escalation states continue to return their semantic colors and labels.
        assert_eq!(
            escalation_color(Some(&AgentState::Blocked), false, &colors),
            Some(colors.state_blocked)
        );
        assert_eq!(
            escalation_label(Some(&AgentState::Blocked), false),
            Some("BLOCKED")
        );
        assert_eq!(
            escalation_color(Some(&AgentState::NeedsYou), false, &colors),
            Some(colors.state_needs_you)
        );
        assert_eq!(
            escalation_label(Some(&AgentState::NeedsYou), false),
            Some("NEEDS YOU")
        );
        assert_eq!(
            escalation_color(Some(&AgentState::Interrupted), false, &colors),
            Some(colors.state_interrupted)
        );
        assert_eq!(
            escalation_label(Some(&AgentState::Interrupted), false),
            Some("INTERRUPTED")
        );
        assert_eq!(
            escalation_color(Some(&AgentState::Working), true, &colors),
            Some(colors.state_error)
        );
        assert_eq!(
            escalation_label(Some(&AgentState::Working), true),
            Some("ERROR")
        );
    }
}
