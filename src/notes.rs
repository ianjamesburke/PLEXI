//! Notes storage: tier resolution, scanning, and the migration onto tiers.
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

/// Parse a `---\nkey: value\n---\nbody` note into its frontmatter and body.
/// Returns an empty frontmatter and the full content as the body when the note
/// does not have a YAML front-matter block.
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

/// Case-insensitive fuzzy subsequence match: every character of `query`
/// (whitespace ignored) must appear in `haystack` in order. `haystack` must
/// already be lowercased.
pub(crate) fn fuzzy_match(query: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .all(|q| chars.any(|h| h == q))
}

// ─── Notes tiers ─────────────────────────────────────────────────────────────
//
// Notes are user data, so they use the two-tier `StateScope` model from
// `crate::host::state_scope` — the same one app state uses, and the reason this
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

use crate::host::context::Context;

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

/// Directory names the rollup walk never enters. Large, machine-generated, or
/// (on macOS) the per-user `Library` tree — never a place a user keeps notes.
/// Hidden directories are also skipped, except `.plexi` itself, which is an
/// anchor. `.git` is both hidden and listed here so the prune set is explicit.
pub(crate) const NOTES_WALK_PRUNE: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".venv",
    "venv",
    ".git",
    "Library",
    "Applications",
    "Containers",
    "Caches",
    "Pods",
    "DerivedData",
    "Carthage",
    "__pycache__",
    "bower_components",
    "coverage",
    "site-packages",
];

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
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut visited = 0usize;

    while let Some((dir, depth)) = queue.pop_front() {
        if depth >= WALK_MAX_DEPTH {
            continue;
        }
        visited += 1;
        record_dir_visit();
        if visited > WALK_DIR_BUDGET {
            record_budget_hit();
            log::warn!(
                "notes: rollup walk hit its {WALK_DIR_BUDGET}-directory budget under {root:?} — \
                 nested tiers below {dir:?} were not scanned"
            );
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            // `file_type` does not follow symlinks, which is what we want: a
            // symlinked directory is skipped rather than walked.
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name();
            let path = entry.path();
            if name == PLEXI_DIR {
                // Found an anchor. Take its tier if present; never walk deeper
                // into a `.plexi` directory.
                let tier = path.join(TIER_DIR);
                if tier.is_dir()
                    && comparable_scope_path(&tier)
                        != comparable_scope_path(&context_notes_dir(root))
                {
                    found.push(tier);
                }
                continue;
            }
            let name = name.to_string_lossy();
            // Hidden dirs (except `.plexi`, handled above) and heavy trees.
            if name.starts_with('.') || name_is_pruned(&name) {
                continue;
            }
            queue.push_back((path, depth + 1));
        }
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

// ─── Migration off the central store ─────────────────────────────────────────
//
// Before the tier model, kept notes lived in a channel-scoped profile under a
// one-way hash of the context root: `config_dir()/notes/ctx-<slug>-<hash8>/`,
// with captures staged in `config_dir()/notes/inbox/`. Because the hash cannot
// be reversed, the only way to place an old directory is to recompute its name
// from a *live* context's root. Anything that does not match a live context is
// left exactly where it is and logged by path — never guessed at, never deleted.

/// Legacy directory-name arithmetic. This module exists only to locate
/// pre-tier directories, and it is the deletion trigger for the whole
/// migration: when no user can still be carrying a pre-tier profile, delete
/// `mod legacy_store`, `migrate_notes_storage`, and their tests together.
mod legacy_store {
    use std::path::Path;

    /// Prefix marking a directory under the old base as context-keyed. Legacy
    /// workspace-slug dirs never carry it, which is what made the two
    /// distinguishable.
    pub(super) const CONTEXT_DIR_PREFIX: &str = "ctx-";

    /// Directory names under the old base that were never a context's dir.
    pub(super) const RESERVED_DIRS: [&str; 4] = ["inbox", "trash", "assets", "templates"];

    /// The old channel-scoped base holding every pre-tier notes directory.
    pub(super) fn base() -> std::path::PathBuf {
        crate::config::config_dir().join("notes")
    }

    /// Lowercase ASCII slug of a path's final component. Was never load-bearing
    /// for identity — the hash was.
    fn readable_slug(root: &Path) -> String {
        let base = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut slug = String::new();
        for ch in base.chars() {
            if ch.is_ascii_alphanumeric() {
                slug.extend(ch.to_lowercase());
            } else if !slug.ends_with('-') {
                slug.push('-');
            }
            if slug.len() >= 24 {
                break;
            }
        }
        let slug = slug.trim_matches('-').to_string();
        if slug.is_empty() {
            "root".to_string()
        } else {
            slug
        }
    }

    /// The directory name the old store used for the context rooted at `root`.
    pub(super) fn ctx_dir_name(root: &Path) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(root.to_string_lossy().as_bytes());
        let hash: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
        format!("{CONTEXT_DIR_PREFIX}{}-{hash}", readable_slug(root))
    }
}

