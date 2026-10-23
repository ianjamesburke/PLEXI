//! `plexi skill install` writes the skill compiled into this binary.
//!
//! The file on disk is `skills/plexi-cli/SKILL.md` as embedded at build time,
//! so `plexi_version` matches this binary's version, minus every section tagged
//! for an experiment that is off (`<!-- experiment: key[, key] -->` ...
//! `<!-- /experiment -->`; markers nest and a multi-key section needs every key
//! on). Each channel installs to its own directory so alpha and stable never
//! overwrite each other.

use std::path::{Path, PathBuf};

use crate::release::ReleaseFeature;

const EMBEDDED_SKILL: &str = include_str!("../../skills/plexi-cli/SKILL.md");

const EXPERIMENT_OPEN: &str = "<!-- experiment:";
const EXPERIMENT_CLOSE: &str = "<!-- /experiment -->";

fn agent_roots(agent: &str) -> Result<Vec<&'static str>, String> {
    match agent {
        "claude" => Ok(vec![".claude"]),
        "codex" => Ok(vec![".codex"]),
        "pi" => Ok(vec![".pi/agent"]),
        "all" => Ok(vec![".claude", ".codex", ".pi/agent"]),
        other => Err(format!(
            "unknown agent '{other}' — use claude, codex, pi, or all"
        )),
    }
}

/// The experiment keys a marker line names, e.g. `<!-- experiment: ledger, assistant -->`.
fn marker_features(line: &str) -> Result<Vec<ReleaseFeature>, String> {
    let keys = line
        .trim()
        .strip_prefix(EXPERIMENT_OPEN)
        .and_then(|rest| rest.strip_suffix("-->"))
        .ok_or_else(|| format!("malformed experiment marker: `{line}`"))?;
    keys.split(',')
        .map(|key| {
            let key = key.trim();
            ReleaseFeature::from_key(key)
                .ok_or_else(|| format!("skill marker names unknown experiment `{key}`"))
        })
        .collect()
}

/// `skill` with every section tagged for an experiment that `enabled` reports
/// off removed. A section tagged with several keys stays only when all are on.
/// Markers nest, and the marker lines themselves never survive.
pub fn render_skill(
    skill: &str,
    enabled: impl Fn(ReleaseFeature) -> bool,
) -> Result<String, String> {
    let mut out = String::with_capacity(skill.len());
    let mut open: Vec<bool> = Vec::new();
    for line in skill.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with(EXPERIMENT_OPEN) {
            let on = marker_features(trimmed)?.into_iter().all(&enabled);
            open.push(on);
        } else if trimmed == EXPERIMENT_CLOSE {
            open.pop().ok_or_else(|| {
                "skill has a closing experiment marker with no opener".to_string()
            })?;
        } else if open.iter().all(|on| *on) {
            out.push_str(line);
        }
    }
    if !open.is_empty() {
        return Err("skill has an experiment marker that is never closed".to_string());
    }
    Ok(out)
}

/// The channel recorded in an installed skill's frontmatter. A skill with no
/// `channel:` line predates per-channel installs and counts as stable.
fn recorded_channel(skill: &str) -> &str {
    let mut lines = skill.lines();
    if lines.next() != Some("---") {
        return "stable";
    }
    lines
        .take_while(|l| *l != "---")
        .find_map(|l| l.strip_prefix("channel:"))
        .map(|c| c.trim().trim_matches('"'))
        .unwrap_or("stable")
}

/// `None` for the stable tier (bare binary, `main`, `rc-*`), else the channel.
fn skill_channel(channel: Option<&str>) -> Option<&str> {
    match channel {
        None | Some("main") => None,
        Some(name) if name.starts_with("rc-") => None,
        Some(name) => Some(name),
    }
}

/// Rewrite the frontmatter so two channels' skills never share a name, and
/// record which channel wrote the file.
fn stamp_channel(skill: &str, channel: Option<&str>) -> String {
    let Some(channel) = channel else {
        return skill.replacen("\nskill_version:", "\nchannel: stable\nskill_version:", 1);
    };
    skill
        .replacen(
            "name: plexi-cli\n",
            &format!("name: plexi-cli-{channel}\n"),
            1,
        )
        .replacen(
            "\nskill_version:",
            &format!("\nchannel: {channel}\nskill_version:"),
            1,
        )
}

