//! Where the text editor left each file (stint 1236).
//!
//! The host owns one bounded store, keyed by canonical path. An entry is only
//! valid while the file still has the content hash recorded with it, so an
//! edit made anywhere else sends the file back to its default position. The
//! store lives in the channel profile beside the other editor state
//! (`editor-buffers.json`): it is view state, not a document the user owns,
//! and the editor never writes metadata to the files it shows.

use crate::editor::{Cursor, Selection};
use std::path::{Path, PathBuf};

/// Most files remembered at once. Recording one more evicts the file whose
/// position was recorded longest ago.
pub const MAX_ENTRIES: usize = 500;
const STORE_FILE: &str = "editor-positions.json";
const STORE_VERSION: u32 = 1;

/// Caret, selection and scroll of one file, valid for one content hash.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EditorPosition {
    pub selection: Selection,
    /// Source position at the top of the viewport (`ScrollAnchor::cursor`).
    pub scroll_top: Cursor,
    /// Points into that display row (`ScrollAnchor::offset`).
    pub scroll_offset: f32,
    pub scroll_x: f32,
    /// FNV-1a of the file bytes the position was recorded against.
    pub content_hash: u64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Entry {
    file: PathBuf,
    position: EditorPosition,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoreFile {
    version: u32,
    /// Oldest first.
    entries: Vec<Entry>,
}

/// The persisted store. Callers outside this module use [`lookup`] and
/// [`remember`]; the struct is the unit the bound and the format are tested on.
#[derive(Debug)]
pub struct EditorPositionStore {
    path: PathBuf,
    cap: usize,
    entries: Vec<Entry>,
}

impl EditorPositionStore {
    /// Reads the store at `path`. A missing file is an empty store; an
    /// unreadable or unknown-version file is an error naming the file.
    pub fn open(path: PathBuf, cap: usize) -> Result<Self, String> {
        let entries = match std::fs::read(&path) {
            Ok(bytes) => {
                let file: StoreFile = serde_json::from_slice(&bytes).map_err(|e| {
                    format!("editor positions: {} does not parse: {e}", path.display())
                })?;
                if file.version != STORE_VERSION {
                    return Err(format!(
                        "editor positions: {} has version {}, this build reads {STORE_VERSION}",
                        path.display(),
                        file.version
                    ));
                }
                file.entries
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(format!(
                    "editor positions: cannot read {}: {e}",
                    path.display()
                ))
            }
        };
        Ok(Self { path, cap, entries })
    }

    /// The position recorded for `file` (a canonical path), if it was
    /// recorded against `content_hash`. A position for other content is
    /// ignored: the file changed since it was recorded.
    pub fn get(&self, file: &Path, content_hash: u64) -> Option<EditorPosition> {
        let entry = self.entries.iter().find(|entry| entry.file == file)?;
        if entry.position.content_hash != content_hash {
            log::info!(
                "editor_positions: ignoring position for {} (file changed since it was recorded)",
                file.display()
            );
            return None;
        }
        Some(entry.position)
    }

    /// Records `position` for `file` (a canonical path) as the newest entry,
    /// evicting the oldest entries beyond the cap.
    pub fn remember(&mut self, file: PathBuf, position: EditorPosition) {
        self.entries.retain(|entry| entry.file != file);
        self.entries.push(Entry { file, position });
        let excess = self.entries.len().saturating_sub(self.cap);
        if excess > 0 {
            self.entries = self.entries.split_off(excess);
            log::info!(
                "editor_positions: evicted {excess} oldest entries (cap {})",
                self.cap
            );
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let body = serde_json::to_vec(&StoreFile {
            version: STORE_VERSION,
            entries: self.entries.clone(),
        })
        .map_err(|e| format!("editor positions: serialize {}: {e}", self.path.display()))?;
        crate::platform::fs::atomic_write(&self.path, &body)
            .map_err(|e| format!("editor positions: {e}"))
    }
}

pub(crate) fn store_path() -> PathBuf {
    crate::config::config_dir().join(STORE_FILE)
}

fn identity(file: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(file)
        .map_err(|e| format!("editor positions: cannot resolve {}: {e}", file.display()))
}

/// The remembered position for `file` whose current content hashes to
/// `content_hash`, or `None` when there is none or the file changed since.
pub fn lookup(file: &Path, content_hash: u64) -> Option<EditorPosition> {
    let file = match identity(file) {
        Ok(file) => file,
        Err(e) => {
            log::warn!("{e}");
            return None;
        }
    };
    match EditorPositionStore::open(store_path(), MAX_ENTRIES) {
        Ok(store) => store.get(&file, content_hash),
        Err(e) => {
            log::warn!("{e}; opening {} at the default position", file.display());
            None
        }
    }
}

/// Records where `file` was left. An unreadable store is replaced, since it
/// only ever holds positions; the warning names the file that was replaced.
pub fn remember(file: &Path, position: EditorPosition) {
    let file = match identity(file) {
        Ok(file) => file,
        Err(e) => {
            log::warn!("{e}; position not recorded");
            return;
        }
    };
    let path = store_path();
    let mut store = match EditorPositionStore::open(path.clone(), MAX_ENTRIES) {
        Ok(store) => store,
        Err(e) => {
            log::warn!("{e}; replacing it with a new store");
            EditorPositionStore {
                path,
                cap: MAX_ENTRIES,
                entries: Vec::new(),
            }
        }
    };
    store.remember(file.clone(), position);
    match store.save() {
        Ok(()) => log::info!(
            "editor_positions: recorded {} caret={}:{}",
            file.display(),
            position.selection.head.line,
            position.selection.head.column
        ),
        Err(e) => log::warn!("{e}; position for {} not recorded", file.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(line: usize, content_hash: u64) -> EditorPosition {
        EditorPosition {
            selection: Selection::collapsed(Cursor::new(line, 0)),
            scroll_top: Cursor::new(line, 0),
            scroll_offset: 0.0,
            scroll_x: 0.0,
            content_hash,
        }
    }

    #[test]
    fn store_stays_bounded_and_evicts_the_oldest_entry_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STORE_FILE);
        let mut store = EditorPositionStore::open(path.clone(), 3).unwrap();
        for n in 0..4 {
            store.remember(PathBuf::from(format!("/files/{n}.txt")), position(n, 7));
        }
        assert_eq!(store.entries.len(), 3, "store must hold at most its cap");
        assert_eq!(
            store.get(Path::new("/files/0.txt"), 7),
            None,
            "oldest evicted"
        );
        assert_eq!(
            store.get(Path::new("/files/3.txt"), 7),
            Some(position(3, 7))
        );

        // Re-recording a file makes it the newest, so it survives the next eviction.
        store.remember(PathBuf::from("/files/1.txt"), position(10, 7));
        store.remember(PathBuf::from("/files/4.txt"), position(4, 7));
        assert_eq!(store.entries.len(), 3);
        assert_eq!(store.get(Path::new("/files/2.txt"), 7), None);
        assert_eq!(
            store.get(Path::new("/files/1.txt"), 7),
            Some(position(10, 7))
        );

        store.save().unwrap();
        let reread = EditorPositionStore::open(path, 3).unwrap();
        assert_eq!(reread.entries, store.entries, "the bound survives a save");
    }

    #[test]
    fn position_recorded_for_other_content_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = EditorPositionStore::open(dir.path().join(STORE_FILE), 3).unwrap();
        store.remember(PathBuf::from("/files/a.txt"), position(9, 1));
        assert_eq!(store.get(Path::new("/files/a.txt"), 2), None);
        assert_eq!(
            store.get(Path::new("/files/a.txt"), 1),
            Some(position(9, 1))
        );
    }

    #[test]
    fn unreadable_or_unknown_version_store_is_an_error_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STORE_FILE);
        std::fs::write(&path, "not json").unwrap();
        let error = EditorPositionStore::open(path.clone(), 3).unwrap_err();
        assert!(error.contains(STORE_FILE), "{error}");

        std::fs::write(&path, r#"{"version":99,"entries":[]}"#).unwrap();
        let error = EditorPositionStore::open(path, 3).unwrap_err();
        assert!(error.contains("version 99"), "{error}");
    }

    #[test]
    fn lookup_and_remember_round_trip_through_the_profile_store() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "a").unwrap();

        assert_eq!(lookup(&file, 5), None);
        remember(&file, position(3, 5));
        assert_eq!(store_path(), profile.join(STORE_FILE));
        assert_eq!(lookup(&file, 5), Some(position(3, 5)));
        assert_eq!(
            lookup(&file, 6),
            None,
            "changed content opens at the default"
        );
    }
}
