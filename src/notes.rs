//! Notes inbox and triage support.
//!
//! Provides types and helpers for reading inbox notes, loading triage actions
//! from config, and executing action commands against a note.

use std::path::{Path, PathBuf};

// ─── Frontmatter ─────────────────────────────────────────────────────────────

/// Frontmatter parsed from a note's YAML-style header block.
#[derive(Default, Clone, Debug)]
pub(crate) struct NoteFrontmatter {
    pub title: Option<String>,
    pub captured_at: Option<String>,
    pub source: Option<String>,
    pub cwd: Option<String>,
    pub workspace: Option<String>,
    pub context_root: Option<String>,
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
        let key = key.trim();
        let val = val.trim().trim_matches('"').to_string();
        match key {
            "title" => {
                if !val.is_empty() {
                    fm.title = Some(val);
                }
            }
            "captured_at" => fm.captured_at = Some(val),
            "source" => fm.source = Some(val),
            "cwd" => fm.cwd = Some(val),
            "workspace" => fm.workspace = Some(val),
            "context_root" => fm.context_root = Some(val),
            _ => {}
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
#[derive(Clone, Debug)]
pub(crate) struct NotePickerEntry {
    pub path: PathBuf,
    /// Display title: frontmatter `title` → first body line → file name.
    pub title: String,
    /// First non-empty body line shown as the secondary preview.
    pub preview: String,
    /// True when the note lives in `notes/inbox/`.
    pub inbox: bool,
    /// Lowercased haystack for fuzzy search: title + file name + body.
    pub search_text: String,
    /// Owning context name, set only for notes aggregated from a *descendant*
    /// context. `None` means the note belongs to the active context (or inbox),
    /// which needs no disambiguating chip.
    pub context_label: Option<String>,
}

impl NotePickerEntry {
    /// Build an entry from a note file. Returns `None` on read errors.
    pub(crate) fn load(path: &Path, inbox: bool) -> Option<Self> {
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
            inbox,
            search_text,
            context_label: None,
        })
    }

