//! Activity dot widget and semantic escalation colors for agent panes,
//! `PortalPane` summary cards, and context sidebar indicators.

use crate::host::context_state::ContextStatus;
use crate::protocol::AgentState;
use crate::ui::theme::Colors;

/// Tier 1 (`Blocked`, master-owned) semantic amber token (`#f59e0b`).
pub const BLOCKED_COLOR: egui::Color32 = egui::Color32::from_rgb(0xf5, 0x9e, 0x0b);

/// Tier 2 (`NeedsYou`, human-owned) semantic coral token (`#f43f5e`).
pub const NEEDS_YOU_COLOR: egui::Color32 = egui::Color32::from_rgb(0xf4, 0x3f, 0x5e);

/// Semantic color for Tier 1 `Blocked` (master-owned) state. Honors an
/// explicit `theme.pip_blocked` user override when it differs from the
/// default `danger` fallback; otherwise returns [`BLOCKED_COLOR`] (`#f59e0b`).
pub fn blocked_color(colors: &Colors) -> egui::Color32 {
    if colors.pip_blocked != colors.danger {
        colors.pip_blocked
    } else {
        BLOCKED_COLOR
    }
}

/// Semantic color for Tier 2 `NeedsYou` (human-owned) state (`#f43f5e`).
pub fn needs_you_color(_colors: &Colors) -> egui::Color32 {
    NEEDS_YOU_COLOR
}

/// Semantic accent color for a rolled-up [`ContextStatus`] on `PortalPane`
/// summary cards and context sidebar indicators.
pub fn context_status_color(status: &ContextStatus, colors: &Colors) -> Option<egui::Color32> {
    match status {
        ContextStatus::NeedsYou => Some(needs_you_color(colors)),
        ContextStatus::Blocked => Some(blocked_color(colors)),
        ContextStatus::Error => Some(colors.danger),
        ContextStatus::Working => Some(colors.pip_working),
        ContextStatus::Done => Some(colors.pip_idle),
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
        return Some(colors.danger);
    }
    match state {
        Some(AgentState::NeedsYou) => Some(needs_you_color(colors)),
        Some(AgentState::Blocked) => Some(blocked_color(colors)),
        Some(AgentState::Working) => Some(colors.pip_working),
        Some(AgentState::Interrupted) => Some(colors.warning),
        Some(AgentState::Idle | AgentState::Completed) | None => None,
    }
}

/// Compact uppercase status badge label for a pane's escalation state.
pub fn escalation_label(state: Option<&AgentState>, is_error: bool) -> Option<&'static str> {
    if is_error {
        return Some("ERROR");
    }
    match state {
        Some(AgentState::NeedsYou) => Some("NEEDS YOU"),
        Some(AgentState::Blocked) => Some("BLOCKED"),
        Some(AgentState::Working) => Some("WORKING"),
        Some(AgentState::Interrupted) => Some("INTERRUPTED"),
        Some(AgentState::Idle | AgentState::Completed) | None => None,
    }
}

/// Single source of truth for pip coloring across every pip surface
/// (sidebar rows, palette rows, portal rows). Handles both the activity
/// color and the focused/unfocused dim uniformly: focused pips render at
/// full strength, unfocused pips at `colors.pip_dim` — regardless of
/// whether the pip carries an activity color or the neutral/text_primary pair.
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
    if focused {
        base
    } else {
        base.gamma_multiply(colors.pip_dim.max(0.72))
    }
}

/// Returns the color for a given agent state given an explicit `time` value.
/// Working state pulses between 0.65–1.0 opacity on a ~2s sine cycle.
/// When Working, call `request_repaint_after(Duration::from_millis(100))` at
/// the call site — never an unconditional `request_repaint()`, which is
/// self-perpetuating and pins the whole window at display refresh.
pub fn dot_color_from_time(
    state: &AgentState,
    colors: &Colors,
    time: f64,
    pip_index: usize,
) -> egui::Color32 {
    match state {
        AgentState::Working => {
            let stagger = pip_index as f64 * 0.35;
            let alpha = 0.65
                + 0.35 * (0.5 + 0.5 * ((time + stagger) * 1.2 * std::f64::consts::PI).sin()) as f32;
            let c = colors.pip_working;
            egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * alpha) as u8)
        }
        AgentState::Idle | AgentState::Completed => colors.pip_idle,
        AgentState::Interrupted => colors.warning,
        AgentState::Blocked => blocked_color(colors),
        AgentState::NeedsYou => needs_you_color(colors),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;

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
        assert_eq!(blocked, BLOCKED_COLOR);
        assert_eq!(needs_you, NEEDS_YOU_COLOR);
        assert_ne!(blocked, needs_you);
        assert_eq!(
            context_status_color(&ContextStatus::Blocked, &colors),
            Some(BLOCKED_COLOR)
        );
        assert_eq!(
            context_status_color(&ContextStatus::NeedsYou, &colors),
            Some(NEEDS_YOU_COLOR)
        );
        assert_eq!(
            context_status_color(&ContextStatus::Error, &colors),
            Some(colors.danger)
        );
    }
}
