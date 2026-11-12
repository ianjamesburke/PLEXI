//! Where the text editor left each file (stint 1236).
//!
//! The host owns one bounded store per profile, keyed by canonical path. An
//! entry is only valid while the file still has the content hash recorded with
//! it, so an edit made anywhere else sends the file back to its default
//! position. The store lives in the channel profile beside the other editor
//! state (`editor-buffers.json`): it is view state, not a document the user
//! owns, and the editor never writes metadata to the files it shows.
//!
//! The editor only touches memory: [`lookup`] and [`remember`] never do file
//! I/O beyond resolving the file's identity. The disk copy is loaded and saved
//! on background threads that [`service`] starts and drains from `App::logic`
//! (the notes-index refresh pattern); [`flush_now`] is the blocking final
//! write on the quit path. The in-memory store is process-global and keyed by
//! profile, like `PermissionMonitor::loaded`, because a view has no handle on
//! the host and must still reach the same store from every pane.

use crate::editor::{Cursor, Selection};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};

/// Most files remembered at once. Recording one more evicts the file whose
/// position was recorded longest ago.
pub const MAX_ENTRIES: usize = 500;
const STORE_FILE: &str = "editor-positions.json";
const STORE_VERSION: u32 = 1;
/// Notice titles. Tests match on them.
pub(crate) const SAVE_FAILED_TITLE: &str = "Editor positions not saved";
pub(crate) const LOAD_FAILED_TITLE: &str = "Editor positions not loaded";

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

/// A user-visible failure the host turns into a notification.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Notice {
    pub title: &'static str,
    pub body: String,
}

/// One profile's positions, in memory. Oldest entry first.
#[derive(Debug)]
pub struct EditorPositionStore {
    path: PathBuf,
    cap: usize,
    entries: Vec<Entry>,
}

impl EditorPositionStore {
    fn empty(path: PathBuf, cap: usize) -> Self {
        Self {
            path,
            cap,
            entries: Vec::new(),
        }
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
        self.evict();
    }

    /// Folds in entries read from disk after this store already took new
    /// positions. The in-memory positions are newer, so they win.
    fn merge_older(&mut self, older: Vec<Entry>) {
        let mut merged: Vec<Entry> = older
            .into_iter()
            .filter(|old| !self.entries.iter().any(|new| new.file == old.file))
            .collect();
        merged.append(&mut self.entries);
        self.entries = merged;
        self.evict();
    }

    fn evict(&mut self) {
        let excess = self.entries.len().saturating_sub(self.cap);
        if excess > 0 {
            self.entries = self.entries.split_off(excess);
            log::info!(
                "editor_positions: evicted {excess} oldest entries (cap {})",
                self.cap
            );
        }
    }

    fn encode(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&StoreFile {
            version: STORE_VERSION,
            entries: self.entries.clone(),
        })
        .map_err(|e| format!("editor positions: serialize {}: {e}", self.path.display()))
    }
}

/// Entries of the store file at `path`. A missing file is empty; an
/// unreadable or unknown-version file is an error naming the file.
fn read_entries(path: &Path) -> Result<Vec<Entry>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(format!(
                "editor positions: cannot read {}: {e}",
                path.display()
            ))
        }
    };
    let file: StoreFile = serde_json::from_slice(&bytes)
        .map_err(|e| format!("editor positions: {} does not parse: {e}", path.display()))?;
    if file.version != STORE_VERSION {
        return Err(format!(
            "editor positions: {} has version {}, this build reads {STORE_VERSION}",
            path.display(),
            file.version
        ));
    }
    Ok(file.entries)
}

/// Serializes every write of one store file, from the background saver and
/// the quit flush. A snapshot older than one already written is dropped, so a
/// late background write never replaces newer positions. Writes go through
/// `atomic_write` (temp file, then rename): a failed write leaves the
/// previous file as it was.
#[derive(Default)]
struct Writer {
    written: Mutex<u64>,
}

impl Writer {
    fn write(&self, path: &Path, generation: u64, body: &[u8]) -> Result<(), String> {
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if generation <= *written {
            return Ok(());
        }
        crate::platform::fs::atomic_write(path, body)
            .map_err(|e| format!("editor positions: {e}"))?;
        *written = generation;
        Ok(())
    }

