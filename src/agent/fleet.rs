//! Derived fleet roster shared by the host pane and CLI.

use crate::protocol::AgentState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const DEFAULT_STALE_FAILURE_HOURS: u64 = 12;

fn strip_raw_id_prefix(raw: &str) -> &str {
    let trimmed = raw.trim();
    for prefix in ["Task #", "task #", "Pane #", "pane #", "#"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let digits_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
            if digits_len > 0 {
                let after = rest[digits_len..].trim_start();
                if after.is_empty() {
                    return "";
                }
                if let Some(after_sep) = after
                    .strip_prefix('·')
                    .or_else(|| after.strip_prefix('-'))
                    .or_else(|| after.strip_prefix(':'))
                {
                    return after_sep.trim();
                }
            }
        }
    }
    trimmed
}

pub fn clean_human_title(raw: &str) -> Option<String> {
    let cleaned = strip_raw_id_prefix(raw).trim();
    if cleaned.is_empty()
        || cleaned.eq_ignore_ascii_case("unknown")
        || cleaned.starts_with("unknown (")
        || cleaned.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(cleaned.to_string())
}

pub fn resolve_agent_label(
    role: Option<&str>,
    harness: Option<&str>,
    pane_name: Option<&str>,
) -> String {
    if let Some(label) = role.and_then(clean_human_title) {
        return label;
    }
    if let Some(label) = pane_name.and_then(clean_human_title) {
        return label;
    }
    if let Some(label) = harness.and_then(clean_human_title) {
        return label;
    }
    "unknown (no agent definition or harness reported)".to_string()
}

pub fn resolve_harness_label(harness: Option<&str>, role: Option<&str>) -> String {
    if let Some(label) = harness.and_then(clean_human_title) {
        return label;
    }
    if let Some(label) = role.and_then(clean_human_title) {
        return label;
    }
    "unknown (no harness reported)".to_string()
}

pub fn resolve_task_title(
    root_label: Option<&str>,
    parent_title: Option<&str>,
    fallback_agent_label: &str,
    context_name: &str,
) -> String {
    let context = clean_human_title(context_name);
    let primary = root_label
        .and_then(clean_human_title)
        .or_else(|| parent_title.and_then(clean_human_title));
    let fallback = clean_human_title(fallback_agent_label);
    match (primary, context) {
        (Some(p), Some(c)) if p != c => format!("{p} · {c}"),
        (Some(p), _) => p,
        (None, Some(c)) => match fallback {
            Some(f) if f != c => format!("{f} · {c}"),
            _ => c,
        },
        (None, None) => fallback.unwrap_or_else(|| "Agent task".to_string()),
    }
}

pub fn format_age(now: u64, since: u64) -> String {
    if since == 0 {
        return "—".to_string();
    }
    let elapsed = now.saturating_sub(since);
    if elapsed < 60 {
        format!("{elapsed}s ago")
    } else if elapsed < 3600 {
        format!("{}m ago", elapsed / 60)
    } else if elapsed < 86400 {
        format!("{}h ago", elapsed / 3600)
    } else {
        format!("{}d ago", elapsed / 86400)
    }
}