/// Outcome of one migration pass. Counts are for logging and tests only.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct NotesMigrationReport {
    /// Notes copied into a tier and removed from the old store.
    pub moved: usize,
    /// Notes already present at the destination — source removed, nothing written.
    pub already_present: usize,
    /// Old directories that could not be placed. Left untouched on disk.
    pub left_behind: Vec<String>,
    /// Notes that failed to migrate. The source copy is always still there.
    pub failed: usize,
}

/// Move every pre-tier notes directory into its tier.
///
/// Data-safety rules, in order, all inherited from the pass this replaces:
/// - copy first, verify the destination byte-for-byte, only then remove the source
/// - a destination already holding identical bytes counts as migrated
/// - a destination holding *different* bytes of the same name gets the source
///   under a suffixed name; nothing is ever overwritten
/// - an old directory with no live context is logged and left completely alone
/// - the old directory is removed only when `remove_dir` succeeds, i.e. it is
///   empty — so a stray non-note file keeps its directory rather than being lost
///
/// Idempotent: a second pass finds nothing to move and writes nothing.
///
/// Runs once per host boot after workspace restore, so the live context list is
/// populated — the hashed directory names cannot be resolved without it.
pub(crate) fn migrate_notes_storage(contexts: &[Context]) -> NotesMigrationReport {
    let mut report = NotesMigrationReport::default();
    let base = legacy_store::base();
    if !base.is_dir() {
        return report;
    }

    // 1. Staged captures and any stray top-level notes → the global tier.
    //    The old inbox was global by nature, so this is the same tier it meant.
    let global = global_notes_dir();
    for src_dir in [base.join("inbox"), base.clone()] {
        migrate_dir_contents(&src_dir, &global, &base, &mut report);
    }

    // 2. Each live context's hashed directory → that context's tier. Sorted by
    //    context id so a partial failure replays identically.
    let mut ordered: Vec<&Context> = contexts.iter().collect();
    ordered.sort_by_key(|c| c.context_id);
    for ctx in ordered {
        let src_dir = base.join(legacy_store::ctx_dir_name(&ctx.root));
        if src_dir.is_dir() {
            migrate_dir_contents(&src_dir, &context_notes_dir(&ctx.root), &base, &mut report);
        }
    }

    // 3. Pre-hash workspace-slug directories, placeable only when their
    //    basename names exactly one live context.
    let mut slug_dirs: Vec<(String, PathBuf)> = match std::fs::read_dir(&base) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let skip = legacy_store::RESERVED_DIRS.contains(&name.as_str())
                    || name.starts_with(legacy_store::CONTEXT_DIR_PREFIX);
                (!skip).then_some((name, e.path()))
            })
            .collect(),
        Err(error) => {
            log::warn!("notes_migration: cannot read {base:?}: {error}");
            return report;
        }
    };
    slug_dirs.sort();

    for (slug, src_dir) in slug_dirs {
        let matches: Vec<&Context> = contexts
            .iter()
            .filter(|c| {
                c.root
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy() == slug.as_str())
            })
            .collect();
        let [ctx] = matches.as_slice() else {
            log::warn!(
                "notes_migration: legacy dir {slug:?} matches {} live contexts — leaving it at {src_dir:?}",
                matches.len()
            );
            report.left_behind.push(slug);
            continue;
        };
        migrate_dir_contents(&src_dir, &context_notes_dir(&ctx.root), &base, &mut report);
    }

    // 4. Whatever is still here cannot be placed. Name every path so a user can
    //    recover by hand; never delete and never guess.
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(legacy_store::CONTEXT_DIR_PREFIX) {
                log::warn!(
                    "notes_migration: {:?} belongs to no live context — its root cannot be \
                     recovered from the hashed name, so it is left in place. Open that \
                     directory as a context and re-run to migrate it.",
                    entry.path()
                );
                report.left_behind.push(name);
            } else if entry.path().is_dir() || name.ends_with(".md") {
                log::info!("notes_migration: leaving {:?} in place", entry.path());
                report.left_behind.push(name);
            }
        }
    }

    report.left_behind.sort();
    report.left_behind.dedup();
    log::info!(
        "notes_migration: moved={} already_present={} failed={} left_behind={:?}",
        report.moved,
        report.already_present,
        report.failed,
        report.left_behind
    );
    report
}