    fn written(&self) -> u64 {
        *self.written.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct SaveOutcome {
    generation: u64,
    result: Result<(), String>,
}

/// One profile's store plus its background load and save state.
struct Slot {
    store: EditorPositionStore,
    /// Bumped by every [`remember`].
    generation: u64,
    /// Highest generation handed to a writer, in flight or finished. A failed
    /// save is retried by the next change or by the quit flush, not per frame.
    queued: u64,
    /// The disk copy has been merged in (or failed to load). Saving waits for
    /// it so a save never drops entries from earlier sessions.
    loaded: bool,
    load_rx: Option<Receiver<Result<Vec<Entry>, String>>>,
    save_rx: Option<Receiver<SaveOutcome>>,
    writer: Arc<Writer>,
}

impl Slot {
    fn new(path: PathBuf) -> Self {
        Self {
            store: EditorPositionStore::empty(path, MAX_ENTRIES),
            generation: 0,
            queued: 0,
            loaded: false,
            load_rx: None,
            save_rx: None,
            writer: Arc::new(Writer::default()),
        }
    }

    fn service(&mut self, wake: &Arc<dyn crate::app::ui_mailbox::UiWake>) -> Vec<Notice> {
        let mut notices = Vec::new();
        self.drain_load(&mut notices);
        if !self.loaded && self.load_rx.is_none() {
            self.kick_load(wake, &mut notices);
        }
        self.drain_save(&mut notices);
        if self.loaded && self.save_rx.is_none() && self.generation > self.queued {
            self.kick_save(wake, &mut notices);
        }
        notices
    }

    fn kick_load(
        &mut self,
        wake: &Arc<dyn crate::app::ui_mailbox::UiWake>,
        notices: &mut Vec<Notice>,
    ) {
        let (tx, rx) = mpsc::channel();
        let path = self.store.path.clone();
        let wake = Arc::clone(wake);
        let spawned = std::thread::Builder::new()
            .name("editor-positions-load".to_string())
            .spawn(move || {
                let _ = tx.send(read_entries(&path));
                wake.wake("editor_positions_load");
            });
        match spawned {
            Ok(_) => self.load_rx = Some(rx),
            Err(e) => {
                log::warn!(
                    "editor_positions: cannot spawn the loader ({e}); reading {} inline",
                    self.store.path.display()
                );
                let result = read_entries(&self.store.path);
                self.finish_load(result, notices);
            }
        }
    }

    fn drain_load(&mut self, notices: &mut Vec<Notice>) {
        let received = {
            let Some(rx) = self.load_rx.as_ref() else {
                return;
            };
            rx.try_recv()
        };
        let result = match received {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(format!(
                "editor positions: loader for {} stopped without a result",
                self.store.path.display()
            )),
        };
        self.load_rx = None;
        self.finish_load(result, notices);
    }

    fn finish_load(&mut self, result: Result<Vec<Entry>, String>, notices: &mut Vec<Notice>) {
        self.loaded = true;
        match result {
            Ok(older) => {
                let count = older.len();
                self.store.merge_older(older);
                log::info!(
                    "editor_positions: loaded {count} entries from {}",
                    self.store.path.display()
                );
            }
            Err(e) => {
                log::warn!("{e}; the next save replaces it");
                notices.push(Notice {
                    title: LOAD_FAILED_TITLE,
                    body: format!(
                        "{e}. Positions from earlier sessions are unavailable; \
                         the file is replaced on the next save."
                    ),
                });
            }
        }
    }

    fn kick_save(
        &mut self,
        wake: &Arc<dyn crate::app::ui_mailbox::UiWake>,
        notices: &mut Vec<Notice>,
    ) {
        let generation = self.generation;
        self.queued = generation;
        let body = match self.store.encode() {
            Ok(body) => body,
            Err(e) => {
                log::warn!("{e}");
                notices.push(save_failed(e));
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        let writer = Arc::clone(&self.writer);
        let path = self.store.path.clone();
        let wake = Arc::clone(wake);
        let spawned = std::thread::Builder::new()
            .name("editor-positions-save".to_string())
            .spawn(move || {
                let result = writer.write(&path, generation, &body);
                let _ = tx.send(SaveOutcome { generation, result });
                wake.wake("editor_positions_save");
            });
        match spawned {
            Ok(_) => self.save_rx = Some(rx),
            Err(e) => {
                let e = format!("editor positions: cannot spawn the save thread: {e}");
                log::warn!("{e}");
                notices.push(save_failed(e));
            }
        }
    }

    fn drain_save(&mut self, notices: &mut Vec<Notice>) {
        let received = {
            let Some(rx) = self.save_rx.as_ref() else {
                return;
            };
            rx.try_recv()
        };
        let outcome = match received {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => SaveOutcome {
                generation: self.queued,
                result: Err(format!(
                    "editor positions: save thread for {} stopped without a result",
                    self.store.path.display()
                )),
            },
        };
        self.save_rx = None;
        match outcome.result {
            Ok(()) => log::info!(
                "editor_positions: saved generation {} to {}",
                outcome.generation,
                self.store.path.display()
            ),
            Err(e) => {
                log::warn!("{e}");
                notices.push(save_failed(e));
            }
        }
    }

    /// Blocking final write for the quit path.
    fn flush_now(&mut self) -> Vec<Notice> {
        let mut notices = Vec::new();
        if self.generation <= self.writer.written() {
            return notices;
        }
        if !self.loaded {
            // Merge the disk copy first so the write keeps earlier sessions.
            // An in-flight loader's result is dropped with its receiver.
            self.load_rx = None;
            let result = read_entries(&self.store.path);
            self.finish_load(result, &mut notices);
        }
        let generation = self.generation;
        self.queued = generation;
        let result = self
            .store
            .encode()
            .and_then(|body| self.writer.write(&self.store.path, generation, &body));
        match result {
            Ok(()) => log::info!(
                "editor_positions: flushed generation {generation} to {}",
                self.store.path.display()
            ),
            Err(e) => {
                log::error!("{e}");
                notices.push(save_failed(e));
            }
        }
        notices
    }
}

fn save_failed(error: String) -> Notice {
    Notice {
        title: SAVE_FAILED_TITLE,
        body: format!(
            "{error}. The previous file is unchanged; positions stay in memory \
             and are saved again on the next change or at quit."
        ),
    }
}

fn slots() -> &'static Mutex<HashMap<PathBuf, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<PathBuf, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Runs `f` on the current profile's slot, creating it on first use.
fn with_slot<T>(f: impl FnOnce(&mut Slot) -> T) -> T {
    let path = store_path();
    let mut slots = slots().lock().unwrap_or_else(|e| e.into_inner());
    let slot = slots.entry(path.clone()).or_insert_with(|| Slot::new(path));
    f(slot)
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
/// Reads memory only; a file opened before the disk copy finishes loading
/// opens at its default position.
pub fn lookup(file: &Path, content_hash: u64) -> Option<EditorPosition> {
    let file = match identity(file) {
        Ok(file) => file,
        Err(e) => {
            log::warn!("{e}");
            return None;
        }
    };
    with_slot(|slot| slot.store.get(&file, content_hash))
}

/// Records where `file` was left, in memory. [`service`] persists it off the
/// UI thread.
pub fn remember(file: &Path, position: EditorPosition) {
    let file = match identity(file) {
        Ok(file) => file,
        Err(e) => {
            log::warn!("{e}; position not recorded");
            return;
        }
    };
    let generation = with_slot(|slot| {
        slot.store.remember(file.clone(), position);
        slot.generation += 1;
        slot.generation
    });
    log::info!(
        "editor_positions: recorded {} caret={}:{} generation={generation}",
        file.display(),
        position.selection.head.line,
        position.selection.head.column
    );
}

/// Per-frame servicing from `App::logic`: starts the one background load,
/// merges it, starts a background save when positions changed, and returns
/// failures for the host to show.
pub(crate) fn service(wake: &Arc<dyn crate::app::ui_mailbox::UiWake>) -> Vec<Notice> {
    with_slot(|slot| slot.service(wake))
}

/// Writes any unsaved positions now, blocking. Quit path only.
pub(crate) fn flush_now() -> Vec<Notice> {
    with_slot(Slot::flush_now)
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

    fn entry(name: &str, line: usize) -> Entry {
        Entry {
            file: PathBuf::from(format!("/files/{name}.txt")),
            position: position(line, 7),
        }
    }

    fn isolated_profile() -> (tempfile::TempDir, crate::config::TestProfileDirGuard) {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let guard = crate::config::set_test_profile_dir(profile);
        (dir, guard)
    }

    #[test]
    fn store_stays_bounded_and_evicts_the_oldest_entry_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STORE_FILE);
        let mut store = EditorPositionStore::empty(path.clone(), 3);
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

        Writer::default()
            .write(&path, 1, &store.encode().unwrap())
            .unwrap();
        assert_eq!(
            read_entries(&path).unwrap(),
            store.entries,
            "the bound survives a save"
        );
    }

    #[test]
    fn position_recorded_for_other_content_is_ignored() {
        let mut store = EditorPositionStore::empty(PathBuf::from("/unused"), 3);
        store.remember(PathBuf::from("/files/a.txt"), position(9, 1));
        assert_eq!(store.get(Path::new("/files/a.txt"), 2), None);
        assert_eq!(
            store.get(Path::new("/files/a.txt"), 1),
            Some(position(9, 1))
        );
    }

    #[test]
    fn loaded_entries_merge_under_newer_in_memory_positions() {
        let mut store = EditorPositionStore::empty(PathBuf::from("/unused"), 3);
        store.remember(PathBuf::from("/files/b.txt"), position(20, 7));
        store.merge_older(vec![
            entry("a", 1),
            entry("b", 2),
            entry("c", 3),
            entry("d", 4),
        ]);
        assert_eq!(
            store.get(Path::new("/files/b.txt"), 7),
            Some(position(20, 7)),
            "the in-memory position is newer than the disk copy"
        );
        assert_eq!(store.entries.len(), 3, "a merge respects the cap");
        assert_eq!(
            store.get(Path::new("/files/a.txt"), 7),
            None,
            "the oldest disk entry is evicted first"
        );
    }

    #[test]
    fn unreadable_or_unknown_version_store_is_an_error_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STORE_FILE);
        std::fs::write(&path, "not json").unwrap();
        let error = read_entries(&path).unwrap_err();
        assert!(error.contains(STORE_FILE), "{error}");

        std::fs::write(&path, r#"{"version":99,"entries":[]}"#).unwrap();
        let error = read_entries(&path).unwrap_err();
        assert!(error.contains("version 99"), "{error}");
    }

    #[test]
    fn writer_never_replaces_a_newer_snapshot_with_an_older_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STORE_FILE);
        let writer = Writer::default();
        writer.write(&path, 2, b"newer").unwrap();
        writer.write(&path, 1, b"older").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"newer");
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_write_leaves_the_previous_file_intact() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("profile");
        std::fs::create_dir_all(&store_dir).unwrap();
        let path = store_dir.join(STORE_FILE);
        let writer = Writer::default();
        writer.write(&path, 1, b"previous").unwrap();

