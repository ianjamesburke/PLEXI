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
    fn borda_requires_complete_independent_ballots_and_breaks_ties_by_option_order() {
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
                (options[0].clone(), 3),
                (options[1].clone(), 3),
                (options[2].clone(), 3)
            ]
        );
        assert!(aggregate(&team, &ballots[..2]).is_err());
        let mut malformed = ballots;
        malformed[0].ranking[1] = options[0].clone();
        assert!(aggregate(&team, &malformed).is_err());
    }
    #[test]
    fn stored_team_rejects_unknown_fields_and_path_traversal() {
        let root = tempfile::tempdir().unwrap();
        assert!(load(root.path(), "../other").is_err());
        let mut team = builtin();
        team.members[1].role = team.members[0].role.clone();
        assert!(team.validate().is_err());
        assert!(toml::from_str::<Team>("version=0\nunknown=true").is_err());
    }
}
pub(crate) fn builtin() -> Team {
    Team {
        version: 0,
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
pub(crate) fn load(workspace: &Path, name: &str) -> Result<Team, String> {
    if !valid_name(name) {
        return Err("team name must contain only lowercase letters, digits and hyphens".into());
    }
    let paths = [
        workspace
            .join(crate::config::workspace_channel_dir())
            .join("teams")
            .join(format!("{name}.toml")),
        crate::config::config_dir()
            .join("teams")
            .join(format!("{name}.toml")),
    ];
    for path in paths {
        match std::fs::read_to_string(&path) {
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
        Err(format!("team '{name}' not found"))
    }
}
impl Team {
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
pub(crate) fn aggregate(team: &Team, ballots: &[Ballot]) -> Result<Vec<(String, usize)>, String> {
    team.validate()?;
    if ballots.len() != team.members.len() {
        return Err("every member must submit one ballot".into());
    }
    let expected: BTreeSet<_> = team.options.iter().collect();
    let mut scores: BTreeMap<_, usize> = team.options.iter().map(|o| (o.clone(), 0)).collect();
    for ballot in ballots {
        if ballot.rationale.trim().is_empty()
            || ballot.ranking.len() != team.options.len()
            || ballot.ranking.iter().collect::<BTreeSet<_>>() != expected
        {
            return Err(
                "ballot must rank every option exactly once and include a rationale".into(),
            );
        }
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
    Ok(ranked)
}
fn table_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}
pub(crate) fn report(team: &Team, question: &str, ballots: &[Ballot]) -> Result<String, String> {
    let scores = aggregate(team, ballots)?;
    let mut text = format!("# Team result\n\n{question}\n\n## Borda ranking\n\nPoints: N-1 to 0. Ties follow the stored option order.\n\n| Rank | Option | Points |\n| --- | --- | --- |\n");
    for (i, (option, score)) in scores.iter().enumerate() {
        text.push_str(&format!(
            "| {} | {} | {score} |\n",
            i + 1,
            table_cell(option)
        ));
    }
    for (member, ballot) in team.members.iter().zip(ballots) {
        text.push_str(&format!(
            "\n## {} ({})\n\n{}\n\n{}\n",
            member.role,
            member.harness,
            ballot.ranking.join(" → "),
            ballot.rationale
        ));
    }
    Ok(text)
}