    /// Tag this entry as belonging to a descendant context, for the picker chip.
    pub(crate) fn with_context_label(mut self, label: Option<String>) -> Self {
        self.context_label = label;
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

// ─── Context-scoped kept notes ───────────────────────────────────────────────
//
// Kept notes live in `notes/ctx-<slug>-<hash8>/`, keyed by the context's scope
// root rather than by the directory Plexi happened to be launched from. The
// picker unions the active context's dir with every descendant context's dir,
// resolved by path-component-boundary prefix match over the live context list —
// never by crawling the filesystem for context roots.

use crate::host::context::Context;

/// Directory names under `notes/` that are never a context's kept-note dir.
const RESERVED_NOTES_DIRS: [&str; 2] = ["inbox", "trash"];

/// Prefix marking a directory under `notes/` as context-keyed. Legacy
/// workspace-slug dirs never carry it, which is what makes migration
/// distinguishable and repeatable.
const CONTEXT_DIR_PREFIX: &str = "ctx-";

/// The path a context scopes its notes to: its root.
pub(crate) fn context_scope_root(ctx: &Context) -> &Path {
    ctx.root.as_path()
}

/// `true` when `candidate` is `ancestor` itself or lives beneath it.
///
/// Component-wise, not textual: `/foo/barbaz` is **not** a descendant of
/// `/foo/bar`. `Path::starts_with` compares whole components, which is exactly
/// the boundary we need.
pub(crate) fn is_self_or_descendant(candidate: &Path, ancestor: &Path) -> bool {
    comparable_scope_path(candidate).starts_with(comparable_scope_path(ancestor))
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

/// Lowercase ASCII slug of a path's final component, for human readability in
/// the dir name. Never load-bearing for identity — the hash is.
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

/// Directory name holding kept notes for the context rooted at `root`:
/// `ctx-<readable-slug>-<sha256-prefix>`. The hash is over the full path, so
/// two same-named roots never collide.
pub(crate) fn context_notes_dir_name(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    let hash: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{CONTEXT_DIR_PREFIX}{}-{hash}", readable_slug(root))
}

/// Absolute kept-notes directory for the context rooted at `root`.
pub(crate) fn context_notes_dir(root: &Path) -> PathBuf {
    crate::config::config_dir()
        .join("notes")
        .join(context_notes_dir_name(root))
}

/// Legacy dir readable by `active` only when its basename identifies exactly
/// one live context. Ambiguous legacy notes stay on disk but are never presented
/// as belonging to an arbitrary context.
pub(crate) fn unambiguous_legacy_notes_dir(
    contexts: &[Context],
    active: &Context,
) -> Option<PathBuf> {
    let slug = context_scope_root(active).file_name()?;
    let matches = contexts
        .iter()
        .filter(|ctx| context_scope_root(ctx).file_name() == Some(slug))
        .count();
    (matches == 1).then(|| crate::config::config_dir().join("notes").join(slug))
}

/// One kept-notes directory to scan, with the label shown on its rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContextNotesScope {
    pub dir: PathBuf,
    /// Context name for descendant scopes; `None` for the active context, whose
    /// notes need no disambiguating chip.
    pub label: Option<String>,
}

/// Kept-note dirs for `active`: its own dir first, then every descendant
/// context's dir in the order they appear in `contexts`. Pure string work over
/// the live context list.
pub(crate) fn context_notes_scopes(
    contexts: &[Context],
    active: &Context,
) -> Vec<ContextNotesScope> {
    let active_root = context_scope_root(active);
    let mut seen = std::collections::HashSet::new();
    let mut scopes = vec![ContextNotesScope {
        dir: context_notes_dir(active_root),
        label: None,
    }];
    seen.insert(context_notes_dir_name(active_root));

    for ctx in contexts {
        if ctx.context_id == active.context_id {
            continue;
        }
        let root = context_scope_root(ctx);
        if !is_self_or_descendant(root, active_root) {
            continue;
        }
        let name = context_notes_dir_name(root);
        if !seen.insert(name) {
            continue;
        }
        scopes.push(ContextNotesScope {
            dir: context_notes_dir(root),
            label: Some(ctx.name.clone()),
        });
    }
    scopes
}

/// Outcome of one migration pass. Counts are for logging and tests only.
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct NotesMigrationReport {
    /// Notes copied into a context dir and removed from the legacy dir.
    pub moved: usize,
    /// Notes already present at the destination — left alone.
    pub already_present: usize,
    /// Legacy dirs with no matching context — left in place untouched.
    pub unmapped_dirs: Vec<String>,
    /// Notes that failed to migrate. The source copy is always still there.
    pub failed: usize,
}

/// Migrate legacy `notes/<workspace-slug>/` dirs into their context's dir.
///
/// Data-safety rules, in order:
/// - copy first, verify the destination byte length, only then remove the source
/// - a destination that already holds an identical file counts as migrated
/// - a destination that holds a *different* file of the same name gets the
///   source under a suffixed name; nothing is ever overwritten
/// - a legacy dir with no matching context is logged and left completely alone
/// - the legacy dir is removed only when `remove_dir` succeeds, i.e. it is empty
///
/// Idempotent: a second pass finds no legacy dirs (or finds identical
/// destinations) and moves nothing.
pub(crate) fn migrate_legacy_workspace_notes(contexts: &[Context]) -> NotesMigrationReport {
    let mut report = NotesMigrationReport::default();
    let notes_base = crate::config::config_dir().join("notes");

    let Ok(entries) = std::fs::read_dir(&notes_base) else {
        return report;
    };

    // Deterministic order so a partial failure replays the same way.
    let mut legacy_dirs: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let reserved = RESERVED_NOTES_DIRS.contains(&name.as_str())
                || name.starts_with(CONTEXT_DIR_PREFIX);
            (!reserved).then_some((name, e.path()))
        })
        .collect();
    legacy_dirs.sort();

