use crate::app_protocol::{AgentState, PaneAgentState};

const STATUS_TAIL_LINES: usize = 32;

pub(crate) fn capture_depth() -> usize {
    STATUS_TAIL_LINES
}

pub(crate) fn composite_status(
    agent: Option<&PaneAgentState>,
    captured: &[String],
) -> serde_json::Value {
    let status_bar = captured
        .iter()
        .rev()
        .find(|line| is_status_bar(line))
        .cloned();
    let status_bar_truncated = status_bar
        .as_deref()
        .is_some_and(|line| line.trim_end().ends_with('…'));
    let last_buffer_line = captured
        .iter()
        .rev()
        .find(|line| {
            !line.trim().is_empty()
                && status_bar
                    .as_deref()
                    .is_none_or(|status| line.as_str() != status)
        })
        .cloned();

    let agent_state = agent.map(|state| match state.state {
        AgentState::Working => "working",
        AgentState::Idle => "idle",
        AgentState::Blocked => "blocked",
    });
    let detail = agent.and_then(|state| state.detail.clone());

    let (verdict, confidence) = if last_buffer_line
        .as_deref()
        .is_some_and(is_tool_call_tail)
    {
        ("working", "high")
    } else {
        match agent.map(|state| &state.state) {
            Some(AgentState::Working) => ("working", "high"),
            Some(AgentState::Blocked) if detail.as_deref().is_some_and(command_detail) => {
                ("blocked", "high")
            }
            Some(AgentState::Idle)
                if status_bar.as_deref().is_some_and(|bar| {
                    !status_bar_truncated
                        && !bar.to_ascii_lowercase().contains("esc to interrupt")
                }) && last_buffer_line
                    .as_deref()
                    .is_some_and(completed_reply_tail) =>
            {
                ("idle", "high")
            }
            _ => ("unknown", "low"),
        }
    };

    serde_json::json!({
        "verdict": verdict,
        "confidence": confidence,
        "agent_state": agent_state,
        "detail": detail,
        "status_bar": status_bar,
        "status_bar_truncated": status_bar_truncated,
        "last_buffer_line": last_buffer_line,
    })
}

fn is_status_bar(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("esc to interrupt")
        || lower.contains("bypass permissions")
        || lower.contains("shift+tab")
}

fn is_tool_call_tail(line: &str) -> bool {
    let line = line.trim();
    (line.starts_with('⏺') || line.starts_with('●') || line.starts_with('•'))
        && line.contains('(')
}

fn command_detail(detail: &str) -> bool {
    let detail = detail.trim();
    !detail.is_empty()
        && (detail.contains(' ')
            || detail.contains('/')
            || detail.contains('`')
            || detail.contains('('))
}

fn completed_reply_tail(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty()
        && !line.starts_with('❯')
        && !line.starts_with('>')
        && !is_tool_call_tail(line)
        && !line.contains("Press up to edit queued messages")
        && !line.contains("paste again to expand")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(state: AgentState, detail: Option<&str>) -> PaneAgentState {
        PaneAgentState {
            pane_id: 7,
            state,
            agent: "codex".into(),
            detail: detail.map(str::to_owned),
            session_id: None,
        }
    }

    #[test]
    fn working_agent_is_believed_even_when_tail_looks_idle() {
        let state = agent(AgentState::Working, None);
        let result = composite_status(
            Some(&state),
            &["Done.".into(), "bypass permissions on (shift+tab)".into()],
        );
        assert_eq!(result["verdict"], "working");
        assert_eq!(result["confidence"], "high");
    }

    #[test]
    fn idle_requires_all_three_signals() {
        let state = agent(AgentState::Idle, None);
        let result = composite_status(
            Some(&state),
            &[
                "Implementation complete.".into(),
                "bypass permissions on (shift+tab)".into(),
            ],
        );
        assert_eq!(result["verdict"], "idle");
        assert_eq!(result["last_buffer_line"], "Implementation complete.");

        let truncated = composite_status(
            Some(&state),
            &[
                "Implementation complete.".into(),
                "bypass permissions on (shift+tab to · …".into(),
            ],
        );
        assert_eq!(truncated["verdict"], "unknown");
        assert_eq!(truncated["confidence"], "low");
        assert_eq!(truncated["status_bar_truncated"], true);
    }

    #[test]
    fn trailing_tool_call_overrides_idle_signals() {
        let state = agent(AgentState::Idle, None);
        let result = composite_status(
            Some(&state),
            &[
                "bypass permissions on (shift+tab)".into(),
                "⏺ Bash(cargo test)".into(),
            ],
        );
        assert_eq!(result["verdict"], "working");
        assert_eq!(result["confidence"], "high");
    }

    #[test]
    fn blocked_command_detail_is_surfaced() {
        let state = agent(AgentState::Blocked, Some("Bash(rm -rf /tmp/owned)"));
        let result = composite_status(Some(&state), &[]);
        assert_eq!(result["verdict"], "blocked");
        assert_eq!(result["detail"], "Bash(rm -rf /tmp/owned)");
    }

    #[test]
    fn disagreement_or_missing_signal_is_unknown_low() {
        let state = agent(AgentState::Idle, None);
        let result = composite_status(Some(&state), &["❯".into()]);
        assert_eq!(result["verdict"], "unknown");
        assert_eq!(result["confidence"], "low");
    }
}
