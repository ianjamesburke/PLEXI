//! Activity dot widget — renders a single agent-state indicator circle and
//! provides a rollup helper for collapsing a slice of states to one.

use crate::app_protocol::AgentState;
use crate::ui::theme::Colors;

/// Opacity multiplier for activity pips on panes that are not the focused
/// pane in their context — keeps the focused pip locatable when several
/// pips carry activity colors.
pub const UNFOCUSED_DIM: f32 = 0.45;

/// Single source of truth for pip coloring across every pip surface
/// (sidebar rows, palette rows, portal rows). Handles both the activity
/// color and the focused/unfocused dim uniformly: focused pips render at
/// full strength, unfocused pips at `UNFOCUSED_DIM` — regardless of
/// whether the pip carries an activity color or the neutral/accent pair.
pub fn pip_color(
    state: Option<&AgentState>,
    focused: bool,
    colors: &Colors,
    time: f64,
) -> egui::Color32 {
    let base = match state {
        Some(s) => dot_color_from_time(s, colors, time),
        None if focused => colors.accent,
        None => colors.text_dim,
    };
    if focused {
        base
    } else {
        base.gamma_multiply(UNFOCUSED_DIM)
    }
}

/// Returns the color for a given agent state given an explicit `time` value.
/// Working state pulses between 0.45–1.0 opacity on a ~2s sine cycle.
/// Call `ui.ctx().request_repaint()` at the call site when Working.
pub fn dot_color_from_time(state: &AgentState, colors: &Colors, time: f64) -> egui::Color32 {
    match state {
        AgentState::Working => {
            // Sine oscillates –1..1; remap to 0.45..1.0.
            let alpha = 0.45 + 0.55 * (0.5 + 0.5 * (time * std::f64::consts::PI).sin()) as f32;
            let c = colors.success;
            egui::Color32::from_rgba_unmultiplied(
                c.r(),
                c.g(),
                c.b(),
                (c.a() as f32 * alpha) as u8,
            )
        }
        AgentState::Idle => colors.warning,
        AgentState::Blocked => colors.danger,
    }
}

/// Reduce an iterator of optional agent states to the highest-precedence state
/// present. Blocked > Working > Idle. Returns `None` when the iterator is empty
/// or all entries are `None`.
pub fn rollup_activity<'a>(states: impl Iterator<Item = &'a Option<AgentState>>) -> Option<AgentState> {
    let mut best: Option<AgentState> = None;
    for entry in states {
        let Some(s) = entry else { continue };
        best = Some(match (&best, s) {
            (_, AgentState::Blocked) => AgentState::Blocked,
            (Some(AgentState::Blocked), _) => AgentState::Blocked,
            (_, AgentState::Working) => AgentState::Working,
            (Some(AgentState::Working), _) => AgentState::Working,
            _ => AgentState::Idle,
        });
        // Short-circuit: can't beat Blocked.
        if best == Some(AgentState::Blocked) {
            return best;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollup_empty_is_none() {
        let states: Vec<Option<AgentState>> = vec![];
        assert_eq!(rollup_activity(states.iter()), None);
    }

    #[test]
    fn rollup_all_none_is_none() {
        let states = vec![None, None, None];
        assert_eq!(rollup_activity(states.iter()), None);
    }

    #[test]
    fn rollup_single_working() {
        let states = vec![Some(AgentState::Working)];
        assert_eq!(rollup_activity(states.iter()), Some(AgentState::Working));
    }

    #[test]
    fn rollup_single_idle() {
        let states = vec![Some(AgentState::Idle)];
        assert_eq!(rollup_activity(states.iter()), Some(AgentState::Idle));
    }

    #[test]
    fn rollup_single_blocked() {
        let states = vec![Some(AgentState::Blocked)];
        assert_eq!(rollup_activity(states.iter()), Some(AgentState::Blocked));
    }

    #[test]
    fn rollup_blocked_beats_working() {
        let states = vec![Some(AgentState::Working), Some(AgentState::Blocked)];
        assert_eq!(rollup_activity(states.iter()), Some(AgentState::Blocked));
    }

    #[test]
    fn rollup_working_beats_idle() {
        let states = vec![Some(AgentState::Idle), Some(AgentState::Working)];
        assert_eq!(rollup_activity(states.iter()), Some(AgentState::Working));
    }

    #[test]
    fn rollup_blocked_beats_all() {
        let states = vec![
            Some(AgentState::Idle),
            Some(AgentState::Working),
            Some(AgentState::Blocked),
        ];
        assert_eq!(rollup_activity(states.iter()), Some(AgentState::Blocked));
    }

    #[test]
    fn rollup_none_entries_skipped() {
        let states = vec![None, Some(AgentState::Working), None];
        assert_eq!(rollup_activity(states.iter()), Some(AgentState::Working));
    }
}