/// Write the embedded skill under `home` for `agent` (`claude`, `codex`, `pi`, or
/// `all`), stripped of every experiment `enabled` reports off. Each channel
/// installs to its own directory (`plexi-cli` for stable, `plexi-cli-<channel>`
/// otherwise) and refuses to overwrite a file another channel wrote.
pub fn install_embedded_skill(
    home: &Path,
    agent: &str,
    channel: Option<&str>,
    enabled: impl Fn(ReleaseFeature) -> bool,
) -> Result<Vec<PathBuf>, String> {
    let roots = agent_roots(agent)?;
    let channel = skill_channel(channel);
    let dir_name = match channel {
        Some(channel) => format!("plexi-cli-{channel}"),
        None => "plexi-cli".to_string(),
    };
    let text = stamp_channel(&render_skill(EMBEDDED_SKILL, enabled)?, channel);
    let mut written = Vec::new();
    for root in roots {
        let path = home
            .join(root)
            .join("skills")
            .join(&dir_name)
            .join("SKILL.md");
        if let Ok(existing) = std::fs::read_to_string(&path) {
            let owner = recorded_channel(&existing);
            let ours = channel.unwrap_or("stable");
            if owner != ours {
                log::error!(
                    "skill_install: agent={agent} path={} refused: owned by channel {owner}",
                    path.display()
                );
                return Err(format!(
                    "{} belongs to the {owner} channel; refusing to overwrite it from {ours}. Remove it first.",
                    path.display()
                ));
            }
        }
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                log::error!(
                    "skill_install: agent={agent} path={} create failed: {error}",
                    parent.display()
                );
                return Err(format!("could not create {}: {error}", parent.display()));
            }
        }
        if let Err(error) = std::fs::write(&path, &text) {
            log::error!(
                "skill_install: agent={agent} path={} write failed: {error}",
                path.display()
            );
            return Err(format!("could not write {}: {error}", path.display()));
        }
        log::info!("skill_install: agent={agent} path={}", path.display());
        written.push(path);
    }
    Ok(written)
}

