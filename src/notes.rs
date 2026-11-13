//! Notes storage: tier resolution, scanning, and the retired pre-tier check.
//!
//! Notes are user data addressed by [`crate::host::state_scope`]'s two-tier
//! model — a global tier and a per-context-root tier, both channel-neutral.
//! This module owns which tiers are in play for a given directory and what
//! Markdown lives in them; rendering belongs to the picker overlay and editing
//! to the text editor.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Maximum saved-note payload admitted by the one-note proposal contract.
pub const MAX_PROPOSAL_NOTE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OneNoteInputError {
    NotFound,
    PathOutsideTier,
    Oversized { bytes: usize },
    InvalidEncoding,
    Io(String),
    StaleSourceHash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneNoteProposalInput {
    pub canonical_path: PathBuf,
    pub source_hash: String,
    pub byte_size: usize,
    pub source_content: String,
    pub payload: String,
}

/// Read and stage exactly one saved note for a proposal worker.
///
/// The returned payload is deliberately self-contained: callers must pass it
/// to a worker over stdin and must not give the worker a note directory or a
/// filesystem tool. Attachments and neighboring notes are never read here.
pub fn prepare_one_note_proposal_input(
    path: &Path,
) -> Result<OneNoteProposalInput, OneNoteInputError> {
    let canonical_path = std::fs::canonicalize(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            OneNoteInputError::NotFound
        } else {
            OneNoteInputError::Io(error.to_string())
        }
    })?;
    if tier_root_for_note(&canonical_path).is_none() {
        return Err(OneNoteInputError::PathOutsideTier);
    }
    let bytes =
        std::fs::read(&canonical_path).map_err(|error| OneNoteInputError::Io(error.to_string()))?;
    if bytes.len() > MAX_PROPOSAL_NOTE_BYTES {
        return Err(OneNoteInputError::Oversized { bytes: bytes.len() });
    }
    let content = std::str::from_utf8(&bytes).map_err(|_| OneNoteInputError::InvalidEncoding)?;
    if content.contains('\0') {
        return Err(OneNoteInputError::InvalidEncoding);
    }
    let source_hash = format!("{:x}", Sha256::digest(&bytes));
    let payload = format!(
        "PLEXI_ONE_NOTE_PROPOSAL_V1\nsource_hash: {source_hash}\nbyte_size: {}\n---\n{content}",
        bytes.len()
    );
    Ok(OneNoteProposalInput {
        canonical_path,
        source_hash,
        byte_size: bytes.len(),
        source_content: content.to_string(),
        payload,
    })
}

/// Confirm that the saved source still has the hash captured at intake.
pub fn verify_one_note_source_unchanged(
    input: &OneNoteProposalInput,
) -> Result<(), OneNoteInputError> {
    let current = prepare_one_note_proposal_input(&input.canonical_path)?;
    if current.source_hash != input.source_hash {
        return Err(OneNoteInputError::StaleSourceHash);
    }
    Ok(())
}

// ─── Frontmatter ─────────────────────────────────────────────────────────────

/// Frontmatter parsed from a note's YAML-style header block.
///
/// Only `title` is parsed, because only `title` is read. Captures still *write*
/// `captured_at`, `source`, and `cwd` provenance lines and `parse_note` preserves
/// them verbatim on rewrite — provenance belongs in the file a user can read,
/// not in a struct field nothing consults.
#[derive(Default, Clone, Debug)]
pub(crate) struct NoteFrontmatter {
    pub title: Option<String>,
}

/// Keys the capture paths write into a note's `---` header block.
const NOTE_METADATA_KEYS: [&str; 4] = ["title", "captured_at", "source", "cwd"];

/// Split `content` into the Plexi capture metadata header and the body after it.
///
/// `None` unless the leading `---` block is one Plexi wrote: every line must be
/// a `key: value` pair for a known [`NOTE_METADATA_KEYS`] key, and the block must
/// be terminated. An absent, unterminated, empty, or unrecognized header is user
/// text, so the whole content is the body — a delimiter block a person typed
/// must never be silently dropped.
fn split_note_metadata(content: &str) -> Option<(&str, &str)> {
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    let header = &rest[..end];
    let is_metadata = !header.trim().is_empty()
        && header.lines().all(|line| {
            line.split_once(':')
                .is_some_and(|(key, _)| NOTE_METADATA_KEYS.contains(&key.trim()))
        });
    // `+ 5` skips the `\n---\n` terminator.
    is_metadata.then_some((header, &rest[end + 5..]))
}

/// The user-authored body of a note: the file text minus a Plexi capture
/// metadata envelope. This is what the picker previews and what copy/cut put on
/// the clipboard, so storage provenance never leaks into pasted text.
pub(crate) fn note_body(content: &str) -> &str {
    split_note_metadata(content).map_or(content, |(_, body)| body)
}

/// Parse a `---\nkey: value\n---\nbody` note into its frontmatter and body.
/// Returns an empty frontmatter and the full content as the body when the note
/// does not have a YAML front-matter block.
///
/// Deliberately looser than [`note_body`]: this also parses generic Markdown
/// frontmatter (`name:`, arbitrary keys) that a non-note file may carry, because
/// the text editor titles any Markdown file from its `title:` line.
pub(crate) fn parse_note(content: &str) -> (NoteFrontmatter, String) {
    let mut fm = NoteFrontmatter::default();

    let Some(rest) = content.strip_prefix("---\n") else {
        return (fm, content.to_string());
    };

    let Some(end) = rest.find("\n---\n") else {
        return (fm, content.to_string());
    };

    let header = &rest[..end];
    let body = rest[end + 5..].to_string(); // skip `\n---\n`

    for line in header.lines() {
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        if key.trim() != "title" {
            continue;
        }
        let val = val.trim().trim_matches('"');
        if !val.is_empty() {
            fm.title = Some(val.to_string());
        }
    }

    (fm, body)
}

/// Rewrite the `title:` line of a note's frontmatter, preserving every other
/// line verbatim. Creates a frontmatter block when the note has none.
/// An empty `title` removes the line.
pub(crate) fn set_title_in_content(content: &str, title: &str) -> String {
    let title = title.trim();
    let title_line = format!("title: \"{}\"", title.replace('"', "'"));

    if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            let header = &rest[..end];
            let tail = &rest[end..]; // includes `\n---\n` + body
            let mut lines: Vec<&str> = header
                .lines()
                .filter(|l| l.split_once(':').map(|(k, _)| k.trim()) != Some("title"))
                .collect();
            if !title.is_empty() {
                lines.insert(0, &title_line);
            }
            format!("---\n{}{tail}", lines.join("\n"))
        } else {
            // Malformed block — treat as body-only content.
            format!("---\n{title_line}\n---\n{content}")
        }
    } else if title.is_empty() {
        content.to_string()
    } else {
        format!("---\n{title_line}\n---\n{content}")
    }
}

// ─── Notes picker entries ────────────────────────────────────────────────────

/// One selectable row in the Cmd+O notes picker.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct NotePickerEntry {
    pub path: PathBuf,
    /// Display title: frontmatter `title` → first body line → file name.
    pub title: String,
    /// First non-empty body line shown as the secondary preview.
    pub preview: String,
    /// Lowercased haystack for fuzzy search: title + file name + body.
    pub search_text: String,
    /// Which tier this note came from, for the disambiguating chip. `None` for
    /// the primary tier, which needs no chip; otherwise a nested tier's path
    /// relative to the anchoring root, or `"global"`.
    pub tier_label: Option<String>,
}

impl NotePickerEntry {
    /// Build an entry from a note file. Returns `None` on read errors.
    pub(crate) fn load(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| log::warn!("notes_picker: failed to read {:?}: {e}", path))
            .ok()?;
        let (fm, body) = parse_note(&content);
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let first_line = body
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
        let title = fm
            .title
            .filter(|t| !t.is_empty())
            .or_else(|| (!first_line.is_empty()).then(|| first_line.clone()))
            .unwrap_or_else(|| file_name.clone());
        let search_text = format!("{title} {file_name} {body}").to_lowercase();
        Some(Self {
            path: path.to_path_buf(),
            title,
            preview: first_line,
            search_text,
            tier_label: None,
        })
    }

    /// Tag this entry with the tier it came from, for the picker chip.
    pub(crate) fn with_tier_label(mut self, label: Option<String>) -> Self {
        self.tier_label = label;
        self
    }
}

