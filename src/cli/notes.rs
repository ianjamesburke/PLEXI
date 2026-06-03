use super::pane::pane_send_cli;
use super::binary_in_path;

pub fn notes_list_cli() -> i32 {
    let notes_dir = crate::config::config_dir().join("notes");
    log::info!("notes_list: scanning {:?}", notes_dir);
    let entries = match std::fs::read_dir(&notes_dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("notes_list: notes dir does not exist yet");
            return 0;
        }
        Err(e) => {
            eprintln!("error: could not read notes directory: {e}");
            return 1;
        }
    };
    let mut paths: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "md"))
        .filter_map(|e| {
            let p = e.path();
            let mtime = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((mtime, p))
        })
        .collect();
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
    let notes_dir = crate::config::config_dir().join("notes");
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
            .map(|d| d.filter_map(|e| e.ok()).any(|e| std::path::Path::new(&e.file_name()).extension().map_or(false, |x| x == "md")))
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

    let editor = crate::pane_ops::create::preferred_editor();
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

