//! Unified host search primitive (`stint 0814`).
//!
//! Owns the three shared building blocks for all host-side search and discovery:
//! 1. [`ScopedWalker`]: bounded, symlink-safe directory walker with canonical
//!    prune list ([`DEFAULT_PRUNE_DIRS`]), hidden-entry policy ([`HiddenPolicy`]),
//!    depth/directory/file budgets, and atomic generation cancellation.
//! 2. [`score_candidate`]: deterministic fuzzy + word-boundary + camelCase +
//!    path-aware + recency (`mtime`) scorer that returns both a composite rank
//!    score and UTF-8 byte ranges for UI match highlighting.
//! 3. [`SearchSession`]: off-thread, cancellation-safe two-phase search engine
//!    (Phase 1: scoped path walk + in-memory corpus cache; Phase 2: bounded
//!    content grep with line numbers and snippets) integrated with [`UiWake`].

use crate::app::ui_mailbox::UiWake;
use std::collections::VecDeque;
use std::fs;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Canonical directory names pruned by every host filesystem walk.
///
/// Consolidates the previously duplicated lists in `src/notes.rs` and
/// `src/app/assistant_host_tools.rs`.
pub const DEFAULT_PRUNE_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".stint",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    ".cache",
    ".gradle",
    ".idea",
    ".vscode",
    "node_modules",
    "bower_components",
    "site-packages",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    "Pods",
    "DerivedData",
    "Carthage",
    "vendor",
    "Library",
    "Applications",
    "Containers",
    "Caches",
];

/// Maximum file size scanned during Phase 2 content grep (`1 MiB`).
pub const MAX_CONTENT_FILE_BYTES: u64 = 1_048_576;

/// Number of leading bytes inspected for NUL (`\0`) to reject binary files.
const BINARY_SNIFF_BYTES: usize = 8_192;

/// Maximum number of search hits retained per session snapshot.
const MAX_SEARCH_HITS: usize = 250;

/// Maximum length (in chars) of a content match line preview.
const MAX_PREVIEW_CHARS: usize = 120;

/// Policy governing dot-prefixed files and directories during a walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HiddenPolicy {
    /// Skip all dot-prefixed entries (`name.starts_with('.')`).
    SkipHidden,
    /// Skip dot-prefixed entries except the specified anchor names (e.g. `&[".plexi"]`).
    AllowAnchors(&'static [&'static str]),
    /// Include dot-prefixed entries (entries in `prune_dirs` such as `.git` stay pruned).
    IncludeHidden,
}

impl HiddenPolicy {
    pub fn from_show_hidden(show_hidden: bool) -> Self {
        if show_hidden {
            Self::IncludeHidden
        } else {
            Self::SkipHidden
        }
    }

    fn allows(self, name: &str) -> bool {
        if !name.starts_with('.') {
            return true;
        }
        match self {
            Self::SkipHidden => false,
            Self::AllowAnchors(anchors) => anchors.contains(&name),
            Self::IncludeHidden => true,
        }
    }
}

/// Configuration for a [`ScopedWalker`] traversal.
#[derive(Clone, Debug)]
pub struct WalkConfig<'a> {
    pub root: &'a Path,
    pub hidden: HiddenPolicy,
    pub prune_dirs: &'a [&'a str],
    pub max_depth: usize,
    pub max_dirs: usize,
    pub max_files: usize,
    pub include_dirs: bool,
    pub include_files: bool,
    pub cancel: Option<&'a AtomicU64>,
    pub expected_generation: u64,
}

impl<'a> WalkConfig<'a> {
    pub fn new(root: &'a Path) -> Self {
        Self {
            root,
            hidden: HiddenPolicy::SkipHidden,
            prune_dirs: DEFAULT_PRUNE_DIRS,
            max_depth: 12,
            max_dirs: 2_500,
            max_files: 20_000,
            include_dirs: true,
            include_files: true,
            cancel: None,
            expected_generation: 0,
        }
    }

    pub fn with_hidden(mut self, hidden: HiddenPolicy) -> Self {
        self.hidden = hidden;
        self
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    pub fn with_max_dirs(mut self, max_dirs: usize) -> Self {
        self.max_dirs = max_dirs;
        self
    }

    pub fn with_max_files(mut self, max_files: usize) -> Self {
        self.max_files = max_files;
        self
    }

    pub fn with_include_dirs(mut self, include_dirs: bool) -> Self {
        self.include_dirs = include_dirs;
        self
    }

    pub fn with_include_files(mut self, include_files: bool) -> Self {
        self.include_files = include_files;
        self
    }

    pub fn with_cancel(mut self, cancel: &'a AtomicU64, expected_generation: u64) -> Self {
        self.cancel = Some(cancel);
        self.expected_generation = expected_generation;
        self
    }

    fn is_cancelled(&self) -> bool {
        self.cancel
            .is_some_and(|c| c.load(Ordering::Relaxed) != self.expected_generation)
    }

    pub fn is_pruned_dir(&self, name: &str) -> bool {
        if let HiddenPolicy::AllowAnchors(anchors) = self.hidden {
            if anchors.contains(&name) {
                return false;
            }
        }
        self.prune_dirs.contains(&name)
    }
}

/// A filesystem entry discovered by [`ScopedWalker`].
#[derive(Clone, Debug)]
pub struct WalkEntry {
    pub path: PathBuf,
    pub rel_path: String,
    pub file_name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
}

/// Summary and collected entries from a [`ScopedWalker`] run.
#[derive(Clone, Debug, Default)]
pub struct WalkOutcome {
    pub entries: Vec<WalkEntry>,
    pub dirs_visited: usize,
    pub files_seen: usize,
    pub budget_hit: bool,
    pub cancelled: bool,
}

/// Action returned by the directory visitor in [`ScopedWalker::walk_dirs_filtered`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirVisit {
    /// Descend into this directory's children (subject to depth/dir budgets).
    Descend,
    /// Do not descend into this directory's children.
    SkipChildren,
}

/// Bounded, symlink-skipping filesystem walker.
pub struct ScopedWalker;

