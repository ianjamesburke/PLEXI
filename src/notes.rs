//! Notes inbox and triage support.
//!
//! Provides types and helpers for reading inbox notes, loading triage actions
//! from config, and executing action commands against a note.

use std::path::{Path, PathBuf};

// ─── Frontmatter ─────────────────────────────────────────────────────────────

/// Frontmatter parsed from a note's YAML-style header block.
#[derive(Default, Clone, Debug)]
pub(crate) struct NoteFrontmatter {
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
        let val = val.trim().to_string();
        match key {
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
command = "echo {note} | pbcopy"
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
    let actions_path = crate::config::config_dir().join("notes").join("actions.toml");

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
pub(crate) fn substitute_action_tokens(
    cmd: &str,
    note_body: &str,
    fm: &NoteFrontmatter,
) -> String {
    let quoted_note = crate::host::shell::shell_quote(note_body.trim());
    let cwd_str = fm
        .cwd
        .as_deref()
        .map(|s| crate::host::shell::shell_quote(s))
        .unwrap_or_default();
    let ctx_root_str = fm
        .context_root
        .as_deref()
        .map(|s| crate::host::shell::shell_quote(s))
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
        .filter(|e| {
            e.path()
                .extension()
                .map_or(false, |x| x == "md")
        })
        .filter_map(|e| {
            let mtime = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((mtime, e.path()))
        })
        .collect();

    // Newest first.
    paths.sort_by(|a, b| b.0.cmp(&a.0));

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
