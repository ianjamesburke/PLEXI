//! Channel-neutral usage history for palette rows: one file,
//! `palette-usage.json` in `shared_dir()`, read and written only here, off
//! the UI thread, debounced, by atomic replace (`docs/palette-picker.md`,
//! Files).

use plexi_host_model::picker::UsageEntry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const USAGE_FILE: &str = "palette-usage.json";
pub(crate) const USAGE_FORGET_AFTER: Duration = Duration::from_secs(90 * 24 * 60 * 60);
pub(crate) const USAGE_WRITE_DEBOUNCE: Duration = Duration::from_millis(500);

const USAGE_VERSION: u64 = 1;

pub(crate) fn usage_path() -> PathBuf {
    crate::config::shared_dir().join(USAGE_FILE)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UsageBook {
    entries: BTreeMap<String, UsageEntry>,
}

#[derive(Serialize, Deserialize)]
struct UsageFile<T> {
    version: u64,
    #[serde(default)]
    entries: T,
}

impl UsageBook {
    pub(crate) fn record(&mut self, key: &str, now: SystemTime) {
        let last_pick = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let entry = self.entries.entry(key.to_owned()).or_insert(UsageEntry {
            count: 0,
            last_pick,
        });
        entry.count = entry.count.saturating_add(1);
        entry.last_pick = last_pick;
    }

    pub(crate) fn get(&self, key: &str) -> Option<UsageEntry> {
        self.entries.get(key).copied()
    }

    pub(crate) fn prune(&mut self, now: SystemTime) {
        let cutoff = now
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .saturating_sub(USAGE_FORGET_AFTER);
        self.entries
            .retain(|_, entry| Duration::from_secs(entry.last_pick) >= cutoff);
    }

    pub(crate) fn to_json(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&UsageFile {
            version: USAGE_VERSION,
            entries: &self.entries,
        })
        .map_err(|err| format!("serialize palette usage: {err}"))
    }

    pub(crate) fn from_json(bytes: &[u8]) -> Result<Self, String> {
        // Check the version before decoding the version-specific entry format.
        let file: UsageFile<serde_json::Value> =
            serde_json::from_slice(bytes).map_err(|err| format!("parse palette usage: {err}"))?;
        if file.version != USAGE_VERSION {
            return Err(format!(
                "unsupported palette usage version {}",
                file.version
            ));
        }
        let entries = serde_json::from_value(file.entries)
            .map_err(|err| format!("parse palette usage entries: {err}"))?;
        Ok(Self { entries })
    }
}

pub(crate) fn load_usage(path: &Path) -> Result<UsageBook, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(UsageBook::default()),
        Err(err) => return Err(format!("read palette usage at {}: {err}", path.display())),
    };
    let mut book =
        UsageBook::from_json(&bytes).map_err(|err| format!("{}: {err}", path.display()))?;
    book.prune(SystemTime::now());
    Ok(book)
}

pub(crate) struct UsageWriter {
    sender: Option<Sender<UsageBook>>,
    worker: Option<JoinHandle<()>>,
}

impl UsageWriter {
    pub(crate) fn spawn(path: PathBuf, debounce: Duration) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("plexi-palette-usage".to_owned())
            .spawn(move || run_writer(&path, debounce, receiver))
            .map_err(|err| format!("spawn palette usage writer: {err}"))?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    pub(crate) fn submit(&self, book: UsageBook) {
        let Some(sender) = &self.sender else {
            return;
        };
        if sender.send(book).is_err() {
            log::warn!("palette usage: writer thread is gone; the ranking change is not saved");
        }
    }
}

/// The palette's usage history as the UI thread holds it. Loading and
/// writing run on their own threads. Changes made before the file loads are
/// replayed in order so a reset also forgets any previously saved picks.
#[derive(Default)]
pub(crate) struct PaletteUsage {
    book: UsageBook,
    early: Vec<(String, UsageChange)>,
    loading: Option<Receiver<Result<UsageBook, String>>>,
    writer: Option<UsageWriter>,
    started: bool,
}