impl ScopedWalker {
    /// Walk `config.root` breadth-first (shallow directories first, sorted per
    /// directory for deterministic ordering), skipping symlinks and pruned
    /// directories, and enforcing depth, directory, file, and cancellation caps.
    pub fn walk(config: &WalkConfig<'_>) -> WalkOutcome {
        let mut outcome = WalkOutcome::default();
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
        queue.push_back((config.root.to_path_buf(), 0));

        while let Some((dir, depth)) = queue.pop_front() {
            if config.is_cancelled() {
                outcome.cancelled = true;
                break;
            }
            if outcome.dirs_visited >= config.max_dirs {
                outcome.budget_hit = true;
                break;
            }
            outcome.dirs_visited += 1;

            let Ok(read_dir) = fs::read_dir(&dir) else {
                continue;
            };

            let mut dir_entries: Vec<fs::DirEntry> = Vec::new();
            for entry_res in read_dir {
                if config.is_cancelled() {
                    outcome.cancelled = true;
                    break;
                }
                if let Ok(entry) = entry_res {
                    dir_entries.push(entry);
                }
            }
            if outcome.cancelled {
                break;
            }

            dir_entries.sort_by_key(|a| a.file_name());

            for entry in dir_entries {
                if config.is_cancelled() {
                    outcome.cancelled = true;
                    break;
                }
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                // Never follow symlinks (files or directories) to prevent loops
                // and escaping the scoped root.
                if file_type.is_symlink() {
                    continue;
                }

                let file_name_os = entry.file_name();
                let Some(name_str) = file_name_os.to_str() else {
                    continue;
                };

                if !config.hidden.allows(name_str) {
                    continue;
                }

                let is_dir = file_type.is_dir();
                let is_file = file_type.is_file();
                if !is_dir && !is_file {
                    continue;
                }

                if is_dir && config.is_pruned_dir(name_str) {
                    continue;
                }
                if is_file && name_str == ".DS_Store" {
                    continue;
                }

                let entry_depth = depth + 1;
                let path = entry.path();
                let rel_path = path
                    .strip_prefix(config.root)
                    .ok()
                    .map(normalize_rel_path)
                    .unwrap_or_else(|| name_str.to_string());

                let meta = entry.metadata().ok();
                let size_bytes = meta
                    .as_ref()
                    .and_then(|m| if is_dir { None } else { Some(m.len()) });
                let modified = meta.as_ref().and_then(|m| m.modified().ok());

                if (is_dir && config.include_dirs) || (is_file && config.include_files) {
                    if outcome.files_seen >= config.max_files {
                        outcome.budget_hit = true;
                        break;
                    }
                    outcome.files_seen += 1;
                    outcome.entries.push(WalkEntry {
                        path: path.clone(),
                        rel_path,
                        file_name: name_str.to_string(),
                        is_dir,
                        depth: entry_depth,
                        size_bytes,
                        modified,
                    });
                }

                if is_dir && entry_depth < config.max_depth {
                    queue.push_back((path, entry_depth));
                }
            }

            if outcome.budget_hit || outcome.cancelled {
                break;
            }
        }

        outcome
    }

    /// Breadth-first directory-only walk with a caller callback (`on_dir`) that
    /// can inspect each child directory and decide whether to descend into it.
    /// Used by tiered note discovery (`src/notes.rs`) so all walks share one
    /// prune list, symlink check, hidden policy, and directory budget.
    pub fn walk_dirs_filtered<F>(config: &WalkConfig<'_>, mut on_dir: F) -> WalkOutcome
    where
        F: FnMut(&Path, &str, usize) -> DirVisit,
    {
        let mut outcome = WalkOutcome::default();
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
        queue.push_back((config.root.to_path_buf(), 0));

        while let Some((dir, depth)) = queue.pop_front() {
            if config.is_cancelled() {
                outcome.cancelled = true;
                break;
            }
            if depth >= config.max_depth {
                continue;
            }
            if outcome.dirs_visited >= config.max_dirs {
                outcome.budget_hit = true;
                break;
            }
            outcome.dirs_visited += 1;

            let Ok(read_dir) = fs::read_dir(&dir) else {
                continue;
            };
            let mut children: Vec<(PathBuf, String)> = Vec::new();
            for entry in read_dir.flatten() {
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                if !ft.is_dir() || ft.is_symlink() {
                    continue;
                }
                let name = entry.file_name();
                let Some(name_str) = name.to_str() else {
                    continue;
                };
                if !config.hidden.allows(name_str) {
                    continue;
                }
                if config.is_pruned_dir(name_str) {
                    continue;
                }
                children.push((entry.path(), name_str.to_string()));
            }
            children.sort_by(|a, b| a.0.cmp(&b.0));

            for (child_path, child_name) in children {
                let next_depth = depth + 1;
                if on_dir(&child_path, &child_name, next_depth) == DirVisit::Descend
                    && next_depth < config.max_depth
                {
                    queue.push_back((child_path, next_depth));
                }
            }
        }

        outcome
    }
}

fn normalize_rel_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '/' {
        raw.into_owned()
    } else {
        raw.replace(std::path::MAIN_SEPARATOR, "/")
    }
}

// ─── Scoring Engine ──────────────────────────────────────────────────────────

/// Candidate metadata supplied to [`score_candidate`].
#[derive(Clone, Copy, Debug)]
pub struct CandidateInput<'a> {
    /// Primary display label: filename for files, title for notes/commands.
    pub primary: &'a str,
    /// Optional relative path (`"src/overlays/command_palette.rs"`) for path-spanning queries.
    pub relative_path: Option<&'a str>,
    /// Optional secondary searchable text (subtitle, keywords, category, preview).
    pub secondary: Option<&'a str>,
    /// Optional modification timestamp for recency boost.
    pub mtime: Option<SystemTime>,
    /// Directory depth relative to the search root (`1` = immediate child).
    pub depth: usize,
}

impl<'a> CandidateInput<'a> {
    pub fn new(primary: &'a str) -> Self {
        Self {
            primary,
            relative_path: None,
            secondary: None,
            mtime: None,
            depth: 1,
        }
    }

    pub fn with_relative_path(mut self, relative_path: &'a str) -> Self {
        self.relative_path = Some(relative_path);
        self
    }

    pub fn with_secondary(mut self, secondary: &'a str) -> Self {
        self.secondary = Some(secondary);
        self
    }

    pub fn with_mtime(mut self, mtime: Option<SystemTime>) -> Self {
        self.mtime = mtime;
        self
    }

    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }
}

/// Composite rank score and matched byte ranges returned by [`score_candidate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchScore {
    /// Total rank score; higher is better.
    pub total: i64,
    /// Matched UTF-8 byte ranges within `CandidateInput::primary`.
    pub primary_ranges: Vec<Range<usize>>,
    /// Matched UTF-8 byte ranges within `CandidateInput::relative_path`.
    pub path_ranges: Vec<Range<usize>>,
}

/// Score `candidate` against `query`. Returns `None` if `query` does not match.
///
/// Key properties:
/// - **Basename priority (2x)**: Matches in `primary` receive a 2x weight
///   multiplier over matches that only occur in parent path components or
///   secondary text.
/// - **Lexical tiers**: Exact match > prefix match > word-boundary contiguous
///   substring > general contiguous substring > word-boundary/camelCase
///   subsequence > scattered subsequence.
/// - **Recency boost**: Smooth monotonic boost from `mtime` relative to `now`
///   so active files outrank stale files on similar lexical quality.
/// - **Shallow depth tie-breaker**: Shallower paths beat deeper paths when
///   lexical score and recency are tied.
pub fn score_candidate(
    query: &str,
    candidate: &CandidateInput<'_>,
    now: SystemTime,
) -> Option<MatchScore> {
    let trimmed = query.trim();
    let recency = recency_boost(candidate.mtime, now);
    let depth_adj = -((candidate.depth.min(20) as i64) * 15);
    let len_adj = -((candidate.primary.len().min(120)) as i64);

    if trimmed.is_empty() {
        return Some(MatchScore {
            total: recency + depth_adj + len_adj,
            primary_ranges: Vec::new(),
            path_ranges: Vec::new(),
        });
    }

    // Try matching as a single pattern first, or across whitespace-separated tokens.
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() <= 1 {
        let token = tokens[0];
        score_single_token(token, candidate, recency, depth_adj, len_adj)
    } else {
        // Multi-token query: every token must match somewhere in the candidate.
        let mut total_lex = 0i64;
        let mut primary_ranges = Vec::new();
        let mut path_ranges = Vec::new();
        for token in tokens {
            let sub = score_single_token(token, candidate, 0, 0, 0)?;
            total_lex += sub.total;
            primary_ranges.extend(sub.primary_ranges);
            path_ranges.extend(sub.path_ranges);
        }
        Some(MatchScore {
            total: total_lex + recency + depth_adj + len_adj,
            primary_ranges: merge_byte_ranges(primary_ranges),
            path_ranges: merge_byte_ranges(path_ranges),
        })
    }
}