/// Move every `*.md` directly inside `src_dir` into `dest_dir`, repairing asset
/// references as it goes. `old_base` is the pre-tier collection root, whose
/// shared `assets/` directory referenced attachments used to live in.
fn migrate_dir_contents(
    src_dir: &Path,
    dest_dir: &Path,
    old_base: &Path,
    report: &mut NotesMigrationReport,
) {
    // On the stable channel `config_dir()` IS `~/.plexi`, so the pre-tier base
    // and the global tier are the same directory. Without this guard every note
    // there would match itself as "already present" and be removed — the source
    // and the destination are the same file. Nothing to migrate; already home.
    if comparable_scope_path(src_dir) == comparable_scope_path(dest_dir) {
        log::info!(
            "notes_migration: {src_dir:?} is already the destination tier — nothing to move"
        );
        return;
    }
    let Ok(entries) = std::fs::read_dir(src_dir) else {
        return;
    };
    let mut notes: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "md"))
        .collect();
    if notes.is_empty() {
        // Still try to retire an emptied directory from a previous pass.
        let _ = std::fs::remove_dir(src_dir);
        return;
    }
    notes.sort();

    if let Err(error) = std::fs::create_dir_all(dest_dir) {
        log::warn!("notes_migration: cannot create {dest_dir:?}: {error}");
        report.failed += notes.len();
        return;
    }

    for src in notes {
        let Ok(original) = std::fs::read(&src) else {
            log::warn!("notes_migration: cannot read {src:?} — leaving it in place");
            report.failed += 1;
            continue;
        };
        let repaired = repoint_asset_refs(&original);
        match migrate_one_note(&src, dest_dir, &repaired) {
            Ok(MigratedNote::Moved(dest)) => {
                report.moved += 1;
                copy_referenced_assets(&repaired, old_base, dest_dir);
                log::info!("notes_migration: moved {src:?} → {dest:?}");
            }
            Ok(MigratedNote::AlreadyPresent(dest)) => {
                report.already_present += 1;
                // Also on this path: a previous run may have moved the note and
                // then failed to copy its attachment. Copying is idempotent.
                copy_referenced_assets(&repaired, old_base, dest_dir);
                log::info!("notes_migration: {src:?} already present at {dest:?}");
            }
            Err(error) => {
                report.failed += 1;
                log::warn!("notes_migration: leaving {src:?} in place: {error}");
            }
        }
    }

    // Only succeeds when the dir is now empty; a leftover non-note file or a
    // failed note keeps the dir, which is the safe outcome. Never the old base
    // itself, which holds other things.
    if src_dir != old_base && std::fs::remove_dir(src_dir).is_ok() {
        log::info!("notes_migration: removed emptied legacy dir {src_dir:?}");
    }
}

/// Attachment references were relative to the old shared collection root
/// (`../assets/name`) because notes sat one level down, in `inbox/` or a
/// `ctx-` directory. A tier is flat and owns its own `assets/`, so the
/// reference becomes `assets/name`.
///
/// A deliberate plain-text substitution on the exact Markdown token, not a
/// parse: rewriting anything more of a user's prose than this would be a
/// content edit, not a migration.
fn repoint_asset_refs(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    if !text.contains("](../assets/") {
        return bytes.to_vec();
    }
    text.replace("](../assets/", "](assets/").into_bytes()
}