/// `plexi skill install --agent claude|codex|pi|all`
pub fn skill_install_cli(agent: &str) -> i32 {
    let home = match std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => {
            log::error!("skill_install: HOME is unset");
            eprintln!("error: HOME is unset");
            return 1;
        }
    };
    match install_embedded_skill(
        &home,
        agent,
        crate::config::build_channel().as_deref(),
        crate::release::feature_enabled,
    ) {
        Ok(paths) => {
            for path in paths {
                println!("{}", path.display());
            }
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: ReleaseFeature) -> bool {
        false
    }

    #[test]
    fn pi_install_uses_its_agent_skills_directory_and_valid_yaml() {
        let home = tempfile::tempdir().unwrap();
        let paths = install_embedded_skill(home.path(), "pi", Some("alpha"), none).unwrap();
        assert_eq!(
            paths,
            vec![home
                .path()
                .join(".pi/agent/skills/plexi-cli-alpha/SKILL.md")]
        );
        let text = std::fs::read_to_string(&paths[0]).unwrap();
        let yaml = text.split("---").nth(1).unwrap();
        let value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(value["name"].as_str(), Some("plexi-cli-alpha"));
    }

    #[test]
    fn install_writes_the_default_skill_for_all_agents() {
        let home = tempfile::tempdir().unwrap();
        let paths = install_embedded_skill(home.path(), "all", None, none).unwrap();
        assert_eq!(paths.len(), 3);
        let expected = stamp_channel(&render_skill(EMBEDDED_SKILL, none).unwrap(), None);
        for path in &paths {
            let text = std::fs::read_to_string(path).unwrap();
            assert_eq!(text, expected);
            assert!(
                text.contains(&format!("plexi_version: \"{}\"", env!("CARGO_PKG_VERSION"))),
                "embedded skill must match this binary's version"
            );
        }
        assert!(paths[0].ends_with(".claude/skills/plexi-cli/SKILL.md"));
        assert!(paths[1].ends_with(".codex/skills/plexi-cli/SKILL.md"));
        assert!(paths[2].ends_with(".pi/agent/skills/plexi-cli/SKILL.md"));
    }

    #[test]
    fn alpha_and_stable_install_to_distinct_paths_with_distinct_names() {
        let home = tempfile::tempdir().unwrap();
        let stable = install_embedded_skill(home.path(), "claude", None, none).unwrap();
        let alpha = install_embedded_skill(home.path(), "claude", Some("alpha"), none).unwrap();
        assert_ne!(stable, alpha);
        assert!(alpha[0].ends_with(".claude/skills/plexi-cli-alpha/SKILL.md"));
        let stable_text = std::fs::read_to_string(&stable[0]).unwrap();
        let alpha_text = std::fs::read_to_string(&alpha[0]).unwrap();
        assert!(stable_text.contains("\nname: plexi-cli\n"));
        assert!(alpha_text.contains("\nname: plexi-cli-alpha\n"));
        assert!(alpha_text.contains("\nchannel: alpha\n"));
        // rc and main are stable tier.
        let rc = install_embedded_skill(home.path(), "claude", Some("rc-010"), none).unwrap();
        assert_eq!(rc, stable);
    }

    #[test]
    fn refuses_to_overwrite_another_channels_skill() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".claude/skills/plexi-cli/SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "---\nname: plexi-cli\nchannel: alpha\n---\nalpha skill\n",
        )
        .unwrap();
        let err = install_embedded_skill(home.path(), "claude", None, none).unwrap_err();
        assert!(err.contains("belongs to the alpha channel"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().lines().last(),
            Some("alpha skill")
        );
    }

    #[test]
    fn a_legacy_skill_without_a_channel_counts_as_stable() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".claude/skills/plexi-cli/SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, EMBEDDED_SKILL).unwrap();
        install_embedded_skill(home.path(), "claude", None, none).expect("stable may refresh it");
    }

    #[test]
    fn default_render_has_no_experiment_section() {
        let text = render_skill(EMBEDDED_SKILL, none).unwrap();
        assert!(
            !text.contains(EXPERIMENT_OPEN),
            "open markers must not survive"
        );
        assert!(
            !text.contains(EXPERIMENT_CLOSE),
            "close markers must not survive"
        );
        for needle in [
            "assistant send",
            "plexi relay",
            "command-view",
            "plexi ledger",
            "--folder",
            "mcp-config",
            "plexi account",
            "plexi routine",
        ] {
            assert!(
                !text.contains(needle),
                "default skill still mentions `{needle}`"
            );
        }
        // Core surface stays.
        assert!(text.contains("plexi pane"));
        assert!(text.contains("agent report"));
    }

    #[test]
    fn enabling_an_experiment_restores_exactly_its_sections() {
        let on = render_skill(EMBEDDED_SKILL, |f| f == ReleaseFeature::Ledger).unwrap();
        assert!(on.contains("plexi ledger"));
        assert!(!on.contains("plexi relay"));
        let assistant_only =
            render_skill(EMBEDDED_SKILL, |f| f == ReleaseFeature::Assistant).unwrap();
        assert!(assistant_only.contains("assistant send"));
        assert!(!assistant_only.contains("command-view"));
        let both = render_skill(EMBEDDED_SKILL, |f| {
            matches!(f, ReleaseFeature::CommandView | ReleaseFeature::Assistant)
        })
        .unwrap();
        assert!(both.contains("assistant send"));
        assert!(both.contains("command-view"));

        // Combined markers remain supported even when the current catalog has
        // no overlapping command owners.
        let fixture =
            "<!-- experiment: command_view, assistant -->\ncombined\n<!-- /experiment -->\n";
        assert!(
            render_skill(fixture, |f| f == ReleaseFeature::CommandView)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            render_skill(fixture, |f| matches!(
                f,
                ReleaseFeature::CommandView | ReleaseFeature::Assistant
            ))
            .unwrap(),
            "combined\n"
        );
    }

    #[test]
    fn every_marker_names_a_known_experiment_and_is_balanced() {
        render_skill(EMBEDDED_SKILL, |_| true).expect("skill markers must be valid and balanced");
        assert!(
            render_skill("<!-- experiment: nope -->\nx\n<!-- /experiment -->\n", none).is_err()
        );
        assert!(render_skill("<!-- experiment: command_view -->\nx\n", none).is_err());
        assert!(render_skill("x\n<!-- /experiment -->\n", none).is_err());
    }
}
