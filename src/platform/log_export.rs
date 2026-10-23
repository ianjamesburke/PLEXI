//! Bounded, redacted host-log export for the Needs attention review.
//!
//! The export reads only this channel's `plexi.log` (`logging::log_path`). It
//! never touches agent transcripts, never uploads, and always says how much of
//! the session it covers: an arbitrary tail is never labelled a whole session.
//! Reading and redaction are blocking work; callers run [`prepare_excerpt`] off
//! the UI thread.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::NaiveDateTime;

/// Largest excerpt offered to the clipboard.
pub const CLIPBOARD_LIMIT_BYTES: usize = 48 * 1024;
/// Largest excerpt offered to Save logs. Also bounds how much of the log file
/// is ever read into memory.
pub const SAVE_LIMIT_BYTES: usize = 4 * 1024 * 1024;

const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

static SESSION_START: OnceLock<NaiveDateTime> = OnceLock::new();

/// Record the start of this host process's logging session. Idempotent.
pub fn mark_session_start() {
    let _ = SESSION_START.set(chrono::Local::now().naive_local());
}

/// Local start of this process's log session, when one was recorded.
pub fn session_start() -> Option<NaiveDateTime> {
    SESSION_START.get().copied()
}

/// A prepared, reviewable diagnostic excerpt.
#[derive(Debug, Clone)]
pub struct LogExcerpt {
    /// Clipboard-sized text: header plus selected lines.
    pub clipboard_text: String,
    /// Larger text for Save logs (header plus every in-session line that fit
    /// `SAVE_LIMIT_BYTES`).
    pub save_text: String,
    /// Covered time range, human-readable.
    pub covers: String,
    /// Where the lines came from.
    pub sources: Vec<String>,
    /// The session start was unknown or fell outside what could be read.
    pub partial: bool,
    /// The clipboard text omits in-session lines.
    pub clipboard_truncated: bool,
    /// The save text omits in-session lines.
    pub save_truncated: bool,
    pub lines_omitted_for_privacy: usize,
    pub correlated_lines: usize,
}

impl LogExcerpt {
    /// One-line summary for the review panel.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("Covers {}", self.covers)];
        parts.push(format!("source: {}", self.sources.join(", ")));
        if self.correlated_lines > 0 {
            parts.push(format!(
                "{} line(s) mention this issue",
                self.correlated_lines
            ));
        }
        if self.partial {
            parts.push("partial: session start could not be established".to_string());
        }
        if self.clipboard_truncated {
            parts.push("older lines omitted from the clipboard copy; use Save logs".to_string());
        }
        if self.save_truncated {
            parts.push("saved file is also truncated".to_string());
        }
        if self.lines_omitted_for_privacy > 0 {
            parts.push(format!(
                "{} transcript line(s) withheld",
                self.lines_omitted_for_privacy
            ));
        }
        parts.join(" · ")
    }
}

/// Redact values that look like credentials. Conservative: a false positive
/// hides a harmless value, a false negative leaks a secret.
pub fn redact(line: &str) -> String {
    static PATTERNS: OnceLock<Vec<(regex::Regex, &'static str)>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        let build = |re: &str, rep: &'static str| {
            regex::Regex::new(re)
                .ok()
                .map(|compiled| (compiled, rep))
        };
        [
            build(
                r#"(?i)\b(authorization|bearer|token|secret|password|passwd|api[_-]?key|credential|cookie)(["']?\s*[=:]\s*["']?)[^\s"',;]+"#,
                "$1$2[redacted]",
            ),
            build(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{8,}", "Bearer [redacted]"),
            build(r"\bsk-[A-Za-z0-9_-]{16,}", "[redacted]"),
            build(r"\b(?:ghp|gho|ghs|github_pat)_[A-Za-z0-9_]{16,}", "[redacted]"),
            build(r"\b[A-Fa-f0-9]{40,}\b", "[redacted]"),
        ]
        .into_iter()
        .flatten()
        .collect()
    });
    let mut out = line.to_string();
    for (re, rep) in patterns {
        out = re.replace_all(&out, *rep).into_owned();
    }
    out
}

fn line_timestamp(line: &str) -> Option<NaiveDateTime> {
    let rest = line.strip_prefix('[')?;
    let stamp = rest.get(..19)?;
    NaiveDateTime::parse_from_str(stamp, TIMESTAMP_FORMAT).ok()
}

/// Transcripts of agent conversations are never exported automatically.
fn is_transcript_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("transcript")
}

fn read_tail(path: &Path, limit: usize) -> Result<(String, bool), String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|error| format!("could not stat {}: {error}", path.display()))?
        .len();
    let start = len.saturating_sub(limit as u64);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| format!("could not seek {}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), start > 0))
}