    if legacy_dirs.is_empty() {
        return report;
    }

    for (slug, src_dir) in legacy_dirs {
        // A legacy slug contains only a basename. It cannot distinguish two
        // contexts with the same basename, so ambiguity is unmappable rather
        // than a license to pick one and misfile a person's notes.
        let matches: Vec<&Context> = contexts
            .iter()
            .filter(|c| {
                context_scope_root(c)
                    .file_name()
                    .map(|n| n.to_string_lossy() == slug.as_str())
                    .unwrap_or(false)
            })
            .collect();
        let [ctx] = matches.as_slice() else {
            log::warn!(
                "notes_migration: legacy dir {slug:?} has {} matching contexts — leaving notes in place at {src_dir:?}",
                matches.len()
            );
            report.unmapped_dirs.push(slug);
            continue;
        };

        let dest_dir = context_notes_dir(context_scope_root(ctx));
        if let Err(e) = std::fs::create_dir_all(&dest_dir) {
            log::warn!("notes_migration: failed to create {dest_dir:?}: {e}");
            report.failed += 1;
            continue;
        }

        let Ok(notes) = std::fs::read_dir(&src_dir) else {
            log::warn!("notes_migration: failed to read {src_dir:?}");
            report.failed += 1;
            continue;
        };
        let mut note_paths: Vec<PathBuf> = notes
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        note_paths.sort();

        for src in note_paths {
            match migrate_one_note(&src, &dest_dir) {
                Ok(MigratedNote::Moved(dest)) => {
                    report.moved += 1;
                    log::info!("notes_migration: moved {src:?} → {dest:?}");
                }
                Ok(MigratedNote::AlreadyPresent(dest)) => {
                    report.already_present += 1;
                    log::info!("notes_migration: {src:?} already present at {dest:?}");
                }
                Err(e) => {
                    report.failed += 1;
                    log::warn!("notes_migration: leaving {src:?} in place: {e}");
                }
            }
        }

        // Only succeeds when the dir is now empty; a leftover non-note file or a
        // failed note keeps the dir, which is the safe outcome.
        if std::fs::remove_dir(&src_dir).is_ok() {
            log::info!("notes_migration: removed empty legacy dir {src_dir:?}");
        }
    }

    log::info!(
        "notes_migration: moved={} already_present={} failed={} unmapped={:?}",
        report.moved,
        report.already_present,
        report.failed,
        report.unmapped_dirs
    );
    report
}

enum MigratedNote {
    Moved(PathBuf),
    AlreadyPresent(PathBuf),
}

/// Copy one note into `dest_dir`, verify it landed, then remove the source.
/// Never overwrites and never removes a source whose copy is unconfirmed.
fn migrate_one_note(src: &Path, dest_dir: &Path) -> std::io::Result<MigratedNote> {
    use std::io::Write;

    let src_bytes = std::fs::read(src)?;
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
    if let Err(e) = file.write_all(&src_bytes).and_then(|_| file.sync_all()) {
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

// ─── Triage actions ──────────────────────────────────────────────────────────

/// A single user-configured triage action.
#[derive(Clone, Debug)]
pub(crate) struct TriageAction {
    /// Key digit (1–9) that triggers this action.
    pub key: u8,
    /// Short human label shown in the hint bar.
    pub label: String,
    /// Shell command to run. Supports {note}, {cwd}, {context_root} tokens.
    pub command: String,
    /// When true the command runs silently without opening a terminal pane.
    pub hidden: bool,
    /// If set, after-action the note is filed under `notes/<workspace>/` instead
    /// of moved to trash. Overrides the default trash behaviour for this action.
    pub workspace: Option<String>,
}

const DEFAULT_ACTIONS_TOML: &str = r#"# Plexi notes triage actions
# Each [[action]] block defines a key (1-9), a label, and a shell command.
# Tokens: {note} = shell-quoted note body, {cwd} = cwd from frontmatter,
#         {context_root} = context_root from frontmatter.
# hidden = true runs the command without opening a terminal pane.

[[action]]
key = 1
label = "pbcopy"
command = "printf '%s' {note} | pbcopy"
hidden = true

[[action]]
key = 2
label = "append log"
command = "echo {note} >> ~/notes.md"
hidden = true
"#;

/// Load triage actions from `<config_dir>/notes/actions.toml`.
/// Writes the default template on first open. Returns an empty vec on parse errors.
pub(crate) fn load_triage_actions() -> Vec<TriageAction> {
    let actions_path = crate::config::config_dir()
        .join("notes")
        .join("actions.toml");

    if !actions_path.exists() {
        if let Some(parent) = actions_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&actions_path, DEFAULT_ACTIONS_TOML) {
            log::warn!("notes_triage: failed to write default actions.toml: {e}");
        }
        // Return defaults parsed from the constant so the user has actions immediately.
    }

    let toml_str = match std::fs::read_to_string(&actions_path) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("notes_triage: failed to read actions.toml: {e}");
            return Vec::new();
        }
    };

    parse_actions_toml(&toml_str)
}