/// Copy every attachment a migrated note references out of the old shared
/// `assets/` directory into the destination tier's own `assets/`.
///
/// Copies rather than moves: two notes in different tiers may reference the
/// same attachment, and the last one to migrate must still find it. The old
/// copy is left for the leftover sweep to report.
fn copy_referenced_assets(note_bytes: &[u8], old_base: &Path, dest_dir: &Path) {
    let Ok(text) = std::str::from_utf8(note_bytes) else {
        return;
    };
    let old_assets = old_base.join(ASSETS_DIR);
    if !old_assets.is_dir() {
        return;
    }
    let dest_assets = dest_dir.join(ASSETS_DIR);
    for name in referenced_asset_names(text) {
        let src = old_assets.join(&name);
        let dest = dest_assets.join(&name);
        if !src.is_file() {
            log::warn!(
                "notes_migration: {src:?} is referenced but missing — the reference was \
                 repointed and the attachment was not copied"
            );
            continue;
        }
        if dest.exists() {
            continue;
        }
        if let Err(error) = std::fs::create_dir_all(&dest_assets) {
            log::warn!("notes_migration: cannot create {dest_assets:?}: {error}");
            return;
        }
        match std::fs::copy(&src, &dest) {
            Ok(_) => log::info!("notes_migration: copied attachment {src:?} → {dest:?}"),
            Err(error) => log::warn!("notes_migration: cannot copy {src:?}: {error}"),
        }
    }
}

/// Asset file names referenced as `](assets/<name>)` in note text. A plain
/// token scan, deliberately not a Markdown parse — it only needs to find the
/// names this migration itself just wrote.
fn referenced_asset_names(text: &str) -> Vec<String> {
    const OPEN: &str = "](assets/";
    let mut names = Vec::new();
    for tail in text.split(OPEN).skip(1) {
        let Some(end) = tail.find(')') else { continue };
        let name = tail[..end].trim();
        if !name.is_empty() && !name.contains('/') && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

enum MigratedNote {
    Moved(PathBuf),
    AlreadyPresent(PathBuf),
}

/// Write `contents` into `dest_dir` under `src`'s file name, verify it landed
/// byte-for-byte, then remove the source. Never overwrites and never removes a
/// source whose copy is unconfirmed.
///
/// `contents` is what the destination must end up holding — normally `src`'s
/// bytes verbatim, but the migration passes repaired asset references, and the
/// verification and already-present checks both compare against it so a re-run
/// stays idempotent.
fn migrate_one_note(src: &Path, dest_dir: &Path, contents: &[u8]) -> std::io::Result<MigratedNote> {
    use std::io::Write;

    let src_bytes = contents;
    let file_name = src
        .file_name()
        .ok_or_else(|| std::io::Error::other(format!("no file name in {src:?}")))?;

    let primary = dest_dir.join(file_name);
    if std::fs::read(&primary)
        .map(|b| b == src_bytes)
        .unwrap_or(false)
    {
        // Already migrated (or a re-run) — the destination is authoritative.
        std::fs::remove_file(src)?;
        return Ok(MigratedNote::AlreadyPresent(primary));
    }

    // A process may have died after confirming a suffixed collision copy but
    // before removing the source. Reuse that byte-identical copy on the next
    // pass instead of manufacturing `-legacy-1`, `-legacy-2`, and so on.
    for dest in collision_destinations(dest_dir, src) {
        if std::fs::read(&dest)
            .map(|b| b == src_bytes)
            .unwrap_or(false)
        {
            std::fs::remove_file(src)?;
            return Ok(MigratedNote::AlreadyPresent(dest));
        }
    }

    let (dest, mut file) = create_collision_safe_destination(dest_dir, src)?;
    if let Err(e) = file.write_all(src_bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&dest);
        return Err(e);
    }
    drop(file);
    let verified = std::fs::read(&dest).map(|bytes| bytes == src_bytes);
    if !matches!(verified, Ok(true)) {
        let detail = verified
            .err()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "byte mismatch".to_string());
        let _ = std::fs::remove_file(&dest);
        return Err(std::io::Error::other(format!(
            "destination {dest:?} did not verify byte-for-byte ({detail}); source kept"
        )));
    }
    // Persist the new directory entry before removing the only old name.
    std::fs::File::open(dest_dir)?.sync_all()?;
    std::fs::remove_file(src)?;
    Ok(MigratedNote::Moved(dest))
}

fn collision_destinations<'a>(
    dest_dir: &'a Path,
    src: &'a Path,
) -> impl Iterator<Item = PathBuf> + 'a {
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "note".to_string());
    (0..1000).map(move |n| {
        let name = if n == 0 {
            format!("{stem}-legacy.md")
        } else {
            format!("{stem}-legacy-{n}.md")
        };
        dest_dir.join(name)
    })
}