fn score_single_token(
    token: &str,
    candidate: &CandidateInput<'_>,
    recency: i64,
    depth_adj: i64,
    len_adj: i64,
) -> Option<MatchScore> {
    let primary_match = score_text_target(token, candidate.primary);
    let path_match = candidate
        .relative_path
        .and_then(|rel| score_text_target(token, rel));
    let secondary_match = candidate
        .secondary
        .and_then(|sec| score_text_target(token, sec));

    if primary_match.is_none() && path_match.is_none() && secondary_match.is_none() {
        return None;
    }

    // Basename (primary) matches receive a 2x weight multiplier.
    let primary_weighted = primary_match.as_ref().map(|(s, _)| *s * 2);

    // Path match receives 1x weight, plus a bonus if the trailing characters of
    // the match land inside the basename tail of `relative_path`.
    let mut derived_primary_ranges: Vec<Range<usize>> = Vec::new();
    let path_weighted = path_match.as_ref().map(|(s, ranges)| {
        let mut score = *s;
        if let Some(rel) = candidate.relative_path {
            if let Some(base_offset) = basename_byte_offset(rel, candidate.primary) {
                let mut tail_bytes = 0usize;
                for r in ranges {
                    if r.end > base_offset {
                        let start = r.start.max(base_offset) - base_offset;
                        let end = (r.end - base_offset).min(candidate.primary.len());
                        if start < end {
                            tail_bytes += end - start;
                            derived_primary_ranges.push(start..end);
                        }
                    }
                }
                if tail_bytes > 0 {
                    score += 1_200 + (tail_bytes as i64 * 120);
                }
            }
        }
        score
    });

    // Secondary match (e.g. command subtitle/category or note preview) gets 0.75x weight.
    let secondary_weighted = secondary_match.as_ref().map(|(s, _)| (*s * 3) / 4);

    let best_lexical = [primary_weighted, path_weighted, secondary_weighted]
        .into_iter()
        .flatten()
        .max()?;

    let primary_ranges = if let Some((_, ranges)) = primary_match {
        ranges
    } else {
        merge_byte_ranges(derived_primary_ranges)
    };

    let path_ranges = if let Some((_, ranges)) = path_match {
        ranges
    } else if let Some(rel) = candidate.relative_path {
        if let Some(base_offset) = basename_byte_offset(rel, candidate.primary) {
            primary_ranges
                .iter()
                .map(|r| (r.start + base_offset)..(r.end + base_offset))
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let effective_len_adj = if primary_weighted.is_some() || path_weighted.is_some() {
        len_adj
    } else {
        0
    };

    Some(MatchScore {
        total: best_lexical + recency + depth_adj + effective_len_adj,
        primary_ranges,
        path_ranges,
    })
}

fn basename_byte_offset(rel_path: &str, primary: &str) -> Option<usize> {
    let trimmed_primary = primary.trim_end_matches('/');
    let trimmed_rel = rel_path.trim_end_matches('/');
    if trimmed_rel.ends_with(trimmed_primary) {
        let offset = trimmed_rel.len() - trimmed_primary.len();
        if offset == 0 || trimmed_rel.as_bytes().get(offset - 1) == Some(&b'/') {
            return Some(offset);
        }
    }
    None
}

#[derive(Clone, Copy)]
struct CharInfo {
    ch: char,
    lower: char,
    byte_start: usize,
    byte_end: usize,
    boundary_bonus: i64,
}

fn build_char_slice(text: &str) -> Vec<CharInfo> {
    let mut out = Vec::with_capacity(text.len().min(512));
    let mut prev: Option<char> = None;
    for (byte_start, ch) in text.char_indices().take(512) {
        let byte_end = byte_start + ch.len_utf8();
        let lower = ch.to_lowercase().next().unwrap_or(ch);
        let boundary_bonus = match prev {
            None => 260,
            Some('/') => 230,
            Some('_' | '-' | '.' | ' ' | ':') => 210,
            Some(p) if p.is_lowercase() && ch.is_uppercase() => 190,
            Some(p) if p.is_alphabetic() && ch.is_ascii_digit() => 160,
            _ => 0,
        };
        out.push(CharInfo {
            ch,
            lower,
            byte_start,
            byte_end,
            boundary_bonus,
        });
        prev = Some(ch);
    }
    out
}

/// Score `pattern` against a single `text` target, returning `(score, matched_byte_ranges)`.
fn score_text_target(pattern: &str, text: &str) -> Option<(i64, Vec<Range<usize>>)> {
    if pattern.is_empty() || text.is_empty() {
        return None;
    }

    let pat_chars: Vec<(char, char)> = pattern
        .chars()
        .take(64)
        .map(|c| (c, c.to_lowercase().next().unwrap_or(c)))
        .collect();
    let text_chars = build_char_slice(text);
    let m = pat_chars.len();
    let n = text_chars.len();
    if m == 0 || m > n {
        return None;
    }

    let has_upper = pat_chars.iter().any(|(c, _)| c.is_uppercase());

    // Fast Subsequence check before DP: verify every pattern char exists in order.
    let mut check_idx = 0usize;
    for &(_, pat_lower) in &pat_chars {
        while check_idx < n && text_chars[check_idx].lower != pat_lower {
            check_idx += 1;
        }
        if check_idx == n {
            return None;
        }
        check_idx += 1;
    }

    // 1. Check for contiguous substring occurrences first to award exact / prefix /
    //    word-boundary / contiguous tiers when applicable, while also letting DP
    //    compete for multi-word abbreviations like "cmdpal".
    let mut best_contiguous: Option<(i64, Vec<Range<usize>>)> = None;
    for start in 0..=(n - m) {
        let mut matches_here = true;
        let mut exact_case_count = 0usize;
        for i in 0..m {
            let (pat_orig, pat_low) = pat_chars[i];
            let tc = text_chars[start + i];
            if tc.lower != pat_low {
                matches_here = false;
                break;
            }
            if has_upper && pat_orig.is_uppercase() && tc.ch != pat_orig {
                // Smart-case: uppercase query characters prefer exact case,
                // though we still allow case-insensitive match with lower score.
            }
            if tc.ch == pat_orig {
                exact_case_count += 1;
            }
        }
        if matches_here {
            let byte_start = text_chars[start].byte_start;
            let byte_end = text_chars[start + m - 1].byte_end;
            let boundary = text_chars[start].boundary_bonus;
            let is_exact = m == n || (start == 0 && text[byte_end..] == *"/");
            let is_prefix = start == 0;
            let is_word_boundary = boundary > 0;

            let tier_bonus = if is_exact {
                12_000
            } else if is_prefix {
                9_000
            } else if is_word_boundary {
                6_500 + boundary
            } else {
                4_200
            };

            let case_bonus = if exact_case_count == m {
                350
            } else {
                (exact_case_count as i64) * 35
            };
            let length_bonus = (m as i64) * 260;
            let pos_penalty = (start as i64) * 8;
            let score = tier_bonus + length_bonus + case_bonus - pos_penalty;

            if best_contiguous.as_ref().is_none_or(|(best, _)| score > *best) {
                best_contiguous = Some((score, std::iter::once(byte_start..byte_end).collect()));
            }
        }
    }

    // 2. Optimal fuzzy subsequence alignment via DP.
    // `dp[i][j]` = best score matching `pat_chars[0..=i]` ending at `text_chars[j]`.
    const NEG_INF: i64 = i64::MIN / 4;
    let mut dp = vec![vec![NEG_INF; n]; m];
    let mut parent = vec![vec![0usize; n]; m];

    for j in 0..n {
        let (pat_orig, pat_low) = pat_chars[0];
        let tc = text_chars[j];
        if tc.lower == pat_low {
            let case_bonus = if has_upper && pat_orig == tc.ch { 50 } else { 0 };
            let start_penalty = (j as i64) * 4;
            dp[0][j] = 100 + tc.boundary_bonus + case_bonus - start_penalty;
        }
    }

    for i in 1..m {
        let (pat_orig, pat_low) = pat_chars[i];
        for j in i..n {
            let tc = text_chars[j];
            if tc.lower != pat_low {
                continue;
            }
            let case_bonus = if has_upper && pat_orig == tc.ch { 50 } else { 0 };
            let mut best_prev = NEG_INF;
            let mut best_k = i - 1;

            for k in (i - 1)..j {
                let prev_score = dp[i - 1][k];
                if prev_score <= NEG_INF / 2 {
                    continue;
                }
                let transition = if k + 1 == j {
                    // Consecutive character match — always preferred over a jump.
                    let boundary_chain = if text_chars[k].boundary_bonus > 0 {
                        160
                    } else {
                        80
                    };
                    320 + boundary_chain
                } else {
                    let gap = (j - 1 - k).min(40) as i64;
                    if tc.boundary_bonus > 0 {
                        tc.boundary_bonus - (gap * 35)
                    } else {
                        let crossed_boundary =
                            ((k + 1)..j).any(|mid| text_chars[mid].boundary_bonus > 0);
                        if crossed_boundary {
                            -140 - (gap * 45)
                        } else {
                            // Same-word consonant abbreviation (e.g. "cmd" in "command")
                            40 - (gap * 30)
                        }
                    }
                };
                let candidate_score = prev_score + 100 + case_bonus + transition;
                if candidate_score > best_prev {
                    best_prev = candidate_score;
                    best_k = k;
                }
            }

            if best_prev > NEG_INF / 2 {
                dp[i][j] = best_prev;
                parent[i][j] = best_k;
            }
        }
    }

    let mut best_end_j = None;
    let mut best_dp_score = NEG_INF;
    for (j, &score) in dp[m - 1].iter().enumerate().take(n).skip(m - 1) {
        if score > best_dp_score {
            best_dp_score = score;
            best_end_j = Some(j);
        }
    }

    let dp_result = best_end_j.map(|end_j| {
        let mut indices = vec![0usize; m];
        let mut curr_j = end_j;
        for i in (0..m).rev() {
            indices[i] = curr_j;
            if i > 0 {
                curr_j = parent[i][curr_j];
            }
        }
        // Count characters that sit on a word boundary or contiguously extend a
        // word-boundary prefix (e.g. "pal" in "command_palette").
        let mut word_prefix_chars = 0usize;
        let mut in_boundary_run = false;
        let mut non_contiguous_jumps = 0usize;
        for idx in 0..m {
            let pos = indices[idx];
            if idx == 0 || indices[idx - 1] + 1 != pos {
                if idx > 0 {
                    non_contiguous_jumps += 1;
                }
                in_boundary_run = text_chars[pos].boundary_bonus > 0;
            }
            if in_boundary_run || text_chars[pos].boundary_bonus > 0 {
                word_prefix_chars += 1;
            }
        }
        let span = indices[m - 1].saturating_sub(indices[0]) + 1;
        let span_penalty = (span.saturating_sub(m) as i64) * 18;
        let mut total = best_dp_score - span_penalty;
        if word_prefix_chars >= 2 {
            total += (word_prefix_chars as i64) * 260;
        }
        if non_contiguous_jumps == 0 {
            total += 1_500;
        }
        let ranges = merge_byte_ranges(
            indices
                .into_iter()
                .map(|pos| text_chars[pos].byte_start..text_chars[pos].byte_end)
                .collect(),
        );
        (total, ranges)
    });

    match (best_contiguous, dp_result) {
        (Some(c), Some(d)) => {
            if c.0 >= d.0 {
                Some(c)
            } else {
                Some(d)
            }
        }
        (Some(c), None) => Some(c),
        (None, Some(d)) => Some(d),
        (None, None) => None,
    }
}

/// Smooth monotonic recency boost in `[0, 360]` based on `now.duration_since(mtime)`.
fn recency_boost(mtime: Option<SystemTime>, now: SystemTime) -> i64 {
    let Some(mtime) = mtime else {
        return 0;
    };
    let age_secs = now
        .duration_since(mtime)
        .unwrap_or(Duration::ZERO)
        .as_secs();

    if age_secs < 300 {
        // < 5 minutes: 300..=360
        360 - ((age_secs * 60) / 300) as i64
    } else if age_secs < 3_600 {
        // < 1 hour: 220..=300
        300 - (((age_secs - 300) * 80) / 3_300) as i64
    } else if age_secs < 86_400 {
        // < 24 hours: 130..=220
        220 - (((age_secs - 3_600) * 90) / 82_800) as i64
    } else if age_secs < 604_800 {
        // < 7 days: 60..=130
        130 - (((age_secs - 86_400) * 70) / 518_400) as i64
    } else if age_secs < 2_592_000 {
        // < 30 days: 10..=60
        60 - (((age_secs - 604_800) * 50) / 1_987_200) as i64
    } else {
        0
    }
}

/// Merge adjacent or overlapping byte ranges into minimal sorted ranges.
pub fn merge_byte_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.start >= range.end {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

// ─── Content Grep Helper ─────────────────────────────────────────────────────

/// Result of scanning a single file's contents for a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentMatch {
    pub line_number: usize,
    pub line_preview: String,
    pub line_ranges: Vec<Range<usize>>,
    pub additional_matches: usize,
    pub match_score: i64,
}

/// Inspect `path` for a content match against `query`.
///
/// Enforces:
/// - Skips files larger than [`MAX_CONTENT_FILE_BYTES`] (`1 MiB`).
/// - Reads at most `MAX_CONTENT_FILE_BYTES` and rejects files containing NUL
///   bytes in the first `8 KiB` or invalid UTF-8.
pub fn scan_file_for_query(path: &Path, query: &str) -> Option<ContentMatch> {
    let trimmed = query.trim();
    if trimmed.chars().count() < 2 {
        return None;
    }
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_CONTENT_FILE_BYTES {
        return None;
    }

    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity((meta.len() as usize).min(MAX_CONTENT_FILE_BYTES as usize));
    file.take(MAX_CONTENT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_CONTENT_FILE_BYTES {
        return None;
    }

    // Binary file guard: reject files with NUL bytes in the sniff window.
    let sniff_len = bytes.len().min(BINARY_SNIFF_BYTES);
    if bytes[..sniff_len].contains(&0) {
        return None;
    }

    let content = std::str::from_utf8(&bytes).ok()?;
    scan_str_for_query(content, trimmed)
}

/// Scan an in-memory UTF-8 string for `query`, returning the best/first matching
/// 1-based line, trimmed snippet, highlight ranges, and additional match count.
pub fn scan_str_for_query(content: &str, query: &str) -> Option<ContentMatch> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    let query_lower = trimmed.to_lowercase();
    let has_upper = trimmed.chars().any(|c| c.is_uppercase());

    let mut first_match: Option<(usize, &str, Range<usize>, bool)> = None;
    let mut total_matching_lines = 0usize;

    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if let Some((byte_range, exact_case)) = find_substring_in_line(raw_line, trimmed, &query_lower, has_upper) {
            total_matching_lines += 1;
            let replace = match &first_match {
                None => true,
                Some((_, _, _, prev_exact)) => !*prev_exact && exact_case,
            };
            if replace {
                first_match = Some((line_no, raw_line, byte_range, exact_case));
            }
        }
    }

    let (line_number, raw_line, raw_range, exact_case) = first_match?;
    let (line_preview, line_ranges) = build_line_preview(raw_line, raw_range);
    let additional_matches = total_matching_lines.saturating_sub(1);
    let match_score = 3_200
        + if exact_case { 250 } else { 0 }
        + ((additional_matches.min(10) as i64) * 40)
        - ((line_number.min(500) as i64) * 2);

    Some(ContentMatch {
        line_number,
        line_preview,
        line_ranges,
        additional_matches,
        match_score,
    })
}

fn find_substring_in_line(
    line: &str,
    query_orig: &str,
    query_lower: &str,
    has_upper: bool,
) -> Option<(Range<usize>, bool)> {
    if let Some(pos) = line.find(query_orig) {
        return Some((pos..(pos + query_orig.len()), true));
    }
    if has_upper {
        // Still allow case-insensitive match, marked exact_case = false.
    }
    if line.is_ascii() && query_lower.is_ascii() {
        let line_bytes = line.as_bytes();
        let q_bytes = query_lower.as_bytes();
        if q_bytes.is_empty() || q_bytes.len() > line_bytes.len() {
            return None;
        }
        for start in 0..=(line_bytes.len() - q_bytes.len()) {
            if line_bytes[start..start + q_bytes.len()].eq_ignore_ascii_case(q_bytes) {
                return Some((start..(start + q_bytes.len()), false));
            }
        }
        None
    } else {
        let lower_line = line.to_lowercase();
        let pos = lower_line.find(query_lower)?;
        // Map char range back to original `line` byte offsets if UTF-8 lengths preserved.
        if lower_line.len() == line.len()
            && line.is_char_boundary(pos)
            && line.is_char_boundary(pos + query_lower.len())
        {
            Some((pos..(pos + query_lower.len()), false))
        } else {
            // Fallback char-by-char window scan for non-ASCII case folding.
            let q_chars: Vec<char> = query_lower.chars().collect();
            let l_chars: Vec<(usize, char)> = line
                .char_indices()
                .map(|(b, c)| (b, c.to_lowercase().next().unwrap_or(c)))
                .collect();
            if q_chars.len() > l_chars.len() {
                return None;
            }
            for i in 0..=(l_chars.len() - q_chars.len()) {
                if l_chars[i..i + q_chars.len()]
                    .iter()
                    .zip(q_chars.iter())
                    .all(|((_, lc), qc)| lc == qc)
                {
                    let start_b = l_chars[i].0;
                    let end_b = if i + q_chars.len() < l_chars.len() {
                        l_chars[i + q_chars.len()].0
                    } else {
                        line.len()
                    };
                    return Some((start_b..end_b, false));
                }
            }
            None
        }
    }
}

fn build_line_preview(raw_line: &str, match_range: Range<usize>) -> (String, Vec<Range<usize>>) {
    let trimmed_start = raw_line.trim_start();
    let leading_trimmed = raw_line.len() - trimmed_start.len();
    let trimmed = trimmed_start.trim_end();

    let adj_start = match_range.start.saturating_sub(leading_trimmed).min(trimmed.len());
    let adj_end = match_range.end.saturating_sub(leading_trimmed).min(trimmed.len());

    let char_count = trimmed.chars().count();
    if char_count <= MAX_PREVIEW_CHARS {
        let ranges = if adj_start < adj_end {
            std::iter::once(adj_start..adj_end).collect()
        } else {
            Vec::new()
        };
        return (trimmed.to_string(), ranges);
    }

    // Window around the match so the matched span stays visible in the preview.
    let chars: Vec<(usize, char)> = trimmed.char_indices().collect();
    let match_char_idx = chars
        .iter()
        .position(|(b, _)| *b >= adj_start)
        .unwrap_or(0);
    let half = MAX_PREVIEW_CHARS / 3;
    let win_start_char = match_char_idx.saturating_sub(half);
    let win_end_char = (win_start_char + MAX_PREVIEW_CHARS).min(chars.len());
    let win_start_byte = chars[win_start_char].0;
    let win_end_byte = if win_end_char < chars.len() {
        chars[win_end_char].0
    } else {
        trimmed.len()
    };

    let mut preview = String::new();
    let mut prefix_bytes = 0usize;
    if win_start_char > 0 {
        preview.push('\u{2026}');
        prefix_bytes = '\u{2026}'.len_utf8();
    }
    preview.push_str(&trimmed[win_start_byte..win_end_byte]);
    if win_end_char < chars.len() {
        preview.push('\u{2026}');
    }

    let ranges = if adj_end > win_start_byte && adj_start < win_end_byte {
        let s = adj_start.max(win_start_byte) - win_start_byte + prefix_bytes;
        let e = adj_end.min(win_end_byte) - win_start_byte + prefix_bytes;
        if s < e {
            std::iter::once(s..e).collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    (preview, ranges)
}

// ─── Two-Phase Off-Thread SearchSession ──────────────────────────────────────

/// Distinguishes path/filename hits from in-file content hits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchHitKind {
    /// Matched on filename or relative path.
    Path {
        /// Optional inline content match when the same file also matched in Phase 2.
        content: Option<ContentSnippet>,
    },
    /// Matched inside file contents at a specific 1-based line.
    Content(ContentSnippet),
}

/// Line number and highlighted snippet for a content match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentSnippet {
    pub line_number: usize,
    pub line_preview: String,
    pub line_ranges: Vec<Range<usize>>,
    pub additional_matches: usize,
}

impl SearchHitKind {
    pub fn content_snippet(&self) -> Option<&ContentSnippet> {
        match self {
            Self::Path { content } => content.as_ref(),
            Self::Content(snippet) => Some(snippet),
        }
    }
}

/// A ranked search result produced by [`SearchSession`].
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub path: PathBuf,
    pub rel_path: String,
    pub file_name: String,
    pub is_dir: bool,
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
    pub score: i64,
    pub name_ranges: Vec<Range<usize>>,
    pub rel_path_ranges: Vec<Range<usize>>,
    pub kind: SearchHitKind,
}

