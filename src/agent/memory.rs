//! An agent's memory folder (stint 1397): one directory per agent definition,
//! shared by every chat of that agent, so a New chat keeps what it knows.
//!
//! Memory is user data, not config: it lives in the channel-neutral
//! `UserDataKind::AgentMemory` tier (`src/host/AGENTS.md`), so a beta or PR
//! build reads the same memory. The tier follows the definition, never a
//! launch: a workspace definition keeps it in its workspace
//! (`<root>/.plexi/agent_memory/<id>/`), a global or built-in one in
//! `~/.plexi/agent_memory/<id>/`. The host only ever creates the folder; what
//! is inside belongs to the agent and the person, and nothing here writes,
//! moves or deletes it. The definition folder (`AGENT.md`, `settings.toml`) is
//! never touched.

use std::path::{Component, Path, PathBuf};

use super::{AgentDefinition, AgentSource};
use crate::host::state_scope::{self, StateScope, UserDataKind};

/// The scope and context root `definition`'s memory resolves against.
fn memory_scope(definition: &AgentDefinition) -> Result<(StateScope, PathBuf), String> {
    match definition.source {
        AgentSource::User | AgentSource::BuiltIn => Ok((StateScope::Global, PathBuf::new())),
        AgentSource::Workspace => {
            let dir = definition.path.as_deref().ok_or_else(|| {
                format!(
                    "agent '{}' is a workspace definition with no directory; its memory folder cannot be placed",
                    definition.id
                )
            })?;
            let agents_dir = dir.parent();
            let root = agents_dir
                .and_then(Path::parent)
                .and_then(Path::parent)
                .filter(|root| {
                    agents_dir == Some(super::workspace_agents_dir(root).as_path())
                        || super::legacy_workspace_agents_dir(root).as_deref() == agents_dir
                })
                .ok_or_else(|| {
                    format!(
                        "agent '{}' definition {} is not inside a workspace agents directory; its memory folder cannot be placed",
                        definition.id,
                        dir.display()
                    )
                })?;
            Ok((StateScope::Context, root.to_path_buf()))
        }
    }
}

/// Where `definition`'s memory folder is. Pure: creates nothing.
pub fn memory_dir(definition: &AgentDefinition) -> Result<PathBuf, String> {
    let mut parts = Path::new(&definition.id).components();
    if !matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(_)), None)
    ) {
        return Err(format!(
            "agent id '{}' is not a single path name; its memory folder cannot be placed",
            definition.id
        ));
    }
    let (scope, root) = memory_scope(definition)?;
    Ok(state_scope::user_data_dir(scope, UserDataKind::AgentMemory, &root).join(&definition.id))
}

/// Create `definition`'s memory folder if it is missing and return it. An
/// existing folder and everything in it are left exactly as they are. A
/// redirected (symlinked) `.plexi/` or `agent_memory/` is refused.
pub fn ensure_memory_dir(definition: &AgentDefinition) -> Result<PathBuf, String> {
    let dir = memory_dir(definition)?;
    let (scope, root) = memory_scope(definition)?;
    state_scope::assert_within_scope(&dir, scope, UserDataKind::AgentMemory, &root)?;
    if dir.is_dir() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir).map_err(|error| {
        log::error!(
            "agent_memory: create memory folder for agent={} path={} failed: {error}",
            definition.id,
            dir.display()
        );
        format!(
            "create memory folder {} for agent '{}': {error}",
            dir.display(),
            definition.id
        )
    })?;
    log::info!(
        "agent_memory: created memory folder for agent={} scope={} path={}",
        definition.id,
        scope.as_str(),
        dir.display()
    );
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(id: &str, source: AgentSource, path: Option<PathBuf>) -> AgentDefinition {
        let mut def = super::super::builtin_default_agent().expect("built-in default");
        def.id = id.to_string();
        def.source = source;
        def.path = path;
        def
    }

    #[test]
    fn global_agent_memory_is_in_the_channel_neutral_shared_tier() {
        let shared = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_shared_dir(shared.path().to_path_buf());
        let dir = memory_dir(&definition("researcher", AgentSource::User, None)).unwrap();
        assert_eq!(dir, shared.path().join("agent_memory").join("researcher"));
        let builtin = memory_dir(&definition("default", AgentSource::BuiltIn, None)).unwrap();
        assert_eq!(builtin, shared.path().join("agent_memory").join("default"));
    }

    #[test]
    fn workspace_agent_memory_is_in_its_workspace() {
        let ws = tempfile::tempdir().unwrap();
        let def_dir = crate::agent::workspace_agents_dir(ws.path()).join("writer");
        let dir = memory_dir(&definition("writer", AgentSource::Workspace, Some(def_dir))).unwrap();
        assert_eq!(
            dir,
            ws.path().join(".plexi").join("agent_memory").join("writer")
        );
    }

    #[test]
    fn two_agents_never_share_a_memory_folder() {
        let shared = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_shared_dir(shared.path().to_path_buf());
        let ws = tempfile::tempdir().unwrap();
        let a = memory_dir(&definition("researcher", AgentSource::User, None)).unwrap();
        let b = memory_dir(&definition("writer", AgentSource::User, None)).unwrap();
        // The same id in two tiers is two definitions, so two folders.
        let shadow = memory_dir(&definition(
            "researcher",
            AgentSource::Workspace,
            Some(crate::agent::workspace_agents_dir(ws.path()).join("researcher")),
        ))
        .unwrap();
        assert_ne!(a, b);
        assert_ne!(a, shadow);
        assert!(!a.starts_with(&b) && !b.starts_with(&a));
    }

    #[test]
    fn ensure_creates_once_and_never_touches_what_is_inside() {
        let ws = tempfile::tempdir().unwrap();
        let def_dir = crate::agent::workspace_agents_dir(ws.path()).join("writer");
        std::fs::create_dir_all(&def_dir).unwrap();
        std::fs::write(def_dir.join("AGENT.md"), "persona\n").unwrap();
        std::fs::write(def_dir.join("settings.toml"), "[agent]\n").unwrap();
        let def = definition("writer", AgentSource::Workspace, Some(def_dir.clone()));

        let dir = ensure_memory_dir(&def).unwrap();
        assert!(dir.is_dir(), "created at first ensure");
        std::fs::write(dir.join("notes.md"), "remember this\n").unwrap();
        assert_eq!(ensure_memory_dir(&def).unwrap(), dir);

        assert_eq!(
            std::fs::read_to_string(dir.join("notes.md")).unwrap(),
            "remember this\n"
        );
        assert_eq!(
            std::fs::read_to_string(def_dir.join("AGENT.md")).unwrap(),
            "persona\n"
        );
        assert_eq!(
            std::fs::read_to_string(def_dir.join("settings.toml")).unwrap(),
            "[agent]\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_refuses_a_redirected_memory_directory() {
        let ws = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let plexi = ws.path().join(".plexi");
        std::fs::create_dir_all(&plexi).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), plexi.join("agent_memory")).unwrap();
        let def = definition(
            "writer",
            AgentSource::Workspace,
            Some(crate::agent::workspace_agents_dir(ws.path()).join("writer")),
        );
        let error = ensure_memory_dir(&def).unwrap_err();
        assert!(error.contains("escapes its scope"), "{error}");
        assert!(!elsewhere.path().join("writer").exists());
    }

    #[test]
    fn an_id_that_is_not_one_path_name_is_refused() {
        for id in ["../x", "a/b", ""] {
            assert!(
                memory_dir(&definition(id, AgentSource::User, None)).is_err(),
                "{id}"
            );
        }
    }
}
