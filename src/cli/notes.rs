use super::install_hint::{install_hint_for, InstallPlatform, InstallTarget};
use super::pane::pane_send_cli;

const FZF_INSTALL: InstallTarget = InstallTarget {
    brew: Some("fzf"),
    apt: Some("fzf"),
    winget: Some("junegunn.fzf"),
    url: "https://github.com/junegunn/fzf#installation",
};

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
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    match capture_note_into(&dir, text, &cwd) {
        Ok(path) => {
            log::info!("note_capture: wrote {:?}", path);
            println!("{}", path.display());
            0
        }
        Err(msg) => {
            eprintln!("error: {msg}");
            1
        }
    }
}

/// Write one timestamped note into `dir`, returning its path.
fn capture_note_into(
    dir: &std::path::Path,
    text: &str,
    cwd: &str,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("failed to create notes dir {:?}: {e}", dir))?;
    let now = chrono::Utc::now();
    let ts = now.format("%Y%m%dT%H%M%S%.3fZ").to_string();
    // Provenance only — the note's tier, not its frontmatter, is what locates it.
    let frontmatter = format!(
        "---\ncaptured_at: {}\nsource: cli\ncwd: {cwd}\n---\n",
        now.to_rfc3339()
    );
    let content = format!("{frontmatter}{}\n", text.trim());
    let path = dir.join(format!("{ts}.md"));
    std::fs::write(&path, content).map_err(|e| format!("failed to write note: {e}"))?;
    Ok(path)
}

/// `plexi note [add|list|open] | plexi note "<text>"` — the canonical entry point.
///
/// Bare `plexi note` prints the note help and exits without capturing anything
/// or touching a pane.
pub fn note_cli(cmd: Option<super::args::NoteCmd>, text: Option<String>) -> i32 {
    match (cmd, text) {
        (Some(cmd), _) => run_note_cmd(cmd),
        (None, Some(text)) => note_capture_cli(&text),
        (None, None) => {
            use clap::CommandFactory;
            let mut root = super::args::Cli::command();
            if let Some(sub) = root.find_subcommand_mut("note") {
                let _ = sub.print_help();
                println!();
            }
            0
        }
    }
}

/// Hidden `plexi notes [list|open]` compatibility entry point; bare `notes` lists.
pub fn notes_compat_cli(cmd: Option<super::args::NoteCmd>) -> i32 {
    log::info!("notes_compat: legacy `notes` spelling used");
    run_note_cmd(cmd.unwrap_or(super::args::NoteCmd::List))
}

fn run_note_cmd(cmd: super::args::NoteCmd) -> i32 {
    use super::args::NoteCmd;
    match cmd {
        NoteCmd::Add { text } => note_capture_cli(&text),
        NoteCmd::List => notes_list_cli(),
        NoteCmd::Open => notes_open_cli(),
        NoteCmd::Show { path } => notes_show_cli(&path),
        NoteCmd::Rm { path } => notes_rm_cli(&path),
    }
}

/// The notes directories visible from cwd — the exact set `plexi note list`
/// prints from, so every path `list` produced resolves here.
fn visible_note_tiers() -> Vec<std::path::PathBuf> {
    cli_notes_scopes().into_iter().map(|scope| scope.dir).collect()
}