fn render(header: &str, lines: &[&str], keep: &[bool]) -> String {
    let mut out = String::from(header);
    let mut gap = 0usize;
    for (line, &kept) in lines.iter().zip(keep) {
        if kept {
            if gap > 0 {
                out.push_str(&format!("… {gap} line(s) omitted …\n"));
                gap = 0;
            }
            out.push_str(line);
            out.push('\n');
        } else {
            gap += 1;
        }
    }
    if gap > 0 {
        out.push_str(&format!("… {gap} line(s) omitted …\n"));
    }
    out
}

/// Select lines for `budget` bytes: correlated lines first (they explain the
/// selected issue), then the newest remaining ones. Returns the keep mask.
fn select(lines: &[&str], correlated: &[bool], budget: usize) -> Vec<bool> {
    let mut keep = vec![false; lines.len()];
    let mut used = 0usize;
    let correlated_budget = budget / 2;
    for idx in (0..lines.len()).rev() {
        if correlated[idx] {
            let cost = lines[idx].len() + 1;
            if used + cost <= correlated_budget {
                keep[idx] = true;
                used += cost;
            }
        }
    }
    for idx in (0..lines.len()).rev() {
        if keep[idx] {
            continue;
        }
        let cost = lines[idx].len() + 1;
        if used + cost > budget {
            break;
        }
        keep[idx] = true;
        used += cost;
    }
    keep
}

/// Build a reviewable excerpt of this session's host log.
///
/// `session_start` is the local start of the current host session; `None`
/// labels the result partial. `issue_id` correlates the selected issue.
pub fn prepare_excerpt(
    log_path: &Path,
    session_start: Option<NaiveDateTime>,
    issue_id: &str,
    clipboard_limit: usize,
    save_limit: usize,
) -> Result<LogExcerpt, String> {
    let (raw, tail_only) = read_tail(log_path, save_limit)?;
    let mut raw_lines: Vec<&str> = raw.lines().collect();
    if tail_only && !raw_lines.is_empty() {
        // The first line of a mid-file read is almost certainly cut.
        raw_lines.remove(0);
    }

    // Keep continuation lines (no timestamp) with the entry above them.
    let mut in_session = session_start.is_none();
    let mut first_ts: Option<NaiveDateTime> = None;
    let mut last_ts: Option<NaiveDateTime> = None;
    let mut selected: Vec<String> = Vec::new();
    let mut withheld = 0usize;
    for line in &raw_lines {
        if let Some(ts) = line_timestamp(line) {
            if let Some(start) = session_start {
                in_session = ts >= start;
            }
            if in_session {
                first_ts.get_or_insert(ts);
                last_ts = Some(ts);
            }
        }
        if !in_session {
            continue;
        }
        if is_transcript_line(line) {
            withheld += 1;
            continue;
        }
        selected.push(redact(line));
    }
    // A mid-file read that already starts inside the session may have cut
    // the session's beginning, so it cannot claim full coverage.
    let first_line_in_session = raw_lines
        .iter()
        .find_map(|line| line_timestamp(line))
        .zip(session_start)
        .is_some_and(|(ts, start)| ts >= start);
    let partial = match session_start {
        None => true,
        Some(_) => tail_only && first_line_in_session,
    };

    let covers = match (first_ts, last_ts) {
        (Some(a), Some(b)) => format!(
            "{} to {}",
            a.format(TIMESTAMP_FORMAT),
            b.format(TIMESTAMP_FORMAT)
        ),
        _ => "no log entries for this session".to_string(),
    };
    let source = log_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| log_path.display().to_string());

    let refs: Vec<&str> = selected.iter().map(String::as_str).collect();
    let correlated: Vec<bool> = refs
        .iter()
        .map(|line| !issue_id.is_empty() && line.contains(issue_id))
        .collect();
    let correlated_lines = correlated.iter().filter(|c| **c).count();

    let make_header = |truncated: bool| {
        let mut header = format!(
            "Plexi host log excerpt\nIssue: {issue_id}\nSource: {source} (this channel only)\nCovers: {covers}\n"
        );
        if partial {
            header.push_str("Coverage: PARTIAL - the session start could not be established\n");
        } else {
            header.push_str("Coverage: this host session\n");
        }
        if truncated {
            header.push_str("Older in-session lines were omitted to fit the size limit\n");
        }
        header.push_str("Redacted: credential-like values replaced with [redacted]\n");
        if withheld > 0 {
            header.push_str(&format!("Withheld: {withheld} agent transcript line(s)\n"));
        }
        header.push_str("---\n");
        header
    };

    let clip_keep = select(&refs, &correlated, clipboard_limit);
    let clipboard_truncated = clip_keep.iter().any(|kept| !kept);
    let save_keep = select(&refs, &correlated, save_limit);
    let save_truncated = save_keep.iter().any(|kept| !kept) || tail_only;

    Ok(LogExcerpt {
        clipboard_text: render(&make_header(clipboard_truncated), &refs, &clip_keep),
        save_text: render(&make_header(save_truncated), &refs, &save_keep),
        covers,
        sources: vec![source],
        partial,
        clipboard_truncated,
        save_truncated,
        lines_omitted_for_privacy: withheld,
        correlated_lines,
    })
}

