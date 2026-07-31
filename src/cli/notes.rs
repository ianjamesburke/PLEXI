use super::pane::pane_send_cli;

use super::binary_in_path;

/// The context root this CLI invocation belongs to.
///
/// `PLEXI_CONTEXT_ROOT` is exported into every pane (a context is always
/// anchored to a root), so an in-pane `plexi notes` resolves to the same dir
/// the GUI picker scans. Outside a pane, fall back to the workspace root the
/// cwd sits in. A pane missing the var predates the non-optional root and is
/// logged below rather than silently diverging from the picker.
fn cli_context_root() -> Option<std::path::PathBuf> {
    if let Some(root) = std::env::var_os("PLEXI_CONTEXT_ROOT").filter(|v| !v.is_empty()) {
        return Some(std::path::PathBuf::from(root));
    }
    let fallback = crate::config::active_workspace_root();
    if std::env::var_os("PLEXI_PANE_ID").is_some() {
        log::warn!(
            "notes: pane exports no PLEXI_CONTEXT_ROOT (pre-root-collapse pane env) — \
             falling back to workspace root {fallback:?}, which may differ from the picker's dir"
        );
    }
    fallback
}

/// Kept-notes dir for this invocation's context. Falls back to `notes/` itself
/// when no context or workspace root can be resolved, matching the picker.
fn cli_kept_notes_dir() -> std::path::PathBuf {
    match cli_context_root() {
        Some(root) => crate::notes::context_notes_dir(&root),
        None => crate::config::config_dir().join("notes"),
    }
}

/// Every dir holding this context's kept notes: the context-keyed dir, plus the
/// legacy `notes/<workspace-slug>/` dir while it still exists.
///
/// The CLI has no live context list, so it cannot run the migration itself. It
/// reads the legacy dir instead, which keeps existing notes visible to a
/// CLI-only user between upgrade and the GUI's first picker open.
fn cli_kept_notes_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = vec![cli_kept_notes_dir()];
    let legacy = cli_context_root()
        .as_deref()
        .and_then(|root| root.file_name())
        .map(|slug| crate::config::config_dir().join("notes").join(slug))
        .filter(|d| d.is_dir());
    if let Some(legacy) = legacy {
        log::info!("notes: including un-migrated legacy dir {legacy:?}");
        dirs.push(legacy);
    }
    dirs
}

/// `plexi note "<text>"` — capture a quick note to the inbox.
pub fn note_capture_cli(text: &str) -> i32 {
    let inbox_dir = crate::config::config_dir().join("notes").join("inbox");
    if let Err(e) = std::fs::create_dir_all(&inbox_dir) {
        eprintln!("error: failed to create inbox dir {:?}: {e}", inbox_dir);
        return 1;
    }

    // Timestamp for filename and frontmatter.
    let now = chrono::Utc::now();
    let ts = now.format("%Y%m%dT%H%M%S%.3fZ").to_string();
    let ts_human = now.to_rfc3339();

    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let workspace = crate::config::active_workspace_root()
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    // `context_root` is what triage keys the keep destination on.
    let context_root = cli_context_root()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let frontmatter = format!(
        "---\ncaptured_at: {ts_human}\nsource: cli\ncwd: {cwd}\nworkspace: {workspace}\ncontext_root: {context_root}\n---\n"
    );
    let content = format!("{frontmatter}{}\n", text.trim());

    let filename = format!("{ts}.md");
    let path = inbox_dir.join(&filename);
    match std::fs::write(&path, &content) {
        Ok(_) => {
            log::info!("note_capture: wrote {:?}", path);
            println!("{}", path.display());
            0
        }
        Err(e) => {
            eprintln!("error: failed to write note: {e}");
            1
        }
    }
}

/// `plexi notes inbox` — list inbox notes with frontmatter context.
pub fn notes_inbox_cli() -> i32 {
    let notes = crate::notes::scan_inbox();
    if notes.is_empty() {
        println!("Inbox is empty.");
        return 0;
    }
    for note in &notes {
        let ts = note.frontmatter.captured_at.as_deref().unwrap_or("unknown");
        let cwd = note.frontmatter.cwd.as_deref().unwrap_or("");
        let preview: String = note.body.trim().chars().take(60).collect();
        println!("{}\t{}\t{}", ts, cwd, preview);
    }
    0
}

