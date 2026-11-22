//! Unsaved editor text that outlives the host (stint 1538).
//!
//! A system quit (Dock Quit, logout, restart, shut down, a quit Apple event)
//! reaches only `on_exit`, which cannot ask anything, and a crash or Force
//! Quit reaches nothing at all. So the host keeps a journal of every dirty
//! editor buffer in the channel profile (`unsaved-buffers/`) and offers that
//! text again on the next launch, however the last one ended.
//!
//! One file per buffer per session, `<path hash>-<session>.json`, so a session
//! never overwrites text an earlier session left unresolved. At launch
//! [`begin_session`] reads every file into the recovered set. An editor that
//! opens a recovered path offers the newest text; Restore or Discard resolves
//! it and removes the earlier sessions' files for that path. Unresolved text
//! stays on disk and is offered again on the launch after.
//!
//! Views reach the journal without a host handle, so the in-memory state is
//! process-global and keyed by profile, like `editor_positions`. [`record`] and
//! [`forget`] touch memory only; [`service`] writes from `App::logic` on a
//! background thread, and [`flush_now`] is the blocking write on quit paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};

const STORE_DIR: &str = "unsaved-buffers";
const STORE_VERSION: u32 = 1;
/// Notice titles. Tests match on them.
pub(crate) const SAVE_FAILED_TITLE: &str = "Unsaved text not backed up";
pub(crate) const LOAD_FAILED_TITLE: &str = "Unsaved text not recovered";
pub(crate) const RECOVERED_TITLE: &str = "Unsaved text recovered";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct EntryFile {
    version: u32,
    file: PathBuf,
    text: String,
    /// FNV-1a of the file bytes the edits were made against; `None` when the
    /// file did not exist.
    base_hash: Option<u64>,
    saved_at_ms: i64,
}

/// Text an earlier session left unsaved, as offered to the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct Recovered {
    pub text: String,
    pub base_hash: Option<u64>,
    pub saved_at_ms: i64,
}

/// A user-visible failure or offer the host turns into a notification.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Notice {
    pub title: &'static str,
    pub body: String,
}

#[derive(Clone, Debug)]
enum Op {
    Write(Vec<u8>),
    Remove,
}

/// Recovered text for one path: the newest entry and every earlier-session
/// file that holds text for it.
struct RecoveredPath {
    newest: Recovered,
    files: Vec<PathBuf>,
    offered: bool,
}

struct Slot {
    dir: PathBuf,
    session: String,
    recovered: HashMap<PathBuf, RecoveredPath>,
    /// Ops not yet handed to a writer, keyed by journal file.
    pending: HashMap<PathBuf, Op>,
    save_rx: Option<Receiver<Vec<String>>>,
    announced: bool,
    load_error: Option<String>,
}

impl Slot {
    fn begin(dir: PathBuf) -> Self {
        let session = uuid::Uuid::new_v4().simple().to_string();
        let (recovered, load_error) = match read_dir_entries(&dir) {
            Ok(entries) => (group(entries), None),
            Err(e) => {
                log::warn!("{e}");
                (HashMap::new(), Some(e))
            }
        };
        log::info!(
            "unsaved_buffers: session {session} began with {} recovered paths in {}",
            recovered.len(),
            dir.display()
        );
        Self {
            dir,
            session,
            recovered,
            pending: HashMap::new(),
            save_rx: None,
            announced: false,
            load_error,
        }
    }

    fn journal_file(&self, file: &Path) -> PathBuf {
        let hash = crate::app::text_editor_view::fnv1a64(file.as_os_str().as_encoded_bytes());
        self.dir.join(format!("{hash:016x}-{}.json", self.session))
    }