impl SearchHit {
    /// Relative parent directory prefix with trailing `/` (e.g. `"src/ui/"`), or
    /// empty string when the entry is an immediate child of the search root.
    pub fn parent_prefix(&self) -> &str {
        if let Some(slash_pos) = self.rel_path.rfind('/') {
            &self.rel_path[..=slash_pos]
        } else {
            ""
        }
    }
}

enum WorkerCommand {
    Run {
        generation: u64,
        root: PathBuf,
        hidden: HiddenPolicy,
        query: String,
        invalidate_corpus: bool,
    },
}

enum WorkerUpdate {
    Phase1Complete {
        generation: u64,
        root: PathBuf,
        hidden: HiddenPolicy,
        corpus: Arc<Vec<WalkEntry>>,
        hits: Vec<SearchHit>,
        scanning_content: bool,
    },
    Phase2Complete {
        generation: u64,
        hits: Vec<SearchHit>,
    },
}

/// Stateful, cancellation-safe two-phase search session.
///
/// - **Phase 1 (Path walk + cache)**: Walks `root` off-thread via [`ScopedWalker`],
///   caches the discovered [`WalkEntry`] corpus in memory for `(root, hidden)`,
///   and emits ranked [`SearchHitKind::Path`] results immediately. When the
///   corpus is already cached for `(root, hidden)`, re-scoring happens both
///   synchronously on the caller thread (zero frame lag) and in the worker.
/// - **Phase 2 (Content grep)**: For queries with `>= 2` non-whitespace chars,
///   scans candidate text files (`<= 1 MiB`) in recency order, enriching path
///   hits or appending ranked [`SearchHitKind::Content`] hits, then wakes the
///   host via [`UiWake`].
pub struct SearchSession {
    generation: Arc<AtomicU64>,
    wake_slot: Arc<Mutex<Option<Arc<dyn UiWake>>>>,
    cmd_tx: Option<mpsc::Sender<WorkerCommand>>,
    update_rx: Option<mpsc::Receiver<WorkerUpdate>>,
    active_root: Option<PathBuf>,
    active_hidden: HiddenPolicy,
    active_query: String,
    cached_corpus: Option<(PathBuf, HiddenPolicy, Arc<Vec<WalkEntry>>)>,
    hits: Vec<SearchHit>,
    scanning: bool,
}