        // No temp file can be created beside the store, so the write fails
        // before any rename.
        std::fs::set_permissions(&store_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = writer.write(&path, 2, b"next");
        std::fs::set_permissions(&store_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(
            result.is_err(),
            "a write into a read-only directory must fail"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(writer.written(), 1, "the failed generation is not recorded");
    }

    #[test]
    fn remember_stays_in_memory_until_serviced_then_saves_in_the_background() {
        let (_dir, _guard) = isolated_profile();
        let file = tempfile::NamedTempFile::new().unwrap();
        remember(file.path(), position(3, 5));
        assert_eq!(lookup(file.path(), 5), Some(position(3, 5)));
        assert!(!store_path().exists(), "remember must not write the store");

        let wake = Arc::new(crate::app::ui_mailbox::RecordingWake::new());
        let dyn_wake: Arc<dyn crate::app::ui_mailbox::UiWake> = wake.clone();
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
        // The save thread wakes the UI only after its write returned.
        while !wake.sources().contains(&"editor_positions_save") {
            assert!(std::time::Instant::now() < deadline, "store never saved");
            assert_eq!(service(&dyn_wake), Vec::new(), "no failures expected");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(service(&dyn_wake), Vec::new(), "the save must succeed");
        let saved = read_entries(&store_path()).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].position, position(3, 5));
    }

    #[test]
    fn flush_now_writes_unsaved_positions_and_keeps_earlier_sessions() {
        let (_dir, _guard) = isolated_profile();
        let earlier = Entry {
            file: PathBuf::from("/files/earlier.txt"),
            position: position(1, 1),
        };
        Writer::default()
            .write(
                &store_path(),
                1,
                &serde_json::to_vec(&StoreFile {
                    version: STORE_VERSION,
                    entries: vec![earlier.clone()],
                })
                .unwrap(),
            )
            .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        remember(file.path(), position(8, 2));

        assert_eq!(flush_now(), Vec::new());
        let saved = read_entries(&store_path()).unwrap();
        assert_eq!(saved.len(), 2, "{saved:?}");
        assert_eq!(saved[0], earlier, "the flush merges the disk copy first");
        assert_eq!(saved[1].position, position(8, 2));
        assert_eq!(
            flush_now(),
            Vec::new(),
            "a second flush has nothing to write"
        );
    }
}