/// Atomically reserve either the original filename or a legacy suffix.
fn create_collision_safe_destination(
    dest_dir: &Path,
    src: &Path,
) -> std::io::Result<(PathBuf, std::fs::File)> {
    let primary = dest_dir.join(
        src.file_name()
            .ok_or_else(|| std::io::Error::other(format!("no file name in {src:?}")))?,
    );
    for candidate in std::iter::once(primary).chain(collision_destinations(dest_dir, src)) {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other(format!(
        "no free destination name for {src:?} in {dest_dir:?}"
    )))
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

    fn ctx(id: u64, name: &str, root: &Path) -> Context {
        Context {
            name: name.to_string().into(),
            root: root.to_path_buf(),
            description: None,
            context_id: id,
            parent_id: None,
            depth: 0,
            parked: false,
        }
    }

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

    // ─── Migration off the central store ─────────────────────────────────────

    #[test]
    fn migration_moves_the_inbox_to_global_and_a_ctx_dir_to_its_tier() {
        let env = tier_env("migrate");
        seed(&env.legacy.join("inbox").join("staged.md"), "staged body");
        let ctx_dir = env.legacy.join(legacy_store::ctx_dir_name(&env.root));
        seed(&ctx_dir.join("kept.md"), "kept body");

        let contexts = [ctx(1, "project", &env.root)];
        let report = migrate_notes_storage(&contexts);

        assert_eq!(report.moved, 2, "{report:?}");
        assert_eq!(
            std::fs::read_to_string(env.global.join("staged.md")).expect("staged"),
            "staged body"
        );
        assert_eq!(
            std::fs::read_to_string(context_notes_dir(&env.root).join("kept.md")).expect("kept"),
            "kept body"
        );
        assert!(
            !env.legacy.join("inbox").exists(),
            "emptied inbox is retired"
        );
        assert!(!ctx_dir.exists(), "emptied ctx dir is retired");

        // Idempotent: a second pass has nothing to move and writes nothing.
        let again = migrate_notes_storage(&contexts);
        assert_eq!(again.moved, 0, "{again:?}");
        assert_eq!(again.already_present, 0, "{again:?}");
    }

    /// On the stable channel `config_dir()` IS `~/.plexi`, so the pre-tier base
    /// and the global tier are the same directory. Falsifying regression: without
    /// the same-directory guard, every note there matched itself as "already
    /// present" and its source — the same file — was removed. Stable users would
    /// have lost their whole global tier on first boot.
    #[test]
    fn migration_never_eats_notes_when_the_legacy_base_is_the_global_tier() {
        let base = std::env::temp_dir().join(format!(
            "plexi-notes-stable-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        // Both overrides point at the same dir, exactly as the stable channel does.
        let shared = base.join(".plexi");
        std::fs::create_dir_all(&shared).expect("shared dir");
        let _profile = crate::config::set_test_profile_dir(shared.clone());
        let _shared_guard = crate::config::set_test_shared_dir(shared.clone());
        assert_eq!(
            comparable_scope_path(&crate::config::config_dir().join("notes")),
            comparable_scope_path(&global_notes_dir()),
            "fixture must reproduce the stable-channel collision"
        );

        let tier = global_notes_dir();
        seed(&tier.join("precious.md"), "do not eat me");
        seed(&tier.join("inbox").join("staged.md"), "staged body");

        let report = migrate_notes_storage(&[]);

        assert_eq!(
            std::fs::read_to_string(tier.join("precious.md")).expect("note must survive"),
            "do not eat me",
            "a note already in the global tier must never be deleted"
        );
        // The inbox is a real subdirectory, so its notes still migrate up.
        assert_eq!(
            std::fs::read_to_string(tier.join("staged.md")).expect("staged"),
            "staged body"
        );
        assert_eq!(report.moved, 1, "{report:?}");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Never overwrite. A different note of the same name gets a suffix.
    #[test]
    fn migration_never_overwrites_a_differing_destination() {
        let env = tier_env("no-overwrite");
        seed(
            &env.legacy.join("inbox").join("dup.md"),
            "from the old store",
        );
        seed(&env.global.join("dup.md"), "already here");

        let report = migrate_notes_storage(&[]);
        assert_eq!(report.moved, 1, "{report:?}");
        assert_eq!(
            std::fs::read_to_string(env.global.join("dup.md")).expect("original"),
            "already here",
            "the existing note must be untouched"
        );
        assert_eq!(
            std::fs::read_to_string(env.global.join("dup-legacy.md")).expect("suffixed"),
            "from the old store"
        );
    }

    /// An identical destination counts as migrated: source removed, nothing written.
    #[test]
    fn migration_treats_an_identical_destination_as_already_migrated() {
        let env = tier_env("identical");
        seed(&env.legacy.join("inbox").join("same.md"), "same bytes");
        seed(&env.global.join("same.md"), "same bytes");

        let report = migrate_notes_storage(&[]);
        assert_eq!(report.already_present, 1, "{report:?}");
        assert_eq!(report.moved, 0, "{report:?}");
        assert!(!env.legacy.join("inbox").join("same.md").exists());
    }

    /// The hash is one-way, so a `ctx-` dir naming no live context cannot be
    /// placed. It stays on disk, reported, never guessed at and never deleted.
    #[test]
    fn migration_leaves_an_orphan_ctx_dir_alone() {
        let env = tier_env("orphan");
        let orphan_root = env.root.parent().expect("parent").join("long-gone");
        let orphan = env.legacy.join(legacy_store::ctx_dir_name(&orphan_root));
        seed(&orphan.join("stranded.md"), "stranded");

        let report = migrate_notes_storage(&[ctx(1, "project", &env.root)]);
        assert_eq!(report.moved, 0, "{report:?}");
        assert!(
            orphan.join("stranded.md").exists(),
            "an unplaceable note must never be moved or deleted"
        );
        assert!(
            report
                .left_behind
                .iter()
                .any(|name| name.starts_with("ctx-")),
            "the orphan must be reported by name: {report:?}"
        );
    }

    /// A pre-hash workspace-slug dir is placeable only when its basename names
    /// exactly one live context; two candidates is ambiguity, not a coin flip.
    #[test]
    fn migration_leaves_ambiguous_slug_dirs_in_place() {
        let env = tier_env("ambiguous");
        seed(&env.legacy.join("project").join("slug.md"), "slug body");
        let twin = env
            .root
            .parent()
            .expect("parent")
            .join("twin")
            .join("project");
        std::fs::create_dir_all(&twin).expect("twin root");

        let contexts = [
            ctx(1, "a", &env.root.parent().expect("parent").join("project")),
            ctx(2, "b", &twin),
        ];
        let report = migrate_notes_storage(&contexts);
        assert_eq!(report.moved, 0, "{report:?}");
        assert!(env.legacy.join("project").join("slug.md").exists());
        assert!(
            report.left_behind.iter().any(|n| n == "project"),
            "{report:?}"
        );
    }

    /// Attachments were referenced relative to the old shared collection root.
    /// A flat tier owns its own `assets/`, so the reference is repointed and the
    /// file copied — otherwise the image silently breaks.
    #[test]
    fn migration_repairs_asset_references_and_copies_the_file() {
        let env = tier_env("assets");
        seed(
            &env.legacy.join("inbox").join("shot.md"),
            "look: ![](../assets/pic-1234.png)\n",
        );
        seed(&env.legacy.join("assets").join("pic-1234.png"), "PNGBYTES");

        let report = migrate_notes_storage(&[]);
        assert_eq!(report.moved, 1, "{report:?}");
        let migrated = std::fs::read_to_string(env.global.join("shot.md")).expect("migrated");
        assert!(
            migrated.contains("![](assets/pic-1234.png)"),
            "reference must be repointed into the tier: {migrated:?}"
        );
        assert_eq!(
            std::fs::read_to_string(env.global.join("assets").join("pic-1234.png"))
                .expect("attachment copied"),
            "PNGBYTES"
        );
    }

    /// A non-note file keeps its directory alive rather than being deleted with it.
    #[test]
    fn migration_preserves_non_note_files() {
        let env = tier_env("non-note");
        seed(&env.legacy.join("inbox").join("keep.md"), "note");
        seed(
            &env.legacy.join("inbox").join("draft.md.save"),
            "editor swap file",
        );

        let report = migrate_notes_storage(&[]);
        assert_eq!(report.moved, 1, "{report:?}");
        assert!(
            env.legacy.join("inbox").join("draft.md.save").exists(),
            "a stray file must survive, and keep its directory"
        );
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