impl Default for SearchSession {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchSession {
    pub fn new() -> Self {
        Self {
            generation: Arc::new(AtomicU64::new(0)),
            wake_slot: Arc::new(Mutex::new(None)),
            cmd_tx: None,
            update_rx: None,
            active_root: None,
            active_hidden: HiddenPolicy::SkipHidden,
            active_query: String::new(),
            cached_corpus: None,
            hits: Vec::new(),
            scanning: false,
        }
    }

    /// Current ranked search hits.
    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }

    /// True while the background worker is still walking or scanning file contents
    /// for the latest query generation.
    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    /// Bind the host [`UiWake`] handle so background phase completions wake the
    /// event loop even while the window is hidden or occluded.
    pub fn bind_wake(&mut self, wake: &Arc<dyn UiWake>) {
        if let Ok(mut slot) = self.wake_slot.lock() {
            *slot = Some(Arc::clone(wake));
        }
    }

    /// Invalidate the cached directory walk so the next search re-walks disk.
    pub fn invalidate_cache(&mut self) {
        self.cached_corpus = None;
    }

    /// Cancel any in-flight background work and clear results.
    pub fn clear(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.active_query.clear();
        self.hits.clear();
        self.scanning = false;
    }

    /// Drain completed worker updates. Returns `true` if `hits` or `scanning`
    /// state changed this call.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.update_rx else {
            return false;
        };
        let current_gen = self.generation.load(Ordering::SeqCst);
        let mut changed = false;

        while let Ok(update) = rx.try_recv() {
            match update {
                WorkerUpdate::Phase1Complete {
                    generation,
                    root,
                    hidden,
                    corpus,
                    hits,
                    scanning_content,
                } => {
                    // Always keep the latest walked corpus if it matches our active root/hidden.
                    if self.active_root.as_ref() == Some(&root) && self.active_hidden == hidden {
                        self.cached_corpus = Some((root, hidden, corpus));
                    }
                    if generation == current_gen {
                        self.hits = hits;
                        self.scanning = scanning_content;
                        changed = true;
                    }
                }
                WorkerUpdate::Phase2Complete { generation, hits } => {
                    if generation == current_gen {
                        self.hits = hits;
                        self.scanning = false;
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// Service external state from `App::logic`: binds `wake` and drains any
    /// completed background search updates.
    pub fn service_external(&mut self, wake: &Arc<dyn UiWake>) -> bool {
        self.bind_wake(wake);
        self.poll()
    }

    /// Update the search query and root.
    ///
    /// Immediately scores `fallback_shallow` (or the already-cached deep corpus
    /// for `(root, hidden)`) on the calling thread so the UI has instant 0-ms
    /// results, and dispatches the off-thread worker for deep path discovery
    /// and Phase 2 content grep.
    pub fn update_query(
        &mut self,
        root: &Path,
        hidden: HiddenPolicy,
        query: &str,
        fallback_shallow: &[WalkEntry],
        force_rewalk: bool,
    ) {
        self.ensure_worker();
        let next_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let root_buf = root.to_path_buf();
        let invalidate = force_rewalk
            || self
                .cached_corpus
                .as_ref()
                .is_none_or(|(r, h, _)| r != &root_buf || *h != hidden);
        if force_rewalk {
            self.cached_corpus = None;
        }

        self.active_root = Some(root_buf.clone());
        self.active_hidden = hidden;
        self.active_query = query.to_string();

        // Immediate synchronous scoring over the cached deep corpus (if available)
        // or the caller's shallow directory listing so the very first frame is populated.
        let now = SystemTime::now();
        if let Some((_, _, corpus)) = &self.cached_corpus {
            self.hits = rank_path_hits(corpus, query, now);
        } else {
            self.hits = rank_path_hits(fallback_shallow, query, now);
        }

        let needs_content_scan = query.trim().chars().count() >= 2;
        self.scanning = invalidate || needs_content_scan;

        if let Some(tx) = &self.cmd_tx {
            let _ = tx.send(WorkerCommand::Run {
                generation: next_gen,
                root: root_buf,
                hidden,
                query: query.to_string(),
                invalidate_corpus: force_rewalk,
            });
        }
    }

    fn ensure_worker(&mut self) {
        if self.cmd_tx.is_some() {
            return;
        }
        let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCommand>();
        let (update_tx, update_rx) = mpsc::channel::<WorkerUpdate>();
        let generation = Arc::clone(&self.generation);
        let wake_slot = Arc::clone(&self.wake_slot);

        if let Err(err) = std::thread::Builder::new()
            .name("plexi-search".to_string())
            .spawn(move || search_worker_loop(cmd_rx, update_tx, generation, wake_slot))
        {
            log::warn!("search: failed to spawn background worker thread: {err}");
            return;
        }

        self.cmd_tx = Some(cmd_tx);
        self.update_rx = Some(update_rx);
    }
}

impl Drop for SearchSession {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

fn search_worker_loop(
    cmd_rx: mpsc::Receiver<WorkerCommand>,
    update_tx: mpsc::Sender<WorkerUpdate>,
    generation: Arc<AtomicU64>,
    wake_slot: Arc<Mutex<Option<Arc<dyn UiWake>>>>,
) {
    let mut cached_corpus: Option<(PathBuf, HiddenPolicy, Arc<Vec<WalkEntry>>)> = None;

    while let Ok(mut cmd) = cmd_rx.recv() {
        // Coalesce rapid updates: drain to the newest queued command.
        while let Ok(newer) = cmd_rx.try_recv() {
            cmd = newer;
        }

        let WorkerCommand::Run {
            generation: my_gen,
            root,
            hidden,
            query,
            invalidate_corpus,
        } = cmd;

        if generation.load(Ordering::Relaxed) != my_gen {
            continue;
        }

        // Phase 1: Reuse cached walk corpus for `(root, hidden)` or walk disk.
        let corpus = match &cached_corpus {
            Some((c_root, c_hidden, entries))
                if !invalidate_corpus && c_root == &root && *c_hidden == hidden =>
            {
                Arc::clone(entries)
            }
            _ => {
                let walk_cfg = WalkConfig::new(&root)
                    .with_hidden(hidden)
                    .with_cancel(&generation, my_gen);
                let outcome = ScopedWalker::walk(&walk_cfg);
                if outcome.cancelled || generation.load(Ordering::Relaxed) != my_gen {
                    continue;
                }
                let arc = Arc::new(outcome.entries);
                cached_corpus = Some((root.clone(), hidden, Arc::clone(&arc)));
                arc
            }
        };

        let now = SystemTime::now();
        let phase1_hits = rank_path_hits(&corpus, &query, now);
        if generation.load(Ordering::Relaxed) != my_gen {
            continue;
        }

        let trimmed_query = query.trim();
        let run_phase2 = trimmed_query.chars().count() >= 2;

        if update_tx
            .send(WorkerUpdate::Phase1Complete {
                generation: my_gen,
                root: root.clone(),
                hidden,
                corpus: Arc::clone(&corpus),
                hits: phase1_hits.clone(),
                scanning_content: run_phase2,
            })
            .is_err()
        {
            break;
        }
        notify_wake(&wake_slot);

        if !run_phase2 {
            continue;
        }

        // Phase 2: Scan candidate text files ordered by recency (newest first), then shallow depth.
        let mut file_candidates: Vec<&WalkEntry> = corpus
            .iter()
            .filter(|e| {
                !e.is_dir
                    && e.size_bytes
                        .is_some_and(|sz| sz > 0 && sz <= MAX_CONTENT_FILE_BYTES)
            })
            .collect();

        file_candidates.sort_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| a.depth.cmp(&b.depth))
                .then_with(|| a.rel_path.cmp(&b.rel_path))
        });

        let mut merged_hits = phase1_hits;
        let mut existing_by_path: std::collections::HashMap<PathBuf, usize> = merged_hits
            .iter()
            .enumerate()
            .map(|(i, h)| (h.path.clone(), i))
            .collect();

        let mut cancelled = false;
        for entry in file_candidates {
            if generation.load(Ordering::Relaxed) != my_gen {
                cancelled = true;
                break;
            }
            let Some(content_match) = scan_file_for_query(&entry.path, trimmed_query) else {
                continue;
            };
            let snippet = ContentSnippet {
                line_number: content_match.line_number,
                line_preview: content_match.line_preview,
                line_ranges: content_match.line_ranges,
                additional_matches: content_match.additional_matches,
            };

            if let Some(&hit_idx) = existing_by_path.get(&entry.path) {
                // File already matched by path: attach the content snippet and boost score.
                merged_hits[hit_idx].score += 600;
                merged_hits[hit_idx].kind = SearchHitKind::Path {
                    content: Some(snippet),
                };
            } else if merged_hits.len() < MAX_SEARCH_HITS {
                let score = content_match.match_score
                    + recency_boost(entry.modified, now)
                    - ((entry.depth.min(20) as i64) * 15);
                let idx = merged_hits.len();
                existing_by_path.insert(entry.path.clone(), idx);
                merged_hits.push(SearchHit {
                    path: entry.path.clone(),
                    rel_path: entry.rel_path.clone(),
                    file_name: entry.file_name.clone(),
                    is_dir: false,
                    size_bytes: entry.size_bytes,
                    modified: entry.modified,
                    score,
                    name_ranges: Vec::new(),
                    rel_path_ranges: Vec::new(),
                    kind: SearchHitKind::Content(snippet),
                });
            }
        }

        if cancelled || generation.load(Ordering::Relaxed) != my_gen {
            continue;
        }

        merged_hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.rel_path.cmp(&b.rel_path))
        });
        if merged_hits.len() > MAX_SEARCH_HITS {
            merged_hits.truncate(MAX_SEARCH_HITS);
        }

        log::info!(
            "search: completed query_len={} hits={}",
            trimmed_query.len(),
            merged_hits.len()
        );

        if update_tx
            .send(WorkerUpdate::Phase2Complete {
                generation: my_gen,
                hits: merged_hits,
            })
            .is_err()
        {
            break;
        }
        notify_wake(&wake_slot);
    }
}

