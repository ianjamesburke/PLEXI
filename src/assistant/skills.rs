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

/// Name of the builtin app-build skill (frontmatter `name:` in
/// `builtin/build-plexi-app.md`). Callers that need to special-case an
/// app-build turn (e.g. injecting the SDK reference, raising the tool-call
/// cap) match on this instead of repeating the literal.
pub const APP_BUILD_SKILL_NAME: &str = "build-plexi-app";

/// Generated, always-current SDK API reference (`tools/gen_sdk_docs.py` /
/// `just gen-sdk-docs`, drift-checked by `just check-sdk-docs`). Injected into
/// the system prompt when the app-build skill is active so the assistant
/// doesn't need to explore `plexi_sdk` via PTY reads to learn the API —
/// exactly that exploration exhausted the tool-call cap in the 2026-07-17
/// gate run.
const SDK_API_REFERENCE: &str = include_str!("../../website/src/content/docs/sdk.md");

/// The generated SDK API reference to inject for app-build turns.
pub fn app_build_sdk_reference() -> &'static str {
    SDK_API_REFERENCE
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDefinition {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub source: SkillSource,
    pub path: PathBuf,
    /// Optional model-tier floor (frontmatter `tier:`). When the session tier
    /// is still the installed default, dispatch escalates to at least this
    /// tier for turns that load the skill; an explicit user tier always wins.
    pub tier: Option<crate::app_protocol::ModelTier>,
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

    /// Fallback for when `matching_enabled` misses a plain build request.
    /// That matcher's 50%-coverage bar is tuned to disambiguate many
    /// similarly-worded user-installed skills, and silently drops short,
    /// noun-heavy prompts against the builtin skill's own description — e.g.
    /// "Build me a tiny counter app with plus and minus buttons" scores
    /// 2/6 = 0.33 and never matches. Since silently never activating the
    /// app-build skill strands the assistant without the SDK reference or
    /// the raised tool-call cap (stint 0422), this check is independent of
    /// description wording: a build-verb next to a build-noun anywhere in
    /// the prompt is a signal that should never miss for this one,
    /// high-value builtin skill.
    pub fn app_build_fallback(&self, prompt: &str, enabled: &[String]) -> Option<&SkillDefinition> {
        if !looks_like_app_build_request(prompt) {
            return None;
        }
        if !(enabled.is_empty() || enabled.iter().any(|name| name == APP_BUILD_SKILL_NAME)) {
            return None;
        }
        self.get(APP_BUILD_SKILL_NAME)
    }
}

const BUILD_VERBS: &[&str] = &["build", "make", "create", "write", "code"];
const BUILD_NOUNS: &[&str] = &[
    "app", "apps", "game", "games", "tool", "tools", "widget", "widgets", "utility", "utilities",
];

