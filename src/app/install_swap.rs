//! Crash-safe replacement of an installed plugin directory (stint 1175).
//!
//! An update stages the complete new tree outside the plugins directory, marks it
//! complete, then swaps it into place. A crash at any point leaves either the
//! old complete install or the new complete install at the destination, never
//! a missing or half-copied plugin, and never a second manifest-bearing directory
//! inside the plugins directory for the registry to load. Grants bind package
//! content (`package::content_digest`), so the swap itself needs no reapproval
//! logic: new bytes are a new identity and ask again.

use std::io;
use std::path::{Path, PathBuf};

use super::package::INSTALL_COMPLETE_MARKER;

/// Where staged installs live. Outside `plugins/` so the registry never scans them.
pub(crate) fn staging_root() -> PathBuf {
    crate::config::config_dir()
        .join("run")
        .join("install-staging")
}

/// Remove staging directories an interrupted install of `plugin_id` left behind.
pub(crate) fn clear_stale_staging(root: &Path, plugin_id: &str) {
    // App ids never contain '@', so one id cannot prefix another's staging.
    let prefix = format!("{plugin_id}@");
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            if let Err(error) = std::fs::remove_dir_all(entry.path()) {
                log::warn!(
                    "plugin::install: could not remove stale staging {}: {error}",
                    entry.path().display()
                );
            } else {
                log::info!(
                    "plugin::install: removed stale staging {} from an interrupted install",
                    entry.path().display()
                );
            }
        }
    }
}

/// A new, unique staging directory path for `plugin_id` under `root`.
pub(crate) fn new_staging_dir(root: &Path, plugin_id: &str) -> PathBuf {
    root.join(format!("{plugin_id}@{}", uuid::Uuid::new_v4().simple()))
}

/// Mark a fully copied staging tree complete. Written last, before the swap.
pub(crate) fn mark_complete(staging: &Path) -> io::Result<()> {
    std::fs::write(staging.join(INSTALL_COMPLETE_MARKER), b"complete\n")
}

/// Make `staging` the content of `dest`. With an existing `dest` on macOS this
/// is one atomic exchange (`renamex_np(RENAME_SWAP)`); the displaced old tree
/// is then removed. Elsewhere, and when `dest` is absent, it is a rename.
pub(crate) fn replace_dir(staging: &Path, dest: &Path) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !dest.exists() {
        return std::fs::rename(staging, dest);
    }
    swap_in_place(staging, dest)?;
    // `staging` now holds the previous install.
    if let Err(error) = std::fs::remove_dir_all(staging) {
        log::warn!(
            "plugin::install: could not remove replaced install {}: {error}",
            staging.display()
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn swap_in_place(staging: &Path, dest: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let to_c = |path: &Path| {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
    };
    let (from, to) = (to_c(staging)?, to_c(dest)?);
    // SAFETY: both pointers are valid NUL-terminated strings for the call.
    let rc = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn swap_in_place(staging: &Path, dest: &Path) -> io::Result<()> {
    // No atomic directory exchange here: park the old tree, move the new one in,
    // and put the old one back if that fails. A crash between the two renames
    // leaves `dest` absent, which launches nothing and is repaired by
    // reinstalling; it never leaves a partial tree.
    let parked = staging.with_extension("previous");
    std::fs::rename(dest, &parked)?;
    if let Err(error) = std::fs::rename(staging, dest) {
        let _ = std::fs::rename(&parked, dest);
        return Err(error);
    }
    std::fs::rename(&parked, staging)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path, files: &[(&str, &str)]) {
        for (name, body) in files {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
    }

    #[test]
    fn replacing_swaps_content_atomically_and_removes_the_old_tree() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("apps/x");
        let staging = new_staging_dir(&root.path().join("staging"), "x");
        tree(&dest, &[("main.py", "old"), ("only-old.txt", "gone")]);
        tree(&staging, &[("main.py", "new")]);
        mark_complete(&staging).unwrap();
        replace_dir(&staging, &dest).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("main.py")).unwrap(),
            "new"
        );
        assert!(
            !dest.join("only-old.txt").exists(),
            "no file from the old install may survive"
        );
        assert!(dest.join(INSTALL_COMPLETE_MARKER).is_file());
        assert!(!staging.exists(), "the displaced tree is removed");
    }

    #[test]
    fn first_install_has_no_previous_tree() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("apps/x");
        let staging = new_staging_dir(&root.path().join("staging"), "x");
        tree(&staging, &[("main.py", "new")]);
        mark_complete(&staging).unwrap();
        replace_dir(&staging, &dest).unwrap();
        assert!(dest.join(INSTALL_COMPLETE_MARKER).is_file());
    }

    #[test]
    fn stale_staging_of_one_plugin_is_cleared_and_others_are_kept() {
        let root = tempfile::tempdir().unwrap();
        let staging_root = root.path().join("staging");
        let mine = new_staging_dir(&staging_root, "x");
        let other = new_staging_dir(&staging_root, "x-y");
        tree(&mine, &[("main.py", "half")]);
        tree(&other, &[("main.py", "half")]);
        clear_stale_staging(&staging_root, "x");
        assert!(!mine.exists());
        assert!(
            other.exists(),
            "prefix match must not eat another plugin's staging"
        );
    }
}