fn parse_actions_toml(src: &str) -> Vec<TriageAction> {
    // Use the toml crate which is already in the dependency graph.
    let value: toml::Value = match toml::from_str(src) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("notes_triage: failed to parse actions.toml: {e}");
            return Vec::new();
        }
    };

    let Some(array) = value.get("action").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    let mut actions = Vec::new();
    for item in array {
        let Some(key) = item.get("key").and_then(|v| v.as_integer()) else {
            continue;
        };
        let key = key as u8;
        if !(1..=9).contains(&key) {
            log::warn!("notes_triage: action key {key} out of range 1–9, skipping");
            continue;
        }
        let label = item
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("action")
            .to_string();
        let command = item
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if command.is_empty() {
            continue;
        }
        let hidden = item
            .get("hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let workspace = item
            .get("workspace")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        actions.push(TriageAction {
            key,
            label,
            command,
            hidden,
            workspace,
        });
    }
    actions
}

/// Substitute {note}, {cwd}, {context_root} tokens in `cmd`.
pub(crate) fn substitute_action_tokens(cmd: &str, note_body: &str, fm: &NoteFrontmatter) -> String {
    let quoted_note = crate::host::shell::shell_quote(note_body.trim());
    let cwd_str = fm
        .cwd
        .as_deref()
        .map(crate::host::shell::shell_quote)
        .unwrap_or_default();
    let ctx_root_str = fm
        .context_root
        .as_deref()
        .map(crate::host::shell::shell_quote)
        .unwrap_or_default();

    cmd.replace("{note}", &quoted_note)
        .replace("{cwd}", &cwd_str)
        .replace("{context_root}", &ctx_root_str)
}

// ─── InboxNote ───────────────────────────────────────────────────────────────

/// A single note loaded from the inbox directory.
#[derive(Clone, Debug)]
pub(crate) struct InboxNote {
    pub path: PathBuf,
    pub frontmatter: NoteFrontmatter,
    pub body: String,
}

impl InboxNote {
    /// Load a note from `path`. Returns `None` on read errors.
    pub(crate) fn load(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| log::warn!("notes_triage: failed to read {:?}: {e}", path))
            .ok()?;
        let (frontmatter, body) = parse_note(&content);
        Some(Self {
            path: path.to_path_buf(),
            frontmatter,
            body,
        })
    }
}

