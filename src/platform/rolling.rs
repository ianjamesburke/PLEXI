//! Append-only file writer that rolls over *while the host stays open*.
//!
//! Startup-only rotation (`logging::rotate_and_prune`) leaves a host that runs
//! for days growing one file without bound. [`RollingFile`] closes that gap: at
//! a line boundary it archives the live file when the segment exceeds
//! [`DEFAULT_MAX_BYTES`] or the calendar day changed since the segment began.
//!
//! Archives are named `<stem>-<YYYY-MM-DD>[.<n>].<ext>`, where the date is the
//! day the segment *started* and `<n>` disambiguates several segments that
//! share a day. [`archive_date`] is the one parser for that scheme, so pruning
//! and rotation cannot drift apart.
//!
//! The writer never rolls mid-line, so a line is never split across segments
//! even when the underlying sink (fern) emits it in several `write` calls.

use chrono::NaiveDate;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Segment size that triggers a live rollover (10 MB).
pub const DEFAULT_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// How many line starts pass between checks that the live path still names the
/// file we hold open (another process, e.g. a CLI invocation, may roll it).
const REPLACED_CHECK_EVERY: u32 = 128;

fn today_local() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// Find the first `YYYY-MM-DD` in `text`. Used to recover the day a segment
/// started from its first line, which (unlike mtime) a later shutdown write
/// cannot overwrite.
pub(crate) fn first_date_in(text: &str) -> Option<NaiveDate> {
    let bytes = text.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    (0..=bytes.len() - 10).find_map(|start| {
        let window = text.get(start..start + 10)?;
        let b = window.as_bytes();
        let shaped = b[4] == b'-'
            && b[7] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
        if shaped {
            NaiveDate::parse_from_str(window, "%Y-%m-%d").ok()
        } else {
            None
        }
    })
}

/// The day the segment at `path` began: the first date in its first line, else
/// the file's mtime date.
pub(crate) fn segment_start_day(path: &Path, meta: &std::fs::Metadata) -> Option<NaiveDate> {
    use std::io::Read;
    let mut head = [0u8; 256];
    if let Ok(mut file) = File::open(path) {
        if let Ok(n) = file.read(&mut head) {
            let text = String::from_utf8_lossy(&head[..n]);
            let first_line = text.lines().next().unwrap_or("");
            if let Some(day) = first_date_in(first_line) {
                return Some(day);
            }
        }
    }
    meta.modified()
        .ok()
        .map(|mtime| chrono::DateTime::<chrono::Local>::from(mtime).date_naive())
}

/// Parse the day out of an archive name `<stem>-<YYYY-MM-DD>[.<n>].<ext>`.
pub(crate) fn archive_date(name: &str, stem: &str, ext: &str) -> Option<NaiveDate> {
    let middle = name
        .strip_prefix(stem)?
        .strip_prefix('-')?
        .strip_suffix(ext)?
        .strip_suffix('.')?;
    let date_part = middle.split('.').next()?;
    NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()
}

/// First unused archive path for a segment that began on `day`.
pub(crate) fn archive_path(live: &Path, day: NaiveDate) -> PathBuf {
    let dir = live.parent().unwrap_or_else(|| Path::new("."));
    let stem = live
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "log".into());
    let ext = live
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "log".into());
    let first = dir.join(format!("{stem}-{}.{ext}", day.format("%Y-%m-%d")));
    if !first.exists() {
        return first;
    }
    (1u32..)
        .map(|n| dir.join(format!("{stem}-{}.{n}.{ext}", day.format("%Y-%m-%d"))))
        .find(|candidate| !candidate.exists())
        .unwrap_or(first)
}