/// Case-insensitive fuzzy subsequence match delegated to [`crate::search::score_candidate`].
pub(crate) fn fuzzy_match(query: &str, haystack: &str) -> bool {
    let compact_query: String = query.chars().filter(|c| !c.is_whitespace()).collect();
    if compact_query.is_empty() {
        return true;
    }
    crate::search::score_candidate(
        &compact_query,
        &crate::search::CandidateInput::new(haystack),
        std::time::SystemTime::UNIX_EPOCH,
    )
    .is_some()
}

// ─── Notes tiers ─────────────────────────────────────────────────────────────
//
// Notes are user data, so they use the two-tier `StateScope` model from
// `crate::host::state_scope` — the same one plugin state uses, and the reason this
// module owns no path arithmetic of its own beyond locating tiers:
//
//   global   →  ~/.plexi/notes/
//   context  →  <context-root>/.plexi/notes/
//
// Both tiers are flat directories of Markdown, channel-neutral, and readable
// from outside the process. A context's notes live *inside* the directory they
// belong to, so they move with the project and stay findable after the context
// itself is gone.
//
// Rollup is a property of the filesystem, not of the live context list: a tier
// nested under another tier's root rolls up into it. That is what lets the CLI
// and the GUI picker agree by construction instead of by two implementations —
// see `notes_scopes_for_root`.

/// Tier addressing lives in `state_scope` with every other user-data kind;
/// re-exported here so notes callers have one import for "notes storage".
pub(crate) use crate::host::state_scope::{context_notes_dir, global_notes_dir};

/// Directory name of a notes tier, and of the `.plexi` dir it nests under.
/// Both are compared against real path components, never string-matched on a
/// whole path.
const TIER_DIR: &str = "notes";
const PLEXI_DIR: &str = ".plexi";

/// Subdirectory of a tier holding attachments. Never a note, and never
/// descended into when scanning.
const ASSETS_DIR: &str = "assets";

/// Canonical prune list re-exported from [`crate::search::DEFAULT_PRUNE_DIRS`].
pub(crate) const NOTES_WALK_PRUNE: &[&str] = crate::search::DEFAULT_PRUNE_DIRS;

/// How far below a tier root the rollup walk looks for nested tiers.
const WALK_MAX_DEPTH: usize = 8;

/// Hard cap on directories one rollup walk visits. A tree pathological enough
/// to exhaust this gets a truncated scope list and one loud warning. The picker
/// never waits on this walk: it publishes the in-memory index and refreshes
/// off the UI thread. Pruning `Library` (and the rest of [`NOTES_WALK_PRUNE`])
/// is what keeps a normal home directory from hitting the cap.
const WALK_DIR_BUDGET: usize = 20_000;

/// Directories visited by note-index walks on this thread. A cache hit must
/// leave the counter unchanged — that is the "open does not walk" proof.
#[derive(Clone, Copy, Default)]
struct WalkStats {
    visited: u64,
    budget_hit: bool,
}

thread_local! {
    static WALK_STATS: Cell<WalkStats> = const { Cell::new(WalkStats { visited: 0, budget_hit: false }) };
}

fn reset_walk_stats() {
    WALK_STATS.with(|stats| stats.set(WalkStats::default()));
}

fn record_dir_visit() {
    WALK_STATS.with(|stats| {
        let mut current = stats.get();
        current.visited = current.visited.saturating_add(1);
        stats.set(current);
    });
}

fn record_budget_hit() {
    WALK_STATS.with(|stats| {
        let mut current = stats.get();
        current.budget_hit = true;
        stats.set(current);
    });
}

fn walk_stats() -> WalkStats {
    WALK_STATS.with(|stats| stats.get())
}

/// Directories visited and whether the budget tripped during the most recent
/// index build on this thread. The background worker reads this after
/// [`build_notes_index`]; it is not a signal the UI thread can use.
pub(crate) fn notes_walk_result() -> (u64, bool) {
    let stats = walk_stats();
    (stats.visited, stats.budget_hit)
}

#[cfg(test)]
pub(crate) fn notes_walk_dirs() -> u64 {
    walk_stats().visited
}

#[cfg(test)]
pub(crate) fn notes_walk_budget_hit() -> bool {
    walk_stats().budget_hit
}

#[cfg(test)]
pub(crate) fn reset_notes_walk_dirs_for_test() {
    reset_walk_stats();
}

fn name_is_pruned(name: &str) -> bool {
    NOTES_WALK_PRUNE.contains(&name)
}

fn path_name_is_pruned(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(name_is_pruned)
}

/// `true` when `dir` is a notes tier root: either the global tier, or a
/// `notes` directory whose parent is a `.plexi` directory.
///
/// The `.plexi` parent check is why a channel-scoped profile dir is *not* a
/// tier — `<root>/.plexi-alpha/notes` is config-adjacent, and user data must
/// never fork per channel.
pub(crate) fn is_notes_tier_root(dir: &Path) -> bool {
    if comparable_scope_path(dir) == comparable_scope_path(&global_notes_dir()) {
        return true;
    }
    dir.file_name().is_some_and(|n| n == TIER_DIR)
        && dir
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == PLEXI_DIR)
}

/// The tier `path` belongs to, or `None` when it is not a note at all.
///
/// This is the one "is this a note" predicate. A file under a tier's
/// `assets/` is an attachment, not a note, so it resolves to `None`.
pub(crate) fn tier_root_for_note(path: &Path) -> Option<PathBuf> {
    let mut relative: Vec<&std::ffi::OsStr> = Vec::new();
    for ancestor in path.ancestors().skip(1) {
        if is_notes_tier_root(ancestor) {
            // `assets/` is the tier's attachment store, never a note dir.
            if relative.last().is_some_and(|first| *first == ASSETS_DIR) {
                return None;
            }
            return Some(ancestor.to_path_buf());
        }
        relative.push(ancestor.file_name()?);
    }
    None
}

/// The context root a path is anchored to: its nearest ancestor containing a
/// `.plexi` directory. `None` when nothing above it is anchored, in which case
/// callers fall back to the global tier.
///
/// This is how the CLI resolves its tier. It reads the filesystem, never
/// `PLEXI_CONTEXT_ROOT` — a parent process cannot mutate a running child's
/// environment, so a long-lived pane's copy of that variable is permanently
/// stale after `plexi context set-root`, while cwd is always current.
pub(crate) fn anchored_root_for(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(PLEXI_DIR).is_dir())
        .map(Path::to_path_buf)
}

/// One tier to scan, with the chip shown on its rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NotesScope {
    pub dir: PathBuf,
    /// Where this tier sits relative to the scope being listed: `None` for the
    /// primary tier (no chip needed), the path relative to the anchoring root
    /// for a nested tier, and `"global"` for the global tier.
    pub label: Option<String>,
}

/// Every tier visible from `root`: its own tier first, then each nested tier
/// found beneath it, then the global tier last.
///
/// Pass `None` for `root` when nothing is anchored — the result is the global
/// tier alone. The global tier is always included and always deduped against
/// the others, because a context rooted at the home directory (which is what
/// `new_context_empty` produces) resolves its own tier to exactly the global
/// tier and must not be listed twice.
///
/// This is the CLI entry point. The picker builds the same scope list through
/// [`build_notes_index`], adding context roots and previously cached tiers the
/// CLI does not know about.
pub(crate) fn notes_scopes_for_root(root: Option<&Path>) -> Vec<NotesScope> {
    reset_walk_stats();
    discover_notes_scopes(&NotesDiscovery {
        active_root: root.map(Path::to_path_buf),
        known_roots: Vec::new(),
        global_notes: global_notes_dir(),
        home: dirs::home_dir(),
        cached_tiers: Vec::new(),
    })
}