/// `plexi notes process` — print inbox notes in agent-legible format.
pub fn notes_process_cli() -> i32 {
    let notes = crate::notes::scan_inbox();
    let actions = crate::notes::load_triage_actions();

    if notes.is_empty() {
        println!("# Inbox empty");
        return 0;
    }

    println!("# Inbox notes ({} total)", notes.len());
    println!();
    for (i, note) in notes.iter().enumerate() {
        println!("## Note {} of {}", i + 1, notes.len());
        if let Some(ref ts) = note.frontmatter.captured_at {
            println!("captured_at: {ts}");
        }
        if let Some(ref cwd) = note.frontmatter.cwd {
            println!("cwd: {cwd}");
        }
        if let Some(ref ws) = note.frontmatter.workspace {
            println!("workspace: {ws}");
        }
        println!();
        println!("{}", note.body.trim());
        println!();
    }

    if !actions.is_empty() {
        println!("# Triage actions");
        for a in &actions {
            println!("  {} — {}: {}", a.key, a.label, a.command);
        }
    }

    0
}

pub fn notes_list_cli() -> i32 {
    let notes_base = crate::config::config_dir().join("notes");

    // Always include inbox.
    let inbox_dir = notes_base.join("inbox");

    // Context-scoped dirs (kept notes), including any un-migrated legacy dir.
    let mut dirs = vec![inbox_dir];
    dirs.extend(cli_kept_notes_dirs());

    log::info!("notes_list: scanning {dirs:?}");

    let mut paths: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().extension().is_some_and(|x| x == "md") {
                if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
                    paths.push((mtime, entry.path()));
                }
            }
        }
    }

    paths.sort_by_key(|e| std::cmp::Reverse(e.0));
    log::info!("notes_list: found {} notes", paths.len());
    for (_, path) in &paths {
        println!("{}", path.display());
    }
    0
}

/// `plexi notes open` — inject fzf note picker into the focused terminal pane.
///
/// Falls back to printing the notes directory when PLEXI_SOCKET is unset or fzf is absent.
pub fn notes_open_cli() -> i32 {
    let notes_dirs = cli_kept_notes_dirs();
    let notes_dir_str = notes_dirs
        .iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(" ");

    if !binary_in_path("fzf") {
        eprintln!("error: fzf is not installed — run `brew install fzf` to enable the picker");
        return 1;
    }

    if !super::command_socket_available() {
        eprintln!("hint: run inside a Plexi pane for interactive note picking");
        eprintln!("notes directory: {notes_dir_str}");
        return 0;
    }

    let has_notes = notes_dirs.iter().any(|dir| {
        dir.is_dir()
            && std::fs::read_dir(dir)
                .map(|d| {
                    d.filter_map(|e| e.ok()).any(|e| {
                        std::path::Path::new(&e.file_name())
                            .extension()
                            .is_some_and(|x| x == "md")
                    })
                })
                .unwrap_or(false)
    });
    if !has_notes {
        eprintln!("No notes yet. Create one with \u{2318}+Shift+Space.");
        return 0;
    }

    let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not set — run inside a Plexi terminal pane");
            return 1;
        }
    };
    let pane_id: u64 = match pane_id_str.parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
            return 1;
        }
    };

    let editor = if binary_in_path("micro") {
        "micro"
    } else if binary_in_path("nano") {
        "nano"
    } else {
        "vim"
    };
    let globs = notes_dirs
        .iter()
        .map(|d| format!("{}/*.md", d.display()))
        .collect::<Vec<_>>()
        .join(" ");
    let cmd = format!(
        "selected=$(ls -t {globs} 2>/dev/null | fzf --header='Select note'); [ -n \"$selected\" ] && {editor} \"$selected\"\r"
    );
    log::info!("notes_open: injecting fzf picker into pane {pane_id}");
    pane_send_cli(pane_id, &cmd, false)
}

fn print_demo_divider() {
    eprintln!("  \x1b[2m────────────────────\x1b[0m");
    eprintln!();
}

pub(super) fn print_step_header(step: u8, total: u8, title: &str) {
    print_demo_divider();
    eprintln!("  \x1b[1mStep {} of {}   {}\x1b[0m", step, total, title);
    eprintln!();
}

pub(super) fn print_step_complete(step: u8, total: u8) {
    eprintln!("  \x1b[1;32m\u{2713} {}/{}\x1b[0m", step, total);
    eprintln!();
}
