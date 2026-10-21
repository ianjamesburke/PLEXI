//! Derived fleet roster shared by the host pane and CLI.

use crate::protocol::AgentState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

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
}
impl DisplayState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::DoneUnseen => "done-unseen",
            Self::Stalled => "stalled",
        }
    }
    fn rank(self, needs_you: bool) -> u8 {
        match self {
            Self::Blocked if needs_you => 0,
            Self::Blocked => 1,
            Self::Stalled => 2,
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
    pub role: String,
    pub harness: String,
    pub context_name: String,
    pub state: AgentState,
    pub since: u64,
    pub ephemeral: bool,
    pub seen: bool,
    pub done_unseen: bool,
    pub blocked_on: Option<BlockedOn>,
    pub tail: Option<String>,
    pub result: Option<serde_json::Value>,
    pub events: Vec<AgentEvent>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRow {
    pub live: bool,
    pub pane_id: u64,
    pub parent_pane_id: Option<u64>,
    pub role: String,
    pub harness: String,
    pub context_name: String,
    pub display_state: DisplayState,
    pub needs_you: bool,
    pub since: u64,
    pub seen: bool,
    pub completed: bool,
    pub blocked_on: Option<BlockedOn>,
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
}
impl AgentsView {
    pub fn build(
        input: Vec<AgentInput>,
        now: u64,
        stalled_after_secs: u64,
        focused: Option<u64>,
        filter: FleetFilter,
    ) -> Self {
        let parents: BTreeMap<_, _> = input
            .iter()
            .map(|r| (r.pane_id, r.parent_pane_id))
            .collect();
        let titles: BTreeMap<_, _> = input
            .iter()
            .map(|r| (r.pane_id, format!("{} · {}", r.role, r.context_name)))
            .collect();
        let has_children: HashSet<_> = input.iter().filter_map(|r| r.parent_pane_id).collect();
        let mut groups: BTreeMap<u64, Vec<AgentRow>> = BTreeMap::new();
        for r in input {
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
            let row = AgentRow {
                live: r.live,
                pane_id: r.pane_id,
                parent_pane_id: r.parent_pane_id,
                role: r.role,
                harness: r.harness,
                context_name: r.context_name,
                display_state,
                needs_you,
                since: r.since,
                seen,
                completed,
                blocked_on,
                tail: r.tail,
                result: r.result,
                events: if focused == Some(r.pane_id) {
                    vec![]
                } else {
                    r.events
                },
            };
            if filter.matches(&row) {
                groups.entry(root).or_default().push(row);
            }
        }
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
                AgentTask {
                    root_pane_id: root,
                    title: titles
                        .get(&root)
                        .cloned()
                        .unwrap_or_else(|| format!("Task #{root} · {}", rows[0].context_name)),
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
        Self { tasks }
    }
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
            role: format!("worker-{id}"),
            harness: "pi".into(),
            context_name: "Task".into(),
            state,
            since: 100,
            ephemeral: false,
            seen: false,
            done_unseen: false,
            blocked_on: None,
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
        let raw = serde_json::json!({"harness":"pi", "last_state":"completed", "fleet_seen":false, "fleet_done_unseen":true, "fleet_since":100});
        let session: crate::workspace::SavedAgentSession = serde_json::from_value(raw).unwrap();
        let restored: crate::workspace::SavedAgentSession =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert!(!restored.fleet_seen);
        assert!(restored.fleet_done_unseen);
        assert_eq!(restored.fleet_since, 100);
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
}
