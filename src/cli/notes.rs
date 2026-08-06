use super::pane::pane_send_cli;

use super::binary_in_path;

/// The notes tiers this invocation lists, primary first.
///
/// Resolved from **cwd**, never from `PLEXI_CONTEXT_ROOT`: a parent process
/// cannot mutate a running child's environment, so a long-lived pane's copy of
/// that variable is permanently stale after `plexi context set-root`, while cwd
/// is always current. `crate::notes::notes_scopes_for_root` is the same function
/// the GUI picker calls, so the two surfaces list the same tiers by construction
/// rather than by two implementations that can drift.
fn cli_notes_scopes() -> Vec<crate::notes::NotesScope> {
    let anchored = std::env::current_dir()
        .ok()
        .and_then(|cwd| crate::notes::anchored_root_for(&cwd));
    crate::notes::notes_scopes_for_root(anchored.as_deref())
}

/// The tier a new capture belongs to: the anchored context's tier when cwd sits
/// under one, else the global tier.
fn cli_capture_tier() -> std::path::PathBuf {
    match std::env::current_dir()
        .ok()
        .and_then(|cwd| crate::notes::anchored_root_for(&cwd))
    {
        Some(root) => crate::host::state_scope::context_notes_dir(&root),
        None => crate::host::state_scope::global_notes_dir(),
    }
}

/// `plexi note "<text>"` — capture a note into the tier cwd belongs to.
pub fn note_capture_cli(text: &str) -> i32 {
    let dir = cli_capture_tier();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("error: failed to create notes dir {:?}: {e}", dir);
        return 1;
    }

    let now = chrono::Utc::now();
    let ts = now.format("%Y%m%dT%H%M%S%.3fZ").to_string();
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Provenance only — the note's tier, not its frontmatter, is what locates it.
    let frontmatter = format!(
        "---\ncaptured_at: {}\nsource: cli\ncwd: {cwd}\n---\n",
        now.to_rfc3339()
    );
    let content = format!("{frontmatter}{}\n", text.trim());

    let path = dir.join(format!("{ts}.md"));
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

/// `plexi notes list` — every note across every visible tier, newest first.
pub fn notes_list_cli() -> i32 {
    let scopes = cli_notes_scopes();
    log::info!(
        "notes_list: scanning {:?}",
        scopes.iter().map(|s| &s.dir).collect::<Vec<_>>()
    );

    let mut paths: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    for scope in &scopes {
        for path in crate::notes::scan_tier(&scope.dir) {
            let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
                continue;
            };
            paths.push((mtime, path));
        }
    }

    paths.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    log::info!(
        "notes_list: found {} notes across {} tier(s)",
        paths.len(),
        scopes.len()
    );
    for (_, path) in &paths {
        println!("{}", path.display());
    }
    0
}

/// `plexi notes open` — inject an fzf note picker into the focused terminal pane.
///
/// The picker's candidate list is `plexi notes list`, so tier resolution and
/// ordering have exactly one implementation.
pub fn notes_open_cli() -> i32 {
    if !binary_in_path("fzf") {
        eprintln!("error: fzf is not installed — run `brew install fzf` to enable the picker");
        return 1;
    }

    let pane_id = match std::env::var("PLEXI_PANE_ID").map(|v| v.parse::<u64>()) {
        Ok(Ok(id)) => id,
        Ok(Err(_)) => {
            eprintln!("error: PLEXI_PANE_ID is not a valid number");
            return 1;
        }
        Err(_) => {
            eprintln!(
                "error: PLEXI_PANE_ID is not set — `notes open` drives a terminal pane, so it \
                 must run inside one. Use `plexi notes list` outside a pane."
            );
            return 1;
        }
    };

    if !super::command_socket_available() {
        eprintln!(
            "error: no Plexi host reachable — `notes open` types into a live pane. Start a host, \
             or use `plexi notes list` to print paths instead."
        );
        return 1;
    }

    let has_notes = cli_notes_scopes()
        .iter()
        .any(|scope| !crate::notes::scan_tier(&scope.dir).is_empty());
    if !has_notes {
        eprintln!("No notes yet. Create one with `plexi note \"...\"` or \u{2318}+Shift+Space.");
        return 0;
    }

    let editor = if binary_in_path("micro") {
        "micro"
    } else if binary_in_path("nano") {
        "nano"
    } else {
        "vim"
    };
    // Invoke this exact binary, not whatever `plexi` PATH resolves to, so a
    // channel-suffixed build lists through itself rather than the ambient channel.
    let self_exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "plexi".to_string());
    let cmd = format!(
        "selected=$(\"{self_exe}\" notes list | fzf --header='Select note'); \
         [ -n \"$selected\" ] && {editor} \"$selected\"\r"
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
