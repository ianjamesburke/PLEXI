//! Channel-aware Assistant skill discovery.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    Builtin,
    User,
    Workspace,
}

impl SkillSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::User => "user",
            Self::Workspace => "workspace",
        }
    }
}

/// Skills compiled into the binary so a fresh install has them from first
/// launch. Same SKILL.md format as on-disk skills; a user or workspace skill
/// with the same name overrides a builtin.
const BUILTIN_SKILLS: &[&str] = &[include_str!("builtin/build-plexi-app.md")];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDefinition {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub source: SkillSource,
    pub path: PathBuf,
}

#[derive(Debug, Default)]
pub struct SkillRegistry {
    skills: Vec<SkillDefinition>,
}

impl SkillRegistry {
    pub fn load(profile_dir: &Path, workspace_root: &Path) -> Self {
        let mut skills = Vec::new();
        for text in BUILTIN_SKILLS {
            match parse_skill_text(text, SkillSource::Builtin, Path::new("<builtin>")) {
                Ok(skill) => skills.push(skill),
                Err(error) => log::error!("assistant: invalid builtin skill: {error}"),
            }
        }
        load_root(profile_dir.join("skills"), SkillSource::User, &mut skills);
        load_root(
            workspace_root.join(".plexi/skills"),
            SkillSource::Workspace,
            &mut skills,
        );
        // Roots are loaded builtin first, then user, then workspace, so map
        // insertion gives workspace > user > builtin precedence for duplicate
        // names.
        let mut by_name = std::collections::BTreeMap::new();
        for skill in skills {
            by_name.insert(skill.name.clone(), skill);
        }
        let skills = by_name.into_values().collect::<Vec<_>>();
        log::info!("assistant: discovered {} skill(s)", skills.len());
        Self { skills }
    }

    pub fn all(&self) -> &[SkillDefinition] {
        &self.skills
    }

    pub fn get(&self, name: &str) -> Option<&SkillDefinition> {
        self.skills.iter().find(|skill| skill.name == name)
    }

    /// Conservative prompt matching: only suggest when every meaningful word
    /// in a skill's description occurs in the prompt. Explicit `/skill` use
    /// remains the reliable invocation path.
    pub fn matching_enabled(&self, prompt: &str, enabled: &[String]) -> Option<&SkillDefinition> {
        let prompt_words = distinctive_words(prompt);
        let mut matches = self
            .skills
            .iter()
            .filter(|skill| enabled.is_empty() || enabled.contains(&skill.name))
            .filter_map(|skill| {
                let description_words = distinctive_words(&skill.description);
                let overlap = prompt_words.intersection(&description_words).count();
                let name_hit = prompt_words.contains(&skill.name.to_ascii_lowercase());
                let prompt_coverage = overlap as f32 / prompt_words.len().max(1) as f32;
                (name_hit || (overlap >= 2 && prompt_coverage >= 0.5))
                    .then_some((skill, (name_hit as usize, overlap)))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|a, b| b.1.cmp(&a.1));
        match matches.as_slice() {
            [(skill, _), ..] if matches.get(1).is_none_or(|next| next.1 != matches[0].1) => {
                Some(*skill)
            }
            _ => None,
        }
    }
}

fn distinctive_words(text: &str) -> std::collections::HashSet<String> {
    const STOPWORDS: &[&str] = &[
        "about", "after", "also", "and", "asks", "before", "from", "into", "that", "the", "their",
        "this", "use", "user", "when", "with", "your",
    ];
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|word| word.len() >= 4 && !STOPWORDS.contains(word))
        .map(str::to_string)
        .collect()
}

fn load_root(root: PathBuf, source: SkillSource, out: &mut Vec<SkillDefinition>) {
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let candidate = if path.is_dir() {
            path.join("SKILL.md")
        } else {
            path
        };
        if candidate.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        match parse_skill(&candidate, source) {
            Ok(skill) => out.push(skill),
            Err(error) => log::warn!("assistant: skipped skill {}: {error}", candidate.display()),
        }
    }
}

fn parse_skill(path: &Path, source: SkillSource) -> Result<SkillDefinition, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    parse_skill_text(&text, source, path)
}