    fn drain_save(&mut self, block: bool, notices: &mut Vec<Notice>) {
        let Some(rx) = self.save_rx.as_ref() else {
            return;
        };
        let errors = if block {
            rx.recv().unwrap_or_else(|_| {
                vec!["unsaved buffers: writer stopped without a result".to_string()]
            })
        } else {
            match rx.try_recv() {
                Ok(errors) => errors,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    vec!["unsaved buffers: writer stopped without a result".to_string()]
                }
            }
        };
        self.save_rx = None;
        for e in errors {
            log::warn!("{e}");
            notices.push(save_failed(e));
        }
    }

    fn service(&mut self, wake: &Arc<dyn crate::app::ui_mailbox::UiWake>) -> Vec<Notice> {
        let mut notices = Vec::new();
        if let Some(e) = self.load_error.take() {
            notices.push(Notice {
                title: LOAD_FAILED_TITLE,
                body: format!("{e}. Text from the last session cannot be offered."),
            });
        }
        self.drain_save(false, &mut notices);
        if self.save_rx.is_some() || self.pending.is_empty() {
            return notices;
        }
        let ops: Vec<(PathBuf, Op)> = self.pending.drain().collect();
        let (tx, rx) = mpsc::channel();
        let wake = Arc::clone(wake);
        let dir = self.dir.clone();
        let spawned = std::thread::Builder::new()
            .name("unsaved-buffers-save".to_string())
            .spawn(move || {
                let _ = tx.send(apply(&dir, ops));
                wake.wake("unsaved_buffers_save");
            });
        match spawned {
            Ok(_) => self.save_rx = Some(rx),
            Err(e) => {
                let e = format!("unsaved buffers: cannot spawn the save thread: {e}");
                log::warn!("{e}");
                notices.push(save_failed(e));
            }
        }
        notices
    }

    fn flush_now(&mut self) -> Vec<Notice> {
        let mut notices = Vec::new();
        // An in-flight write holds older ops; let it land first.
        self.drain_save(true, &mut notices);
        if self.pending.is_empty() {
            return notices;
        }
        let ops: Vec<(PathBuf, Op)> = self.pending.drain().collect();
        let count = ops.len();
        for e in apply(&self.dir, ops) {
            log::error!("{e}");
            notices.push(save_failed(e));
        }
        log::info!("unsaved_buffers: flushed {count} journal changes now");
        notices
    }
}

/// Applies `ops` in `dir`; returns one message per failure.
fn apply(dir: &Path, ops: Vec<(PathBuf, Op)>) -> Vec<String> {
    let mut errors = Vec::new();
    if ops.iter().any(|(_, op)| matches!(op, Op::Write(_))) {
        if let Err(e) = std::fs::create_dir_all(dir) {
            errors.push(format!(
                "unsaved buffers: cannot create {}: {e}",
                dir.display()
            ));
            return errors;
        }
    }
    for (file, op) in ops {
        let result = match op {
            Op::Write(body) => crate::platform::fs::atomic_write(&file, &body)
                .map_err(|e| format!("unsaved buffers: cannot write {}: {e}", file.display())),
            Op::Remove => match std::fs::remove_file(&file) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!(
                    "unsaved buffers: cannot remove {}: {e}",
                    file.display()
                )),
            },
        };
        if let Err(e) = result {
            errors.push(e);
        }
    }
    errors
}

fn read_dir_entries(dir: &Path) -> Result<Vec<(PathBuf, EntryFile)>, String> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(format!(
                "unsaved buffers: cannot read {}: {e}",
                dir.display()
            ))
        }
    };
    let mut entries = Vec::new();
    let mut failures = Vec::new();
    for item in listing {
        let path = match item {
            Ok(item) => item.path(),
            Err(e) => {
                failures.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let parsed = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<EntryFile>(&bytes).map_err(|e| e.to_string())
            });
        match parsed {
            Ok(entry) if entry.version == STORE_VERSION => entries.push((path, entry)),
            Ok(entry) => failures.push(format!(
                "{} has version {}, this build reads {STORE_VERSION}",
                path.display(),
                entry.version
            )),
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    if !failures.is_empty() {
        // The readable entries are still offered; the unreadable files stay
        // on disk for a person to inspect.
        log::warn!(
            "unsaved buffers: {} unreadable journal files: {}",
            failures.len(),
            failures.join("; ")
        );
    }
    Ok(entries)
}

fn group(entries: Vec<(PathBuf, EntryFile)>) -> HashMap<PathBuf, RecoveredPath> {
    let mut grouped: HashMap<PathBuf, RecoveredPath> = HashMap::new();
    for (journal, entry) in entries {
        let candidate = Recovered {
            text: entry.text,
            base_hash: entry.base_hash,
            saved_at_ms: entry.saved_at_ms,
        };
        match grouped.get_mut(&entry.file) {
            Some(existing) => {
                existing.files.push(journal);
                if candidate.saved_at_ms > existing.newest.saved_at_ms {
                    existing.newest = candidate;
                }
            }
            None => {
                grouped.insert(
                    entry.file,
                    RecoveredPath {
                        newest: candidate,
                        files: vec![journal],
                        offered: false,
                    },
                );
            }
        }
    }
    grouped
}

fn save_failed(error: String) -> Notice {
    Notice {
        title: SAVE_FAILED_TITLE,
        body: format!(
            "{error}. If Plexi ends before you save, these edits cannot be offered again."
        ),
    }
}

fn slots() -> &'static Mutex<HashMap<PathBuf, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<PathBuf, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn store_dir() -> PathBuf {
    crate::config::config_dir().join(STORE_DIR)
}