/// `plexi note show <path>` — print one note's contents.
pub fn notes_show_cli(path: &std::path::Path) -> i32 {
    match note_contents_in(path, &visible_note_tiers()) {
        Ok(body) => {
            log::info!("note_show: read {:?}", path);
            print!("{body}");
            // A note without a trailing newline would glue the player's shell
            // prompt to its last line, so terminate the stream either way.
            if !body.ends_with('\n') {
                println!();
            }
            0
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

/// `plexi note rm <path>` — delete exactly one note. Irreversible.
pub fn notes_rm_cli(path: &std::path::Path) -> i32 {
    match remove_note_in(path, &visible_note_tiers()) {
        Ok(removed) => {
            log::info!("note_rm: removed {:?}", removed);
            0
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

/// Read one note that lives inside one of `tiers`.
fn note_contents_in(
    path: &std::path::Path,
    tiers: &[std::path::PathBuf],
) -> Result<String, String> {
    let resolved = resolve_note_path(path, tiers)?;
    std::fs::read_to_string(&resolved)
        .map_err(|error| format!("failed to read {}: {error}", resolved.display()))
}

/// Delete one note that lives inside one of `tiers`, returning its resolved path.
fn remove_note_in(
    path: &std::path::Path,
    tiers: &[std::path::PathBuf],
) -> Result<std::path::PathBuf, String> {
    let resolved = resolve_note_path(path, tiers)?;
    std::fs::remove_file(&resolved)
        .map_err(|error| format!("failed to remove {}: {error}", resolved.display()))?;
    Ok(resolved)
}

/// Resolve a user-supplied note path against the tiers visible to this call.
///
/// The path's **directory** is canonicalized before the containment check, so a
/// hand-typed `..` cannot escape a tier — while a symlinked note (a real
/// affordance; see `scan_tier`) still resolves by where the link sits, not by
/// where it points. A path outside every tier, or one that is not an existing
/// file, is an error naming the path.
fn resolve_note_path(
    path: &std::path::Path,
    tiers: &[std::path::PathBuf],
) -> Result<std::path::PathBuf, String> {
    let parent = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => parent,
        None => std::path::Path::new("."),
    };
    let Some(file_name) = path.file_name() else {
        return Err(format!("{} is not a note file", path.display()));
    };
    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|error| format!("cannot open {} — {error}", path.display()))?;
    let resolved = canonical_parent.join(file_name);
    if !resolved.is_file() {
        return Err(format!("no note at {}", path.display()));
    }
    let inside = tiers.iter().any(|tier| {
        std::fs::canonicalize(tier).is_ok_and(|tier| canonical_parent.starts_with(&tier))
    });
    if !inside {
        return Err(format!(
            "{} is not in a notes tier visible from this directory — run `plexi note list` and pass one of its paths",
            path.display()
        ));
    }
    Ok(resolved)
}

/// `plexi note list` — every note across every visible tier, newest first.
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

/// `plexi note open` — inject an fzf note picker into the focused terminal pane.
///
/// The picker's candidate list is `plexi note list`, so tier resolution and
/// ordering have exactly one implementation.
pub fn notes_open_cli() -> i32 {
    if !binary_in_path("fzf") {
        eprintln!("{}", missing_picker_error(InstallPlatform::current()));
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
                "error: PLEXI_PANE_ID is not set — `note open` drives a terminal pane, so it \
                 must run inside one. Use `plexi note list` outside a pane."
            );
            return 1;
        }
    };

    if !super::command_socket_available() {
        eprintln!(
            "error: no Plexi host reachable — `note open` types into a live pane. Start a host, \
             or use `plexi note list` to print paths instead."
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
    let cmd = notes_open_command(&self_exe, editor);
    log::info!("notes_open: injecting fzf picker into pane {pane_id}");
    pane_send_cli(pane_id, &cmd, false, false)
}

fn missing_picker_error(platform: InstallPlatform) -> String {
    format!(
        "error: fzf is not installed — {} to enable the picker",
        install_hint_for(platform, FZF_INSTALL)
    )
}

/// Build the shell expression injected into the calling terminal pane.
///
/// The binary path can contain spaces (notably an installed app bundle path on
/// macOS), so it must be shell-quoted as one command argument rather than
/// interpolated into the expression.
fn notes_open_command(self_exe: &str, editor: &str) -> String {
    let list_command = crate::host::shell::shell_join(&[
        self_exe.to_owned(),
        "note".to_owned(),
        "list".to_owned(),
    ]);
    format!(
        "selected=$({list_command} | fzf --header='Select note'); \
         [ -n \"$selected\" ] && {editor} \"$selected\"\r"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        capture_note_into, missing_picker_error, note_contents_in, notes_open_command,
        remove_note_in, InstallPlatform,
    };

    /// A tier-shaped temp dir plus one seeded note, so every show/rm test names a
    /// real path under a real `.plexi/notes`.
    fn seeded_tier(
        tag: &str,
    ) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tier = tmp.path().join(".plexi").join("notes");
        std::fs::create_dir_all(&tier).expect("create tier");
        let note = tier.join(format!("{tag}.md"));
        std::fs::write(&note, format!("{tag} body\n")).expect("seed note");
        (tmp, tier, note)
    }

    #[test]
    fn show_reads_a_note_inside_a_visible_tier() {
        let (_tmp, tier, note) = seeded_tier("shown");
        assert_eq!(
            note_contents_in(&note, &[tier]).expect("read"),
            "shown body\n"
        );
    }

    #[test]
    fn show_reads_notes_from_a_nested_tier_and_the_global_tier() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("project");
        let own = root.join(".plexi").join("notes");
        let nested = root.join("crates").join("engine").join(".plexi").join("notes");
        let global = tmp.path().join("global").join("notes");
        for tier in [&own, &nested, &global] {
            std::fs::create_dir_all(tier).expect("create tier");
        }
        let nested_note = nested.join("nested.md");
        std::fs::write(&nested_note, "nested body\n").expect("seed");
        let global_note = global.join("g.md");
        std::fs::write(&global_note, "global body\n").expect("seed");

        // The exact tier list `notes_scopes_for_root` yields: own, nested, global.
        let tiers = vec![own, nested, global];
        assert_eq!(
            note_contents_in(&nested_note, &tiers).expect("nested"),
            "nested body\n"
        );
        assert_eq!(
            note_contents_in(&global_note, &tiers).expect("global"),
            "global body\n"
        );
    }

    #[test]
    fn show_refuses_a_missing_note_and_names_the_path() {
        let (_tmp, tier, _note) = seeded_tier("present");
        let missing = tier.join("gone.md");
        let error = note_contents_in(&missing, &[tier]).expect_err("missing must fail");
        assert!(error.contains("no note at"), "{error}");
        assert!(error.contains("gone.md"), "{error}");
    }

    #[test]
    fn show_and_rm_refuse_a_path_outside_every_tier() {
        let (tmp, tier, _note) = seeded_tier("inside");
        let outside = tmp.path().join("secret.md");
        std::fs::write(&outside, "not a note\n").expect("seed");
        let tiers = vec![tier];

        let error = note_contents_in(&outside, &tiers).expect_err("outside must fail");
        assert!(error.contains("not in a notes tier"), "{error}");
        let error = remove_note_in(&outside, &tiers).expect_err("outside must fail");
        assert!(error.contains("not in a notes tier"), "{error}");
        assert!(outside.exists(), "the refused file must survive");
    }

    #[test]
    fn show_and_rm_refuse_dotdot_traversal_out_of_a_tier() {
        let (tmp, tier, _note) = seeded_tier("inside");
        let escape = tier.join("..").join("..").join("secret.md");
        std::fs::write(tmp.path().join("secret.md"), "escaped\n").expect("seed");
        let tiers = vec![tier];

        let error = note_contents_in(&escape, &tiers).expect_err("traversal must fail");
        assert!(error.contains("not in a notes tier"), "{error}");
        let error = remove_note_in(&escape, &tiers).expect_err("traversal must fail");
        assert!(error.contains("not in a notes tier"), "{error}");
        assert!(
            tmp.path().join("secret.md").exists(),
            "traversal must not reach the file"
        );
    }

    #[test]
    fn rm_deletes_exactly_the_named_note_and_leaves_siblings() {
        let (_tmp, tier, note) = seeded_tier("doomed");
        let sibling = tier.join("kept.md");
        std::fs::write(&sibling, "keep me\n").expect("seed");

        remove_note_in(&note, &[tier]).expect("remove");
        assert!(!note.exists(), "the named note must be gone");
        assert!(sibling.exists(), "a sibling note must survive");
    }

    #[test]
    fn rm_reports_a_missing_note_instead_of_panicking() {
        let (_tmp, tier, _note) = seeded_tier("present");
        let missing = tier.join("gone.md");
        let error = remove_note_in(&missing, &[tier]).expect_err("missing must fail");
        assert!(error.contains("no note at"), "{error}");
    }

    #[test]
    fn missing_picker_hint_uses_each_platforms_native_install_path() {
        assert_eq!(
            missing_picker_error(InstallPlatform::Macos),
            "error: fzf is not installed — brew install fzf to enable the picker"
        );
        assert_eq!(
            missing_picker_error(InstallPlatform::Linux),
            "error: fzf is not installed — sudo apt install fzf (or https://github.com/junegunn/fzf#installation) to enable the picker"
        );
        assert_eq!(
            missing_picker_error(InstallPlatform::Windows),
            "error: fzf is not installed — winget install junegunn.fzf (or https://github.com/junegunn/fzf#installation) to enable the picker"
        );
    }

    #[test]
    fn capture_writes_into_the_given_tier_and_is_scannable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let tier = tmp.path().join(".plexi").join("notes");
        let path = capture_note_into(&tier, "  list  ", "/work").expect("capture");
        assert!(path.starts_with(&tier));
        let body = std::fs::read_to_string(&path).expect("read");
        assert!(
            body.contains("source: cli") && body.contains("cwd: /work"),
            "{body}"
        );
        assert!(body.ends_with("list\n"), "{body}");
        assert_eq!(crate::notes::scan_tier(&tier), vec![path]);
    }

    #[test]
    fn capture_into_an_unwritable_tier_reports_instead_of_panicking() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let blocker = tmp.path().join("file");
        std::fs::write(&blocker, "x").expect("seed");
        let err = capture_note_into(&blocker.join("notes"), "x", "").unwrap_err();
        assert!(err.contains("failed to create notes dir"), "{err}");
    }

    #[test]
    fn picker_command_uses_the_running_binary_and_opens_the_selected_note() {
        let command = notes_open_command("/Applications/Plexi Beta/plexi", "nano");

        assert!(
            command.starts_with("selected=$('/Applications/Plexi Beta/plexi' note list | fzf"),
            "{command}"
        );
        assert!(command.contains("--header='Select note'"), "{command}");
        assert!(
            command.ends_with("[ -n \"$selected\" ] && nano \"$selected\"\r"),
            "{command}"
        );
    }
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