/// Bounded breadth-first walk for `*/.plexi/notes` directories below `root`.
///
/// Directories only, never following symlinks (a symlinked subtree could point
/// anywhere, including back above the root). Deterministically sorted so two
/// runs list tiers in the same order. Hidden directories other than `.plexi`,
/// and every name in [`NOTES_WALK_PRUNE`], are not entered — that is what keeps
/// `~/Library` from exhausting [`WALK_DIR_BUDGET`] on a normal home directory.
fn nested_tiers_below(root: &Path) -> Vec<PathBuf> {
    if path_name_is_pruned(root) {
        return Vec::new();
    }
    let mut found = Vec::new();
    let root_notes_cmp = comparable_scope_path(&context_notes_dir(root));
    let walk_cfg = crate::search::WalkConfig::new(root)
        .with_hidden(crate::search::HiddenPolicy::AllowAnchors(&[PLEXI_DIR]))
        .with_max_depth(WALK_MAX_DEPTH)
        .with_max_dirs(WALK_DIR_BUDGET);

    let outcome =
        crate::search::ScopedWalker::walk_dirs_filtered(&walk_cfg, |path, name, _depth| {
            if name == PLEXI_DIR {
                let tier = path.join(TIER_DIR);
                if tier.is_dir() && comparable_scope_path(&tier) != root_notes_cmp {
                    found.push(tier);
                }
                crate::search::DirVisit::SkipChildren
            } else {
                crate::search::DirVisit::Descend
            }
        });

    for _ in 0..outcome.dirs_visited {
        record_dir_visit();
    }
    if outcome.budget_hit {
        record_dir_visit();
        record_budget_hit();
        log::warn!(
            "notes: rollup walk hit its {WALK_DIR_BUDGET}-directory budget under {root:?} — \
             some nested tiers were not scanned"
        );
    }
    found.sort();
    found
}

/// Inputs for one notes-index build. Paths are resolved on the calling thread
/// (test overrides for the global tier are thread-local) and then moved to the
/// background worker, which must not call [`global_notes_dir`] itself.
pub(crate) struct NotesDiscovery {
    pub active_root: Option<PathBuf>,
    /// Context roots, the adopted workspace root, and any other directory
    /// Plexi already knows. Checked directly — a tier inside `~/Library` is
    /// still found when its project is one of these, without walking Library.
    pub known_roots: Vec<PathBuf>,
    pub global_notes: PathBuf,
    pub home: Option<PathBuf>,
    /// Tier directories from the previous snapshot. Re-stat, no tree walk.
    pub cached_tiers: Vec<PathBuf>,
}

/// Scope list plus the picker rows built from it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct NotesIndexSnapshot {
    pub scopes: Vec<NotesScope>,
    pub entries: Vec<NotePickerEntry>,
}

/// What the background refresh hands back to the UI thread.
pub(crate) struct NotesIndexBuild {
    pub anchor: PathBuf,
    pub snapshot: NotesIndexSnapshot,
    pub generation: u64,
    pub walk_dirs: u64,
    pub budget_hit: bool,
    pub elapsed: std::time::Duration,
}

/// In-memory picker index, keyed by the anchor the snapshot was built for.
///
/// A hit clones the stored rows and does not touch the filesystem. Invalidation
/// bumps `generation` but keeps the rows so the picker can paint them while a
/// refresh is in flight.
#[derive(Clone, Debug, Default)]
pub(crate) struct NotesIndexCache {
    by_anchor: std::collections::HashMap<PathBuf, NotesIndexSnapshot>,
    generation: u64,
}

impl NotesIndexCache {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn anchor_count(&self) -> usize {
        self.by_anchor.len()
    }

    /// Stored rows for `anchor`, if this process has built or loaded them.
    /// Does not stat, scan, or walk.
    pub(crate) fn cached_entries(&self, anchor: &Path) -> Option<Vec<NotePickerEntry>> {
        self.by_anchor
            .get(&anchor_key(anchor))
            .map(|snapshot| snapshot.entries.clone())
    }

    pub(crate) fn cached_tier_count(&self, anchor: &Path) -> usize {
        self.by_anchor
            .get(&anchor_key(anchor))
            .map(|snapshot| snapshot.scopes.len())
            .unwrap_or(0)
    }

    pub(crate) fn cached_tier_dirs(&self, anchor: &Path) -> Vec<PathBuf> {
        self.by_anchor
            .get(&anchor_key(anchor))
            .map(|snapshot| {
                snapshot
                    .scopes
                    .iter()
                    .map(|scope| scope.dir.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Drop the authority of every stored snapshot. The rows stay so the next
    /// open can paint them; a refresh built at this generation replaces them.
    pub(crate) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        log::info!("notes_index: invalidated generation={}", self.generation);
    }

    /// Store `snapshot` when it was built at the current generation.
    /// Returns false when a later invalidation has already superseded it.
    pub(crate) fn store(
        &mut self,
        anchor: &Path,
        snapshot: NotesIndexSnapshot,
        built_generation: u64,
    ) -> bool {
        if built_generation != self.generation {
            return false;
        }
        self.by_anchor.insert(anchor_key(anchor), snapshot);
        true
    }

    pub(crate) fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let persisted = PersistedNotesIndex {
            version: NOTES_INDEX_VERSION,
            anchors: {
                let mut anchors: Vec<PersistedAnchor> = self
                    .by_anchor
                    .iter()
                    .map(|(anchor, snapshot)| PersistedAnchor {
                        anchor: anchor.clone(),
                        snapshot: snapshot.clone(),
                    })
                    .collect();
                anchors.sort_by(|a, b| a.anchor.cmp(&b.anchor));
                anchors
            },
        };
        let body = serde_json::to_vec_pretty(&persisted)
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub(crate) fn load(path: &Path) -> Option<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => {
                log::warn!("notes_index: failed to read {path:?}: {err}");
                return None;
            }
        };
        match serde_json::from_slice::<PersistedNotesIndex>(&bytes) {
            Ok(parsed) if parsed.version == NOTES_INDEX_VERSION => {
                let mut cache = Self::default();
                for anchor in parsed.anchors {
                    cache.by_anchor.insert(anchor.anchor, anchor.snapshot);
                }
                log::info!(
                    "notes_index: loaded persisted cache anchors={}",
                    cache.anchor_count()
                );
                Some(cache)
            }
            Ok(parsed) => {
                log::warn!(
                    "notes_index: ignoring {:?} — version {} is not {NOTES_INDEX_VERSION}",
                    path,
                    parsed.version
                );
                None
            }
            Err(err) => {
                log::warn!("notes_index: failed to parse {path:?}: {err}");
                None
            }
        }
    }
}

const NOTES_INDEX_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct PersistedNotesIndex {
    version: u32,
    anchors: Vec<PersistedAnchor>,
}

#[derive(Serialize, Deserialize)]
struct PersistedAnchor {
    anchor: PathBuf,
    snapshot: NotesIndexSnapshot,
}

/// Profile file holding the last notes-picker index. `None` in a test that
/// has not installed an isolated profile — those tests must not touch a real
/// channel directory.
pub(crate) fn notes_index_cache_path() -> Option<PathBuf> {
    profile_dir_for_notes_index().map(|dir| dir.join("notes-picker-index.json"))
}

fn profile_dir_for_notes_index() -> Option<PathBuf> {
    #[cfg(test)]
    {
        crate::config::test_profile_dir_override()
    }
    #[cfg(not(test))]
    {
        Some(crate::config::config_dir())
    }
}

/// Build the picker corpus. Walks the filesystem — call it off the UI thread.
pub(crate) fn build_notes_index(query: &NotesDiscovery) -> NotesIndexSnapshot {
    let started = Instant::now();
    reset_walk_stats();
    let scopes = discover_notes_scopes(query);
    let mut entries = Vec::new();
    for scope in &scopes {
        entries.extend(
            scan_tier(&scope.dir)
                .iter()
                .filter_map(|path| NotePickerEntry::load(path))
                .map(|entry| entry.with_tier_label(scope.label.clone())),
        );
    }
    let stats = walk_stats();
    log::info!(
        "notes_index: built notes={} tiers={} walk_dirs={} budget_hit={} build_ms={:.1}",
        entries.len(),
        scopes.len(),
        stats.visited,
        stats.budget_hit,
        started.elapsed().as_secs_f64() * 1000.0
    );
    NotesIndexSnapshot { scopes, entries }
}

fn discover_notes_scopes(query: &NotesDiscovery) -> Vec<NotesScope> {
    let mut scopes: Vec<NotesScope> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |dir: PathBuf, label: Option<String>| {
        if seen.insert(comparable_scope_path(&dir)) {
            scopes.push(NotesScope { dir, label });
        }
    };

    if let Some(root) = query.active_root.as_deref() {
        push(context_notes_dir(root), None);

        // Known roots first, git repos before the rest, so a budget-truncated
        // walk cannot hide a project Plexi already has open.
        let mut known = prefer_known_roots(&query.known_roots);
        known.retain(|candidate| root_visible(root, candidate, query.home.as_deref()));
        for extra in &known {
            if paths_equal(extra, root) {
                continue;
            }
            let tier = context_notes_dir(extra);
            if tier.is_dir() {
                let label = label_for_tier(root, &tier);
                push(tier, label);
            }
        }

        for nested in nested_tiers_below(root) {
            let label = label_for_tier(root, &nested);
            push(nested, label);
        }

        for extra in &known {
            if paths_equal(extra, root) || lexical_is_descendant(root, extra) {
                continue;
            }
            if path_name_is_pruned(extra) {
                continue;
            }
            for nested in nested_tiers_below(extra) {
                let label = label_for_tier(root, &nested);
                push(nested, label);
            }
        }

        for tier in &query.cached_tiers {
            if !tier.is_dir() || !is_tier_dir(tier, &query.global_notes) {
                continue;
            }
            // The trailing push is what labels the global tier. Inserting it
            // here would win the dedupe and drop the "global" chip.
            if paths_equal(tier, &query.global_notes) {
                continue;
            }
            let project = tier_project_root(tier);
            let visible = project
                .is_some_and(|project| root_visible(root, project, query.home.as_deref()))
                || lexical_is_descendant(root, tier);
            if visible {
                let label = label_for_tier(root, tier);
                push(tier.clone(), label);
            }
        }
    }

    push(query.global_notes.clone(), Some("global".to_string()));
    scopes
}

/// Git repos Plexi already knows are checked before other roots. A root with a
/// `.git` directory or a gitlink file is a repository; everything else still
/// counts (a notes tier does not require git) but sorts after.
fn prefer_known_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    let mut rest = Vec::new();
    for root in roots {
        let git = root.join(".git");
        if git.is_dir() || git.is_file() {
            repos.push(root.clone());
        } else {
            rest.push(root.clone());
        }
    }
    repos.sort();
    rest.sort();
    repos.extend(rest);
    repos
}

