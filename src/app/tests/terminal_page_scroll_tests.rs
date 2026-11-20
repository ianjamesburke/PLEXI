//! Shift+PageUp/PageDown scroll the scrollback on the main screen, including
//! when the program has enabled the kitty keyboard protocol (Claude Code,
//! pi). These drive a real PTY child (python3 fixture) through the
//! production `RawInput` path via `HostHarness`.

use std::time::{Duration, Instant};

use crate::host::pane::Pane;
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;

/// Prints 200 lines, optionally after pushing kitty keyboard flags (as
/// Claude Code and pi do; `kitty-types` also requests release events) and
/// optionally on the alternate screen, then idles so the grid stays put.
const FIXTURE: &str = r#"
import sys, time
if sys.argv[1] == "kitty-alt":
    sys.stdout.write("\x1b[?1049h")
if sys.argv[1] in ("kitty", "kitty-alt"):
    sys.stdout.write("\x1b[>1u")
if sys.argv[1] == "kitty-types":
    sys.stdout.write("\x1b[>3u")
for i in range(200):
    print(f"line{i}")
print("FILL_DONE", flush=True)
time.sleep(120)
"#;

pub(super) fn terminal_with_fixture(h: &mut HostHarness, mode: &str) -> PaneId {
    let id = h.add_focused_terminal();
    let terminal = crate::host::pane::TerminalPane::fixture(
        id,
        h.app.ctx.clone(),
        h.app.pty_event_tx.clone(),
        egui_term::BackendSettings {
            shell: "/usr/bin/python3".into(),
            args: vec!["-c".into(), FIXTURE.into(), mode.into()],
            working_directory: Some(std::env::temp_dir()),
            ..Default::default()
        },
        14.0,
    )
    .expect("start page-scroll fixture");
    h.app.windows[0]
        .panes
        .insert(id, Pane::Terminal(Box::new(terminal)))
        .unwrap();
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_frames(2);
        if h.terminal_backend(id)
            .capture_lines(400)
            .iter()
            .any(|line| line.contains("FILL_DONE"))
        {
            break;
        }
        assert!(Instant::now() < deadline, "page-scroll fixture timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
    h.run_frames(3);
    id
}

/// egui marks a press of a key it still holds as a repeat, so a test that
/// presses the same key twice must release it in between, as a user does.
pub(super) fn release_key(h: &mut HostHarness, key: egui::Key, modifiers: egui::Modifiers) {
    h.frame(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        modifiers,
        events: vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        }],
        ..Default::default()
    });
}

fn display_offset(h: &mut HostHarness, id: PaneId) -> usize {
    h.terminal_backend(id).sync().grid.display_offset()
}

#[test]
#[ignore = "requires-pty"]
fn shift_page_up_scrolls_terminal_scrollback() {
    let mut h = HostHarness::new();
    let id = terminal_with_fixture(&mut h, "plain");
    assert_eq!(display_offset(&mut h, id), 0, "starts at the bottom");
    h.terminal_backend(id).enable_input_tap();
    h.press_key(egui::Key::PageUp, egui::Modifiers::SHIFT);
    h.run_frames(2);
    let offset = display_offset(&mut h, id);
    let tap = h.terminal_backend(id).take_input_tap();
    assert!(
        offset > 0,
        "Shift+PageUp must scroll the scrollback; display_offset={offset}, pty bytes={tap:?}"
    );
    h.press_key(egui::Key::PageDown, egui::Modifiers::SHIFT);
    h.run_frames(2);
    assert_eq!(
        display_offset(&mut h, id),
        0,
        "Shift+PageDown returns to the bottom"
    );
}

#[test]
#[ignore = "requires-pty"]
fn bare_page_up_reaches_the_pty() {
    let mut h = HostHarness::new();
    let id = terminal_with_fixture(&mut h, "plain");
    h.terminal_backend(id).enable_input_tap();
    h.press_key(egui::Key::PageUp, egui::Modifiers::NONE);
    h.run_frames(2);
    let tap = h.terminal_backend(id).take_input_tap();
    assert_eq!(
        tap, b"\x1b[5~",
        "bare PageUp must be written to the PTY as CSI 5~"
    );
}

/// Same as the first test, but the child has pushed kitty keyboard flags.
/// Asserts the user-facing expectation (Shift+PageUp scrolls the host
/// scrollback, as kitty/WezTerm/Ghostty do); a failure shows the key was
/// encoded and handed to the program instead.
#[test]
#[ignore = "requires-pty"]
fn shift_page_up_scrolls_scrollback_with_kitty_keyboard_active() {
    let mut h = HostHarness::new();
    let id = terminal_with_fixture(&mut h, "kitty");
    assert!(
        h.terminal_backend(id)
            .sync()
            .terminal_mode
            .intersects(egui_term::TerminalMode::KITTY_KEYBOARD_PROTOCOL),
        "fixture must have enabled the kitty keyboard protocol"
    );
    h.terminal_backend(id).enable_input_tap();
    // No binding claims Cmd+PageUp, so the program still gets it encoded.
    let cmd = egui::Modifiers {
        mac_cmd: true,
        command: true,
        ..egui::Modifiers::NONE
    };
    h.press_key(egui::Key::PageUp, cmd);
    release_key(&mut h, egui::Key::PageUp, cmd);
    h.run_frames(2);
    assert_eq!(
        String::from_utf8_lossy(&h.terminal_backend(id).take_input_tap()),
        "\x1b[5;9~"
    );
    assert_eq!(display_offset(&mut h, id), 0);
    h.press_key(egui::Key::PageUp, egui::Modifiers::SHIFT);
    h.run_frames(2);
    let offset = display_offset(&mut h, id);
    let tap = h.terminal_backend(id).take_input_tap();
    assert!(
        offset > 0,
        "Shift+PageUp under kitty keyboard mode must scroll the scrollback; \
         display_offset={offset}, pty bytes={:?}",
        String::from_utf8_lossy(&tap)
    );
    assert!(tap.is_empty(), "the program must not see the scroll key");
}

/// On the alternate screen the scroll binding does not apply, so the kitty
/// program still receives Shift+PageUp.
#[test]
#[ignore = "requires-pty"]
fn shift_page_up_reaches_kitty_program_on_alternate_screen() {
    let mut h = HostHarness::new();
    let id = terminal_with_fixture(&mut h, "kitty-alt");
    let mode = h.terminal_backend(id).sync().terminal_mode;
    assert!(
        mode.contains(egui_term::TerminalMode::ALT_SCREEN)
            && mode.intersects(egui_term::TerminalMode::KITTY_KEYBOARD_PROTOCOL),
        "fixture must be on the alternate screen with kitty keyboard on: {mode:?}"
    );
    h.terminal_backend(id).enable_input_tap();
    h.press_key(egui::Key::PageUp, egui::Modifiers::SHIFT);
    h.run_frames(2);
    assert_eq!(
        String::from_utf8_lossy(&h.terminal_backend(id).take_input_tap()),
        "\x1b[5;2~"
    );
    assert_eq!(display_offset(&mut h, id), 0);
}