#[cfg(unix)]
fn same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (file.metadata(), std::fs::metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(_file: &File, _path: &Path) -> bool {
    true
}

/// A line-oriented append writer with live size/day rollover.
pub struct RollingFile {
    path: PathBuf,
    file: Option<File>,
    max_bytes: u64,
    len: u64,
    segment_day: NaiveDate,
    at_line_start: bool,
    lines_since_check: u32,
    today: fn() -> NaiveDate,
    trace_rolls: bool,
}

impl RollingFile {
    /// Open (creating parents and the file) with the default 10 MB cap.
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::open_with(path, DEFAULT_MAX_BYTES, today_local)
    }

    /// Open with an explicit size cap and clock (tests drive both).
    pub(crate) fn open_with(
        path: &Path,
        max_bytes: u64,
        today: fn() -> NaiveDate,
    ) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let meta = file.metadata()?;
        let segment_day = if meta.len() > 0 {
            segment_start_day(path, &meta).unwrap_or_else(today)
        } else {
            today()
        };
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(file),
            max_bytes,
            len: meta.len(),
            segment_day,
            at_line_start: true,
            lines_since_check: 0,
            today,
            trace_rolls: true,
        })
    }

    /// Stop logging successful rollovers. The process logger's own sink must
    /// not log from inside its write path, so `plexi.log` uses this; the
    /// dated archive is the evidence.
    pub fn quiet(mut self) -> Self {
        self.trace_rolls = false;
        self
    }

    /// Archive the live file and start a new segment. Errors are logged and the
    /// writer keeps appending to the old handle rather than dropping lines.
    fn roll(&mut self, reason: &str) {
        let archive = archive_path(&self.path, self.segment_day);
        if let Err(e) = std::fs::rename(&self.path, &archive) {
            eprintln!(
                "[plexi::rolling] could not roll {} to {}: {e}",
                self.path.display(),
                archive.display()
            );
            // Do not retry on every line.
            self.segment_day = (self.today)();
            self.len = 0;
            return;
        }
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                self.file = Some(file);
                self.len = 0;
                self.segment_day = (self.today)();
                if self.trace_rolls {
                    log::info!(
                        target: "plexi::rolling",
                        "rolled {} -> {} ({reason})",
                        self.path.display(),
                        archive.display()
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "[plexi::rolling] could not reopen {}: {e}",
                    self.path.display()
                );
                self.file = None;
            }
        }
    }

    /// Re-open when another process rolled the path out from under us.
    fn reopen_if_replaced(&mut self) {
        let replaced = match &self.file {
            Some(file) => !same_file(file, &self.path),
            None => true,
        };
        if !replaced {
            return;
        }
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(file) => {
                self.len = file.metadata().map(|m| m.len()).unwrap_or(0);
                self.segment_day = (self.today)();
                self.file = Some(file);
            }
            Err(e) => {
                eprintln!(
                    "[plexi::rolling] could not reopen {}: {e}",
                    self.path.display()
                );
                self.file = None;
            }
        }
    }

    /// Called at a line boundary before the next line is written.
    fn maybe_roll(&mut self) {
        self.lines_since_check += 1;
        if self.lines_since_check >= REPLACED_CHECK_EVERY || self.file.is_none() {
            self.lines_since_check = 0;
            self.reopen_if_replaced();
        }
        if self.len == 0 {
            // Nothing to archive; just keep the day current.
            self.segment_day = (self.today)();
            return;
        }
        if self.len >= self.max_bytes {
            self.roll("size");
        } else if (self.today)() > self.segment_day {
            self.roll("day");
        }
    }
}

impl Write for RollingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.at_line_start {
            self.maybe_roll();
        }
        let Some(file) = self.file.as_mut() else {
            return Err(io::Error::other(format!(
                "{} is not open",
                self.path.display()
            )));
        };
        let n = file.write(buf)?;
        self.len += n as u64;
        self.at_line_start = buf[..n].last() == Some(&b'\n');
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    // Thread-local so parallel tests cannot move each other's clock.
    thread_local! {
        static DAY_OFFSET: Cell<i64> = const { Cell::new(0) };
    }

    fn fake_today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
            + chrono::Duration::days(DAY_OFFSET.with(|d| d.get()))
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn archive_date_parses_plain_and_numbered_archives() {
        let day = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        assert_eq!(
            archive_date("events-2026-03-01.jsonl", "events", "jsonl"),
            Some(day)
        );
        assert_eq!(
            archive_date("events-2026-03-01.2.jsonl", "events", "jsonl"),
            Some(day)
        );
        assert_eq!(archive_date("events.jsonl", "events", "jsonl"), None);
        assert_eq!(
            archive_date("effects-2026-03-01.jsonl", "events", "jsonl"),
            None
        );
    }

    #[test]
    fn rolls_over_on_size_at_a_line_boundary() {
        DAY_OFFSET.with(|d| d.set(0));
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("events.jsonl");
        let mut f = RollingFile::open_with(&live, 20, fake_today).unwrap();
        f.write_all(b"0123456789012345\n").unwrap(); // 17 bytes, under cap
        f.write_all(b"second-").unwrap(); // pushes past the cap mid-line
        f.write_all(b"half\n").unwrap(); // must stay in the same segment
        f.write_all(b"third\n").unwrap(); // next line start: rolls
        assert_eq!(
            names(dir.path()),
            vec!["events-2026-03-01.jsonl", "events.jsonl"]
        );
        let archived = std::fs::read_to_string(dir.path().join("events-2026-03-01.jsonl")).unwrap();
        assert_eq!(archived, "0123456789012345\nsecond-half\n");
        assert_eq!(std::fs::read_to_string(&live).unwrap(), "third\n");
    }

    #[test]
    fn rolls_over_when_the_calendar_day_changes_while_open() {
        DAY_OFFSET.with(|d| d.set(0));
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("plexi.log");
        let mut f = RollingFile::open_with(&live, DEFAULT_MAX_BYTES, fake_today).unwrap();
        f.write_all(b"day one\n").unwrap();
        DAY_OFFSET.with(|d| d.set(1));
        f.write_all(b"day two\n").unwrap();
        assert_eq!(names(dir.path()), vec!["plexi-2026-03-01.log", "plexi.log"]);
        assert_eq!(std::fs::read_to_string(&live).unwrap(), "day two\n");
        DAY_OFFSET.with(|d| d.set(0));
    }

    #[test]
    fn same_day_size_rollovers_number_their_archives() {
        DAY_OFFSET.with(|d| d.set(0));
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("effects.jsonl");
        let mut f = RollingFile::open_with(&live, 5, fake_today).unwrap();
        for line in ["aaaaaa\n", "bbbbbb\n", "cccccc\n"] {
            f.write_all(line.as_bytes()).unwrap();
        }
        assert_eq!(
            names(dir.path()),
            vec![
                "effects-2026-03-01.1.jsonl",
                "effects-2026-03-01.jsonl",
                "effects.jsonl"
            ]
        );
    }
}