/// Scan `<config_dir>/notes/inbox/` for `.md` files, sorted newest-first.
pub(crate) fn scan_inbox() -> Vec<InboxNote> {
    let inbox_dir = crate::config::config_dir().join("notes").join("inbox");

    let entries = match std::fs::read_dir(&inbox_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut paths: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter_map(|e| {
            let mtime = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((mtime, e.path()))
        })
        .collect();

    // Newest first.
    paths.sort_by_key(|e| std::cmp::Reverse(e.0));

    paths
        .iter()
        .filter_map(|(_, p)| InboxNote::load(p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_note_with_frontmatter() {
        let content = "---\ncaptured_at: 2026-01-01\ncwd: /tmp\n---\nhello world\n";
        let (fm, body) = parse_note(content);
        assert_eq!(fm.captured_at.as_deref(), Some("2026-01-01"));
        assert_eq!(fm.cwd.as_deref(), Some("/tmp"));
        assert_eq!(body, "hello world\n");
    }

    #[test]
    fn parse_note_without_frontmatter() {
        let content = "just a plain note\n";
        let (fm, body) = parse_note(content);
        assert!(fm.captured_at.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn substitute_tokens() {
        let fm = NoteFrontmatter {
            cwd: Some("/home/user".to_string()),
            context_root: Some("/project".to_string()),
            ..Default::default()
        };
        let result = substitute_action_tokens("cat {note} && cd {cwd}", "hello world", &fm);
        assert!(result.contains("hello world"));
        assert!(result.contains("/home/user") || result.contains("home"));
    }

    #[test]
    fn parse_note_reads_title() {
        let content = "---\ntitle: \"My Note\"\ncaptured_at: 2026-01-01\n---\nbody\n";
        let (fm, body) = parse_note(content);
        assert_eq!(fm.title.as_deref(), Some("My Note"));
        assert_eq!(body, "body\n");
    }

    #[test]
    fn fuzzy_match_subsequence() {
        assert!(fuzzy_match("idea", "big ideas live here"));
        assert!(fuzzy_match("blh", "big ideas live here"));
        assert!(fuzzy_match("BIG", "big ideas live here"));
        assert!(fuzzy_match("", "anything"));
        assert!(!fuzzy_match("xyz", "big ideas live here"));
    }

    #[test]
    fn picker_entry_title_fallbacks() {
        let dir = std::env::temp_dir().join(format!("plexi-notes-entry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let titled = dir.join("titled.md");
        std::fs::write(&titled, "---\ntitle: \"Named\"\n---\nbody line\n").expect("seed");
        let entry = NotePickerEntry::load(&titled, false).expect("load");
        assert_eq!(entry.title, "Named");
        assert_eq!(entry.preview, "body line");

        let untitled = dir.join("untitled.md");
        std::fs::write(&untitled, "---\nsource: \"quick-note\"\n---\nfirst line\n").expect("seed");
        let entry = NotePickerEntry::load(&untitled, true).expect("load");
        assert_eq!(entry.title, "first line");
        assert!(entry.inbox);

        let empty = dir.join("note-20260611.md");
        std::fs::write(&empty, "---\nsource: \"scratchpad\"\n---\n\n").expect("seed");
        let entry = NotePickerEntry::load(&empty, true).expect("load");
        assert_eq!(entry.title, "note-20260611.md");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── Context-scoped kept notes ───────────────────────────────────────────

    fn ctx(id: u64, name: &str, root: &str, depth: u32) -> Context {
        Context {
            name: name.to_string(),
            root: PathBuf::from(root),
            description: None,
            context_id: id,
            parent_id: None,
            depth,
            parked: false,
        }
    }

    /// Isolated profile dir + the `notes/` base inside it.
    fn notes_profile(tag: &str) -> (crate::config::TestProfileDirGuard, PathBuf) {
        let profile = std::env::temp_dir().join(format!(
            "plexi-notes-scope-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&profile);
        let notes = profile.join("notes");
        std::fs::create_dir_all(&notes).expect("create notes base");
        let guard = crate::config::set_test_profile_dir(profile);
        (guard, notes)
    }

    #[test]
    fn descendant_match_respects_path_component_boundary() {
        let bar = Path::new("/foo/bar");
        assert!(
            is_self_or_descendant(Path::new("/foo/bar"), bar),
            "a context is its own scope"
        );
        assert!(is_self_or_descendant(Path::new("/foo/bar/sub"), bar));
        assert!(is_self_or_descendant(Path::new("/foo/bar/sub/deep"), bar));
        assert!(
            !is_self_or_descendant(Path::new("/foo/barbaz"), bar),
            "a textual prefix that is not a component boundary is not a descendant"
        );
        assert!(!is_self_or_descendant(Path::new("/foo"), bar));
        assert!(!is_self_or_descendant(Path::new("/other/bar"), bar));
    }

    #[test]
    fn descendant_match_normalizes_equivalent_live_context_paths() {
        assert!(is_self_or_descendant(
            Path::new("/foo/project/tmp/../child"),
            Path::new("/foo/project")
        ));

        let fixture = tempfile::tempdir().expect("temp fixture");
        let real = fixture.path().join("real");
        let child = real.join("child");
        let alias = fixture.path().join("alias");
        std::fs::create_dir_all(&child).expect("real child");
        std::os::unix::fs::symlink(&real, &alias).expect("symlink");
        assert!(
            is_self_or_descendant(&alias.join("child"), &real),
            "existing symlink aliases should compare by canonical components"
        );
    }

    #[test]
    fn context_dir_name_is_readable_stable_and_collision_free() {
        let a = context_notes_dir_name(Path::new("/Users/x/Code/My Project"));
        assert!(a.starts_with("ctx-my-project-"), "unexpected dir name {a}");
        assert_eq!(
            a,
            context_notes_dir_name(Path::new("/Users/x/Code/My Project"))
        );

        // Same basename, different root — must not collide.
        let b = context_notes_dir_name(Path::new("/Users/x/Other/My Project"));
        assert_ne!(a, b);

        assert!(context_notes_dir_name(Path::new("/")).starts_with("ctx-root-"));
    }

    #[test]
    fn scopes_union_active_with_descendants_only() {
        let (_guard, _notes) = notes_profile("scopes");
        let contexts = vec![
            ctx(1, "proj", "/proj", 0),
            ctx(2, "sub", "/proj/sub", 1),
            ctx(3, "deep", "/proj/sub/deep", 2),
            ctx(4, "sibling", "/projx", 0),
            ctx(5, "elsewhere", "/other", 0),
        ];

        let from_root = context_notes_scopes(&contexts, &contexts[0]);
        assert_eq!(
            from_root
                .iter()
                .map(|s| s.label.clone())
                .collect::<Vec<_>>(),
            vec![None, Some("sub".to_string()), Some("deep".to_string())],
            "root sees itself unlabeled plus every descendant, and never /projx"
        );
        assert_eq!(from_root[0].dir, context_notes_dir(Path::new("/proj")));

        // A child sees only itself and its own descendants — no ancestor walk (v2).
        let from_sub = context_notes_scopes(&contexts, &contexts[1]);
        assert_eq!(
            from_sub.iter().map(|s| s.label.clone()).collect::<Vec<_>>(),
            vec![None, Some("deep".to_string())]
        );

        let from_leaf = context_notes_scopes(&contexts, &contexts[2]);
        assert_eq!(from_leaf.len(), 1);
    }

    #[test]
    fn scopes_dedupe_contexts_sharing_a_root() {
        let (_guard, _notes) = notes_profile("dedupe");
        let contexts = vec![ctx(1, "a", "/proj", 0), ctx(2, "a-again", "/proj", 0)];
        let scopes = context_notes_scopes(&contexts, &contexts[0]);
        assert_eq!(scopes.len(), 1, "one dir per root, not one per context");
    }

    #[test]
    fn migration_moves_legacy_notes_and_is_idempotent() {
        let (_guard, notes) = notes_profile("migrate");
        let contexts = vec![ctx(1, "proj", "/tmp/plexi-mig/proj", 0)];

        let legacy = notes.join("proj");
        std::fs::create_dir_all(&legacy).expect("legacy dir");
        std::fs::write(legacy.join("a.md"), "note a").expect("seed a");
        std::fs::write(legacy.join("b.md"), "note b").expect("seed b");
        // Non-note files are never touched, and keep the legacy dir alive.
        std::fs::write(legacy.join("README.txt"), "keep me").expect("seed txt");

        let report = migrate_legacy_workspace_notes(&contexts);
        assert_eq!(report.moved, 2);
        assert_eq!(report.failed, 0);
        assert!(report.unmapped_dirs.is_empty());

        let dest = context_notes_dir(Path::new("/tmp/plexi-mig/proj"));
        assert_eq!(
            std::fs::read_to_string(dest.join("a.md")).expect("a migrated"),
            "note a"
        );
        assert!(!legacy.join("a.md").exists(), "source removed after verify");
        assert!(
            legacy.join("README.txt").exists(),
            "non-note files are left alone"
        );

        // Second pass moves nothing.
        let again = migrate_legacy_workspace_notes(&contexts);
        assert_eq!(again.moved, 0);
        assert_eq!(again.already_present, 0);
        assert_eq!(again.failed, 0);
        assert_eq!(
            std::fs::read_dir(&dest).unwrap().count(),
            2,
            "a re-run must not duplicate notes"
        );
    }

    #[test]
    fn migration_never_overwrites_a_differing_destination() {
        let (_guard, notes) = notes_profile("collide");
        let root = Path::new("/tmp/plexi-mig-collide/proj");
        let contexts = vec![ctx(1, "proj", "/tmp/plexi-mig-collide/proj", 0)];

        let legacy = notes.join("proj");
        std::fs::create_dir_all(&legacy).expect("legacy dir");
        std::fs::write(legacy.join("a.md"), "legacy body").expect("seed legacy");

        let dest = context_notes_dir(root);
        std::fs::create_dir_all(&dest).expect("dest dir");
        std::fs::write(dest.join("a.md"), "existing body").expect("seed dest");

        let report = migrate_legacy_workspace_notes(&contexts);
        assert_eq!(report.moved, 1);
        assert_eq!(
            std::fs::read_to_string(dest.join("a.md")).unwrap(),
            "existing body",
            "the pre-existing note must survive untouched"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("a-legacy.md")).unwrap(),
            "legacy body",
            "the legacy note lands under a suffixed name"
        );
    }

    #[test]
    fn migration_retry_reuses_a_confirmed_collision_copy() {
        let (_guard, notes) = notes_profile("collision-retry");
        let root = Path::new("/tmp/plexi-mig-collision-retry/proj");
        let contexts = vec![ctx(1, "proj", "/tmp/plexi-mig-collision-retry/proj", 0)];

        let legacy = notes.join("proj");
        std::fs::create_dir_all(&legacy).expect("legacy dir");
        std::fs::write(legacy.join("a.md"), "legacy body").expect("seed legacy");
        let dest = context_notes_dir(root);
        std::fs::create_dir_all(&dest).expect("dest dir");
        std::fs::write(dest.join("a.md"), "newer body").expect("seed primary");
        // Simulate death after the collision copy was confirmed but before the
        // source was removed.
        std::fs::write(dest.join("a-legacy.md"), "legacy body").expect("seed prior copy");

        let report = migrate_legacy_workspace_notes(&contexts);
        assert_eq!(report.already_present, 1);
        assert_eq!(report.moved, 0);
        assert!(!legacy.exists());
        assert!(
            !dest.join("a-legacy-1.md").exists(),
            "a retry must not duplicate the already-confirmed collision copy"
        );
    }

    #[test]
    fn migration_treats_an_identical_destination_as_already_migrated() {
        let (_guard, notes) = notes_profile("identical");
        let root = Path::new("/tmp/plexi-mig-identical/proj");
        let contexts = vec![ctx(1, "proj", "/tmp/plexi-mig-identical/proj", 0)];

        let legacy = notes.join("proj");
        std::fs::create_dir_all(&legacy).expect("legacy dir");
        std::fs::write(legacy.join("a.md"), "same body").expect("seed legacy");
        let dest = context_notes_dir(root);
        std::fs::create_dir_all(&dest).expect("dest dir");
        std::fs::write(dest.join("a.md"), "same body").expect("seed dest");

        let report = migrate_legacy_workspace_notes(&contexts);
        assert_eq!(report.already_present, 1);
        assert_eq!(report.moved, 0);
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 1);
        assert!(!legacy.exists(), "emptied legacy dir is removed");
    }

    #[test]
    fn migration_leaves_unmapped_legacy_notes_in_place() {
        let (_guard, notes) = notes_profile("unmapped");
        let contexts = vec![ctx(1, "proj", "/tmp/plexi-mig-unmapped/proj", 0)];

        let orphan = notes.join("some-old-workspace");
        std::fs::create_dir_all(&orphan).expect("orphan dir");
        std::fs::write(orphan.join("keep.md"), "orphan note").expect("seed orphan");

        let report = migrate_legacy_workspace_notes(&contexts);
        assert_eq!(report.moved, 0);
        assert_eq!(report.failed, 0);
        assert_eq!(report.unmapped_dirs, vec!["some-old-workspace".to_string()]);
        assert_eq!(
            std::fs::read_to_string(orphan.join("keep.md")).unwrap(),
            "orphan note",
            "an unmappable note is never dropped"
        );
    }

    #[test]
    fn migration_leaves_ambiguous_legacy_notes_in_place() {
        let (_guard, notes) = notes_profile("ambiguous");
        let contexts = vec![
            ctx(1, "proj-a", "/tmp/plexi-mig-ambiguous/a/proj", 0),
            ctx(2, "proj-b", "/tmp/plexi-mig-ambiguous/b/proj", 0),
        ];

        let legacy = notes.join("proj");
        std::fs::create_dir_all(&legacy).expect("legacy dir");
        std::fs::write(legacy.join("keep.md"), "ambiguous note").expect("seed");

        let report = migrate_legacy_workspace_notes(&contexts);
        assert_eq!(report.unmapped_dirs, vec!["proj".to_string()]);
        assert_eq!(
            std::fs::read_to_string(legacy.join("keep.md")).unwrap(),
            "ambiguous note"
        );
        assert!(!context_notes_dir(Path::new("/tmp/plexi-mig-ambiguous/a/proj")).exists());
        assert!(!context_notes_dir(Path::new("/tmp/plexi-mig-ambiguous/b/proj")).exists());
    }

    #[test]
    fn migration_skips_reserved_and_already_context_keyed_dirs() {
        let (_guard, notes) = notes_profile("reserved");
        let contexts = vec![ctx(1, "proj", "/tmp/plexi-mig-reserved/proj", 0)];

        for reserved in ["inbox", "trash"] {
            let dir = notes.join(reserved);
            std::fs::create_dir_all(&dir).expect("reserved dir");
            std::fs::write(dir.join("n.md"), "stay").expect("seed");
        }
        let ctx_dir = notes.join(context_notes_dir_name(Path::new(
            "/tmp/plexi-mig-reserved/proj",
        )));
        std::fs::create_dir_all(&ctx_dir).expect("ctx dir");
        std::fs::write(ctx_dir.join("n.md"), "stay").expect("seed");

        let report = migrate_legacy_workspace_notes(&contexts);
        assert_eq!(report, NotesMigrationReport::default());
        assert!(notes.join("inbox").join("n.md").exists());
        assert!(notes.join("trash").join("n.md").exists());
        assert!(ctx_dir.join("n.md").exists());
    }

    #[test]
    fn parse_actions_toml_valid() {
        let src = r#"
[[action]]
key = 1
label = "test"
command = "echo {note}"
hidden = true
"#;
        let actions = parse_actions_toml(src);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].key, 1);
        assert_eq!(actions[0].label, "test");
        assert!(actions[0].hidden);
    }
}
