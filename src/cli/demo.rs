use super::notes::{print_step_complete, print_step_header};

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

    // Seek to end — only watch events that occur after demo starts.
    let start_offset = match std::fs::metadata(&events_path) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => {
            log::warn!("demo_cli: could not read events file metadata: {e}");
            0
        }
    };

    // Welcome banner
    eprintln!("\x1b[1;36m");
    eprintln!("  Plexi — Quick Tutorial");
    eprintln!("\x1b[0m");
    eprintln!("  Seven moves. That's all you need to know.");
    eprintln!();

    // Step 1 — split
    print_step_header(1, 7, "Split a pane");
    eprintln!("    Press  \x1b[1m[ \u{2318}D ]\x1b[0m  to split the current pane.");
    eprintln!();

    // Capture the new pane's ID from the split event so step 2 can verify
    // focus specifically returns from that pane (not a bounce from the split itself).
    let mut split_pane_id: u64 = 0;
    let after_split_offset = match poll_event(&events_path, start_offset, |kind, obj| {
        if kind == "pane_split" {
            if let Some(id) = obj.get("pane_id").and_then(|v| v.as_u64()) {
                split_pane_id = id;
                return true;
            }
        }
        false
    }) {
        Ok(offset) => offset,
        Err(e) => {
            eprintln!("error watching {}: {e}", events_path.display());
            return 1;
        }
    };
    print_step_complete(1, 7);

    // Step 2 — navigate
    print_step_header(2, 7, "Navigate panes");
    eprintln!("       \x1b[2m^\x1b[0m");
    eprintln!("       K");
    eprintln!("    H     L");
    eprintln!("       J");
    eprintln!();
    eprintln!("    Press  \x1b[1m[ \u{2318}H ]\x1b[0m  to return to this pane, then  \x1b[1m[ \u{2318}L ]\x1b[0m  to go back.");
    eprintln!();

    // Require a confirmed round-trip between split_pane and demo pane.
    // After the split, focus is on split_pane_id. The valid event sequence is:
    //   focus_changed(split_pane_id)  — user left split pane via ⌘H
    //   focus_changed(my_pane_id)     — user left demo pane via ⌘L
    // Any other pane ID appearing between these two resets the state so that
    // a second ⌘D split cannot satisfy the round-trip without actual navigation.
    let mut saw_split_depart = false;
    let after_nav_offset = match poll_event(&events_path, after_split_offset, |kind, obj| {
        if kind != "focus_changed" {
            return false;
        }
        let Some(pid) = obj.get("pane_id").and_then(|v| v.as_u64()) else {
            return false;
        };
        if !saw_split_depart {
            if pid == split_pane_id {
                saw_split_depart = true;
            }
            false
        } else if pid == my_pane_id {
            true
        } else {
            saw_split_depart = pid == split_pane_id;
            false
        }
    }) {
        Ok(offset) => offset,
        Err(e) => {
            eprintln!("error watching {}: {e}", events_path.display());
            return 1;
        }
    };
    print_step_complete(2, 7);

    // Step 3 — close pane
    print_step_header(3, 7, "Close a pane");
    eprintln!("    Press  \x1b[1m[ \u{2318}W ]\x1b[0m  to close the split pane.");
    eprintln!();
    let after_close_offset = match poll_event(&events_path, after_nav_offset, |kind, obj| {
        if kind == "pane_closed" {
            if let Some(id) = obj.get("pane_id").and_then(|v| v.as_u64()) {
                return id == split_pane_id;
            }
        }
        false
    }) {
        Ok(offset) => offset,
        Err(e) => {
            eprintln!("error watching {}: {e}", events_path.display());
            return 1;
        }
    };
    print_step_complete(3, 7);

    // Step 4 — open an app
    print_step_header(4, 7, "Open an app");
    eprintln!("    Open a new split (\x1b[1m\u{2318}D\x1b[0m), then in the new pane run:");
    eprintln!("    \x1b[1mplexi open balls\x1b[0m");
    eprintln!();
    let after_app_offset = match poll_event(&events_path, after_close_offset, |kind, _| {
        kind == "app_spawned"
    }) {
        Ok(offset) => offset,
        Err(e) => {
            eprintln!("error watching {}: {e}", events_path.display());
            return 1;
        }
    };
    print_step_complete(4, 7);

    // Step 5 — send a notification
    print_step_header(5, 7, "Send a notification");
    eprintln!("    In any pane, run:");
    eprintln!("    \x1b[1mplexi notify --title \"Hello\"\x1b[0m");
    eprintln!();
    let after_notify_offset = match poll_event(&events_path, after_app_offset, |kind, _| {
        kind == "notification_posted"
    }) {
        Ok(offset) => offset,
        Err(e) => {
            eprintln!("error watching {}: {e}", events_path.display());
            return 1;
        }
    };
    print_step_complete(5, 7);

    // Step 6 — scaffold a new app (keypress-advance; app init has no event log path)
    print_step_header(6, 7, "Scaffold a new app");
    eprintln!("    In any pane, run:");
    eprintln!("    \x1b[1mplexi app init my-app\x1b[0m");
    eprintln!();
    eprintln!("    Then switch back here and press  \x1b[1m[Enter]\x1b[0m  to continue.");
    eprintln!();
    {
        use std::io::BufRead;
        let _ = std::io::stdin().lock().lines().next();
    }
    // Snapshot offset after keypress so step 7 only catches context_created events
    // that follow — not any that may have been emitted during steps 1–6.
    let after_app_init_offset = std::fs::metadata(&events_path)
        .map(|m| m.len())
        .unwrap_or(after_notify_offset);
    print_step_complete(6, 7);

    // Step 7 — create a new context
    print_step_header(7, 7, "Create a context");
    eprintln!("    In any pane, run:");
    eprintln!("    \x1b[1mplexi context new\x1b[0m");
    eprintln!();
    eprintln!("    Contexts are how you organise work in Plexi.");
    eprintln!();
    if let Err(e) = poll_event(&events_path, after_app_init_offset, |kind, _| {
        kind == "context_created"
    }) {
        eprintln!("error watching {}: {e}", events_path.display());
        return 1;
    }

    eprintln!("  \x1b[1;32m\u{2713} 7/7 \u{2014} Plexi is yours.\x1b[0m");
    eprintln!();
    log::info!("demo_cli: tutorial completed for pane_id={my_pane_id}");
    0
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
                    // Only process lines up to the last newline to avoid partial writes.
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