/// Write `text` to `<dir>/exports/` and return the file path. Never overwrites.
pub fn save_export(dir: &Path, text: &str) -> Result<PathBuf, String> {
    let exports = dir.join("exports");
    std::fs::create_dir_all(&exports)
        .map_err(|error| format!("could not create {}: {error}", exports.display()))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut path = exports.join(format!("plexi-session-logs-{stamp}.txt"));
    let mut n = 1;
    while path.exists() {
        path = exports.join(format!("plexi-session-logs-{stamp}-{n}.txt"));
        n += 1;
    }
    std::fs::write(&path, text)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, TIMESTAMP_FORMAT).unwrap()
    }

    fn write_log(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("plexi.log");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn redacts_credentials_but_keeps_context() {
        let out = redact("[t] [INFO] [x] token=abc123def456 user=ian sk-abcdefghijklmnop1234");
        assert!(!out.contains("abc123def456"), "{out}");
        assert!(!out.contains("sk-abcdefghijklmnop1234"), "{out}");
        assert!(out.contains("user=ian"));
        assert!(redact("Authorization: Bearer abcdefghijkl").contains("[redacted]"));
    }

    #[test]
    fn filters_to_session_and_labels_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_log(
            dir.path(),
            "[2026-10-15 09:00:00] [INFO] [a] old session\n\
             [2026-10-16 03:00:00] [INFO] [a] new start\n\
             continuation line\n\
             [2026-10-16 03:05:00] [WARN] [a] ny_abc failed\n",
        );
        let ex = prepare_excerpt(
            &path,
            Some(at("2026-10-16 02:59:00")),
            "ny_abc",
            4096,
            65536,
        )
        .unwrap();
        assert!(!ex.clipboard_text.contains("old session"));
        assert!(ex.clipboard_text.contains("continuation line"));
        assert!(ex.clipboard_text.contains("Issue: ny_abc"));
        assert!(!ex.partial, "{}", ex.summary());
        assert_eq!(ex.correlated_lines, 1);
        assert!(ex.covers.contains("2026-10-16 03:00:00"));
    }

    #[test]
    fn unknown_session_start_is_labelled_partial() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_log(dir.path(), "[2026-10-16 03:00:00] [INFO] [a] hi\n");
        let ex = prepare_excerpt(&path, None, "ny_x", 4096, 65536).unwrap();
        assert!(ex.partial);
        assert!(ex.clipboard_text.contains("PARTIAL"));
    }

    #[test]
    fn size_limit_truncates_oldest_and_says_so_but_keeps_correlated() {
        let dir = tempfile::tempdir().unwrap();
        let mut body = String::from("[2026-10-16 03:00:00] [WARN] [a] ny_key early failure\n");
        for i in 0..400 {
            body.push_str(&format!(
                "[2026-10-16 03:01:00] [INFO] [a] filler line {i}\n"
            ));
        }
        let path = write_log(dir.path(), &body);
        let ex = prepare_excerpt(
            &path,
            Some(at("2026-10-16 02:00:00")),
            "ny_key",
            2048,
            1 << 20,
        )
        .unwrap();
        assert!(ex.clipboard_truncated);
        assert!(ex.clipboard_text.contains("ny_key early failure"));
        assert!(ex.clipboard_text.contains("omitted"));
        assert!(ex.clipboard_text.len() < 4096);
        assert!(!ex.save_truncated, "save file holds everything in-session");
    }

    #[test]
    fn transcript_lines_are_withheld() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_log(
            dir.path(),
            "[2026-10-16 03:00:00] [INFO] [plexi::agent_transcript] user said hello\n\
             [2026-10-16 03:00:01] [INFO] [a] ok\n",
        );
        let ex = prepare_excerpt(&path, None, "ny_x", 4096, 65536).unwrap();
        assert!(!ex.clipboard_text.contains("user said hello"));
        assert_eq!(ex.lines_omitted_for_privacy, 1);
    }

    #[test]
    fn missing_log_is_a_visible_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = prepare_excerpt(&dir.path().join("nope.log"), None, "x", 10, 10).unwrap_err();
        assert!(error.contains("could not open"));
    }

    #[test]
    fn save_export_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let a = save_export(dir.path(), "one").unwrap();
        let b = save_export(dir.path(), "two").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read_to_string(a).unwrap(), "one");
    }
}