fn notify_wake(wake_slot: &Arc<Mutex<Option<Arc<dyn UiWake>>>>) {
    if let Ok(guard) = wake_slot.lock() {
        if let Some(wake) = guard.as_ref() {
            wake.wake("search");
        }
    }
}

/// Rank a slice of [`WalkEntry`] items against `query` for Phase 1 path hits.
pub fn rank_path_hits(entries: &[WalkEntry], query: &str, now: SystemTime) -> Vec<SearchHit> {
    let trimmed = query.trim();
    let mut hits: Vec<SearchHit> = entries
        .iter()
        .filter_map(|entry| {
            if trimmed.is_empty() && entry.depth > 1 {
                // When the query is empty, show the current directory's immediate
                // children rather than dumping every nested file before typing.
                return None;
            }
            let input = CandidateInput::new(&entry.file_name)
                .with_relative_path(&entry.rel_path)
                .with_mtime(entry.modified)
                .with_depth(entry.depth);
            let match_score = score_candidate(trimmed, &input, now)?;
            Some(SearchHit {
                path: entry.path.clone(),
                rel_path: entry.rel_path.clone(),
                file_name: entry.file_name.clone(),
                is_dir: entry.is_dir,
                size_bytes: entry.size_bytes,
                modified: entry.modified,
                score: match_score.total,
                name_ranges: match_score.primary_ranges,
                rel_path_ranges: match_score.path_ranges,
                kind: SearchHitKind::Path { content: None },
            })
        })
        .collect();

    if !trimmed.is_empty() {
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.rel_path.cmp(&b.rel_path))
        });
    }
    if hits.len() > MAX_SEARCH_HITS {
        hits.truncate(MAX_SEARCH_HITS);
    }
    hits
}

