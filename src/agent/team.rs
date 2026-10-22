//! Stored team v0: independent ballots over one option set, then Borda.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Team {
    pub version: u32,
    pub options: Vec<String>,
    pub members: Vec<Member>,
    pub tie_break: Option<String>,
    pub quorum: Option<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Member {
    pub role: String,
    pub harness: String,
    #[serde(default)]
    pub grants: Vec<String>,
    pub command: Option<String>,
    pub model: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ballot {
    pub ranking: Vec<String>,
    pub rationale: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn borda_requires_quorum_and_breaks_ties_conservatively() {
        let team = builtin();
        let options = team.options.clone();
        let ballots: Vec<_> = (0..3)
            .map(|i| {
                let mut ranking = options.clone();
                ranking.rotate_left(i);
                Ballot {
                    ranking,
                    rationale: format!("Reason {i}"),
                }
            })
            .collect();
        assert_eq!(
            aggregate(&team, &ballots).unwrap(),
            vec![
                (options[2].clone(), 3),
                (options[0].clone(), 3),
                (options[1].clone(), 3)
            ]
        );
        assert!(aggregate(&team, &ballots[..2]).is_err());
        let mut malformed = ballots;
        malformed[0].ranking[1] = options[0].clone();
        assert!(aggregate(&team, &malformed).is_err());
    }
    #[test]
    fn team_policy_fields_validate_and_old_files_still_load() {
        let raw = toml::to_string(&builtin()).unwrap();
        assert!(toml::from_str::<Team>(&raw).unwrap().validate().is_ok());
        for policy in ["tie_break = \"Defer\"", "quorum = 2"] {
            let parsed = toml::from_str::<Team>(&format!("{policy}\n{raw}")).unwrap();
            assert!(parsed.validate().is_ok());
        }
        for policy in ["tie_break = \"Unknown\"", "quorum = 0", "quorum = 4"] {
            let parsed = toml::from_str::<Team>(&format!("{policy}\n{raw}")).unwrap();
            assert!(parsed.validate().is_err());
        }
    }
    #[test]
    fn explicit_and_default_ties_are_visible_in_report_and_cli() {
        for explicit in [false, true] {
            let mut team = builtin();
            if explicit {
                team.tie_break = Some("Defer".into());
            }
            let outcomes: Vec<_> = (0..3)
                .map(|i| {
                    let mut ranking = team.options.clone();
                    ranking.rotate_left(i);
                    MemberOutcome::Ballot(Ballot {
                        ranking,
                        rationale: format!("Reason {i}"),
                    })
                })
                .collect();
            let report = report(&team, "Choose", &outcomes).unwrap();
            for text in [
                &report.text,
                &crate::cli::agents_api::team_run_text(
                    &serde_json::json!({"report":report.text,"result_path":"RESULT.md"}),
                ),
            ] {
                assert!(text.contains("TIE: Proceed, Experiment, Defer (3 points)"));
                assert!(text.contains("Resolved to Defer by tie_break"));
                assert!(text.contains(if explicit {
                    "(explicit)"
                } else {
                    "(default: last option)"
                }));
            }
        }
    }
    #[test]
    fn untied_safe_option_does_not_displace_a_winner_and_lower_ties_keep_order() {
        let mut team = builtin();
        team.quorum = Some(2);
        let ballots = vec![
            Ballot {
                ranking: vec!["Proceed".into(), "Experiment".into(), "Defer".into()],
                rationale: "A".into(),
            },
            Ballot {
                ranking: vec!["Experiment".into(), "Proceed".into(), "Defer".into()],
                rationale: "B".into(),
            },
        ];
        assert_eq!(aggregate(&team, &ballots).unwrap()[0].0, "Experiment");
        team.tie_break = Some("Proceed".into());
        assert_eq!(aggregate(&team, &ballots).unwrap()[0].0, "Proceed");
    }
    #[test]
    fn unreported_member_does_not_mask_a_genuine_failed_member() {
        let team = builtin();
        let ballot = Ballot {
            ranking: team.options.clone(),
            rationale: "valid".into(),
        };
        let report = report(
            &team,
            "Choose",
            &[
                MemberOutcome::Ballot(ballot),
                MemberOutcome::ExitedWithoutReport("final text".into()),
                MemberOutcome::Failed("exit 2".into()),
            ],
        )
        .unwrap();
        assert_eq!(report.status, "FAILED");
        assert!(!report.text.contains("Quorum degraded"));
    }
    #[test]
    fn stored_team_rejects_unknown_fields_and_path_traversal() {
        let root = tempfile::tempdir().unwrap();
        assert!(load(Some(root.path()), "../other").is_err());
        let mut team = builtin();
        team.members[1].role = team.members[0].role.clone();
        assert!(team.validate().is_err());
        assert!(toml::from_str::<Team>("version=0\nunknown=true").is_err());
    }
    #[test]
    fn workspace_team_overrides_global_then_builtin() {
        let _channel = crate::config::set_test_channel("alpha");
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let workspace = tempfile::tempdir().unwrap();
        let workspace_dir = workspace
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("teams");
        let global_dir = crate::config::config_dir().join("teams");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::fs::create_dir_all(&global_dir).unwrap();

        let mut global = builtin();
        global.options[0] = "Global".into();
        std::fs::write(
            global_dir.join("consult-experts.toml"),
            toml::to_string(&global).unwrap(),
        )
        .unwrap();
        let mut local = global.clone();
        local.options[0] = "Workspace".into();
        std::fs::write(
            workspace_dir.join("consult-experts.toml"),
            toml::to_string(&local).unwrap(),
        )
        .unwrap();

        assert_eq!(
            load(Some(workspace.path()), "consult-experts")
                .unwrap()
                .options[0],
            "Workspace"
        );
        std::fs::remove_file(workspace_dir.join("consult-experts.toml")).unwrap();
        assert_eq!(
            load(Some(workspace.path()), "consult-experts")
                .unwrap()
                .options[0],
            "Global"
        );
        std::fs::remove_file(global_dir.join("consult-experts.toml")).unwrap();
        assert_eq!(load(None, "consult-experts").unwrap().options[0], "Proceed");
    }
    #[test]
    fn missing_team_lists_workspace_global_and_builtin_searches() {
        let _channel = crate::config::set_test_channel("alpha");
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let workspace = tempfile::tempdir().unwrap();
        let error = load(Some(workspace.path()), "missing-team").unwrap_err();
        let workspace_path = workspace
            .path()
            .join(".plexi-alpha/teams/missing-team.toml")
            .display()
            .to_string();
        let global_path = profile
            .path()
            .join("teams/missing-team.toml")
            .display()
            .to_string();
        assert!(error.contains("built-in team: consult-experts"));
        assert!(error.contains(".plexi-alpha"));
        assert!(error.find(&workspace_path).unwrap() < error.find(&global_path).unwrap());
    }
}
pub(crate) fn builtin() -> Team {
    Team {
        version: 0,
        tie_break: None,
        quorum: None,
        options: vec!["Proceed".into(), "Experiment".into(), "Defer".into()],
        members: [
            ("pragmatist", "pi"),
            ("critic", "claude"),
            ("strategist", "codex"),
        ]
        .into_iter()
        .map(|(role, harness)| Member {
            role: role.into(),
            harness: harness.into(),
            grants: vec![],
            command: None,
            model: None,
        })
        .collect(),
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
/// Resolve a team definition. A workspace definition, when present, shadows the
/// channel profile definition; built-ins are the final fallback.
pub(crate) fn load(workspace: Option<&Path>, name: &str) -> Result<Team, String> {
    if !valid_name(name) {
        return Err("team name must contain only lowercase letters, digits and hyphens".into());
    }
    let mut paths = Vec::with_capacity(2);
    if let Some(workspace) = workspace {
        paths.push(
            workspace
                .join(crate::config::workspace_channel_dir())
                .join("teams")
                .join(format!("{name}.toml")),
        );
    }
    paths.push(
        crate::config::config_dir()
            .join("teams")
            .join(format!("{name}.toml")),
    );
    for path in &paths {
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                let team: Team =
                    toml::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
                team.validate()?;
                return Ok(team);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("read {}: {e}", path.display())),
        }
    }
    if name == "consult-experts" {
        Ok(builtin())
    } else {
        Err(format!(
            "team '{name}' not found; searched:\n{}\nbuilt-in team: consult-experts",
            paths
                .iter()
                .map(|path| format!("  {}", path.display()))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}
impl Team {
    pub(crate) fn quorum(&self) -> usize {
        self.quorum.unwrap_or(self.members.len())
    }
    pub(crate) fn tie_break(&self) -> &str {
        self.tie_break
            .as_deref()
            .unwrap_or_else(|| self.options.last().map(String::as_str).unwrap_or(""))
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.version != 0 {
            return Err("only team version 0 is supported".into());
        }
        if !(3..=16).contains(&self.members.len()) {
            return Err("a team requires 3 to 16 members".into());
        }
        if !(2..=64).contains(&self.options.len())
            || self.options.iter().any(|o| o.trim().is_empty())
            || self.options.iter().collect::<BTreeSet<_>>().len() != self.options.len()
        {
            return Err("options must contain 2 to 64 unique nonempty strings".into());
        }
        if !(1..=self.members.len()).contains(&self.quorum()) {
            return Err("quorum must be between 1 and the number of members".into());
        }
        if self
            .tie_break
            .as_ref()
            .is_some_and(|option| !self.options.contains(option))
        {
            return Err("tie_break must name one of the options".into());
        }
        let mut names = BTreeSet::new();
        for member in &self.members {
            if !valid_name(&member.role) || !names.insert(&member.role) {
                return Err("member roles must be unique slugs".into());
            }
            if member.harness == "script" {
                if member
                    .command
                    .as_deref()
                    .is_none_or(|c| c.trim().is_empty())
                {
                    return Err(format!("{} requires a script command", member.role));
                }
                if member.model.is_some() {
                    return Err("script members cannot specify a model".into());
                }
            } else if crate::cli::AgentHarness::parse(&member.harness).is_none()
                || member.command.is_some()
            {
                return Err(format!(
                    "{}: use pi, claude, codex, agy or script; command is script-only",
                    member.role
                ));
            }
        }
        Ok(())
    }
    pub(crate) fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object", "required":["ranking","rationale"], "additionalProperties":false,
            "properties":{"ranking":{"type":"array", "minItems":self.options.len(), "maxItems":self.options.len(), "uniqueItems":true,
                "items":{"type":"string","enum":self.options}}, "rationale":{"type":"string","minLength":1}}})
    }
}
impl Ballot {
    pub(crate) fn validate(&self, team: &Team) -> Result<(), String> {
        let expected: BTreeSet<_> = team.options.iter().collect();
        if self.rationale.trim().is_empty()
            || self.ranking.len() != team.options.len()
            || self.ranking.iter().collect::<BTreeSet<_>>() != expected
        {
            return Err(
                "ballot must rank every option exactly once and include a rationale".into(),
            );
        }
        Ok(())
    }
}
pub(crate) fn aggregate(team: &Team, ballots: &[Ballot]) -> Result<Vec<(String, usize)>, String> {
    team.validate()?;
    if ballots.len() < team.quorum() || ballots.len() > team.members.len() {
        return Err("valid ballots must meet quorum without exceeding member count".into());
    }
    let mut scores: BTreeMap<_, usize> = team.options.iter().map(|o| (o.clone(), 0)).collect();
    for ballot in ballots {
        ballot.validate(team)?;
        for (rank, option) in ballot.ranking.iter().enumerate() {
            if let Some(score) = scores.get_mut(option) {
                *score += team.options.len() - rank - 1;
            }
        }
    }
    let mut ranked: Vec<_> = team
        .options
        .iter()
        .map(|o| (o.clone(), scores[o]))
        .collect();
    ranked.sort_by_key(|(_, score)| std::cmp::Reverse(*score));
    let top = ranked[0].1;
    // The configured safe option only wins if it is tied for first. If it is
    // absent from the tie, prefer the last stored option among the leaders.
    let winner = ranked
        .iter()
        .position(|(option, score)| *score == top && option == team.tie_break())
        .unwrap_or_else(|| {
            ranked
                .iter()
                .rposition(|(_, score)| *score == top)
                .unwrap_or(0)
        });
    let winner = ranked.remove(winner);
    ranked.insert(0, winner);
    Ok(ranked)
}
fn table_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}
/// One outcome per stored member, preserving identity when ballots are missing.
#[derive(Clone, Debug)]
pub(crate) enum MemberOutcome {
    Ballot(Ballot),
    ExitedWithoutReport(String),
    Failed(String),
    TimedOut,
}
impl MemberOutcome {
    pub(crate) fn summary(&self) -> String {
        match self {
            Self::Ballot(ballot) => format!("ballot: {}", ballot.ranking.join(" → ")),
            Self::ExitedWithoutReport(text) => format!("exited without report (not a valid ballot): {text}"),
            Self::Failed(reason) => format!("failed: {reason}"),
            Self::TimedOut => "timed out".into(),
        }
    }
}
pub(crate) struct Report {
    pub status: &'static str,
    pub text: String,
}
pub(crate) fn report(
    team: &Team,
    question: &str,
    outcomes: &[MemberOutcome],
) -> Result<Report, String> {
    team.validate()?;
    if outcomes.len() != team.members.len() {
        return Err("one outcome required per member".into());
    }
    let ballots: Vec<_> = outcomes
        .iter()
        .filter_map(|o| match o {
            MemberOutcome::Ballot(b) => Some(b.clone()),
            _ => None,
        })
        .collect();
    let unreported: Vec<_> = outcomes.iter().filter_map(|outcome| match outcome {
        MemberOutcome::ExitedWithoutReport(text) => Some(text),
        _ => None,
    }).collect();
    // A clean single-turn harness exit can still carry useful final text. It is an
    // abstention, not a failed vote: publish the remaining ballots as degraded.
    let has_hard_failure = outcomes
        .iter()
        .any(|outcome| matches!(outcome, MemberOutcome::Failed(_) | MemberOutcome::TimedOut));
    let degraded = !unreported.is_empty()
        && !ballots.is_empty()
        && !has_hard_failure
        && ballots.len() + unreported.len() >= team.quorum();
    let status = if ballots.len() < team.quorum() && !degraded {
        "FAILED"
    } else if ballots.len() < team.members.len() {
        "PARTIAL"
    } else {
        "COMPLETE"
    };
    let mut text = format!(
        "# Team result\n\nStatus: {status}\n\n{question}\n\nQuorum: {} / {}; valid ballots: {}.\n",
        team.quorum(),
        team.members.len(),
        ballots.len()
    );
    if degraded {
        text.push_str(&format!("\nQuorum degraded: {} of {} valid ballots; {} exited without report.\n", ballots.len(), team.members.len(), unreported.len()));
    }
    let excluded: Vec<_> = team
        .members
        .iter()
        .zip(outcomes)
        .filter_map(|(m, o)| (!matches!(o, MemberOutcome::Ballot(_))).then_some(m.role.as_str()))
        .collect();
    if !excluded.is_empty() {
        text.push_str(&format!("\nExcluded: {}\n", excluded.join(", ")));
    }
    for (member, outcome) in team.members.iter().zip(outcomes) {
        text.push_str(&format!("\n{}: {}\n", member.role, outcome.summary()));
        if let MemberOutcome::Ballot(ballot) = outcome {
            text.push_str(&format!("\n{}\n", ballot.rationale));
        }
    }
    if ballots.len() >= team.quorum() || degraded {
        let mut aggregation_team = team.clone();
        if degraded {
            aggregation_team.quorum = Some(ballots.len());
        }
        let scores = aggregate(&aggregation_team, &ballots)?;
        let tied: Vec<_> = team
            .options
            .iter()
            .filter(|option| {
                scores
                    .iter()
                    .any(|(o, score)| o == *option && *score == scores[0].1)
            })
            .cloned()
            .collect();
        if tied.len() > 1 {
            let rule = if tied.iter().any(|o| o == team.tie_break()) {
                if team.tie_break.is_some() {
                    "tie_break (explicit)"
                } else {
                    "tie_break (default: last option)"
                }
            } else {
                "last stored option among tied leaders (tie_break is not tied)"
            };
            text.push_str(&format!(
                "\nTIE: {} ({} points). Resolved to {} by {rule}.\n",
                tied.join(", "),
                scores[0].1,
                scores[0].0
            ));
        }
        text.push_str(&format!("\nWinner: {}\n\n## Borda ranking\n\nPoints: N-1 to 0.\n\n| Rank | Option | Points |\n| --- | --- | --- |\n", scores[0].0));
        for (i, (option, score)) in scores.iter().enumerate() {
            text.push_str(&format!(
                "| {} | {} | {score} |\n",
                i + 1,
                table_cell(option)
            ));
        }
    } else {
        text.push_str("\nNo winner: quorum not met.\n");
    }
    Ok(Report { status, text })
}
