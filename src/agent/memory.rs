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

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Once;

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
/// symlink anywhere below the scope base is refused.
pub fn ensure_memory_dir(definition: &AgentDefinition) -> Result<PathBuf, String> {
    let dir = memory_dir(definition)?;
    let (scope, root) = memory_scope(definition)?;
    let parent = state_scope::assert_within_scope(&dir, scope, UserDataKind::AgentMemory, &root)?;
    ensure_canonical_memory_dir(&dir, &parent.join(&definition.id))
}

/// Recheck the exact address saved at admission before handing it to Claude.
pub(crate) fn saved_memory_dir(dir: Option<&Path>) -> Result<Option<PathBuf>, String> {
    let Some(dir) = dir else {
        static LEGACY_NOTICE: Once = Once::new();
        LEGACY_NOTICE.call_once(|| {
            log::info!("agent_memory: saved Claude run has no recorded named memory directory");
        });
        return Ok(None);
    };
    if !dir.is_absolute()
        || dir.ancestors().nth(3).is_none()
        || dir.parent().and_then(Path::file_name) != Some(UserDataKind::AgentMemory.dir().as_ref())
    {
        return Err(format!(
            "invalid saved agent memory folder {}",
            dir.display()
        ));
    }
    ensure_canonical_memory_dir(dir, dir).map(Some)
}

fn ensure_canonical_memory_dir(dir: &Path, expected: &Path) -> Result<PathBuf, String> {
    let metadata = match fs::symlink_metadata(dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_directory_tree(expected)
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

/// Open each canonical ancestor without following links, then create only the
/// scope's three new components relative to the open parent descriptors.
#[cfg(unix)]
fn create_directory_tree(dir: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    #[cfg(target_vendor = "apple")]
    let access = libc::O_SEARCH;
    #[cfg(target_os = "linux")]
    let access = libc::O_PATH;
    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    let access = libc::O_RDONLY;

    let base = dir.ancestors().nth(3).ok_or(io::ErrorKind::InvalidInput)?;
    let mut parent = OwnedFd::from(fs::File::open("/")?);
    let mut path = PathBuf::from("/");
    for component in dir.components() {
        let Component::Normal(name) = component else {
            if component == Component::RootDir {
                continue;
            }
            return Err(io::ErrorKind::InvalidInput.into());
        };
        path.push(name);
        let name = CString::new(name.as_bytes())?;
        // SAFETY: parent is an open directory and name is one C string component.
        if path != base
            && path.starts_with(base)
            && unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o777) } == -1
        {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        // SAFETY: parent and name remain valid for the syscall.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                access | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a new owned descriptor.
        parent = unsafe { OwnedFd::from_raw_fd(fd) };
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_directory_tree(dir: &Path) -> io::Result<()> {
    let scope = dir.ancestors().take(4).collect::<Vec<_>>();
    let mut directories = scope.into_iter().rev();
    let base = directories.next().ok_or(io::ErrorKind::InvalidInput)?;
    if base.canonicalize()? != base {
        return Err(io::Error::other("redirected memory scope"));
    }
    for component in directories {
        match fs::create_dir(component) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            result => result?,
        }
        if fs::symlink_metadata(component)?.file_type().is_symlink()
            || component.canonicalize()? != component
        {
            return Err(io::Error::other(format!(
                "redirected directory {}",
                component.display()
            )));
        }
    }
    Ok(())
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

    #[cfg(unix)]
    #[test]
    fn saved_memory_refuses_a_redirected_scope() {
        let ws = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let root = ws
            .path()
            .canonicalize()
            .unwrap()
            .join(".plexi/agent_memory");
        let dir = root.join("writer");
        fs::create_dir_all(root.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), &root).unwrap();

        let error = saved_memory_dir(Some(&dir)).unwrap_err();
        assert!(error.contains(dir.to_str().unwrap()), "{error}");
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