/// Whether `prompt` reads as a request to build something. Word-boundary
/// tokenized (not substring `contains`) so it doesn't false-positive on
/// unrelated words that happen to embed a build noun/verb.
fn looks_like_app_build_request(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let words: std::collections::HashSet<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|word| !word.is_empty())
        .collect();
    BUILD_VERBS.iter().any(|verb| words.contains(verb))
        && BUILD_NOUNS.iter().any(|noun| words.contains(noun))
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
    let tier = match value("tier").as_deref() {
        None => None,
        Some("low") => Some(crate::app_protocol::ModelTier::Low),
        Some("medium") => Some(crate::app_protocol::ModelTier::Medium),
        Some("high") => Some(crate::app_protocol::ModelTier::High),
        Some(other) => {
            return Err(format!(
                "frontmatter tier must be low, medium, or high — got '{other}'"
            ))
        }
    };
    Ok(SkillDefinition {
        name,
        description,
        instructions: instructions.trim().to_string(),
        source,
        path: path.to_path_buf(),
        tier,
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
    fn tier_frontmatter_parses_and_rejects_unknown_values() {
        let skill = parse_skill_text(
            "---\nname: deploy\ndescription: deploy a release\ntier: high\n---\nbody",
            SkillSource::User,
            Path::new("x"),
        )
        .expect("valid tier");
        assert_eq!(skill.tier, Some(crate::app_protocol::ModelTier::High));

        let no_tier = parse_skill_text(
            "---\nname: deploy\ndescription: deploy a release\n---\nbody",
            SkillSource::User,
            Path::new("x"),
        )
        .expect("tier is optional");
        assert_eq!(no_tier.tier, None);

        let err = parse_skill_text(
            "---\nname: deploy\ndescription: deploy a release\ntier: turbo\n---\nbody",
            SkillSource::User,
            Path::new("x"),
        )
        .expect_err("unknown tier must fail loudly");
        assert!(err.contains("turbo"), "{err}");
    }

    #[test]
    fn builtin_app_build_skill_declares_a_high_tier_floor() {
        let root = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(root.path(), root.path());
        let skill = registry.get(APP_BUILD_SKILL_NAME).expect("builtin present");
        assert_eq!(
            skill.tier,
            Some(crate::app_protocol::ModelTier::High),
            "app builds must escalate off a default-sourced weak tier"
        );
        assert!(
            !skill.instructions.contains("host.files.grep to find SDK symbols"),
            "the skill must not instruct SDK discovery — the API reference is embedded"
        );
        assert!(
            skill.instructions.contains("other installed apps"),
            "the no-discovery rule must cover example-app spelunking, not just SDK sources \
             — the 2026-07-19 ant-simulator run reverse-engineered signatures from \
             installed apps instead"
        );
        assert!(
            skill.instructions.contains("App placement"),
            "the skill must point at the host-injected placement line so init never \
             burns a failed no-workspace probe"
        );
        assert!(
            skill.instructions.contains("`view()` is pure and returns exactly one component"),
            "the skill must state the effect contract — the 2026-07-19 tictactoe run \
             returned (tree, SetTimer) from view() and crashed on the first click"
        );
    }

    #[test]
    fn app_build_sdk_reference_is_generated_and_nonempty() {
        let reference = app_build_sdk_reference();
        assert!(
            !reference.is_empty(),
            "SDK reference must be embedded, not blank"
        );
        assert!(
            reference.contains("Generated by tools/gen_sdk_docs.py"),
            "must be the generated doc, not a hand-written stand-in"
        );
        assert!(
            reference.contains("## UI Components") && reference.contains("## Effects"),
            "must cover the widget/effect API the app-build skill needs"
        );
        assert!(
            reference.contains("SetTimer(id: int, delay_ms: int, repeat: bool = False)"),
            "the reference must carry real constructor signatures — headings alone \
             taught the model nothing and caused guessed-kwarg first-try failures \
             (2026-07-19 ant-simulator run)"
        );
        assert!(
            reference.matches("```python").count() > 100,
            "signature coverage must span the API surface, not a hand-picked few"
        );
    }

    /// Regression for the 0422 gate-fix follow-up: a real tester prompt
    /// ("Build me a tiny counter app with plus and minus buttons") scores
    /// only 2/6 = 0.33 word-overlap against the skill's own description, so
    /// `matching_enabled` misses it — confirming the bug the fallback
    /// exists to catch. `app_build_fallback` must still activate the skill.
    #[test]
    fn app_build_fallback_catches_prompt_the_general_matcher_misses() {
        let root = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(root.path(), root.path());
        let prompt = "Build me a tiny counter app with plus and minus buttons";

        assert!(
            registry.matching_enabled(prompt, &[]).is_none(),
            "this prompt is the known miss the fallback exists for; if this \
             now passes, the general matcher changed and this assertion \
             should be revisited"
        );
        let skill = registry
            .app_build_fallback(prompt, &[])
            .expect("fallback must catch a build-verb + build-noun prompt");
        assert_eq!(skill.name, APP_BUILD_SKILL_NAME);
    }

    #[test]
    fn app_build_fallback_respects_enabled_list_and_ignores_unrelated_prompts() {
        let root = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(root.path(), root.path());

        assert!(registry
            .app_build_fallback("what's the weather like today", &[])
            .is_none());
        assert!(registry
            .app_build_fallback(
                "build me a small app",
                &["some-other-skill".to_string()]
            )
            .is_none());
        assert!(registry
            .app_build_fallback("make me a game", &[APP_BUILD_SKILL_NAME.to_string()])
            .is_some());
    }

    #[test]
    fn builtin_build_plexi_app_ships_and_auto_matches() {
        let root = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(root.path(), root.path());
        let skill = registry.get(APP_BUILD_SKILL_NAME).unwrap();
        assert_eq!(skill.name, APP_BUILD_SKILL_NAME);
        assert_eq!(skill.source, SkillSource::Builtin);
        assert!(skill
            .instructions
            .contains(r#"host.build.run {"args": ["app", "init", "<kebab-name>"]}"#));
        assert!(skill
            .instructions
            .contains(r#"host.build.run {"args": ["app", "check", "<app-dir>"]}"#));
        assert!(skill.instructions.contains("host.files.write"));
        assert!(
            !skill.instructions.contains("host.terminals.run"),
            "the build flow must never route through a user-visible terminal"
        );
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
            tier: None,
        };
        let docs = SkillDefinition {
            name: "docs".into(), description: "Write and revise product documentation with clear examples and accurate command references.".into(),
            instructions: "write".into(), source: SkillSource::User, path: PathBuf::new(),
            tier: None,
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
            tier: None,
        };
        let registry = SkillRegistry {
            skills: vec![make("release-a"), make("release-b")],
        };
        assert!(registry
            .matching_enabled("audit production artifacts", &[])
            .is_none());
    }
}