fn with_slot<T>(f: impl FnOnce(&mut Slot) -> T) -> T {
    let dir = store_dir();
    let mut slots = slots().lock().unwrap_or_else(|e| e.into_inner());
    let slot = slots.entry(dir.clone()).or_insert_with(|| Slot::begin(dir));
    f(slot)
}

/// The path the journal keys `file` by.
pub(crate) fn identity(file: &Path) -> PathBuf {
    crate::app::text_editor_view::note_path_identity(file)
}

/// Starts this host's session: reads what earlier sessions left unsaved.
/// Called once at host start, before the workspace restores its editors.
pub(crate) fn begin_session() {
    let dir = store_dir();
    let slot = Slot::begin(dir.clone());
    slots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(dir, slot);
}

/// Journals `text` as the unsaved content of `file`, in memory.
pub fn record(file: &Path, text: &str, base_hash: Option<u64>) {
    let file = identity(file);
    let entry = EntryFile {
        version: STORE_VERSION,
        file: file.clone(),
        text: text.to_string(),
        base_hash,
        saved_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    let body = match serde_json::to_vec(&entry) {
        Ok(body) => body,
        Err(e) => {
            log::warn!(
                "unsaved buffers: cannot encode {}: {e}; not journaled",
                file.display()
            );
            return;
        }
    };
    with_slot(|slot| {
        let journal = slot.journal_file(&file);
        slot.pending.insert(journal, Op::Write(body));
    });
    log::info!(
        "unsaved_buffers: journaled {} bytes={}",
        file.display(),
        text.len()
    );
}

/// Drops this session's journal entry for `file`: the buffer is clean.
pub fn forget(file: &Path) {
    let file = identity(file);
    with_slot(|slot| {
        let journal = slot.journal_file(&file);
        slot.pending.insert(journal, Op::Remove);
    });
    log::info!("unsaved_buffers: forgot {}", file.display());
}

/// Text an earlier session left unsaved for `file`, unless it equals
/// `current` (it was saved after all), in which case it is resolved here.
pub fn offer(file: &Path, current: &str) -> Option<Recovered> {
    let file = identity(file);
    let same = with_slot(|slot| {
        let entry = slot.recovered.get_mut(&file)?;
        entry.offered = true;
        Some(entry.newest.text == current)
    })?;
    if same {
        log::info!(
            "unsaved_buffers: recovered text for {} already matches the file; resolved",
            file.display()
        );
        resolve(&file);
        return None;
    }
    log::info!(
        "unsaved_buffers: offering recovered text for {}",
        file.display()
    );
    with_slot(|slot| slot.recovered.get(&file).map(|entry| entry.newest.clone()))
}

/// Restore or Discard was chosen for `file`: earlier sessions' text is done.
pub fn resolve(file: &Path) {
    let file = identity(file);
    let resolved = with_slot(|slot| {
        let entry = slot.recovered.remove(&file)?;
        for journal in &entry.files {
            slot.pending.insert(journal.clone(), Op::Remove);
        }
        Some(entry.files.len())
    });
    if let Some(count) = resolved {
        log::info!(
            "unsaved_buffers: resolved recovered text for {} ({count} files)",
            file.display()
        );
    }
}

/// Per-frame servicing from `App::logic`: writes journal changes off the UI
/// thread and returns notices. The first call after launch also names the
/// recovered paths no restored editor offered.
pub(crate) fn service(wake: &Arc<dyn crate::app::ui_mailbox::UiWake>) -> Vec<Notice> {
    with_slot(|slot| {
        let mut notices = slot.service(wake);
        if !slot.announced {
            slot.announced = true;
            let mut unoffered: Vec<String> = slot
                .recovered
                .iter()
                .filter(|(_, entry)| !entry.offered)
                .map(|(file, _)| file.display().to_string())
                .collect();
            unoffered.sort();
            if !unoffered.is_empty() {
                log::info!(
                    "unsaved_buffers: {} recovered paths have no open editor",
                    unoffered.len()
                );
                notices.push(Notice {
                    title: RECOVERED_TITLE,
                    body: format!(
                        "Plexi ended before these edits were saved. Open a file to \
                         restore or discard its text: {}",
                        unoffered.join(", ")
                    ),
                });
            }
        }
        notices
    })
}

/// Writes every pending journal change now, blocking. Quit paths only.
pub(crate) fn flush_now() -> Vec<Notice> {
    with_slot(Slot::flush_now)
}

/// The journal directory of the current profile.
#[cfg(test)]
pub(crate) fn test_store_dir() -> PathBuf {
    store_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_profile() -> (tempfile::TempDir, crate::config::TestProfileDirGuard) {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let guard = crate::config::set_test_profile_dir(profile);
        (dir, guard)
    }

    #[test]
    fn journaled_text_is_offered_by_the_next_session_until_resolved() {
        let (dir, _guard) = isolated_profile();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "disk\n").unwrap();
        begin_session();
        record(&file, "typed\n", Some(1));
        assert!(flush_now().is_empty());

        begin_session();
        let offered = offer(&file, "disk\n").expect("recovered text is offered");
        assert_eq!(offered.text, "typed\n");
        assert_eq!(offered.base_hash, Some(1));

        // Unresolved text survives another launch.
        begin_session();
        assert!(offer(&file, "disk\n").is_some());
        resolve(&file);
        assert!(flush_now().is_empty());
        begin_session();
        assert_eq!(offer(&file, "disk\n"), None);
        assert_eq!(std::fs::read_dir(test_store_dir()).unwrap().count(), 0);
    }

    #[test]
    fn a_clean_buffer_leaves_nothing_to_offer() {
        let (dir, _guard) = isolated_profile();
        let file = dir.path().join("b.txt");
        begin_session();
        record(&file, "typed\n", None);
        forget(&file);
        assert!(flush_now().is_empty());
        begin_session();
        assert_eq!(offer(&file, ""), None);
    }

    #[test]
    fn recovered_text_equal_to_the_file_is_resolved_silently() {
        let (dir, _guard) = isolated_profile();
        let file = dir.path().join("c.txt");
        begin_session();
        record(&file, "same\n", None);
        assert!(flush_now().is_empty());
        begin_session();
        assert_eq!(offer(&file, "same\n"), None);
        assert!(flush_now().is_empty());
        assert_eq!(std::fs::read_dir(test_store_dir()).unwrap().count(), 0);
    }

    #[test]
    fn the_newest_of_two_unresolved_sessions_is_offered() {
        let (dir, _guard) = isolated_profile();
        let file = dir.path().join("d.txt");
        begin_session();
        record(&file, "older\n", None);
        assert!(flush_now().is_empty());
        begin_session();
        std::thread::sleep(std::time::Duration::from_millis(5));
        record(&file, "newer\n", None);
        assert!(flush_now().is_empty());
        begin_session();
        assert_eq!(offer(&file, "").unwrap().text, "newer\n");
        resolve(&file);
        assert!(flush_now().is_empty());
        assert_eq!(std::fs::read_dir(test_store_dir()).unwrap().count(), 0);
    }

    #[test]
    fn unoffered_recovered_paths_are_announced_once() {
        let (dir, _guard) = isolated_profile();
        let file = dir.path().join("e.txt");
        begin_session();
        record(&file, "typed\n", None);
        assert!(flush_now().is_empty());
        begin_session();
        let wake: Arc<dyn crate::app::ui_mailbox::UiWake> =
            Arc::new(crate::app::ui_mailbox::RecordingWake::new());
        let notices = service(&wake);
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0].title, RECOVERED_TITLE);
        assert!(notices[0].body.contains("e.txt"), "{}", notices[0].body);
        assert!(service(&wake).is_empty(), "announced once");
    }
}
