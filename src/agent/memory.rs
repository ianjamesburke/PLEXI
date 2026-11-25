//! An agent's memory folder (stint 1397): one directory per agent definition,
//! shared by every chat of that agent, so a New chat keeps what it knows.
//!
//! Memory is user data, not config: it lives in the channel-neutral
//! `UserDataKind::AgentMemory` tier (`src/host/AGENTS.md`), so a beta or PR
//! build reads the same memory. The tier follows the definition, never a
//! launch. For harnesses that read memory by prompt, a workspace definition
//! keeps it in `<root>/.plexi/agent_memory/<id>/`. Named Claude launches
//! bind workspace memory under `.workspaces/<key>/<id>/` in the global memory
//! tier. Global and built-in definitions use `~/.plexi/agent_memory/<id>/`.
//! The host only ever creates the folder; what
//! is inside belongs to the agent and the person, and nothing here writes,
//! moves or deletes it. The definition folder (`AGENT.md`, `settings.toml`) is
//! never touched.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    super::validate_agent_id(&definition.id)?;
    let (scope, root) = memory_scope(definition)?;
    Ok(state_scope::user_data_dir(scope, UserDataKind::AgentMemory, &root).join(&definition.id))
}

/// Create `definition`'s memory folder if it is missing and return it. An
/// existing folder and everything in it are left exactly as they are. A
/// symlink anywhere below the scope base is refused.
pub fn ensure_memory_dir(definition: &AgentDefinition) -> Result<PathBuf, String> {
    let dir = memory_dir(definition)?;
    let (scope, root) = memory_scope(definition)?;
    let parent = state_scope::assert_within_scope(&dir, scope, UserDataKind::AgentMemory, &root)?;
    ensure_canonical_memory_dir(&dir, &parent.join(&definition.id))
}

/// The named Claude memory selected by the host, independent of later definition changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ClaudeMemoryIdentity {
    scope: ClaudeMemoryScope,
    #[serde(deserialize_with = "decode_agent_id")]
    agent_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClaudeMemoryScope {
    Global,
    Workspace(#[serde(deserialize_with = "decode_workspace_key")] String),
}

fn decode_agent_id<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<String, D::Error> {
    let id = String::deserialize(decoder)?;
    super::validate_agent_id(&id).map_err(serde::de::Error::custom)?;
    Ok(id)
}

fn decode_workspace_key<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<String, D::Error> {
    let key = String::deserialize(decoder)?;
    if key.len() != 64
        || !key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(serde::de::Error::custom(
            "invalid Claude memory workspace key",
        ));
    }
    Ok(key)
}

impl ClaudeMemoryIdentity {
    pub(crate) fn new(
        definition: &AgentDefinition,
        workspace: Option<&Path>,
    ) -> Result<Self, String> {
        super::validate_agent_id(&definition.id)?;
        let scope = match definition.source {
            AgentSource::User | AgentSource::BuiltIn => ClaudeMemoryScope::Global,
            AgentSource::Workspace => {
                let root = workspace.ok_or_else(|| {
                    format!(
                        "agent '{}' has no resolved workspace for Claude memory",
                        definition.id
                    )
                })?;
                let canonical = root.canonicalize().map_err(|error| {
                    format!(
                        "resolve Claude memory workspace {}: {error}",
                        root.display()
                    )
                })?;
                ClaudeMemoryScope::Workspace(format!(
                    "{:x}",
                    Sha256::digest(canonical.as_os_str().as_encoded_bytes())
                ))
            }
        };
        Ok(Self {
            scope,
            agent_id: definition.id.clone(),
        })
    }

    pub(crate) fn matches_role(&self, role: Option<&str>) -> bool {
        role == Some(self.agent_id.as_str())
    }

    pub(crate) fn ensure_dir(&self) -> Result<PathBuf, String> {
        let root = Path::new("");
        let kind = UserDataKind::AgentMemory;
        let base = state_scope::user_data_dir(StateScope::Global, kind, root);
        let mut expected = state_scope::assert_within_scope(
            &base.join(&self.agent_id),
            StateScope::Global,
            kind,
            root,
        )?;
        if let ClaudeMemoryScope::Workspace(key) = &self.scope {
            for part in [".workspaces", key.as_str()] {
                expected.push(part);
                ensure_canonical_memory_dir(&expected, &expected)?;
            }
        }
        let dir = expected.join(&self.agent_id);
        ensure_canonical_memory_dir(&dir, &dir)
    }
}

fn ensure_canonical_memory_dir(dir: &Path, expected: &Path) -> Result<PathBuf, String> {
    let metadata = match fs::symlink_metadata(dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(dir)
                .map_err(|error| format!("create memory folder {}: {error}", dir.display()))?;
            log::info!(
                "agent_memory: prepared memory folder path={}",
                dir.display()
            );
            fs::symlink_metadata(dir)
        }
        result => result,
    }
    .map_err(|error| format!("inspect memory folder {}: {error}", dir.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "agent memory folder {} is a symlink",
            dir.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "agent memory folder {} is not a directory",
            dir.display()
        ));
    }
    let actual = fs::canonicalize(dir)
        .map_err(|error| format!("canonicalize memory folder {}: {error}", dir.display()))?;
    if actual != expected {
        return Err(format!(
            "agent memory folder {} escapes its scope: resolves to {} instead of {}",
            dir.display(),
            actual.display(),
            expected.display(),
        ));
    }
    Ok(actual)
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

    #[cfg(unix)]
    #[test]
    fn ensure_refuses_a_symlink_at_the_agent_directory() {
        let ws = tempfile::tempdir().unwrap();
        let def = definition(
            "writer",
            AgentSource::Workspace,
            Some(crate::agent::workspace_agents_dir(ws.path()).join("writer")),
        );
        let dir = memory_dir(&def).unwrap();
        let other_agent = dir.with_file_name("researcher");
        std::fs::create_dir_all(&other_agent).unwrap();
        std::os::unix::fs::symlink(&other_agent, &dir).unwrap();

        let error = ensure_memory_dir(&def).expect_err("refuse another agent's memory");
        assert!(error.contains("symlink"), "{error}");
        assert!(error.contains(dir.to_str().unwrap()), "{error}");
    }

    #[test]
    fn saved_memory_identity_rejects_aliases_and_invalid_workspace_keys() {
        for value in [
            serde_json::json!({"scope": "global", "agent_id": "writer/"}),
            serde_json::json!({"scope": {"workspace": "../other"}, "agent_id": "writer"}),
        ] {
            assert!(serde_json::from_value::<ClaudeMemoryIdentity>(value).is_err());
        }
        let identity: ClaudeMemoryIdentity =
            serde_json::from_value(serde_json::json!({"scope": "global", "agent_id": "default"}))
                .unwrap();
        assert_eq!(identity.agent_id, "default");
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