#[derive(Clone, Copy)]
enum UsageChange {
    Pick(SystemTime),
    Reset,
}

impl UsageChange {
    fn apply(self, book: &mut UsageBook, key: &str) -> bool {
        match self {
            Self::Pick(at) => {
                book.record(key, at);
                true
            }
            Self::Reset => book.entries.remove(key).is_some(),
        }
    }
}

impl PaletteUsage {
    /// Start loading `path` in the background, once.
    pub(crate) fn start(&mut self, path: PathBuf) {
        if self.started {
            return;
        }
        self.started = true;
        let (sender, receiver) = mpsc::channel();
        let load_path = path.clone();
        let spawned = thread::Builder::new()
            .name("plexi-palette-usage-load".to_owned())
            .spawn(move || {
                // The receiver is gone only when the app is shutting down.
                if sender.send(load_usage(&load_path)).is_err() {
                    log::debug!("palette usage: loaded after its owner closed");
                }
            });
        match spawned {
            Ok(_) => self.loading = Some(receiver),
            Err(err) => {
                log::warn!("palette usage: spawn loader for {}: {err}", path.display());
                return;
            }
        }
        match UsageWriter::spawn(path, USAGE_WRITE_DEBOUNCE) {
            Ok(writer) => self.writer = Some(writer),
            Err(err) => log::warn!("palette usage: {err}; picks are not saved"),
        }
    }

    /// Install the loaded file, if it has arrived. Serviced from `App::logic`.
    pub(crate) fn poll(&mut self) {
        let Some(receiver) = &self.loading else {
            return;
        };
        let loaded = match receiver.try_recv() {
            Ok(loaded) => loaded,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("palette usage loader exited without an answer".to_owned())
            }
        };
        self.loading = None;
        match loaded {
            Ok(book) => {
                log::info!("palette usage: loaded {} row(s)", book.entries.len());
                self.book = book;
                let mut changed = false;
                for (key, change) in std::mem::take(&mut self.early) {
                    changed |= change.apply(&mut self.book, &key);
                }
                if changed {
                    self.save();
                }
            }
            Err(err) => {
                // Writing would replace the file this could not read.
                log::warn!("palette usage: {err}; ranking starts empty and is not saved");
                self.writer = None;
                self.early.clear();
            }
        }
    }

    /// Count one pick of `key` and save the history in the background.
    pub(crate) fn record(&mut self, key: &str) {
        self.update(key, UsageChange::Pick(SystemTime::now()));
    }

    /// Forget one row's ranking history.
    pub(crate) fn reset(&mut self, key: &str) {
        log::info!("palette usage: requested a ranking reset for one file row");
        self.update(key, UsageChange::Reset);
    }

    fn update(&mut self, key: &str, change: UsageChange) {
        let changed = change.apply(&mut self.book, key);
        if self.loading.is_some() {
            self.early.push((key.to_owned(), change));
        } else if changed {
            self.save();
        }
    }

    fn save(&self) {
        if let Some(writer) = &self.writer {
            writer.submit(self.book.clone());
        }
    }

    pub(crate) fn book(&self) -> &UsageBook {
        &self.book
    }
}

impl Drop for UsageWriter {
    fn drop(&mut self) {
        drop(self.sender.take());
        if let Some(worker) = self.worker.take() {
            if let Err(err) = worker.join() {
                log::warn!("palette usage writer panicked: {err:?}");
            }
        }
    }
}

