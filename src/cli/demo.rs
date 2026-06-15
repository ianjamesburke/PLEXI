use super::notes::{print_step_complete, print_step_header};

const TOTAL_STEPS: u8 = 11;
const CMD: &str = "\u{2318}";

pub fn demo_cli() -> i32 {
    let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: run `plexi demo` inside a Plexi terminal pane");
            eprintln!("hint: open Plexi, then run this command from a pane");
            return 1;
        }
    };
    let my_pane_id: u64 = match pane_id_str.parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
            return 1;
        }
    };

    log::info!("demo_cli: starting interactive tutorial for pane_id={my_pane_id}");

    let events_path = crate::config::config_dir().join("events.jsonl");
    let start_offset = file_offset(&events_path).unwrap_or(0);

    print_intro();

    step(1, "Split right");
    explain(&["Press Command-D. Plexi splits this pane to the right and focuses the new pane."]);
    key(&format!("{CMD}D"), "split right");
    let mut right_pane_id = 0;
    let after_right_split = match poll_event(&events_path, start_offset, |kind, obj| {
        capture_pane_split(kind, obj, &mut right_pane_id)
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(1);

    step(2, "Split below");
    explain(&[
        "The pane on the right should be focused now.",
        "Press Command-Shift-D to split that pane below.",
    ]);
    key(&format!("{CMD}+Shift+D"), "split below");
    let mut lower_pane_id = 0;
    let after_lower_split = match poll_event(&events_path, after_right_split, |kind, obj| {
        capture_pane_split(kind, obj, &mut lower_pane_id)
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(2);

    step(3, "Move with HJKL");
    explain(&[
        "Hold Command and use H J K L to move between panes.",
        "Your hand naturally rests there during normal typing. Arrow keys work too, but HJKL is worth learning if you spend the day in panes.",
        "Move around for a moment. Focus each pane at least twice, then come back to this demo pane and press Enter.",
    ]);
    eprintln!("        left  down   up  right");
    eprintln!("          <-    v     ^     ->");
    eprintln!("          H     J     K     L");
    eprintln!();
    wait_for_enter();
    done(3);
    let after_move = file_offset(&events_path).unwrap_or(after_lower_split);

    step(4, "Close the extra panes");
    explain(&[
        "Close the two panes you created with Command-W.",
        "Do not close this demo pane. Move back here after each close if you need to.",
    ]);
    key(&format!("{CMD}W"), "close the focused extra pane");
    let mut closed_right = false;
    let mut closed_lower = false;
    let after_close = match poll_event(&events_path, after_move, |kind, obj| {
        if kind != "pane_closed" {
            return false;
        }
        let Some(id) = obj.get("pane_id").and_then(|v| v.as_u64()) else {
            return false;
        };
        if id == right_pane_id {
            closed_right = true;
        }
        if id == lower_pane_id {
            closed_lower = true;
        }
        closed_right && closed_lower
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(4);

    step(5, "Open an app from the palette");
    explain(&[
        "Split to a fresh terminal, open the command palette, type Balls, and choose the Balls app.",
        "The app should take over that pane. This demo waits for the Balls app to spawn.",
    ]);
    key(&format!("{CMD}D"), "open a new terminal pane");
    key(&format!("{CMD}P"), "open the command palette");
    eprintln!("    Type Balls, choose the Balls app, and press Enter.");
    eprintln!();
    let mut balls_pane_id = 0;
    let after_balls = match poll_event(&events_path, after_close, |kind, obj| {
        if kind != "app_spawned"
            || !obj
                .get("type_id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id == "balls")
        {
            return false;
        }
        if let Some(id) = obj.get("pane_id").and_then(|v| v.as_u64()) {
            balls_pane_id = id;
            true
        } else {
            false
        }
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(5);

    step(6, "Return to the terminal");
    explain(&[
        "Congrats, you can press Escape or Command-W to exit and return back to the regular terminal pane.",
    ]);
    key("Esc", "exit the app");
    let after_balls_close = match poll_event(&events_path, after_balls, |kind, obj| {
        kind == "app_closed"
            && obj
                .get("type_id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id == "balls")
            && obj
                .get("pane_id")
                .and_then(|v| v.as_u64())
                .is_some_and(|id| id == balls_pane_id)
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(6);

    step(7, "Rename this app pane");
    explain(&[
        "You should be back in the terminal that launched Balls.",
        "Rename it so the pane list is easier to scan.",
        "Use the rename shortcut, enter Demo app pane, and confirm it.",
    ]);
    key(&format!("{CMD}R"), "rename this pane");
    let after_app_rename = match poll_event(&events_path, after_balls_close, |kind, obj| {
        kind == "pane_renamed"
            && obj
                .get("pane_id")
                .and_then(|v| v.as_u64())
                .is_some_and(|pid| pid == balls_pane_id)
            && obj
                .get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|name| name == "Demo app pane")
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(7);

    step(8, "Open File Browser");
    explain(&[
        "File Browser opens on the current pane.",
        "Use H J K L, or the arrow keys, to move through the file tree and view files.",
    ]);
    key(&format!("{CMD}E"), "open File Browser");
    let after_file = match poll_event(&events_path, after_app_rename, |kind, obj| {
        kind == "app_spawned"
            && obj
                .get("type_id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id == "file_browser")
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(8);

    step(9, "Split below File Browser");
    explain(&[
        "Keep File Browser open and add a terminal underneath it.",
        "The new terminal should be focused after the split.",
    ]);
    key(&format!("{CMD}+Shift+D"), "split below");
    let mut note_terminal_pane_id = 0;
    let after_file_split = match poll_event(&events_path, after_file, |kind, obj| {
        capture_pane_split(kind, obj, &mut note_terminal_pane_id)
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(9);

    step(10, "Write a scratch note");
    explain(&[
        "Scratch notes land in your Plexi notes inbox.",
        "Open one from the terminal under File Browser, type a short note, then press Escape to leave it.",
    ]);
    key(&format!("{CMD}+Shift+Space"), "open a scratch note");
    let mut scratch_note_path = String::new();
    let after_scratch_open = match poll_event(&events_path, after_file_split, |kind, obj| {
        if kind != "scratchpad_opened" {
            return false;
        }
        if let Some(path) = obj.get("path").and_then(|v| v.as_str()) {
            scratch_note_path = path.to_string();
            true
        } else {
            false
        }
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    let after_scratch_close = match poll_event(&events_path, after_scratch_open, |kind, obj| {
        kind == "app_closed"
            && obj
                .get("type_id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id == "text-editor")
            && obj
                .get("pane_id")
                .and_then(|v| v.as_u64())
                .is_some_and(|id| id == note_terminal_pane_id)
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(10);

    step(11, "Open your notes");
    explain(&[
        "From any terminal, Command-O opens your notes.",
        "Press Enter in the picker to return to the scratch note you just wrote.",
    ]);
    key(&format!("{CMD}O"), "open your notes");
    let after_notes_picker = match poll_event(&events_path, after_scratch_close, |kind, _obj| {
        kind == "notes_picker_opened"
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    let _after_note_open = match poll_event(&events_path, after_notes_picker, |kind, obj| {
        kind == "note_opened"
            && obj
                .get("path")
                .and_then(|v| v.as_str())
                .is_some_and(|path| path == scratch_note_path)
    }) {
        Ok(offset) => offset,
        Err(e) => return watch_error(&events_path, e),
    };
    done(11);

    eprintln!("  More to cover");
    explain(&[
        "Here is where the full intro link will go when I actually get around to making it.",
    ]);
    eprintln!();
    log::info!("demo_cli: tutorial completed for pane_id={my_pane_id}");
    0
}

fn print_intro() {
    eprintln!("\x1b[1;36mPlexi - Quick Tutorial\x1b[0m");
    explain(&[
        "Plexi is a local app workspace for terminals, app panes, notes, contexts, notifications, and agent-driven work.",
        "This demo watches Plexi events and advances as you try the core host controls. Keep this terminal visible when you can.",
    ]);
}

fn step(step: u8, title: &str) {
    print_step_header(step, TOTAL_STEPS, title);
    progress(step - 1);
}

fn done(step: u8) {
    print_step_complete(step, TOTAL_STEPS);
}

fn progress(done: u8) {
    let mut dots = String::with_capacity(TOTAL_STEPS as usize);
    for idx in 0..TOTAL_STEPS {
        dots.push(if idx < done { '●' } else { '○' });
    }
    eprintln!("    \x1b[2m{dots} {done}/{TOTAL_STEPS}\x1b[0m");
    eprintln!();
}

fn explain(lines: &[&str]) {
    for line in lines {
        wrapped("    ", line);
    }
    eprintln!();
}

fn key(keys: &str, action: &str) {
    wrapped("    ", &format!("Press [ {keys} ] to {action}."));
    eprintln!();
}

fn wrapped(prefix: &str, text: &str) {
    wrapped_styled(prefix, "", text, "");
}

fn wrapped_styled(prefix: &str, style_start: &str, text: &str, style_end: &str) {
    let width = terminal_width().clamp(46, 72);
    let content_width = width.saturating_sub(visible_len(prefix)).max(24);
    let mut line = String::new();
    for word in text.split_whitespace() {
        let next_len = if line.is_empty() {
            visible_len(word)
        } else {
            visible_len(&line) + 1 + visible_len(word)
        };
        if next_len > content_width && !line.is_empty() {
            eprintln!("{prefix}{style_start}{line}{style_end}");
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        eprintln!("{prefix}{style_start}{line}{style_end}");
    }
}

fn wait_for_enter() {
    use std::io::BufRead;
    let _ = std::io::stdin().lock().lines().next();
}

fn terminal_width() -> usize {
    terminal_width_from_tty().unwrap_or_else(|| {
        std::env::var("COLUMNS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(72)
    })
}

#[cfg(unix)]
fn terminal_width_from_tty() -> Option<usize> {
    use std::os::fd::AsRawFd;

    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let fd = std::io::stderr().as_raw_fd();
    let rc = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) };
    if rc == 0 && size.ws_col > 0 {
        Some(size.ws_col as usize)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn terminal_width_from_tty() -> Option<usize> {
    None
}

fn visible_len(text: &str) -> usize {
    let mut len = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for code_ch in chars.by_ref() {
                if code_ch.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            len += 1;
        }
    }
    len
}

fn file_offset(path: &std::path::Path) -> Option<u64> {
    std::fs::metadata(path).map(|m| m.len()).ok()
}

fn capture_pane_split(kind: &str, obj: &serde_json::Value, out: &mut u64) -> bool {
    if kind == "pane_split" {
        if let Some(id) = obj.get("pane_id").and_then(|v| v.as_u64()) {
            *out = id;
            return true;
        }
    }
    false
}

fn watch_error(path: &std::path::Path, error: std::io::Error) -> i32 {
    eprintln!("error watching {}: {error}", path.display());
    1
}

/// Tails `path` from `offset`, advancing the cursor as lines are consumed.
/// Returns the byte offset immediately after the matched line when the predicate fires.
/// Handles missing files gracefully; only processes complete newline-terminated lines.
fn poll_event<F>(path: &std::path::Path, mut offset: u64, mut predicate: F) -> std::io::Result<u64>
where
    F: FnMut(&str, &serde_json::Value) -> bool,
{
    use std::io::{Read, Seek, SeekFrom};
    loop {
        match std::fs::File::open(path) {
            Ok(mut f) => {
                let file_len = f.seek(SeekFrom::End(0))?;
                if file_len > offset {
                    f.seek(SeekFrom::Start(offset))?;
                    let mut buf = String::new();
                    f.read_to_string(&mut buf)?;
                    let process_len = match buf.rfind('\n') {
                        Some(pos) => pos + 1,
                        None => {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            continue;
                        }
                    };
                    let complete = &buf[..process_len];
                    let mut byte_pos: u64 = 0;
                    for line in complete.split_inclusive('\n') {
                        let line_bytes = line.len() as u64;
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(trimmed) {
                                if let Some(kind) = obj.get("kind").and_then(|v| v.as_str()) {
                                    if predicate(kind, &obj) {
                                        return Ok(offset + byte_pos + line_bytes);
                                    }
                                }
                            }
                        }
                        byte_pos += line_bytes;
                    }
                    offset += process_len as u64;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
