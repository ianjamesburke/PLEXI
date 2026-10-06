//! `plexi skill install` writes the skill compiled into this binary.
//!
//! The file on disk is the same bytes as `skills/plexi-cli/SKILL.md` at
//! build time, so `plexi_version` matches this binary's version.

use std::path::{Path, PathBuf};

const EMBEDDED_SKILL: &str = include_str!("../../skills/plexi-cli/SKILL.md");

fn agent_roots(agent: &str) -> Result<Vec<&'static str>, String> {
    match agent {
        "claude" => Ok(vec![".claude"]),
        "codex" => Ok(vec![".codex"]),
        "all" => Ok(vec![".claude", ".codex"]),
        other => Err(format!("unknown agent '{other}' — use claude, codex, or all")),
    }
}

/// Write the embedded skill under `home` for `agent` (`claude`, `codex`, or `all`).
pub fn install_embedded_skill(home: &Path, agent: &str) -> Result<Vec<PathBuf>, String> {
    let roots = agent_roots(agent)?;
    let mut written = Vec::new();
    for root in roots {
        let path = home.join(root).join("skills").join("plexi-cli").join("SKILL.md");
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                log::error!(
                    "skill_install: agent={agent} path={} create failed: {error}",
                    parent.display()
                );
                return Err(format!("could not create {}: {error}", parent.display()));
            }
        }
        if let Err(error) = std::fs::write(&path, EMBEDDED_SKILL) {
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

/// `plexi skill install --agent claude|codex|all`
pub fn skill_install_cli(agent: &str) -> i32 {
    let home = match std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => {
            log::error!("skill_install: HOME is unset");
            eprintln!("error: HOME is unset");
            return 1;
        }
    };
    match install_embedded_skill(&home, agent) {
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
    use super::{install_embedded_skill, EMBEDDED_SKILL};

    #[test]
    fn install_writes_the_embedded_skill_for_both_agents() {
        let home = tempfile::tempdir().unwrap();
        let paths = install_embedded_skill(home.path(), "all").unwrap();
        assert_eq!(paths.len(), 2);
        for path in &paths {
            let text = std::fs::read_to_string(path).unwrap();
            assert_eq!(text, EMBEDDED_SKILL);
            assert!(
                text.contains(&format!("plexi_version: \"{}\"", env!("CARGO_PKG_VERSION"))),
                "embedded skill must match this binary's version"
            );
        }
        assert!(paths[0].ends_with(".claude/skills/plexi-cli/SKILL.md"));
        assert!(paths[1].ends_with(".codex/skills/plexi-cli/SKILL.md"));
    }
}