fn root_visible(active: &Path, candidate: &Path, home: Option<&Path>) -> bool {
    if paths_equal(active, candidate) || lexical_is_descendant(active, candidate) {
        return true;
    }
    home.is_some_and(|home| paths_equal(active, home))
}

fn label_for_tier(anchor: &Path, tier: &Path) -> Option<String> {
    tier.strip_prefix(anchor)
        .ok()
        .and_then(|rel| rel.parent().and_then(Path::parent))
        .map(|rel| rel.to_string_lossy().into_owned())
        .filter(|rel| !rel.is_empty())
}

fn tier_project_root(tier: &Path) -> Option<&Path> {
    tier.parent()?.parent()
}

fn is_tier_dir(dir: &Path, global_notes: &Path) -> bool {
    if comparable_scope_path(dir) == comparable_scope_path(global_notes) {
        return true;
    }
    dir.file_name().is_some_and(|name| name == TIER_DIR)
        && dir
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == PLEXI_DIR)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    anchor_key(left) == anchor_key(right)
}

fn lexical_is_descendant(root: &Path, candidate: &Path) -> bool {
    let root = anchor_key(root);
    let candidate = anchor_key(candidate);
    candidate.starts_with(&root) && candidate != root
}

fn anchor_key(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Every note in one tier, newest first. Recursive within the tier (a tier may
/// hold subdirectories — `[[project/idea]]` wiki links create them) but never
/// into `assets/`.
///
/// Symlinks are treated asymmetrically on purpose. A symlinked **file** is a
/// note: pointing a tier at a Markdown file kept elsewhere (a repo's `TODO.md`,
/// say) is a real affordance, so `metadata()` — which follows links — decides
/// file-ness. A symlinked **directory** is never descended: `file_type()`, which
/// does not follow links, gates recursion, so a link cycle cannot spin the walk
/// and a link out of the tier cannot smuggle in unrelated files.
pub(crate) fn scan_tier(dir: &Path) -> Vec<PathBuf> {
    let mut with_mtime: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let mut queue = std::collections::VecDeque::from([dir.to_path_buf()]);
    while let Some(current) = queue.pop_front() {
        record_dir_visit();
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            // Real directory (not a symlink to one) → recurse.
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                if entry.file_name() != ASSETS_DIR {
                    queue.push_back(path);
                }
                continue;
            }
            if path.extension().is_none_or(|x| x != "md") {
                continue;
            }
            // Follows symlinks: a dangling link yields `Err` and is skipped.
            let Ok(meta) = entry.path().metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            if let Ok(mtime) = meta.modified() {
                with_mtime.push((mtime, path));
            }
        }
    }
    with_mtime.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    with_mtime.into_iter().map(|(_, path)| path).collect()
}

/// Resolve symlinks when both roots exist; otherwise remove lexical `.` / `..`
/// components. This keeps descendant matching component-safe for live contexts
/// whose configured roots use different but equivalent path spellings.
fn comparable_scope_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

// ─── Retired pre-tier store ──────────────────────────────────────────────────
//
// Before the tier model (0.3.0), kept notes lived in the channel profile at
// `config_dir()/notes/`: captures in `inbox/`, each context's notes in
// `ctx-<slug>-<hash8>/` (or, earlier still, a folder named after the context
// root), loose notes at the top. The migration that moved them onto tiers is
// deleted. A pre-tier folder that still holds notes is reported by path with
// the manual move, on every launch, and is never read for content, moved,
// copied, or removed.

/// Folders under the pre-tier base that never held a context's notes.
const PRE_TIER_RESERVED_DIRS: [&str; 3] = [ASSETS_DIR, "trash", "templates"];

/// The pre-tier staging folder. The migration always moved its notes.
const PRE_TIER_INBOX_DIR: &str = "inbox";

/// True for the pre-tier context folder name `ctx-<slug>-<hash8>`, the only
/// shape that cannot be ordinary tier content.
fn is_hashed_pre_tier_dir_name(name: &str) -> bool {
    name.strip_prefix("ctx-")
        .and_then(|rest| rest.rsplit_once('-'))
        .is_some_and(|(slug, hash)| {
            !slug.is_empty()
                && hash.len() == 8
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

/// True when `dir` directly holds at least one `.md` file.
fn holds_a_note(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let path = entry.path();
            path.is_file() && path.extension().is_some_and(|x| x == "md")
        })
    })
}