fn parse_skill_text(text: &str, source: SkillSource, path: &Path) -> Result<SkillDefinition, String> {
    let body = text
        .strip_prefix("---\n")
        .ok_or("missing YAML frontmatter")?;
    let (frontmatter, instructions) = body
        .split_once("\n---\n")
        .ok_or("unterminated YAML frontmatter")?;
    let value = |key: &str| {
        frontmatter.lines().find_map(|line| {
            let (found, value) = line.split_once(':')?;
            (found.trim() == key).then(|| value.trim().trim_matches(['\"', '\'']).to_string())
        })
    };
    let name = value("name").ok_or("frontmatter requires name")?;
    let description = value("description").ok_or("frontmatter requires description")?;
    if name.is_empty() || description.is_empty() || instructions.trim().is_empty() {
        return Err("name, description, and instructions must be non-empty".to_string());
    }
    Ok(SkillDefinition {
        name,
        description,
        instructions: instructions.trim().to_string(),
        source,
        path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_channel_and_workspace_skills_with_workspace_precedence() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join(".plexi-pr-1");
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(profile.join("skills/deploy")).unwrap();
        std::fs::create_dir_all(workspace.join(".plexi/skills/deploy")).unwrap();
        std::fs::write(
            profile.join("skills/deploy/SKILL.md"),
            "---\nname: deploy\ndescription: deploy a release\n---\nuser body",
        )
        .unwrap();
        std::fs::write(
            workspace.join(".plexi/skills/deploy/SKILL.md"),
            "---\nname: deploy\ndescription: deploy workspace release\n---\nworkspace body",
        )
        .unwrap();

        let registry = SkillRegistry::load(&profile, &workspace);
        let skill = registry.get("deploy").unwrap();
        assert_eq!(skill.source, SkillSource::Workspace);
        assert_eq!(skill.instructions, "workspace body");
    }

    #[test]
    fn malformed_skills_are_not_exposed() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("skills/bad")).unwrap();
        std::fs::write(root.path().join("skills/bad/SKILL.md"), "# no frontmatter").unwrap();
        let registry = SkillRegistry::load(root.path(), root.path());
        assert!(registry
            .all()
            .iter()
            .all(|skill| skill.source == SkillSource::Builtin));
    }

    #[test]
    fn builtin_build_plexi_app_ships_and_auto_matches() {
        let root = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(root.path(), root.path());
        let skill = registry.get("build-plexi-app").unwrap();
        assert_eq!(skill.source, SkillSource::Builtin);
        assert!(skill.instructions.contains("plexi app init --global"));
        assert!(skill.instructions.contains("plexi app check ."));
        assert_eq!(
            registry
                .matching_enabled("build me a small timer app", &[])
                .unwrap()
                .name,
            "build-plexi-app"
        );
        assert_eq!(
            registry
                .matching_enabled("can you make a tic tac toe game", &[])
                .unwrap()
                .name,
            "build-plexi-app"
        );
    }

    #[test]
    fn workspace_skill_overrides_builtin_of_same_name() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(workspace.join(".plexi/skills/build-plexi-app")).unwrap();
        std::fs::write(
            workspace.join(".plexi/skills/build-plexi-app/SKILL.md"),
            "---\nname: build-plexi-app\ndescription: workspace override\n---\noverride body",
        )
        .unwrap();
        let registry = SkillRegistry::load(root.path(), &workspace);
        let skill = registry.get("build-plexi-app").unwrap();
        assert_eq!(skill.source, SkillSource::Workspace);
        assert_eq!(skill.instructions, "override body");
    }

    #[test]
    fn auto_match_is_conservative() {
        let release = SkillDefinition {
            name: "release".into(),
            description: "Prepare and deploy a production release by checking changelog, version, artifacts, and rollout health when the user asks to ship software.".into(),
            instructions: "do it".into(),
            source: SkillSource::User,
            path: PathBuf::new(),
        };
        let docs = SkillDefinition {
            name: "docs".into(), description: "Write and revise product documentation with clear examples and accurate command references.".into(),
            instructions: "write".into(), source: SkillSource::User, path: PathBuf::new(),
        };
        let registry = SkillRegistry {
            skills: vec![release, docs],
        };
        assert_eq!(
            registry
                .matching_enabled(
                    "please check the changelog and deploy the production release",
                    &[],
                )
                .unwrap()
                .name,
            "release"
        );
        assert!(registry
            .matching_enabled("help me think about tomorrow", &[])
            .is_none());
    }

    #[test]
    fn auto_match_refuses_ambiguous_equal_scores() {
        let make = |name: &str| SkillDefinition {
            name: name.into(),
            description: "audit production release artifacts".into(),
            instructions: "do it".into(),
            source: SkillSource::User,
            path: PathBuf::new(),
        };
        let registry = SkillRegistry {
            skills: vec![make("release-a"), make("release-b")],
        };
        assert!(registry
            .matching_enabled("audit production artifacts", &[])
            .is_none());
    }
}
