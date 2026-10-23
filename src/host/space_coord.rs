//! Nested space coordinates.
//!
//! A pane can contain a space. The coordinate of a pane is the path of human
//! names from the root context down to that pane. Team runs, agent calls, and
//! pane-created sub-contexts all name their spaces through [`resolve_space_title`]
//! and summarize them through [`SpaceFace`].

/// Separator between coordinate segments. U+203A, with spaces, as in
/// `PLEXI › researcher › helper-2`.
pub const COORD_SEP: &str = " \u{203a} ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrgentKind {
    NeedsYou,
    Blocked,
    Error,
    Working,
    Done,
    Idle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UrgentCounts {
    pub panes: usize,
    pub needs_you: usize,
    pub blocked: usize,
    pub error: usize,
    pub working: usize,
    pub done: usize,
}

/// Compact live summary of a space: a human title, a pane count, and the
/// single most urgent state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceFace {
    pub title: String,
    pub pane_count: usize,
    pub urgent: UrgentKind,
    pub urgent_count: usize,
}

impl SpaceFace {
    pub fn from_counts(title: impl Into<String>, counts: UrgentCounts) -> Self {
        let raw = title.into();
        let title = raw.trim();
        let title = if title.is_empty() {
            "space".to_string()
        } else {
            title.to_string()
        };
        let (urgent, urgent_count) = if counts.needs_you > 0 {
            (UrgentKind::NeedsYou, counts.needs_you)
        } else if counts.blocked > 0 {
            (UrgentKind::Blocked, counts.blocked)
        } else if counts.error > 0 {
            (UrgentKind::Error, counts.error)
        } else if counts.working > 0 {
            (UrgentKind::Working, counts.working)
        } else if counts.done > 0 {
            (UrgentKind::Done, counts.done)
        } else {
            (UrgentKind::Idle, 0)
        };
        Self {
            title,
            pane_count: counts.panes,
            urgent,
            urgent_count,
        }
    }

    pub fn summary_line(&self) -> String {
        format!(
            "{} · {}",
            count_phrase(self.pane_count, "pane", "panes"),
            self.urgent_phrase()
        )
    }

    /// Title and summary, the string tests and the accessibility label share.
    pub fn label(&self) -> String {
        format!("{} · {}", self.title, self.summary_line())
    }

    fn urgent_phrase(&self) -> String {
        let n = self.urgent_count;
        match self.urgent {
            UrgentKind::NeedsYou => {
                if n == 1 {
                    "1 needs you".to_string()
                } else {
                    format!("{n} need you")
                }
            }
            UrgentKind::Blocked => count_phrase(n.max(1), "blocked", "blocked"),
            UrgentKind::Error => count_phrase(n.max(1), "error", "errors"),
            UrgentKind::Working => count_phrase(n.max(1), "working", "working"),
            UrgentKind::Done => count_phrase(n.max(1), "done", "done"),
            UrgentKind::Idle => "idle".to_string(),
        }
    }
}

pub struct SpaceTitleInput<'a> {
    pub is_root: bool,
    pub context_name: &'a str,
    pub team_name: Option<&'a str>,
    pub agent_names: &'a [String],
    pub task: Option<&'a str>,
}

/// Human name for a space. Never returns a raw id (`team-<hex>`,
/// `agent-tools-<digits>`, or a UUID).
pub fn resolve_space_title(input: SpaceTitleInput<'_>) -> String {
    if input.is_root {
        let name = input.context_name.trim();
        return if name.is_empty() {
            "PLEXI".to_string()
        } else {
            name.to_string()
        };
    }
    if let Some(team) = input
        .team_name
        .map(str::trim)
        .filter(|name| !name.is_empty() && !is_machine_id(name))
    {
        return team.to_string();
    }
    human_fallback(input.context_name, input.agent_names, input.task)
}

pub fn is_machine_id(name: &str) -> bool {
    let name = name.trim();
    if let Some(rest) = name.strip_prefix("team-") {
        return rest.len() >= 8 && rest.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some(rest) = name.strip_prefix("agent-tools-") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    }
    is_uuid(name)
}

/// Root-first space names, then the focused pane when it adds a new segment.
pub fn coordinate_segments(spaces_root_first: &[String], focused: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    for segment in spaces_root_first {
        push_unique(&mut out, segment);
    }
    if let Some(focused) = focused {
        push_unique(&mut out, focused);
    }
    out
}

pub fn join_coordinate(segments: &[String]) -> String {
    segments.join(COORD_SEP)
}

fn human_fallback(context_name: &str, agent_names: &[String], task: Option<&str>) -> String {
    let context_name = context_name.trim();
    if !context_name.is_empty() && !is_machine_id(context_name) {
        return context_name.to_string();
    }
    let mut unique = Vec::new();
    for name in agent_names {
        push_unique(&mut unique, name);
    }
    if unique.len() == 1 {
        return unique.pop().unwrap();
    }
    if let Some(derived) = task.and_then(task_derived_name) {
        return derived;
    }
    if let Some(first) = unique.first() {
        return first.clone();
    }
    "space".to_string()
}

