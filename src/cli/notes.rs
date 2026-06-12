use super::pane::pane_send_cli;

use super::binary_in_path;

/// `plexi note "<text>"` — capture a quick note to the inbox.
pub fn note_capture_cli(text: &str) -> i32 {
    let inbox_dir = crate::config::config_dir().join("notes").join("inbox");
    if let Err(e) = std::fs::create_dir_all(&inbox_dir) {
        eprintln!("error: failed to create inbox dir {:?}: {e}", inbox_dir);
        return 1;
    }

    // Timestamp for filename and frontmatter.
    let now = chrono::Utc::now();
    let ts = now.format("%Y%m%dT%H%M%SZ").to_string();
    let ts_human = now.to_rfc3339();

    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let workspace = crate::config::active_workspace_root()
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let frontmatter = format!(
        "---\ncaptured_at: {ts_human}\nsource: cli\ncwd: {cwd}\nworkspace: {workspace}\n---\n"
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
        let ts = note
            .frontmatter
            .captured_at
            .as_deref()
            .unwrap_or("unknown");
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

    // Workspace-scoped dir (kept notes).
    let workspace_slug = crate::config::active_workspace_root()
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        .map(|n| n.to_string_lossy().into_owned());
    let workspace_dir = workspace_slug.map(|slug| notes_base.join(slug));

    log::info!("notes_list: scanning inbox={:?} workspace={:?}", inbox_dir, workspace_dir);

    let mut paths: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();

    for dir in [Some(inbox_dir), workspace_dir].into_iter().flatten() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().extension().map_or(false, |x| x == "md") {
                if let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) {
                    paths.push((mtime, entry.path()));
                }
            }
        }
    }

    paths.sort_by(|a, b| b.0.cmp(&a.0));
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
    let notes_base = crate::config::config_dir().join("notes");
    let workspace_slug = crate::config::active_workspace_root()
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        .map(|n| n.to_string_lossy().into_owned());
    let notes_dir = match workspace_slug {
        Some(ref slug) => notes_base.join(slug),
        None => notes_base,
    };
    let notes_dir_str = notes_dir.display().to_string();

    if !binary_in_path("fzf") {
        eprintln!("error: fzf is not installed — run `brew install fzf` to enable the picker");
        return 1;
    }

    if std::env::var("PLEXI_SOCKET").is_err() {
        eprintln!("hint: run inside a Plexi pane for interactive note picking");
        eprintln!("notes directory: {notes_dir_str}");
        return 0;
    }

    let has_notes = notes_dir.is_dir()
        && std::fs::read_dir(&notes_dir)
            .map(|d| {
                d.filter_map(|e| e.ok()).any(|e| {
                    std::path::Path::new(&e.file_name())
                        .extension()
                        .map_or(false, |x| x == "md")
                })
            })
            .unwrap_or(false);
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
    let cmd = format!(
        "selected=$(ls -t {notes_dir_str}/*.md 2>/dev/null | fzf --header='Select note'); [ -n \"$selected\" ] && {editor} \"$selected\"\r"
    );
    log::info!("notes_open: injecting fzf picker into pane {pane_id}");
    pane_send_cli(pane_id, &cmd)
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