// ─── Unit Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_walker_prunes_default_junk_dirs_and_respects_hidden_policy() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(dir.path().join("node_modules/pkg/index.js"), "x").unwrap();
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        fs::write(dir.path().join("target/debug/bin"), "x").unwrap();
        fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        fs::write(dir.path().join(".git/HEAD"), "ref").unwrap();
        fs::create_dir_all(dir.path().join(".config")).unwrap();
        fs::write(dir.path().join(".config/settings.toml"), "a = 1").unwrap();
        fs::write(dir.path().join(".env"), "SECRET=1").unwrap();

        let skip_hidden = ScopedWalker::walk(&WalkConfig::new(dir.path()));
        let rels: Vec<&str> = skip_hidden
            .entries
            .iter()
            .map(|e| e.rel_path.as_str())
            .collect();
        assert!(rels.contains(&"src"));
        assert!(rels.contains(&"src/main.rs"));
        assert!(!rels.iter().any(|r| r.contains("node_modules")));
        assert!(!rels.iter().any(|r| r.contains("target")));
        assert!(!rels.iter().any(|r| r.contains(".git")));
        assert!(!rels.iter().any(|r| r.contains(".config")));
        assert!(!rels.contains(&".env"));

        // IncludeHidden admits `.config` and `.env` while still pruning `.git`, `node_modules`, `target`.
        let with_hidden = ScopedWalker::walk(
            &WalkConfig::new(dir.path()).with_hidden(HiddenPolicy::IncludeHidden),
        );
        let hidden_rels: Vec<&str> = with_hidden
            .entries
            .iter()
            .map(|e| e.rel_path.as_str())
            .collect();
        assert!(hidden_rels.contains(&".env"));
        assert!(hidden_rels.contains(&".config/settings.toml"));
        assert!(!hidden_rels.iter().any(|r| r.contains(".git")));
        assert!(!hidden_rels.iter().any(|r| r.contains("node_modules")));
    }

    #[cfg(unix)]
    #[test]
    fn scoped_walker_skips_symlinks_and_avoids_cycles() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(outside.path().join("escaped.txt"), "secret").unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/real.txt"), "ok").unwrap();

        symlink(outside.path(), dir.path().join("linked_out")).unwrap();
        symlink(dir.path(), dir.path().join("sub/cycle")).unwrap();

        let outcome = ScopedWalker::walk(&WalkConfig::new(dir.path()));
        let rels: Vec<&str> = outcome
            .entries
            .iter()
            .map(|e| e.rel_path.as_str())
            .collect();
        assert!(rels.contains(&"sub/real.txt"));
        assert!(!rels.iter().any(|r| r.contains("escaped.txt")));
        assert!(!rels.iter().any(|r| r.contains("cycle")));
    }

    #[test]
    fn scoped_walker_enforces_budgets_and_cancellation() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..10 {
            fs::write(dir.path().join(format!("file_{i:02}.txt")), "hello").unwrap();
        }

        let capped = ScopedWalker::walk(&WalkConfig::new(dir.path()).with_max_files(4));
        assert_eq!(capped.entries.len(), 4);
        assert!(capped.budget_hit);

        let gen = AtomicU64::new(2);
        let cancelled = ScopedWalker::walk(&WalkConfig::new(dir.path()).with_cancel(&gen, 1));
        assert!(cancelled.cancelled);
        assert!(cancelled.entries.is_empty());
    }

    #[test]
    fn score_candidate_ranks_basename_above_path_only_match() {
        let now = SystemTime::now();
        let basename_hit = CandidateInput::new("mod.rs").with_relative_path("src/search/mod.rs");
        let dir_only_hit =
            CandidateInput::new("helpers.rs").with_relative_path("src/mod.rs_bak/helpers.rs");

        let s_base = score_candidate("mod.rs", &basename_hit, now).expect("basename match");
        let s_dir = score_candidate("mod.rs", &dir_only_hit, now).expect("dir match");

        assert!(
            s_base.total > s_dir.total,
            "basename match ({}) must rank above directory-only match ({})",
            s_base.total,
            s_dir.total
        );
        assert_eq!(s_base.primary_ranges, vec![0..6]);
    }

    #[test]
    fn score_candidate_ranks_word_boundary_abbreviation_above_scattered_subsequence() {
        let now = SystemTime::now();
        let boundary = CandidateInput::new("command_palette.rs")
            .with_relative_path("src/overlays/command_palette.rs");
        let scattered = CandidateInput::new("custom_data_processor_all.rs")
            .with_relative_path("src/custom_data_processor_all.rs");

        let s_boundary = score_candidate("cmdpal", &boundary, now).expect("cmdpal matches");
        let s_scattered = score_candidate("cmdpal", &scattered, now).expect("scattered matches");

        assert!(
            s_boundary.total > s_scattered.total,
            "word-boundary match ({}) must outrank scattered subsequence ({})",
            s_boundary.total,
            s_scattered.total
        );
        assert!(!s_boundary.primary_ranges.is_empty());
    }

    #[test]
    fn score_candidate_supports_path_spanning_queries() {
        let now = SystemTime::now();
        let target = CandidateInput::new("command_palette.rs")
            .with_relative_path("src/overlays/command_palette.rs");
        let other =
            CandidateInput::new("command_palette.rs").with_relative_path("docs/command_palette.rs");

        let s_target = score_candidate("ov/cmd", &target, now).expect("ov/cmd matches overlays");
        assert!(score_candidate("ov/cmd", &other, now).is_none());
        assert!(!s_target.path_ranges.is_empty());
        assert!(!s_target.primary_ranges.is_empty());
    }

    #[test]
    fn score_candidate_uses_mtime_recency_and_depth_as_tie_breakers() {
        let now = SystemTime::now();
        let recent = CandidateInput::new("config.toml")
            .with_relative_path("a/config.toml")
            .with_mtime(Some(now - Duration::from_secs(30)))
            .with_depth(2);
        let stale = CandidateInput::new("config.toml")
            .with_relative_path("b/config.toml")
            .with_mtime(Some(now - Duration::from_secs(86_400 * 45)))
            .with_depth(2);

        let s_recent = score_candidate("config", &recent, now).unwrap();
        let s_stale = score_candidate("config", &stale, now).unwrap();
        assert!(
            s_recent.total > s_stale.total,
            "recently modified file ({}) must outrank stale file ({})",
            s_recent.total,
            s_stale.total
        );

        let shallow = CandidateInput::new("config.toml")
            .with_relative_path("config.toml")
            .with_depth(1);
        let deep = CandidateInput::new("config.toml")
            .with_relative_path("a/b/c/d/config.toml")
            .with_depth(5);
        assert!(
            score_candidate("config", &shallow, now).unwrap().total
                > score_candidate("config", &deep, now).unwrap().total
        );
    }

    #[test]
    fn content_scan_finds_line_numbers_and_skips_binary_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let text_file = dir.path().join("notes.md");
        fs::write(
            &text_file,
            "first line\nsecond line with Needle_Token inside\nthird Needle_Token line\n",
        )
        .unwrap();

        let binary_file = dir.path().join("blob.bin");
        fs::write(&binary_file, b"prefix\x00Needle_Token\x00suffix").unwrap();

        let hit = scan_file_for_query(&text_file, "needle_token").expect("text match");
        assert_eq!(hit.line_number, 2);
        assert!(hit.line_preview.contains("Needle_Token"));
        assert_eq!(hit.additional_matches, 1);
        let highlighted = &hit.line_preview[hit.line_ranges[0].clone()];
        assert_eq!(highlighted, "Needle_Token");

        assert!(
            scan_file_for_query(&binary_file, "needle_token").is_none(),
            "binary file containing NUL bytes must be skipped"
        );
    }

    #[test]
    fn search_session_walks_deep_paths_and_streams_content_hits_with_wake() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("nested/deep")).unwrap();
        fs::write(
            dir.path().join("nested/deep/architecture.md"),
            "Overview\nThe QuantumFluxCapacitor lives here.\n",
        )
        .unwrap();
        fs::write(dir.path().join("nested/deep/binary.dat"), b"\x00QuantumFluxCapacitor").unwrap();

        let wake: Arc<dyn UiWake> = Arc::new(crate::app::ui_mailbox::RecordingWake::new());
        let mut session = SearchSession::new();
        session.bind_wake(&wake);

        // Query by deep filename
        session.update_query(dir.path(), HiddenPolicy::SkipHidden, "arch", &[], false);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while session.is_scanning() && std::time::Instant::now() < deadline {
            session.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        session.poll();
        assert!(
            session
                .hits()
                .iter()
                .any(|h| h.rel_path == "nested/deep/architecture.md"),
            "Phase 1 must discover nested file by filename"
        );

        // Query by content inside nested file
        session.update_query(
            dir.path(),
            HiddenPolicy::SkipHidden,
            "QuantumFluxCapacitor",
            &[],
            false,
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while session.is_scanning() && std::time::Instant::now() < deadline {
            session.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        session.poll();
        let hit = session
            .hits()
            .iter()
            .find(|h| h.rel_path == "nested/deep/architecture.md")
            .expect("Phase 2 must surface content match");
        let snippet = hit.kind.content_snippet().expect("content snippet");
        assert_eq!(snippet.line_number, 2);
        assert!(snippet.line_preview.contains("QuantumFluxCapacitor"));
        assert!(
            !session.hits().iter().any(|h| h.file_name == "binary.dat"),
            "binary file must not appear in content hits"
        );
    }
}
