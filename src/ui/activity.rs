//! Activity dot widget — renders a single agent-state indicator circle.

use crate::app_protocol::AgentState;
use crate::ui::theme::Colors;

/// Single source of truth for pip coloring across every pip surface
/// (sidebar rows, palette rows, portal rows). Handles both the activity
/// color and the focused/unfocused dim uniformly: focused pips render at
/// full strength, unfocused pips at `colors.pip_dim` — regardless of
/// whether the pip carries an activity color or the neutral/accent pair.
pub fn pip_color(
    state: Option<&AgentState>,
    focused: bool,
    colors: &Colors,
    time: f64,
    pip_index: usize,
) -> egui::Color32 {
    let base = match state {
        Some(s) => dot_color_from_time(s, colors, time, pip_index),
        None if focused => colors.accent,
        None => colors.text_dim,
    };
    if focused {
        base
    } else {
        base.gamma_multiply(colors.pip_dim)
    }
}

/// Returns the color for a given agent state given an explicit `time` value.
/// Working state pulses between 0.45–1.0 opacity on a ~2s sine cycle.
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
            let alpha =
                0.20 + 0.80 * (0.5 + 0.5 * ((time + stagger) * 1.2 * std::f64::consts::PI).sin()) as f32;
            let c = colors.pip_working;
            egui::Color32::from_rgba_unmultiplied(
                c.r(),
                c.g(),
                c.b(),
                (c.a() as f32 * alpha) as u8,
            )
        }
        AgentState::Idle => colors.pip_idle,
        AgentState::Blocked => colors.pip_blocked,
    }
}