pub fn resolve_failure_reason(
    display_state: DisplayState,
    detail: Option<&str>,
    tail: Option<&str>,
    result: Option<&serde_json::Value>,
) -> Option<String> {
    if !display_state.is_failure_like() {
        return None;
    }
    let strip_state_prefix = |s: &str| -> String {
        let t = s.trim();
        let t = t
            .strip_prefix("failed:")
            .or_else(|| t.strip_prefix("exited without report:"))
            .unwrap_or(t)
            .trim();
        t.to_string()
    };
    let classify = |raw: &str| -> String {
        let lower = raw.to_ascii_lowercase();
        if lower.contains("schema") && (lower.contains("valid") || lower.contains("failed")) {
            return format!("schema validation failure ({raw})");
        }
        if lower.contains("permission denied")
            || lower.contains("permission_denied")
            || lower.contains("denied by policy")
        {
            return if lower == "permission denied" || lower == "permission_denied" {
                "permission denied".to_string()
            } else {
                format!("permission denied ({raw})")
            };
        }
        if lower.contains("command not found")
            || lower.contains("harness not found")
            || lower.contains("not found in path")
            || lower.contains("no such file or directory")
        {
            return if lower.contains("harness not found") {
                raw.to_string()
            } else {
                format!("harness not found ({raw})")
            };
        }
        if lower.contains("timed out") || lower.contains("timeout") {
            return if lower == "timeout" || lower == "timed out" {
                "timeout".to_string()
            } else {
                format!("timeout ({raw})")
            };
        }
        raw.to_string()
    };

    if let Some(d) = detail.map(strip_state_prefix).filter(|s| !s.is_empty()) {
        return Some(classify(&d));
    }
    if let Some(res) = result {
        for key in ["error", "reason", "message", "summary"] {
            if let Some(val) = res.get(key).and_then(|v| v.as_str()).map(str::trim) {
                if !val.is_empty() {
                    return Some(classify(&strip_state_prefix(val)));
                }
            }
        }
    }
    if let Some(t) = tail.map(strip_state_prefix).filter(|s| !s.is_empty()) {
        if let Some(last_line) = t.lines().rev().map(str::trim).find(|line| !line.is_empty()) {
            let truncated: String = last_line.chars().take(96).collect();
            return Some(classify(&truncated));
        }
    }
    Some(match display_state {
        DisplayState::Failed => "failed (no exit code or error detail recorded)".to_string(),
        DisplayState::Stalled => "timeout (idle past stall threshold without report)".to_string(),
        DisplayState::ExitedUnreported => "exited (0) without calling plexi agent done".to_string(),
        _ => "failed".to_string(),
    })
}