fn run_writer(path: &Path, debounce: Duration, receiver: Receiver<UsageBook>) {
    while let Ok(mut latest) = receiver.recv() {
        let disconnected = loop {
            match receiver.recv_timeout(debounce) {
                Ok(book) => latest = book,
                Err(RecvTimeoutError::Timeout) => break false,
                Err(RecvTimeoutError::Disconnected) => break true,
            }
        };

        let result = latest
            .to_json()
            .and_then(|bytes| crate::platform::fs::atomic_write(path, &bytes));
        match result {
            Ok(()) => log::debug!("wrote palette usage at {}", path.display()),
            Err(err) => log::warn!("write palette usage at {}: {err}", path.display()),
        }

        if disconnected {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn record_increments_count_and_updates_last_pick() {
        let mut book = UsageBook::default();
        book.record("first", UNIX_EPOCH + Duration::from_secs(10));
        assert_eq!(
            book.get("first"),
            Some(UsageEntry {
                count: 1,
                last_pick: 10
            })
        );
        book.record("first", UNIX_EPOCH + Duration::from_secs(15));
        assert_eq!(
            book.get("first"),
            Some(UsageEntry {
                count: 2,
                last_pick: 15
            })
        );
    }

    #[test]
    fn record_saturates_count() {
        let mut book = UsageBook::from_json(
            br#"{"version":1,"entries":{"file":{"count":4294967295,"last_pick":1}}}"#,
        )
        .expect("load saturated count");
        book.record("file", UNIX_EPOCH + Duration::from_secs(2));
        assert_eq!(
            book.get("file"),
            Some(UsageEntry {
                count: u32::MAX,
                last_pick: 2
            })
        );
    }

    #[test]
    fn prune_removes_only_entries_older_than_the_cutoff() {
        let mut book = UsageBook::default();
        let cutoff = UNIX_EPOCH + Duration::from_secs(10);
        book.record("stale", cutoff - Duration::from_secs(1));
        book.record("boundary", cutoff);
        book.record("fresh", cutoff + Duration::from_secs(1));
        book.prune(cutoff + USAGE_FORGET_AFTER);
        assert_eq!(book.get("stale"), None);
        assert!(book.get("boundary").is_some());
        assert!(book.get("fresh").is_some());
    }

    #[test]
    fn json_round_trip_preserves_the_wire_shape() {
        let mut book = UsageBook::default();
        book.record("row/key", UNIX_EPOCH + Duration::from_secs(10));
        let bytes = book.to_json().expect("serialize usage");
        assert_eq!(
            bytes.as_slice(),
            br#"{"version":1,"entries":{"row/key":{"count":1,"last_pick":10}}}"#
        );
        assert_eq!(
            UsageBook::from_json(&bytes).expect("deserialize usage"),
            book
        );
    }

    #[test]
    fn unknown_version_reports_the_version_and_purpose() {
        let err = UsageBook::from_json(br#"{"version":42,"entries":[]}"#)
            .expect_err("reject an unknown version");
        assert!(err.contains("palette usage"), "{err}");
        assert!(err.contains("42"), "{err}");
    }

    #[test]
    fn load_reports_corrupt_file_and_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(USAGE_FILE);
        fs::write(&path, b"{").expect("write corrupt usage");
        let err = load_usage(&path).expect_err("reject corrupt file");
        assert!(err.contains(&path.display().to_string()), "{err}");
        assert!(err.contains("EOF"), "{err}");
    }

    #[test]
    fn load_returns_empty_for_a_missing_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(
            load_usage(&dir.path().join(USAGE_FILE)).expect("load missing usage"),
            UsageBook::default()
        );
    }

    #[test]
    fn load_prunes_stale_entries() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(USAGE_FILE);
        let now = SystemTime::now();
        let mut book = UsageBook::default();
        book.record("stale", UNIX_EPOCH);
        book.record("fresh", now);
        fs::write(&path, book.to_json().expect("serialize usage")).expect("write usage");
        let mut fresh = UsageBook::default();
        fresh.record("fresh", now);
        assert_eq!(load_usage(&path).expect("load and prune usage"), fresh);
    }

    #[test]
    fn writer_persists_the_last_book_after_a_burst() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(USAGE_FILE);
        let writer = UsageWriter::spawn(path.clone(), USAGE_WRITE_DEBOUNCE).expect("start writer");
        let mut book = UsageBook::default();
        let start = Instant::now();
        for key in ["first", "second", "last"] {
            book.record(key, UNIX_EPOCH);
            writer.submit(book.clone());
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        let bytes = loop {
            match fs::read(&path) {
                Ok(bytes) => break bytes,
                Err(err) if err.kind() == ErrorKind::NotFound && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("read debounced usage at {}: {err}", path.display()),
            }
        };
        assert!(
            start.elapsed() >= USAGE_WRITE_DEBOUNCE,
            "write landed before the debounce window elapsed"
        );
        assert_eq!(UsageBook::from_json(&bytes).expect("read final book"), book);
        drop(writer);
        assert_eq!(
            fs::read_dir(dir.path())
                .expect("list usage directory")
                .count(),
            1
        );
    }

    #[test]
    fn dropping_writer_flushes_a_pending_book() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(USAGE_FILE);
        let debounce = Duration::from_secs(10);
        let writer = UsageWriter::spawn(path.clone(), debounce).expect("start writer");
        let mut book = UsageBook::default();
        book.record("pending", SystemTime::now());
        let start = Instant::now();
        writer.submit(book.clone());
        drop(writer);
        assert!(
            start.elapsed() < debounce,
            "drop waited for the debounce window"
        );
        assert_eq!(load_usage(&path).expect("load flushed book"), book);
    }

    #[test]
    fn impossible_write_does_not_prevent_another_writer_from_working() {
        let dir = tempfile::tempdir().expect("temp dir");
        let blocked_parent = dir.path().join("regular-file");
        fs::write(&blocked_parent, b"original").expect("create a file where a directory is needed");
        let mut book = UsageBook::default();
        book.record("picked", SystemTime::now());
        let writer = UsageWriter::spawn(blocked_parent.join(USAGE_FILE), USAGE_WRITE_DEBOUNCE)
            .expect("start writer with an invalid destination");
        writer.submit(book.clone());
        drop(writer);
        assert_eq!(
            fs::read(blocked_parent).expect("read original file"),
            b"original"
        );

        let path = dir.path().join(USAGE_FILE);
        let writer =
            UsageWriter::spawn(path.clone(), USAGE_WRITE_DEBOUNCE).expect("start good writer");
        writer.submit(book.clone());
        drop(writer);
        assert_eq!(load_usage(&path).expect("load good writer's book"), book);
    }

    #[test]
    fn usage_file_is_channel_neutral() {
        let dir = tempfile::tempdir().expect("temp dir");
        let _guard = crate::config::set_test_shared_dir(dir.path().to_path_buf());
        assert_eq!(usage_path(), dir.path().join("palette-usage.json"));
        assert!(
            !usage_path().starts_with(crate::config::config_dir()),
            "usage history must not live in the channel-scoped profile"
        );
    }

    #[test]
    fn a_pick_before_the_file_loads_is_kept() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(USAGE_FILE);
        let mut saved = UsageBook::default();
        saved.record("files:a", SystemTime::now());
        fs::write(&path, saved.to_json().expect("serialize")).expect("write usage");
        let mut usage = PaletteUsage::default();
        usage.start(path.clone());
        usage.record("files:b");
        let deadline = Instant::now() + Duration::from_secs(5);
        while usage.loading.is_some() && Instant::now() < deadline {
            usage.poll();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(usage.book().get("files:a").is_some(), "saved pick survives");
        assert!(usage.book().get("files:b").is_some(), "early pick replays");
    }

    #[test]
    fn usage_replays_early_resets_and_picks_in_order() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(USAGE_FILE);
        let mut saved = UsageBook::default();
        for key in ["forgotten", "repicked", "untouched"] {
            saved.record(key, SystemTime::now());
        }
        fs::write(&path, saved.to_json().expect("serialize")).expect("write usage");

        let mut usage = PaletteUsage::default();
        usage.start(path.clone());
        usage.record("forgotten");
        usage.reset("forgotten");
        usage.reset("repicked");
        usage.record("repicked");
        let deadline = Instant::now() + Duration::from_secs(5);
        while usage.loading.is_some() && Instant::now() < deadline {
            usage.poll();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(usage.loading.is_none(), "usage never loaded");
        drop(usage);

        let reloaded = load_usage(&path).expect("load saved usage");
        assert_eq!(reloaded.get("forgotten"), None);
        assert_eq!(reloaded.get("repicked").map(|entry| entry.count), Some(1));
        assert_eq!(reloaded.get("untouched"), saved.get("untouched"));
    }
}