fn task_derived_name(task: &str) -> Option<String> {
    let mut words = Vec::new();
    for word in task.split_whitespace() {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
        if word.is_empty() || is_machine_id(word) || word.starts_with("PLEXI_") {
            continue;
        }
        if word.eq_ignore_ascii_case("role") {
            continue;
        }
        words.push(word);
        if words.len() == 4 {
            break;
        }
    }
    if words.is_empty() {
        None
    } else {
        Some(words.join(" "))
    }
}

fn push_unique(out: &mut Vec<String>, raw: &str) {
    let text = raw.trim();
    if text.is_empty() || is_machine_id(text) {
        return;
    }
    if out
        .last()
        .is_some_and(|prev| prev.eq_ignore_ascii_case(text))
    {
        return;
    }
    out.push(text.to_string());
}

fn count_phrase(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn is_uuid(name: &str) -> bool {
    let hex: String = name.chars().filter(|c| *c != '-').collect();
    let dashed = name.chars().filter(|c| *c == '-').count() == 4
        && hex.len() == 32
        && hex.chars().all(|c| c.is_ascii_hexdigit());
    let bare = name.len() == 32 && name.chars().all(|c| c.is_ascii_hexdigit());
    dashed || bare
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title<'a>(
        is_root: bool,
        context_name: &'a str,
        team_name: Option<&'a str>,
        agent_names: &'a [String],
        task: Option<&'a str>,
    ) -> String {
        resolve_space_title(SpaceTitleInput {
            is_root,
            context_name,
            team_name,
            agent_names,
            task,
        })
    }

    #[test]
    fn machine_ids_are_team_uuids_agent_tools_and_uuids() {
        assert!(is_machine_id("team-017f341796ab4976bee42030d6f5a6c0"));
        assert!(is_machine_id("agent-tools-42"));
        assert!(is_machine_id("017f3417-96ab-4976-bee4-2030d6f5a6c0"));
        assert!(!is_machine_id("researcher"));
        assert!(!is_machine_id("agent-tools-preview"));
        assert!(!is_machine_id("helper-2"));
        assert!(!is_machine_id("PLEXI"));
    }

    #[test]
    fn titles_prefer_team_name_then_agent_then_task_never_a_raw_id() {
        let raw = "team-017f341796ab4976bee42030d6f5a6c0";
        assert_eq!(title(false, raw, Some("design"), &[], None), "design");
        let agents = vec!["Researcher".to_string()];
        assert_eq!(
            title(false, "agent-tools-7", None, &agents, Some("ignored")),
            "Researcher"
        );
        assert_eq!(
            title(
                false,
                "agent-tools-7",
                None,
                &[],
                Some("Audit the auth module")
            ),
            "Audit the auth module"
        );
        assert_eq!(
            title(false, "auth-audit", None, &agents, None),
            "auth-audit"
        );
        assert_eq!(title(true, "", None, &[], None), "PLEXI");
        assert_eq!(title(true, "PLEXI", None, &agents, None), "PLEXI");
        let named = title(false, raw, Some("design"), &agents, None);
        assert!(!named.contains("team-"));
        assert!(!is_machine_id(&named));
    }

    #[test]
    fn breadcrumb_is_the_path_in_human_names() {
        let spaces = vec!["PLEXI".to_string(), "researcher".to_string()];
        let segments = coordinate_segments(&spaces, Some("helper-2"));
        assert_eq!(
            join_coordinate(&segments),
            "PLEXI \u{203a} researcher \u{203a} helper-2"
        );
        let same = coordinate_segments(&spaces, Some("researcher"));
        assert_eq!(join_coordinate(&same), "PLEXI \u{203a} researcher");
        let raw = coordinate_segments(
            &[
                "PLEXI".into(),
                "team-017f341796ab4976bee42030d6f5a6c0".into(),
            ],
            Some("helper-2"),
        );
        assert_eq!(join_coordinate(&raw), "PLEXI \u{203a} helper-2");
    }

    #[test]
    fn team_and_agent_spaces_share_one_summary() {
        let counts = UrgentCounts {
            panes: 4,
            needs_you: 0,
            blocked: 1,
            error: 0,
            working: 2,
            done: 0,
        };
        let team = SpaceFace::from_counts("design", counts);
        let agent = SpaceFace::from_counts("researcher", counts);
        assert_eq!(team.summary_line(), agent.summary_line());
        assert_eq!(team.summary_line(), "4 panes · 1 blocked");
        assert_eq!(team.urgent, UrgentKind::Blocked);
        assert_eq!(agent.label(), "researcher · 4 panes · 1 blocked");

        let needs = SpaceFace::from_counts(
            "researcher",
            UrgentCounts {
                panes: 2,
                needs_you: 1,
                blocked: 1,
                ..UrgentCounts::default()
            },
        );
        assert_eq!(needs.summary_line(), "2 panes · 1 needs you");
        let idle = SpaceFace::from_counts(
            "notes",
            UrgentCounts {
                panes: 1,
                ..UrgentCounts::default()
            },
        );
        assert_eq!(idle.summary_line(), "1 pane · idle");
    }
}