/// The launch error for pre-tier notes the deleted migration would have
/// moved, or `None`. Names the folder, what it holds, and the manual move.
/// Reads only.
///
/// The migration always moved `inbox/` and loose `.md` files, so those are
/// always reported. It deliberately left context folders it could not match
/// (orphan `ctx-` folders, ambiguous slug folders) in place; those get one
/// info line instead, unless `workspace_retired` says no build carrying the
/// workspace converter (0.4.1 or later) opened this profile. Every profile
/// that can hold a genuinely unmigrated folder (opened by no 0.3 or later
/// build) is in that case.
///
/// On the stable channel the pre-tier base and the global tier are the same
/// directory, so only hashed `ctx-` folders count there: everything else in
/// it is already global-tier content.
pub(crate) fn pre_tier_notes_error(workspace_retired: bool) -> Option<String> {
    let base = crate::config::config_dir().join(TIER_DIR);
    let global = global_notes_dir();
    let base_is_global_tier = comparable_scope_path(&base) == comparable_scope_path(&global);
    let entries = match std::fs::read_dir(&base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            return Some(format!(
                "notes: cannot read {} to check for pre-tier notes ({error}). Fix: make the \
                 folder readable, or move it aside if it holds no notes.",
                base.display()
            ));
        }
    };
    // Always moved by the migration, so never left on purpose.
    let mut unmigrated: Vec<String> = Vec::new();
    // Context folders the migration may have left on purpose.
    let mut folders: Vec<String> = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            let pre_tier = if base_is_global_tier {
                is_hashed_pre_tier_dir_name(&name)
            } else {
                !PRE_TIER_RESERVED_DIRS.contains(&name.as_str())
            };
            if !pre_tier || !holds_a_note(&path) {
                continue;
            }
            if !base_is_global_tier && name == PRE_TIER_INBOX_DIR {
                unmigrated.push(format!("{name}/"));
            } else {
                folders.push(format!("{name}/"));
            }
        } else if !base_is_global_tier
            && path.is_file()
            && path.extension().is_some_and(|x| x == "md")
        {
            unmigrated.push(name);
        }
    }
    folders.sort();
    if workspace_retired {
        unmigrated.append(&mut folders);
    } else if !folders.is_empty() {
        log::info!(
            "notes: {} holds pre-tier folders the old migration left in place because no \
             open context matched them: {}. Left unchanged; move their notes into a \
             project's .plexi/notes/ by hand if wanted.",
            base.display(),
            folders.join(", ")
        );
    }
    if unmigrated.is_empty() {
        return None;
    }
    unmigrated.sort();
    Some(format!(
        "notes: {} holds notes in the pre-tier layout, which Plexi no longer migrates: {}. \
         They were left unchanged. Fix: move the notes in inbox/ and any loose .md files \
         into {}, and the notes in each other folder into its project's \
         <project>/.plexi/notes/ (the folder is named after the project directory, as \
         ctx-<name>-<hash> in newer profiles). Then delete the emptied folders.",
        base.display(),
        unmigrated.join(", "),
        global.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_note_with_frontmatter() {
        let content = "---\ncaptured_at: 2026-01-01\ncwd: /tmp\n---\nhello world\n";
        let (fm, body) = parse_note(content);
        // Provenance keys are preserved in the file but not parsed into fields.
        assert!(fm.title.is_none());
        assert_eq!(body, "hello world\n");
    }

    #[test]
    fn parse_note_without_frontmatter() {
        let content = "just a plain note\n";
        let (fm, body) = parse_note(content);
        assert!(fm.title.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn parse_note_reads_title() {
        let content = "---\ntitle: \"My Note\"\ncaptured_at: 2026-01-01\n---\nbody\n";
        let (fm, body) = parse_note(content);
        assert_eq!(fm.title.as_deref(), Some("My Note"));
        assert_eq!(body, "body\n");
    }

    /// The clipboard projection: a Plexi capture envelope is dropped, the body
    /// after it is returned verbatim (Markdown, links, fenced code, newlines).
    #[test]
    fn note_body_strips_the_capture_envelope_and_keeps_the_body() {
        let content = "---\ntitle: \"\"\ncaptured_at: \"2026-10-26T22:00:00Z\"\nsource: \"scratchpad\"\ncwd: \"/tmp\"\n---\n\n# Heading\n\nsee [x](https://e.com)\n\n```rs\nlet a = 1;\n```\n";
        let body = note_body(content);
        assert!(!body.contains("captured_at"), "{body:?}");
        assert!(!body.contains("source:"), "{body:?}");
        assert_eq!(
            body,
            "\n# Heading\n\nsee [x](https://e.com)\n\n```rs\nlet a = 1;\n```\n"
        );
        // The parser's body and the clipboard projection agree.
        assert_eq!(parse_note(content).1, body);
    }

    /// A plain note has no envelope; every byte is body.
    #[test]
    fn note_body_returns_plain_notes_unchanged() {
        let content = "just a plain note\nsecond line\n";
        assert_eq!(note_body(content), content);
    }

    /// A `---` block a person typed is not storage metadata. Neither an
    /// unrecognized key nor a non-`key: value` line may be stripped, or the
    /// clipboard would silently lose the top of the user's note.
    #[test]
    fn note_body_preserves_user_authored_delimiter_blocks() {
        let frontmatter_like = "---\ntags: [a, b]\n---\nuser body\n";
        assert_eq!(note_body(frontmatter_like), frontmatter_like);

        let prose_header = "---\nchapter one\n---\nuser body\n";
        assert_eq!(note_body(prose_header), prose_header);

        // An empty block is not metadata either.
        let empty = "---\n\n---\nuser body\n";
        assert_eq!(note_body(empty), empty);
    }

    /// Unterminated or partially recognized headers are malformed metadata, not
    /// a reason to drop text: the whole file is preserved.
    #[test]
    fn note_body_preserves_malformed_or_unterminated_headers() {
        let unterminated = "---\ntitle: \"x\"\ncaptured_at: 2026-01-01\nbody without a close\n";
        assert_eq!(note_body(unterminated), unterminated);

        // One unrecognized line makes the whole block user text.
        let mixed = "---\ntitle: \"x\"\nnot a pair\ncaptured_at: 2026-01-01\n---\nbody\n";
        assert_eq!(note_body(mixed), mixed);
    }

    /// A rewrite keeps provenance lines the parser ignores — they are the record
    /// of where a note came from, so losing them on a title edit is data loss.
    #[test]
    fn set_title_preserves_unparsed_provenance_lines() {
        let content =
            "---\ntitle: \"old\"\ncaptured_at: \"2026-01-01\"\nsource: \"cli\"\n---\nbody\n";
        let rewritten = set_title_in_content(content, "new");
        assert!(rewritten.contains("captured_at: \"2026-01-01\""));
        assert!(rewritten.contains("source: \"cli\""));
        assert!(rewritten.contains("title: \"new\""));
        assert!(!rewritten.contains("\"old\""));
    }

    #[test]
    fn fuzzy_match_subsequence() {
        assert!(fuzzy_match("abc", "a b c note"));
        assert!(fuzzy_match("nt", "note"));
        assert!(!fuzzy_match("zzz", "note"));
    }

    #[test]
    fn picker_entry_title_fallbacks() {
        let dir = std::env::temp_dir().join(format!("plexi-notes-entry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let titled = dir.join("titled.md");
        std::fs::write(&titled, "---\ntitle: \"Named\"\n---\nbody line\n").expect("seed");
        let entry = NotePickerEntry::load(&titled).expect("load");
        assert_eq!(entry.title, "Named");
        assert_eq!(entry.preview, "body line");
        assert!(entry.tier_label.is_none());

        let untitled = dir.join("untitled.md");
        std::fs::write(&untitled, "---\nsource: \"quick-note\"\n---\nfirst line\n").expect("seed");
        assert_eq!(
            NotePickerEntry::load(&untitled).expect("load").title,
            "first line"
        );

        let empty = dir.join("note-20260611.md");
        std::fs::write(&empty, "---\nsource: \"scratchpad\"\n---\n\n").expect("seed");
        assert_eq!(
            NotePickerEntry::load(&empty).expect("load").title,
            "note-20260611.md"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── Tier resolution ─────────────────────────────────────────────────────

    /// An isolated channel profile (the pre-tier store's home) plus an isolated
    /// channel-neutral shared dir (the global tier's home), so no test ever
    /// touches a real profile or a real `~/.plexi`.
    struct TierEnv {
        _profile: crate::config::TestProfileDirGuard,
        _shared: crate::config::TestSharedDirGuard,
        /// The pre-tier `config_dir()/notes` base.
        legacy: PathBuf,
        /// The channel-neutral global tier.
        global: PathBuf,
        root: PathBuf,
        /// The fixture's enclosing dir, standing in for the home directory.
        base: PathBuf,
    }

    fn tier_env(tag: &str) -> TierEnv {
        let base = std::env::temp_dir().join(format!(
            "plexi-notes-tier-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let profile = base.join("profile");
        // Named `.plexi` like the real shared dir, so `context_notes_dir(base)`
        // resolves to exactly `global_notes_dir()` — the home-rooted-context case.
        let shared = base.join(".plexi");
        let root = base.join("project");
        let legacy = profile.join("notes");
        std::fs::create_dir_all(&legacy).expect("legacy base");
        std::fs::create_dir_all(&shared).expect("shared dir");
        std::fs::create_dir_all(&root).expect("project root");
        let _profile = crate::config::set_test_profile_dir(profile);
        let _shared = crate::config::set_test_shared_dir(shared.clone());
        TierEnv {
            _profile,
            _shared,
            legacy,
            global: shared.join("notes"),
            root,
            base,
        }
    }

    fn seed(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("seed note");
    }

    /// A tier is a `notes` dir under a `.plexi` dir, or the global tier itself.
    /// A channel-suffixed profile dir is deliberately NOT a tier — user data must
    /// never fork per channel.
    #[test]
    fn tier_root_predicate_accepts_only_real_tiers() {
        let env = tier_env("predicate");
        assert!(is_notes_tier_root(&env.global), "global tier");
        assert!(
            is_notes_tier_root(&env.root.join(".plexi").join("notes")),
            "context tier"
        );
        assert!(
            !is_notes_tier_root(&env.root.join(".plexi-alpha").join("notes")),
            "a channel-suffixed dir must never be a notes tier"
        );
        assert!(
            !is_notes_tier_root(&env.root.join(".plexi").join("app_states")),
            "a sibling user-data kind is not a notes tier"
        );
        assert!(
            !is_notes_tier_root(&env.root.join("notes")),
            "a bare notes dir"
        );
    }

    /// `tier_root_for_note` is the one "is this a note" predicate. Attachments
    /// live in the tier but are not notes.
    #[test]
    fn tier_root_for_note_finds_tiers_and_excludes_assets() {
        let env = tier_env("for-note");
        let tier = context_notes_dir(&env.root);
        assert_eq!(
            tier_root_for_note(&tier.join("captured.md")).as_deref(),
            Some(tier.as_path())
        );
        // A wiki link like `[[project/idea]]` creates a subdirectory.
        assert_eq!(
            tier_root_for_note(&tier.join("project").join("idea.md")).as_deref(),
            Some(tier.as_path())
        );
        assert_eq!(
            tier_root_for_note(&env.global.join("captured.md")).as_deref(),
            Some(env.global.as_path())
        );
        assert!(
            tier_root_for_note(&tier.join("assets").join("shot.png")).is_none(),
            "an attachment is not a note"
        );
        assert!(
            tier_root_for_note(&env.root.join("README.md")).is_none(),
            "arbitrary Markdown is not a note"
        );
    }

    #[test]
    fn one_note_review_intake_boundary_and_hash_qualification() {
        let env = tier_env("one-note-intake");
        let ctx_tier = context_notes_dir(&env.root);
        let note_path = ctx_tier.join("scratch.md");
        let initial_bytes = b"---\ntitle: Scratch\n---\nInitial review target.\n";
        seed(&note_path, std::str::from_utf8(initial_bytes).unwrap());

        let intake = prepare_one_note_proposal_input(&note_path).unwrap();
        assert_eq!(
            intake.canonical_path,
            std::fs::canonicalize(&note_path).unwrap()
        );
        assert_eq!(intake.byte_size, initial_bytes.len());
        assert_eq!(intake.source_hash.len(), 64);
        assert!(intake
            .payload
            .contains(std::str::from_utf8(initial_bytes).unwrap()));
        assert!(intake.payload.contains(&intake.source_hash));

        // Attachments and outside files are rejected before their bytes enter a payload.
        let asset_path = ctx_tier.join("assets").join("diagram.png");
        std::fs::create_dir_all(asset_path.parent().unwrap()).unwrap();
        std::fs::write(&asset_path, b"fake png").unwrap();
        assert_eq!(
            prepare_one_note_proposal_input(&asset_path),
            Err(OneNoteInputError::PathOutsideTier)
        );

        let outside_path = env.root.join("config.toml");
        std::fs::write(&outside_path, b"key = 'val'").unwrap();
        assert_eq!(
            prepare_one_note_proposal_input(&outside_path),
            Err(OneNoteInputError::PathOutsideTier)
        );

        std::fs::write(&note_path, b"changed after intake\n").unwrap();
        assert_eq!(
            verify_one_note_source_unchanged(&intake),
            Err(OneNoteInputError::StaleSourceHash)
        );

        std::fs::write(&note_path, b"a\0b\n").unwrap();
        assert_eq!(
            prepare_one_note_proposal_input(&note_path),
            Err(OneNoteInputError::InvalidEncoding)
        );
    }

    /// The CLI resolves its tier this way. Nearest anchored ancestor wins, so a
    /// nested project inside a bigger one gets its own tier.
    #[test]
    fn anchored_root_prefers_the_nearest_anchor() {
        let env = tier_env("anchor");
        let nested = env.root.join("packages").join("inner");
        std::fs::create_dir_all(env.root.join(".plexi")).expect("outer anchor");
        std::fs::create_dir_all(nested.join(".plexi")).expect("inner anchor");
        std::fs::create_dir_all(nested.join("src")).expect("cwd");

        assert_eq!(
            anchored_root_for(&nested.join("src")).as_deref(),
            Some(nested.as_path()),
            "the nearest anchor wins"
        );
        assert_eq!(
            anchored_root_for(&env.root).as_deref(),
            Some(env.root.as_path())
        );
        // The unanchored → `None` case is deliberately NOT asserted here: the walk
        // runs to `/`, and a shared `/tmp/.plexi` or `$TMPDIR/.plexi` (both common
        // on a developer machine, and neither ours to control) would anchor any
        // temp path. Its user-visible consequence — falling back to the global
        // tier — is covered hermetically by `unanchored_scopes_are_global_only`,
        // which passes `None` directly instead of asking the filesystem.
    }

    /// Rollup: own tier, then each nested tier labelled by its relative path,
    /// then the global tier. Same function the picker and the CLI both call.
    #[test]
    fn scopes_roll_up_nested_tiers_then_global() {
        let env = tier_env("rollup");
        let nested = env.root.join("crates").join("engine");
        seed(&context_notes_dir(&env.root).join("own.md"), "own");
        seed(&context_notes_dir(&nested).join("nested.md"), "nested");
        seed(&env.global.join("g.md"), "global");

        let scopes = notes_scopes_for_root(Some(&env.root));
        let dirs: Vec<&Path> = scopes.iter().map(|s| s.dir.as_path()).collect();
        assert_eq!(
            dirs,
            vec![
                context_notes_dir(&env.root).as_path(),
                context_notes_dir(&nested).as_path(),
                env.global.as_path(),
            ],
            "own tier first, nested next, global last"
        );
        assert_eq!(scopes[0].label, None, "the primary tier needs no chip");
        assert_eq!(
            scopes[1].label.as_deref(),
            Some("crates/engine"),
            "a nested tier is chipped with its path relative to the root"
        );
        assert_eq!(scopes[2].label.as_deref(), Some("global"));
    }

    /// `new_context_empty` roots a context at the home directory, so its context
    /// tier IS the global tier. It must appear once, not twice.
    #[test]
    fn a_root_whose_tier_is_the_global_tier_is_listed_once() {
        let env = tier_env("dedupe");
        // `new_context_empty` roots a context at the home directory, whose tier is
        // `~/.plexi/notes` — the global tier itself. `env.base` stands in for home.
        let home_like = env.base.clone();
        assert_eq!(
            comparable_scope_path(&context_notes_dir(&home_like)),
            comparable_scope_path(&env.global),
            "fixture must actually reproduce the collision"
        );
        seed(&env.global.join("g.md"), "global");

        let scopes = notes_scopes_for_root(Some(&home_like));
        let hits = scopes
            .iter()
            .filter(|s| comparable_scope_path(&s.dir) == comparable_scope_path(&env.global))
            .count();
        assert_eq!(
            hits, 1,
            "the global tier must not be listed twice: {scopes:?}"
        );
    }

    /// With nothing anchored, the global tier is the whole answer.
    #[test]
    fn unanchored_scopes_are_global_only() {
        let env = tier_env("unanchored");
        let scopes = notes_scopes_for_root(None);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].dir, env.global);
    }

    /// The walk must not wander into machine-generated trees or follow symlinks
    /// (a symlink can point anywhere, including back above the root).
    #[test]
    fn rollup_walk_skips_ignored_dirs_and_symlinks() {
        let env = tier_env("walk-bounds");
        seed(&context_notes_dir(&env.root).join("own.md"), "own");
        // A tier buried in an ignored directory must not be found.
        let ignored = env.root.join("node_modules").join("pkg");
        seed(&context_notes_dir(&ignored).join("vendored.md"), "vendored");
        // A tier reachable only through a symlinked directory must not be found.
        let outside = env.root.parent().expect("parent").join("outside");
        seed(&context_notes_dir(&outside).join("outside.md"), "outside");
        std::os::unix::fs::symlink(&outside, env.root.join("linked")).expect("symlink");

        let dirs: Vec<PathBuf> = notes_scopes_for_root(Some(&env.root))
            .into_iter()
            .map(|s| s.dir)
            .collect();
        assert!(
            !dirs
                .iter()
                .any(|d| d.starts_with(env.root.join("node_modules"))),
            "ignored dirs must be pruned: {dirs:?}"
        );
        assert!(
            !dirs.iter().any(|d| d.starts_with(&outside)),
            "a symlinked subtree must not be walked: {dirs:?}"
        );
    }

    /// Recursive within a tier, `assets/` excluded, newest first.
    #[test]
    fn scan_tier_is_recursive_but_skips_assets() {
        let env = tier_env("scan");
        let tier = context_notes_dir(&env.root);
        seed(&tier.join("one.md"), "one");
        seed(&tier.join("project").join("idea.md"), "idea");
        seed(&tier.join("assets").join("note.md"), "not a note");
        seed(&tier.join("readme.txt"), "not markdown");

        let found = scan_tier(&tier);
        assert!(found.contains(&tier.join("one.md")));
        assert!(
            found.contains(&tier.join("project").join("idea.md")),
            "wiki subdirectories are part of the tier"
        );
        assert!(
            !found.iter().any(|p| p.starts_with(tier.join("assets"))),
            "assets/ is an attachment store, not notes: {found:?}"
        );
        assert!(!found
            .iter()
            .any(|p| p.extension().is_some_and(|e| e == "txt")));
    }

    /// A symlinked Markdown file inside a tier is a note — pointing a tier at a
    /// file kept elsewhere is a real affordance and a live user relies on it. A
    /// symlinked *directory* is still never descended.
    #[test]
    fn scan_tier_follows_symlinked_notes_but_not_symlinked_dirs() {
        let env = tier_env("scan-symlink");
        let tier = context_notes_dir(&env.root);
        seed(&tier.join("own.md"), "own");

        let elsewhere = env.base.join("elsewhere");
        seed(&elsewhere.join("TODO.md"), "linked note");
        std::os::unix::fs::symlink(elsewhere.join("TODO.md"), tier.join("TODO.md"))
            .expect("symlink note");
        std::os::unix::fs::symlink(&elsewhere, tier.join("linked-dir")).expect("symlink dir");
        // Only reachable by descending the symlinked directory.
        seed(&elsewhere.join("hidden.md"), "must not appear");

        let found = scan_tier(&tier);
        assert!(
            found.contains(&tier.join("TODO.md")),
            "a symlinked note must be listed: {found:?}"
        );
        assert!(
            !found.iter().any(|p| p.starts_with(tier.join("linked-dir"))),
            "a symlinked directory must not be descended: {found:?}"
        );

        // A dangling link is skipped rather than panicking.
        std::fs::remove_file(elsewhere.join("TODO.md")).expect("break the link");
        let after = scan_tier(&tier);
        assert!(!after.contains(&tier.join("TODO.md")), "{after:?}");
        assert!(after.contains(&tier.join("own.md")));
    }

    // ─── Retired pre-tier store ──────────────────────────────────────────────

    /// Stint 1312: the pre-tier migration is deleted. A pre-tier directory is
    /// reported by path with the manual move, every time, and reporting it
    /// only reads: nothing is moved, copied, or removed.
    #[test]
    fn pre_tier_notes_are_reported_by_path_and_never_moved() {
        let env = tier_env("pre-tier-report");
        seed(&env.legacy.join("inbox").join("staged.md"), "staged body");
        let ctx_dir = env.legacy.join("ctx-project-1a2b3c4d");
        seed(&ctx_dir.join("kept.md"), "kept body");
        seed(&env.legacy.join("assets").join("pic-1.png"), "PNGBYTES");

        // Never-migrated profile (its workspace is version 2): every shape.
        let message = pre_tier_notes_error(true).expect("a pre-tier notes dir must be reported");
        assert!(
            message.contains(&env.legacy.display().to_string()),
            "{message}"
        );
        assert!(message.contains("inbox"), "{message}");
        assert!(message.contains("ctx-project-1a2b3c4d"), "{message}");
        assert!(message.contains("no longer migrates"), "{message}");
        assert!(message.contains("left unchanged"), "{message}");
        assert!(
            message.contains(&env.global.display().to_string()),
            "the fix names the global tier: {message}"
        );
        assert!(
            message.contains(".plexi/notes"),
            "the fix names the project tier: {message}"
        );
        assert!(!message.contains("pic-1.png"), "{message}");

        assert_eq!(
            std::fs::read_to_string(env.legacy.join("inbox").join("staged.md")).expect("staged"),
            "staged body"
        );
        assert_eq!(
            std::fs::read_to_string(ctx_dir.join("kept.md")).expect("kept"),
            "kept body"
        );
        assert!(scan_tier(&env.global).is_empty(), "nothing is copied");
        assert!(scan_tier(&context_notes_dir(&env.root)).is_empty());
        assert_eq!(
            pre_tier_notes_error(true),
            Some(message),
            "reported again on every launch until the user moves it"
        );
    }

    /// The deleted migration left context folders it could not match in
    /// place on purpose. In an upgraded profile they are not reported; loose
    /// and inbox notes, which it always moved, still are.
    #[test]
    fn folders_left_by_the_migration_are_not_reported_in_an_upgraded_profile() {
        let env = tier_env("pre-tier-left");
        let orphan = env.legacy.join("ctx-gone-0a1b2c3d").join("orphan.md");
        let ambiguous = env.legacy.join("project").join("slug.md");
        seed(&orphan, "orphan body");
        seed(&ambiguous, "slug body");
        assert_eq!(pre_tier_notes_error(false), None);
        let message = pre_tier_notes_error(true).expect("a never-migrated profile reports them");
        assert!(message.contains("ctx-gone-0a1b2c3d/"), "{message}");
        assert!(message.contains("project/"), "{message}");

        seed(&env.legacy.join("loose.md"), "loose body");
        for retired in [false, true] {
            let message = pre_tier_notes_error(retired).expect("a loose note is always reported");
            assert!(message.contains("loose.md"), "{message}");
            assert_eq!(
                message.contains("ctx-gone-0a1b2c3d"),
                retired,
                "folders only when the profile was never migrated: {message}"
            );
        }
        seed(&env.legacy.join("inbox").join("staged.md"), "staged body");
        let message = pre_tier_notes_error(false).expect("inbox is always reported");
        assert!(message.contains("inbox/"), "{message}");

        assert_eq!(std::fs::read_to_string(&orphan).unwrap(), "orphan body");
        assert_eq!(std::fs::read_to_string(&ambiguous).unwrap(), "slug body");
        assert!(scan_tier(&env.global).is_empty(), "nothing is copied");
    }

    /// An empty pre-tier base, or one holding only attachments, holds no note.
    #[test]
    fn a_pre_tier_base_without_notes_is_not_reported() {
        let env = tier_env("pre-tier-empty");
        assert_eq!(pre_tier_notes_error(true), None);
        seed(&env.legacy.join("assets").join("pic-1.png"), "PNGBYTES");
        seed(&env.legacy.join("trash").join("gone.md"), "trashed");
        assert_eq!(pre_tier_notes_error(true), None);
    }

    /// On the stable channel the pre-tier base IS the global tier, so only a
    /// hashed `ctx-` folder is unmistakably pre-tier. Ordinary global notes and
    /// folders, `inbox` included, are tier content and are never reported.
    #[test]
    fn stable_channel_global_tier_is_not_mistaken_for_the_pre_tier_store() {
        let base = std::env::temp_dir().join(format!(
            "plexi-notes-stable-report-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let shared = base.join(".plexi");
        std::fs::create_dir_all(&shared).expect("shared dir");
        let _profile = crate::config::set_test_profile_dir(shared.clone());
        let _shared_guard = crate::config::set_test_shared_dir(shared.clone());
        let tier = global_notes_dir();
        seed(&tier.join("precious.md"), "do not eat me");
        seed(&tier.join("inbox").join("staged.md"), "staged body");
        seed(&tier.join("ctx-manual").join("own.md"), "a user folder");
        assert_eq!(pre_tier_notes_error(true), None);

        seed(
            &tier.join("ctx-project-1a2b3c4d").join("kept.md"),
            "kept body",
        );
        assert_eq!(
            pre_tier_notes_error(false),
            None,
            "a ctx folder in an upgraded profile was left on purpose"
        );
        let message = pre_tier_notes_error(true).expect("a hashed ctx dir must be reported");
        assert!(message.contains("ctx-project-1a2b3c4d"), "{message}");
        assert!(!message.contains("precious"), "{message}");
        assert!(!message.contains("ctx-manual"), "{message}");
        assert_eq!(
            std::fs::read_to_string(tier.join("ctx-project-1a2b3c4d").join("kept.md"))
                .expect("kept"),
            "kept body"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    fn discovery_for(env: &TierEnv) -> NotesDiscovery {
        NotesDiscovery {
            active_root: Some(env.root.clone()),
            known_roots: vec![env.root.clone()],
            global_notes: env.global.clone(),
            home: Some(env.base.clone()),
            cached_tiers: Vec::new(),
        }
    }

    /// A warm index answers from memory. The walk counter is the proof: the
    /// hit path must not enter another directory.
    #[test]
    fn notes_index_cache_hit_does_not_walk() {
        let env = tier_env("cache-hit");
        seed(
            &context_notes_dir(&env.root).join("one.md"),
            "---\ntitle: \"One\"\n---\nbody\n",
        );
        let query = discovery_for(&env);
        let first = build_notes_index(&query);
        assert!(
            first.entries.iter().any(|entry| entry.title == "One"),
            "the build must see the seeded note: {:?}",
            first
                .entries
                .iter()
                .map(|entry| &entry.title)
                .collect::<Vec<_>>()
        );
        let walked = notes_walk_dirs();
        assert!(walked > 0, "building the index has to walk");

        let mut cache = NotesIndexCache::default();
        let generation = cache.generation();
        assert!(cache.store(&env.root, first, generation));
        let hit = cache.cached_entries(&env.root).expect("warm cache");
        assert_eq!(
            notes_walk_dirs(),
            walked,
            "a cache hit must not visit any directory"
        );
        assert!(hit.iter().any(|entry| entry.title == "One"));
    }

    /// Invalidation does not rescan by itself. The next build — what the
    /// background refresh runs after `invalidate` — is what surfaces the note.
    #[test]
    fn notes_index_invalidation_surfaces_a_new_note() {
        let env = tier_env("cache-invalidate");
        seed(
            &context_notes_dir(&env.root).join("one.md"),
            "---\ntitle: \"One\"\n---\nbody\n",
        );
        let query = discovery_for(&env);
        let mut cache = NotesIndexCache::default();
        let generation = cache.generation();
        assert!(cache.store(&env.root, build_notes_index(&query), generation));

        seed(
            &context_notes_dir(&env.root).join("two.md"),
            "---\ntitle: \"Two\"\n---\nnew body\n",
        );
        let stale = cache.cached_entries(&env.root).expect("stale rows");
        assert!(
            !stale.iter().any(|entry| entry.title == "Two"),
            "the cached index must not notice a note written after it was built"
        );

        cache.invalidate();
        let fresh = build_notes_index(&query);
        let generation = cache.generation();
        assert!(
            cache.store(&env.root, fresh, generation),
            "the post-invalidation build is the current generation"
        );
        let fresh_rows = cache.cached_entries(&env.root).expect("fresh rows");
        let titles: Vec<&str> = fresh_rows
            .iter()
            .map(|entry| entry.title.as_str())
            .collect();
        assert!(
            titles.contains(&"Two"),
            "the new note must appear after invalidation: {titles:?}"
        );
        assert!(titles.contains(&"One"), "{titles:?}");
    }

    /// Home discovery must not enter `~/Library`, hidden directories, or the
    /// heavy-dir list. A project Plexi already knows is still listed even when
    /// it sits inside a pruned tree, because that path is stated directly
    /// instead of found by walking.
    #[test]
    fn notes_index_prune_list_skips_heavy_dirs_and_keeps_known_roots() {
        for required in ["Library", "node_modules", "target", ".git"] {
            assert!(
                NOTES_WALK_PRUNE.contains(&required),
                "{required} must stay on the prune list"
            );
        }

        let env = tier_env("prune-home");
        let home = env.base.clone();
        let project = home.join("Documents").join("GitHub").join("proj");
        seed(
            &context_notes_dir(&project).join("yes.md"),
            "---\ntitle: \"Visible\"\n---\nyes\n",
        );
        seed(
            &context_notes_dir(&home.join(".hidden").join("secret")).join("no.md"),
            "hidden",
        );
        seed(
            &context_notes_dir(&project.join(".git").join("objects")).join("no.md"),
            "git",
        );

        let mut padded = 0usize;
        for name in NOTES_WALK_PRUNE {
            let buried = home.join(name).join("pkg");
            seed(&context_notes_dir(&buried).join("no.md"), "pruned");
            // A wide `Library/Containers` is what exhausted the 20000-dir budget
            // under a real home. The pruned walk must not enter it.
            if *name == "Library" {
                for index in 0..1500 {
                    std::fs::create_dir_all(
                        home.join("Library")
                            .join("Containers")
                            .join(format!("app{index}")),
                    )
                    .expect("pad Library");
                    padded += 1;
                }
            }
        }
        assert!(padded >= 1500);

        let known_app = home.join("Library").join("Containers").join("KnownApp");
        seed(
            &context_notes_dir(&known_app).join("kept.md"),
            "---\ntitle: \"Known\"\n---\nkept\n",
        );
        let unknown_app = home.join("Library").join("Containers").join("UnknownApp");
        seed(
            &context_notes_dir(&unknown_app).join("lost.md"),
            "---\ntitle: \"Lost\"\n---\nlost\n",
        );

        let unpruned_started = std::time::Instant::now();
        let (unpruned_dirs, _) = count_dirs_unpruned(&home);
        let unpruned_ms = unpruned_started.elapsed().as_secs_f64() * 1000.0;

        let pruned_started = std::time::Instant::now();
        let snapshot = build_notes_index(&NotesDiscovery {
            active_root: Some(home.clone()),
            known_roots: vec![home.clone(), project.clone(), known_app.clone()],
            global_notes: env.global.clone(),
            home: Some(home.clone()),
            cached_tiers: Vec::new(),
        });
        let pruned_ms = pruned_started.elapsed().as_secs_f64() * 1000.0;
        let titles: Vec<&str> = snapshot
            .entries
            .iter()
            .map(|entry| entry.title.as_str())
            .collect();

        assert!(
            titles.contains(&"Visible"),
            "a project under Documents must still be found: {titles:?}"
        );
        assert!(
            titles.contains(&"Known"),
            "a known root inside Library is stated directly, not walked: {titles:?}"
        );
        assert!(
            !titles.contains(&"Lost"),
            "an unknown tier inside Library must not be walked: {titles:?}"
        );
        assert!(
            !titles
                .iter()
                .any(|title| *title == "hidden" || *title == "git" || *title == "pruned"),
            "hidden dirs and the prune list must not contribute notes: {titles:?}"
        );
        assert!(
            !notes_walk_budget_hit(),
            "a normal home-shaped tree must not trip the {WALK_DIR_BUDGET}-dir budget"
        );
        assert!(
            notes_walk_dirs() < padded as u64,
            "visited {} dirs but Library alone contains {padded} — the walk entered it",
            notes_walk_dirs()
        );
        eprintln!(
            "notes_walk before_dirs={unpruned_dirs} before_ms={unpruned_ms:.1} after_dirs={} after_ms={pruned_ms:.1} budget_hit={}",
            notes_walk_dirs(),
            notes_walk_budget_hit()
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Persist under the isolated profile and read it back without walking.
    #[test]
    fn notes_index_roundtrips_through_the_profile_cache() {
        let env = tier_env("persist");
        seed(
            &context_notes_dir(&env.root).join("saved.md"),
            "---\ntitle: \"Saved\"\n---\nbody\n",
        );
        let mut cache = NotesIndexCache::default();
        let generation = cache.generation();
        assert!(cache.store(
            &env.root,
            build_notes_index(&discovery_for(&env)),
            generation
        ));
        let path = notes_index_cache_path().expect("isolated profile");
        cache.save(&path).expect("persist");

        reset_notes_walk_dirs_for_test();
        let loaded = NotesIndexCache::load(&path).expect("load");
        assert_eq!(notes_walk_dirs(), 0, "loading the cache file is not a walk");
        let loaded_rows = loaded.cached_entries(&env.root).expect("anchor");
        let titles: Vec<&str> = loaded_rows
            .iter()
            .map(|entry| entry.title.as_str())
            .collect();
        assert!(titles.contains(&"Saved"), "{titles:?}");
    }

    fn count_dirs_unpruned(root: &Path) -> (usize, std::time::Duration) {
        let started = std::time::Instant::now();
        let mut count = 0usize;
        let mut queue = vec![root.to_path_buf()];
        while let Some(dir) = queue.pop() {
            count += 1;
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    queue.push(entry.path());
                }
            }
        }
        (count, started.elapsed())
    }
}
