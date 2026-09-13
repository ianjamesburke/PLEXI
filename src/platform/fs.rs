//! Filesystem primitives shared by every subsystem that persists state.
//!
//! One atomic writer for the whole tree. Before this module the temp-file +
//! rename dance was open-coded in nine places with nine slightly different
//! error postures; the canonical implementation lived in
//! `host::state_scope` and was the only one that created the parent
//! directory and fsynced before renaming.

use std::path::Path;

/// Write `bytes` to `path` atomically: sibling temp file, fsync, rename.
/// A reader never observes a partial file; a crash leaves at worst an
/// orphaned `.{name}.tmp-{uuid}` sibling, which the next successful write
/// does not touch and no reader will ever open.
///
/// The parent directory is created if missing. The temp name is uuid-unique,
/// so concurrent writers to the same destination never clobber each other's
/// staged content.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic(path, bytes, None)
}

/// `atomic_write` for a file only the owner may read. `unix_mode` is applied
/// to the temp file *before* the rename, so the destination is never visible
/// with wider permissions than intended. On non-unix targets the mode is
/// ignored and this is exactly `atomic_write`.
pub fn atomic_write_with_mode(path: &Path, bytes: &[u8], unix_mode: u32) -> Result<(), String> {
    write_atomic(path, bytes, Some(unix_mode))
}

fn write_atomic(path: &Path, bytes: &[u8], unix_mode: Option<u32>) -> Result<(), String> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| format!("write {}: missing parent", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut file =
            std::fs::File::create(&temp).map_err(|e| format!("create {}: {e}", temp.display()))?;
        file.write_all(bytes)
            .map_err(|e| format!("write {}: {e}", temp.display()))?;
        file.sync_all()
            .map_err(|e| format!("sync {}: {e}", temp.display()))?;
        apply_mode(&temp, unix_mode)?;
        std::fs::rename(&temp, path)
            .map_err(|e| format!("rename {} to {}: {e}", temp.display(), path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(unix)]
fn apply_mode(temp: &Path, unix_mode: Option<u32>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let Some(mode) = unix_mode else {
        return Ok(());
    };
    std::fs::set_permissions(temp, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("chmod {}: {e}", temp.display()))
}

#[cfg(not(unix))]
fn apply_mode(_temp: &Path, _unix_mode: Option<u32>) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_leaves_no_temp_residue() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("app_states").join("todo.json");
        atomic_write(&path, b"{\"k\":1}").expect("atomic write");
        assert_eq!(std::fs::read(&path).expect("read back"), b"{\"k\":1}");
        let residue: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(residue.is_empty(), "temp residue left behind: {residue:?}");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_with_mode_lands_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("identity.json");
        atomic_write_with_mode(&path, b"{\"port\":1}", 0o600).expect("atomic write");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "identity file must never be world-readable"
        );
    }
}