pub fn resolve_next_hint(
    pane_id: u64,
    display_state: DisplayState,
    live: bool,
    needs_you: bool,
) -> Option<String> {
    match display_state {
        DisplayState::Failed | DisplayState::Stalled | DisplayState::ExitedUnreported => {
            if live {
                Some(format!(
                    "plexi pane focus {pane_id} · plexi agent fleet dismiss {pane_id}"
                ))
            } else {
                Some(format!(
                    "plexi agent reopen {pane_id} · plexi agent fleet dismiss {pane_id}"
                ))
            }
        }
        DisplayState::Blocked if needs_you => Some(format!("plexi pane focus {pane_id}")),
        DisplayState::Blocked => Some(format!("plexi pane focus {pane_id}")),
        _ => None,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlockedOn {
    Human {
        question: String,
    },
    Agent {
        pane_id: u64,
        role: String,
        question: String,
    },
    Permission {
        summary: String,
    },
    Message {
        from_pane: u64,
    },
    Unknown,
}
impl BlockedOn {
    pub fn label(&self) -> String {
        match self {
            Self::Human { question } => format!("Waiting for you: {question}"),
            Self::Agent {
                pane_id,
                role,
                question,
            } => format!("Waiting for {role} (#{pane_id}): {question}"),
            Self::Permission { summary } => format!("Approval: {summary}"),
            Self::Message { from_pane } => format!("Reply to pane #{from_pane}"),
            Self::Unknown => "blocked (no reason reported)".into(),
        }
    }
}

/// Shared projection of the notification queue's agent events. No second inbox.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentEvent {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayState {
    Idle,
    Working,
    Blocked,
    DoneUnseen,
    Stalled,
    Failed,
    ExitedUnreported,
}
impl DisplayState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::DoneUnseen => "done-unseen",
            Self::Stalled => "stalled",
            Self::Failed => "failed",
            Self::ExitedUnreported => "exited without report",
        }
    }
    pub fn is_failure_like(self) -> bool {
        matches!(self, Self::Stalled | Self::Failed | Self::ExitedUnreported)
    }
    fn rank(self, needs_you: bool) -> u8 {
        match self {
            Self::Blocked if needs_you => 0,
            Self::Blocked => 1,
            Self::Stalled | Self::Failed | Self::ExitedUnreported => 2,
            Self::DoneUnseen => 3,
            Self::Working => 4,
            Self::Idle => 5,
        }
    }
}
pub use crate::protocol::FleetFilter;
impl FleetFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::NeedsAttention => "Needs attention",
            Self::Working => "Working",
            Self::Done => "Done",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::NeedsAttention,
            Self::NeedsAttention => Self::Working,
            Self::Working => Self::Done,
            Self::Done => Self::All,
        }
    }
    pub(crate) fn matches(self, row: &AgentRow) -> bool {
        match self {
            Self::All => true,
            Self::NeedsAttention => row.display_state.rank(row.needs_you) <= 3,
            Self::Working => row.display_state == DisplayState::Working,
            Self::Done => row.completed || row.display_state == DisplayState::DoneUnseen,
        }
    }
}
#[derive(Clone, Debug)]
pub struct AgentInput {
    pub live: bool,
    pub pane_id: u64,
    pub parent_pane_id: Option<u64>,
    pub spawned_by: Option<String>,
    pub pending_child_results_count: usize,
    pub role: String,
    pub harness: String,
    pub pane_name: Option<String>,
    pub parent_title: Option<String>,
    pub context_name: String,
    pub state: AgentState,
    pub since: u64,
    pub ephemeral: bool,
    pub seen: bool,
    pub dismissed: bool,
    pub previous_session: bool,
    pub done_unseen: bool,
    pub blocked_on: Option<BlockedOn>,
    pub failure_detail: Option<String>,
    pub tail: Option<String>,
    pub result: Option<serde_json::Value>,
    pub events: Vec<AgentEvent>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRow {
    pub live: bool,
    pub pane_id: u64,
    pub parent_pane_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_by: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub pending_child_results_count: usize,
    pub role: String,
    pub harness: String,
    pub context_name: String,
    #[serde(default)]
    pub task_title: String,
    pub display_state: DisplayState,
    pub needs_you: bool,
    pub since: u64,
    #[serde(default)]
    pub age: String,
    pub seen: bool,
    #[serde(default)]
    pub dismissed: bool,
    #[serde(default)]
    pub previous_session: bool,
    pub completed: bool,
    pub blocked_on: Option<BlockedOn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_hint: Option<String>,
    pub tail: Option<String>,
    pub result: Option<serde_json::Value>,
    pub events: Vec<AgentEvent>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTask {
    pub root_pane_id: u64,
    pub title: String,
    pub display_state: DisplayState,
    pub needs_you: bool,
    pub since: u64,
    pub rows: Vec<AgentRow>,
}
impl AgentTask {
    pub fn counts(&self) -> String {
        [
            DisplayState::Working,
            DisplayState::Blocked,
            DisplayState::Stalled,
            DisplayState::Failed,
            DisplayState::ExitedUnreported,
            DisplayState::DoneUnseen,
            DisplayState::Idle,
        ]
        .into_iter()
        .filter_map(|state| {
            let n = self
                .rows
                .iter()
                .filter(|r| r.display_state == state)
                .count();
            (n > 0).then(|| format!("{n} {}", state.label()))
        })
        .collect::<Vec<_>>()
        .join(" · ")
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentsView {
    pub tasks: Vec<AgentTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub older: Vec<AgentRow>,
}
impl AgentsView {
    #[cfg(test)]
    pub fn build(
        input: Vec<AgentInput>,
        now: u64,
        stalled_after_secs: u64,
        focused: Option<u64>,
        filter: FleetFilter,
    ) -> Self {
        Self::build_with_stale_hours(
            input,
            now,
            stalled_after_secs,
            DEFAULT_STALE_FAILURE_HOURS,
            focused,
            filter,
        )
    }

    pub fn build_with_stale_hours(
        input: Vec<AgentInput>,
        now: u64,
        stalled_after_secs: u64,
        stale_failure_hours: u64,
        focused: Option<u64>,
        filter: FleetFilter,
    ) -> Self {
        let stale_failure_secs = stale_failure_hours.saturating_mul(3600);
        let parents: BTreeMap<_, _> = input
            .iter()
            .map(|r| (r.pane_id, r.parent_pane_id))
            .collect();
        let root_labels: BTreeMap<_, _> = input
            .iter()
            .map(|r| {
                let resolved_role =
                    resolve_agent_label(Some(&r.role), Some(&r.harness), r.pane_name.as_deref());
                (r.pane_id, (resolved_role, r.context_name.clone()))
            })
            .collect();
        let has_children: HashSet<_> = input.iter().filter_map(|r| r.parent_pane_id).collect();
        let mut groups: BTreeMap<u64, Vec<AgentRow>> = BTreeMap::new();
        let mut older: Vec<AgentRow> = Vec::new();
        for r in input {
            if r.dismissed {
                continue;
            }
            let mut root = r.pane_id;
            let mut visited = HashSet::new();
            while visited.insert(root) {
                match parents.get(&root).copied().flatten() {
                    Some(parent) if !visited.contains(&parent) => root = parent,
                    _ => break,
                }
            }
            // A root with children labels its task; workers are its rows.
            if root == r.pane_id && has_children.contains(&root) {
                continue;
            }
            let seen = r.seen || focused == Some(r.pane_id);
            let needs_you = r.state == AgentState::NeedsYou;
            let completed = r.state == AgentState::Completed;
            let display_state = match r.state {
                AgentState::NeedsYou | AgentState::Blocked => DisplayState::Blocked,
                AgentState::Interrupted => DisplayState::Stalled,
                AgentState::Failed => DisplayState::Failed,
                AgentState::ExitedUnreported => DisplayState::ExitedUnreported,
                AgentState::Completed if !seen => DisplayState::DoneUnseen,
                AgentState::Idle if r.done_unseen && !seen => DisplayState::DoneUnseen,
                AgentState::Idle
                    if r.ephemeral
                        && !r.done_unseen
                        && r.blocked_on.is_none()
                        && now.saturating_sub(r.since) > stalled_after_secs =>
                {
                    DisplayState::Stalled
                }
                AgentState::Working => DisplayState::Working,
                _ => DisplayState::Idle,
            };
            let blocked_on = if display_state == DisplayState::Blocked {
                Some(r.blocked_on.unwrap_or(BlockedOn::Unknown))
            } else {
                r.blocked_on
            };
            let role = resolve_agent_label(Some(&r.role), Some(&r.harness), r.pane_name.as_deref());
            let harness = resolve_harness_label(Some(&r.harness), Some(&r.role));
            let root_label = root_labels.get(&root).map(|(lbl, _)| lbl.as_str());
            let task_title = resolve_task_title(
                root_label,
                r.parent_title.as_deref(),
                &role,
                &r.context_name,
            );
            let age = format_age(now, r.since);
            let reason = resolve_failure_reason(
                display_state,
                r.failure_detail.as_deref(),
                r.tail.as_deref(),
                r.result.as_ref(),
            );
            let next_hint = resolve_next_hint(r.pane_id, display_state, r.live, needs_you);
            let is_stale_failure = display_state.is_failure_like()
                && (!r.live
                    || r.previous_session
                    || (r.since > 0 && now.saturating_sub(r.since) > stale_failure_secs));
            let row = AgentRow {
                live: r.live,
                pane_id: r.pane_id,
                parent_pane_id: r.parent_pane_id,
                spawned_by: r.spawned_by,
                pending_child_results_count: r.pending_child_results_count,
                role,
                harness,
                context_name: r.context_name,
                task_title,
                display_state,
                needs_you,
                since: r.since,
                age,
                seen,
                dismissed: r.dismissed,
                previous_session: r.previous_session,
                completed,
                blocked_on,
                reason,
                next_hint,
                tail: r.tail,
                result: r.result,
                events: if focused == Some(r.pane_id) {
                    vec![]
                } else {
                    r.events
                },
            };
            if filter.matches(&row) {
                if is_stale_failure {
                    older.push(row);
                } else {
                    groups.entry(root).or_default().push(row);
                }
            }
        }
        older.sort_by_key(|r| (std::cmp::Reverse(r.since), r.pane_id));
        let mut tasks: Vec<_> = groups
            .into_iter()
            .map(|(root, mut rows)| {
                rows.sort_by_key(|r| {
                    (
                        r.display_state.rank(r.needs_you),
                        std::cmp::Reverse(r.since),
                        r.pane_id,
                    )
                });
                let title = root_labels
                    .get(&root)
                    .map(|(lbl, ctx)| resolve_task_title(Some(lbl), None, lbl, ctx))
                    .unwrap_or_else(|| rows[0].task_title.clone());
                AgentTask {
                    root_pane_id: root,
                    title,
                    display_state: rows[0].display_state,
                    needs_you: rows[0].needs_you,
                    since: rows.iter().map(|r| r.since).max().unwrap_or(0),
                    rows,
                }
            })
            .collect();
        tasks.sort_by_key(|t| {
            (
                t.display_state.rank(t.needs_you),
                std::cmp::Reverse(t.since),
                t.root_pane_id,
            )
        });
        Self { tasks, older }
    }
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AgentState;

    fn row(id: u64, state: AgentState) -> AgentInput {
        AgentInput {
            live: true,
            pane_id: id,
            parent_pane_id: None,
            spawned_by: None,
            pending_child_results_count: 0,
            role: format!("worker-{id}"),
            harness: "pi".into(),
            pane_name: None,
            parent_title: None,
            context_name: "Task".into(),
            state,
            since: 100,
            ephemeral: false,
            seen: false,
            dismissed: false,
            previous_session: false,
            done_unseen: false,
            blocked_on: None,
            failure_detail: None,
            tail: None,
            result: None,
            events: vec![],
        }
    }
    fn view(rows: Vec<AgentInput>) -> AgentsView {
        AgentsView::build(rows, 221, 120, None, FleetFilter::All)
    }
    #[test]
    fn e1_tree_rollup_and_counts() {
        let parent = row(1, AgentState::Working);
        let mut children = vec![
            row(2, AgentState::Working),
            row(3, AgentState::Blocked),
            row(4, AgentState::Completed),
        ];
        for child in &mut children {
            child.parent_pane_id = Some(1);
        }
        children[1].blocked_on = Some(BlockedOn::Agent {
            pane_id: 1,
            role: "lead".into(),
            question: "which branch?".into(),
        });
        let v = view(std::iter::once(parent).chain(children).collect());
        assert_eq!(v.tasks.len(), 1);
        assert_eq!(v.tasks[0].rows.len(), 3);
        assert_eq!(v.tasks[0].display_state, DisplayState::Blocked);
        assert_eq!(v.tasks[0].counts(), "1 working · 1 blocked · 1 done-unseen");
    }
    #[test]
    fn e2_reasons_preserve_question_permission_and_unknown() {
        let mut child = row(2, AgentState::Blocked);
        child.parent_pane_id = Some(1);
        child.blocked_on = Some(BlockedOn::Agent {
            pane_id: 1,
            role: "lead".into(),
            question: "which branch?".into(),
        });
        let mut permission = row(3, AgentState::NeedsYou);
        permission.blocked_on = Some(BlockedOn::Permission {
            summary: "Allow read".into(),
        });
        let v = view(vec![child, permission, row(4, AgentState::Blocked)]);
        let rows: Vec<_> = v.tasks.iter().flat_map(|t| &t.rows).collect();
        assert!(rows.iter().any(|r| matches!(&r.blocked_on, Some(BlockedOn::Agent { pane_id: 1, question, .. }) if question == "which branch?")));
        assert!(rows.iter().any(|r| matches!(&r.blocked_on, Some(BlockedOn::Permission { summary }) if summary == "Allow read")));
        assert_eq!(
            rows.iter()
                .find(|r| r.pane_id == 4)
                .unwrap()
                .blocked_on
                .as_ref()
                .unwrap()
                .label(),
            "blocked (no reason reported)"
        );
    }
    #[test]
    fn e3_stalled_threshold_from_config() {
        let cfg: crate::config::AgentsConfig = toml::from_str("stalled_after_secs = 120").unwrap();
        let mut idle = row(1, AgentState::Idle);
        idle.ephemeral = true;
        for (now, expected) in [(219, DisplayState::Idle), (221, DisplayState::Stalled)] {
            let v = AgentsView::build(
                vec![idle.clone()],
                now,
                cfg.stalled_after_secs.unwrap_or(120),
                None,
                FleetFilter::All,
            );
            assert_eq!(v.tasks[0].display_state, expected);
        }
        assert_eq!(
            view(vec![row(2, AgentState::Interrupted)]).tasks[0].display_state,
            DisplayState::Stalled
        );
    }
    #[test]
    fn e4_focus_and_seen_completion() {
        let completed = row(1, AgentState::Completed);
        assert_eq!(
            view(vec![completed.clone()]).tasks[0].display_state,
            DisplayState::DoneUnseen
        );
        let focused =
            AgentsView::build(vec![completed.clone()], 221, 120, Some(1), FleetFilter::All);
        assert!(focused.tasks[0].rows[0].seen);
        assert_eq!(focused.tasks[0].display_state, DisplayState::Idle);
        let mut seen = completed;
        seen.seen = true;
        assert_eq!(view(vec![seen]).tasks[0].display_state, DisplayState::Idle);
        let mut idle = row(2, AgentState::Idle);
        idle.done_unseen = true;
        assert_eq!(
            view(vec![idle]).tasks[0].display_state,
            DisplayState::DoneUnseen
        );
    }
    #[test]
    fn e5_attention_sort_and_recent_ties() {
        let mut recent = row(5, AgentState::Working);
        recent.since = 200;
        let v = view(vec![
            row(1, AgentState::Idle),
            row(2, AgentState::Working),
            row(3, AgentState::Completed),
            row(4, AgentState::Blocked),
            recent,
        ]);
        assert_eq!(
            v.tasks.iter().map(|t| t.root_pane_id).collect::<Vec<_>>(),
            vec![4, 3, 5, 2, 1]
        );
    }
    #[test]
    fn e6_needs_you_outranks_blocked() {
        let v = view(vec![
            row(1, AgentState::Blocked),
            row(2, AgentState::NeedsYou),
        ]);
        assert_eq!(v.tasks[0].root_pane_id, 2);
        let mut child = row(3, AgentState::NeedsYou);
        child.parent_pane_id = Some(1);
        let mut sibling = row(4, AgentState::Blocked);
        sibling.parent_pane_id = Some(1);
        let v = view(vec![row(1, AgentState::Working), sibling, child]);
        assert!(v.tasks[0].needs_you);
        assert_eq!(v.tasks[0].rows[0].pane_id, 3);
    }
    #[test]
    fn e7_seen_round_trip_in_saved_session() {
        let raw = serde_json::json!({"harness":"pi", "last_state":"completed", "fleet_seen":false, "fleet_done_unseen":true, "fleet_since":100, "fleet_result":{"ok":true}});
        let session: crate::workspace::SavedAgentSession = serde_json::from_value(raw).unwrap();
        let restored: crate::workspace::SavedAgentSession =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert!(!restored.fleet_seen);
        assert!(restored.fleet_done_unseen);
        assert_eq!(restored.fleet_since, 100);
        assert_eq!(restored.fleet_result, Some(serde_json::json!({"ok":true})));
    }
    #[test]
    fn e8_focused_agent_event_suppressed() {
        let mut agent = row(1, AgentState::Blocked);
        agent.events.push(AgentEvent {
            title: "Blocked".into(),
            body: "which branch?".into(),
        });
        assert_eq!(view(vec![agent.clone()]).tasks[0].rows[0].events.len(), 1);
        let v = AgentsView::build(vec![agent], 221, 120, Some(1), FleetFilter::All);
        assert!(v.tasks[0].rows[0].events.is_empty());
    }

    #[test]
    fn e9_human_label_resolution_and_no_raw_task_ids() {
        // Fallback to harness when role is "unknown", and pane_name when role is absent.
        let mut r1 = row(229, AgentState::Working);
        r1.role = "unknown".into();
        r1.harness = "claude-code".into();
        r1.parent_pane_id = Some(239);
        r1.context_name = "Task #239 · agent-tools-239".into();

        let mut r2 = row(227, AgentState::Working);
        r2.role = "unknown".into();
        r2.harness = "unknown".into();
        r2.pane_name = Some("probe".into());
        r2.context_name = "agent-tools-239".into();

        let mut r3 = row(230, AgentState::Working);
        r3.role = "unknown".into();
        r3.harness = "unknown".into();
        r3.pane_name = None;

        let v = view(vec![r1, r2, r3]);
        let task_239 = v.tasks.iter().find(|t| t.root_pane_id == 239).unwrap();
        assert!(
            !task_239.title.contains("Task #239"),
            "task title must not use raw Task #<id>: {}",
            task_239.title
        );
        assert_eq!(task_239.title, "claude-code · agent-tools-239");
        assert_eq!(task_239.rows[0].role, "claude-code");

        let row_227 = v
            .tasks
            .iter()
            .flat_map(|t| &t.rows)
            .find(|r| r.pane_id == 227)
            .unwrap();
        assert_eq!(row_227.role, "probe");

        let row_230 = v
            .tasks
            .iter()
            .flat_map(|t| &t.rows)
            .find(|r| r.pane_id == 230)
            .unwrap();
        assert_eq!(
            row_230.role,
            "unknown (no agent definition or harness reported)"
        );
    }

    #[test]
    fn e10_age_bucketing_stale_and_previous_session_failures_and_dismiss() {
        let now = 100_000;
        let mut recent_fail = row(10, AgentState::Failed);
        recent_fail.since = now - 3600; // 1h ago
        recent_fail.failure_detail = Some("exited with code 1".into());

        let mut stale_fail = row(215, AgentState::Failed);
        stale_fail.since = now - (13 * 3600); // 13h ago (> 12h default)
        stale_fail.failure_detail = Some("command not found: twcodex".into());

        let mut prev_session_fail = row(209, AgentState::Failed);
        prev_session_fail.since = now - 600; // 10m ago, but from previous host session
        prev_session_fail.previous_session = true;

        let mut dismissed_fail = row(240, AgentState::Failed);
        dismissed_fail.since = now - 600;
        dismissed_fail.dismissed = true;

        let v = AgentsView::build_with_stale_hours(
            vec![recent_fail, stale_fail, prev_session_fail, dismissed_fail],
            now,
            120,
            12,
            None,
            FleetFilter::All,
        );
        let active_ids: Vec<_> = v
            .tasks
            .iter()
            .flat_map(|t| &t.rows)
            .map(|r| r.pane_id)
            .collect();
        assert_eq!(active_ids, vec![10]);
        assert_eq!(v.tasks[0].rows[0].age, "1h ago");
        assert_eq!(
            v.tasks[0].rows[0].reason.as_deref(),
            Some("exited with code 1")
        );
        assert!(v.tasks[0].rows[0]
            .next_hint
            .as_deref()
            .unwrap()
            .contains("plexi agent fleet dismiss 10"));

        let older_ids: Vec<_> = v.older.iter().map(|r| r.pane_id).collect();
        assert_eq!(older_ids, vec![209, 215]);
        let row_215 = v.older.iter().find(|r| r.pane_id == 215).unwrap();
        assert_eq!(row_215.age, "13h ago");
        assert_eq!(
            row_215.reason.as_deref(),
            Some("harness not found (command not found: twcodex)")
        );
        let row_209 = v.older.iter().find(|r| r.pane_id == 209).unwrap();
        assert_eq!(
            row_209.reason.as_deref(),
            Some("failed (no exit code or error detail recorded)")
        );
    }

    #[test]
    fn e11_failure_reason_mapping_covers_schema_permission_timeout_and_unreported() {
        assert_eq!(
            resolve_failure_reason(
                DisplayState::Failed,
                Some("schema validation failed: missing field `status`"),
                None,
                None,
            )
            .as_deref(),
            Some("schema validation failure (schema validation failed: missing field `status`)")
        );
        assert_eq!(
            resolve_failure_reason(
                DisplayState::Failed,
                None,
                Some("Error: permission denied reading secret"),
                None,
            )
            .as_deref(),
            Some("permission denied (Error: permission denied reading secret)")
        );
        assert_eq!(
            resolve_failure_reason(DisplayState::Stalled, None, None, None).as_deref(),
            Some("timeout (idle past stall threshold without report)")
        );
        assert_eq!(
            resolve_failure_reason(DisplayState::ExitedUnreported, None, None, None).as_deref(),
            Some("exited (0) without calling plexi agent done")
        );
    }
}
