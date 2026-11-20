//! Stint 1391: Cmd+P input ownership through the production `RawInput` path.
//! Each delivery is one egui pass holding the listed events in order.

use crate::host::pane::{Pane, ViewRuntime};
use crate::host::picker::PickerId;
use crate::overlays::command_palette::palette_test_log;
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;
use egui::{Event, Key, Modifiers, RawInput};

const KEYBOARD_ROW: &str = "commands:cmd/Keyboard shortcuts";

fn screen() -> Option<egui::Rect> {
    Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1280.0, 800.0),
    ))
}

fn deliver(h: &mut HostHarness, events: Vec<Event>) {
    h.frame(RawInput {
        screen_rect: screen(),
        events,
        ..Default::default()
    });
}

fn press(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn release(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers,
    }
}

fn open_palette(h: &mut HostHarness) {
    deliver(
        h,
        vec![
            press(Key::P, Modifiers::COMMAND),
            release(Key::P, Modifiers::COMMAND),
        ],
    );
    h.run_frames(3);
    assert!(h.app.command_palette_open(), "Cmd+P opens the palette");
    palette_test_log::take_activations();
}

fn row_titles(h: &HostHarness) -> Vec<String> {
    h.app
        .host
        .picker(PickerId::Palette)
        .expect("palette picker")
        .rows()
        .iter()
        .map(|row| row.title.clone())
        .collect()
}

/// Stint 1391: an Enter waiting on its query's rows completes on a hidden
/// pass alone (`App::logic`), runs once, and the window never has to draw.
#[test]
fn command_palette_pending_enter_completes_on_a_hidden_pass() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(2);
    open_palette(&mut h);
    deliver(
        &mut h,
        vec![
            Event::Paste("keyboard".into()),
            press(Key::Enter, Modifiers::NONE),
        ],
    );
    h.hidden_frame();
    assert_eq!(
        palette_test_log::take_activations(),
        vec![KEYBOARD_ROW.to_owned()]
    );
    assert!(!h.app.command_palette_open());
    h.run_hidden_frames(3);
    h.run_frames(2);
    assert!(palette_test_log::take_activations().is_empty(), "runs once");
}

/// Stint 1391 review round 1: editing and drawing never ask the source for
/// rows; each logic pass answers the current query once.
#[test]
fn command_palette_source_runs_only_from_logic() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(2);
    open_palette(&mut h);
    palette_test_log::take_source_calls();
    deliver(&mut h, vec![Event::Paste("keyboard".into())]);
    assert_eq!(
        palette_test_log::take_source_calls(),
        1,
        "one logic answer per pass; the edit and the draw make none"
    );
    h.hidden_frame();
    assert_eq!(palette_test_log::take_source_calls(), 1);
    let picker = h
        .app
        .host
        .picker(PickerId::Palette)
        .expect("palette picker");
    assert_eq!(picker.query(), "keyboard");
    assert_eq!(
        picker.rows().first().map(|row| row.key.as_str()),
        Some(KEYBOARD_ROW),
        "the hidden pass answered the edited query"
    );
}

/// Stint 1391: the Cmd+P that opens the palette is the boundary of its
/// delivery; what follows it in the same delivery is the palette's, in order.
#[test]
fn command_palette_opening_chord_owns_the_rest_of_its_delivery() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(2);
    deliver(
        &mut h,
        vec![
            press(Key::P, Modifiers::COMMAND),
            Event::Paste("keyboard".into()),
            press(Key::Enter, Modifiers::NONE),
        ],
    );
    h.run_frames(3);
    assert_eq!(
        palette_test_log::take_activations(),
        vec![KEYBOARD_ROW.to_owned()],
        "the paste and Enter after Cmd+P ran in the palette it opened"
    );
}

/// Stint 1391: an Enter that completes in the next pass's logic closes the
/// session before that pass's input is routed. That input was the session's:
/// a host chord in it (Cmd+T) is consumed, never run against the pane that
/// regains focus.
#[test]
fn command_palette_session_closed_in_logic_consumes_its_delivery() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(2);
    open_palette(&mut h);
    deliver(
        &mut h,
        vec![
            Event::Paste("reset text size".into()),
            press(Key::Enter, Modifiers::NONE),
        ],
    );
    assert!(
        h.app.command_palette_open(),
        "the Enter waits for its answer"
    );
    let panes = h.pane_count();
    deliver(
        &mut h,
        vec![
            press(Key::T, Modifiers::COMMAND),
            release(Key::T, Modifiers::COMMAND),
        ],
    );
    h.run_frames(2);
    assert!(!h.app.command_palette_open());
    assert_eq!(palette_test_log::take_activations().len(), 1);
    assert_eq!(
        h.pane_count(),
        panes,
        "Cmd+T in the closing pass was consumed"
    );
}

/// Stint 1391 review round 1: text after Enter belongs to the closed
/// session, including the next delivery when the Enter completes in that
/// pass's logic. The terminal behind the palette receives none of it, and does
/// receive input once focus is back with it.
#[test]
fn command_palette_input_never_leaks_to_the_terminal_behind_it() {
    let mut h = HostHarness::new();
    let term = h.add_focused_terminal();
    h.terminal_backend(term).enable_input_tap();
    for (label, events) in [
        (
            "Enter, Paste",
            vec![
                press(Key::Enter, Modifiers::NONE),
                Event::Paste("keyboard".into()),
            ],
        ),
        (
            "Enter, IME commit",
            vec![
                press(Key::Enter, Modifiers::NONE),
                Event::Ime(egui::ImeEvent::Enabled),
                Event::Ime(egui::ImeEvent::Commit("keyboard".into())),
                Event::Ime(egui::ImeEvent::Disabled),
            ],
        ),
        // The Enter waits for its answer, which lands in the next pass's
        // logic; that pass's paste still belongs to the closing session.
        // The row opens no overlay, so focus is back on the terminal when
        // that paste is delivered.
        (
            "Paste, Enter | Paste",
            vec![
                Event::Paste("reset text size".into()),
                press(Key::Enter, Modifiers::NONE),
            ],
        ),
    ] {
        h.focus_pane(term);
        h.run_frames(2);
        open_palette(&mut h);
        h.terminal_backend(term).take_input_tap();
        let deferred = h.app.command_palette_open() && label.contains('|');
        deliver(&mut h, events);
        if deferred {
            deliver(&mut h, vec![Event::Paste("leak".into())]);
        }
        h.run_frames(3);
        assert!(!h.app.command_palette_open(), "{label}: Enter ran a row");
        assert_eq!(
            palette_test_log::take_activations().len(),
            1,
            "{label}: one activation"
        );
        assert_eq!(
            String::from_utf8_lossy(&h.terminal_backend(term).take_input_tap()),
            "",
            "{label}: the terminal behind the palette received nothing"
        );
        if h.app.show_shortcuts {
            deliver(&mut h, vec![press(Key::Escape, Modifiers::NONE)]);
            h.run_frames(2);
        }
    }
    h.focus_pane(term);
    h.run_frames(3);
    h.terminal_backend(term).take_input_tap();
    deliver(&mut h, vec![Event::Paste("control".into())]);
    h.run_frames(2);
    let tap = String::from_utf8_lossy(&h.terminal_backend(term).take_input_tap()).into_owned();
    assert!(
        tap.contains("control"),
        "the tap sees a paste once focus is back with the terminal: {tap:?}"
    );
}

#[cfg(unix)]
mod note_identity {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    fn open_editor(h: &mut HostHarness, path: &Path) -> PaneId {
        let pane_id = h.add_test_pane();
        let root = h.workspace_root();
        let Some(Pane::View(view)) = h.app.windows[0].panes.get_mut(&pane_id) else {
            panic!("test pane is a view pane");
        };
        view.runtime = ViewRuntime::Builtin(Box::new(
            crate::app::text_editor_view::TextEditorView::new_with_workspace(
                path.to_path_buf(),
                Some(root),
            ),
        ));
        view.manifest_id = "text-editor".to_string();
        pane_id
    }

    /// `add_test_pane` attaches only the first pane to the tree; make every
    /// pane of window 0 a reachable tab so navigation can find it.
    fn attach_all_panes_as_tabs(h: &mut HostHarness) {
        let win = &mut h.app.windows[0];
        let tiles: Vec<_> = win
            .tree
            .tiles
            .iter()
            .filter(|(_, tile)| matches!(tile, egui_tiles::Tile::Pane(_)))
            .map(|(id, _)| *id)
            .collect();
        win.tree.root = Some(win.tree.tiles.insert_tab_tile(tiles));
    }

    fn all_panes(h: &HostHarness) -> Vec<PaneId> {
        h.app
            .windows
            .iter()
            .flat_map(|win| win.panes.keys().copied())
            .collect()
    }

    fn open_note_path_of(h: &HostHarness, pane: PaneId) -> Option<Vec<u8>> {
        h.app.windows.iter().find_map(|win| {
            let view = win.panes.get(&pane)?.as_view()?;
            let ViewRuntime::Builtin(builtin) = &view.runtime else {
                return None;
            };
            builtin
                .open_note_path()
                .map(|path| path.as_os_str().as_bytes().to_vec())
        })
    }

    fn focused_pane(h: &HostHarness) -> Option<PaneId> {
        let win = &h.app.windows[h.app.active_window];
        match win.tree.tiles.get(win.focused_pane?)? {
            egui_tiles::Tile::Pane(id) => Some(*id),
            egui_tiles::Tile::Container(_) => None,
        }
    }

    /// Select the row titled `title` with arrows, then press Enter.
    fn run_row_titled(h: &mut HostHarness, title: &str) {
        let index = row_titles(h)
            .iter()
            .position(|row| row.ends_with(title))
            .unwrap_or_else(|| panic!("no row titled {title:?}: {:?}", row_titles(h)));
        let mut events: Vec<Event> = (0..index)
            .map(|_| press(Key::ArrowDown, Modifiers::NONE))
            .collect();
        events.push(press(Key::Enter, Modifiers::NONE));
        deliver(h, events);
        h.run_frames(3);
    }

    /// Two note names a display string cannot tell apart. APFS refuses
    /// names that are not UTF-8 (EILSEQ), so on macOS the twins are two
    /// UTF-8 names and only the end-to-end path is exercised; the lossy-key
    /// collision itself is covered by the row-key unit tests.
    fn twin_note_paths(tier: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let lossy = tier.join("n\u{FFFD}.md");
        if cfg!(target_os = "macos") {
            (tier.join("n\u{FFFD}\u{FFFD}.md"), lossy)
        } else {
            (tier.join(OsStr::from_bytes(b"n\xff.md")), lossy)
        }
    }

    /// Stint 1391 review rounds 1 and 2: two notes whose names differ only
    /// in bytes a display string loses (`n\xff.md` and the literal `n\u{FFFD}.md`)
    /// stay two rows that each open their own file, whether the note is
    /// still open (navigate) or was closed after Cmd+P opened (launch).
    #[test]
    fn command_palette_note_rows_keep_native_path_identity() {
        let mut h = HostHarness::new();
        let tier = h.workspace_root().join(".plexi").join("notes");
        std::fs::create_dir_all(&tier).expect("notes tier");
        let (raw, lossy) = twin_note_paths(&tier);
        std::fs::write(&raw, "# raw bytes note\n\nraw body\n").expect("raw note");
        std::fs::write(&lossy, "# lossy twin note\n\nlossy body\n").expect("lossy note");
        let notes = [
            (raw.clone(), "raw bytes note", "raw body"),
            (lossy.clone(), "lossy twin note", "lossy body"),
        ];

        let raw_pane = open_editor(&mut h, &raw);
        let lossy_pane = open_editor(&mut h, &lossy);
        let start = h.add_test_pane();
        attach_all_panes_as_tabs(&mut h);
        h.focus_pane(start);
        h.run_frames(3);

        for ((path, title, _), pane) in notes.iter().zip([raw_pane, lossy_pane]) {
            open_palette(&mut h);
            run_row_titled(&mut h, title);
            assert_eq!(palette_test_log::take_activations().len(), 1, "{title}");
            assert_eq!(focused_pane(&h), Some(pane), "{title}: its live pane");
            assert_eq!(
                open_note_path_of(&h, pane),
                Some(path.as_os_str().as_bytes().to_vec()),
                "{title}: the live editor's native path"
            );
            h.focus_pane(start);
            h.run_frames(2);
        }

        for ((path, title, body), pane) in notes.iter().zip([raw_pane, lossy_pane]) {
            if !h
                .app
                .windows
                .iter()
                .any(|win| win.panes.contains_key(&pane))
            {
                open_editor(&mut h, path);
            }
            h.focus_pane(start);
            h.run_frames(2);
            open_palette(&mut h);
            // Closed after Cmd+P listed it; its colliding sibling stays open.
            h.app.close_pane_by_id(pane);
            h.focus_pane(start);
            h.run_frames(1);
            run_row_titled(&mut h, title);
            assert_eq!(palette_test_log::take_activations().len(), 1, "{title}");
            // The listed pane is gone, so the pane editing this exact path
            // is the one Enter opened.
            let target = path.as_os_str().as_bytes().to_vec();
            let editing: Vec<PaneId> = all_panes(&h)
                .into_iter()
                .filter(|id| open_note_path_of(&h, *id).as_ref() == Some(&target))
                .collect();
            assert_eq!(editing.len(), 1, "{title}: Enter opened its own note");
            assert!(!editing.contains(&pane), "{title}: in a new pane");
            let bytes = target;
            let content = std::fs::read_to_string(OsStr::from_bytes(&bytes)).expect("note content");
            assert!(content.contains(body), "{title}: its own file's content");
        }
    }
}

fn native_cmd(key: Key, extra: Modifiers) -> Event {
    let mut modifiers = extra;
    modifiers.command = true;
    if cfg!(target_os = "macos") {
        modifiers.mac_cmd = true;
    } else {
        modifiers.ctrl = true;
    }
    press(key, modifiers)
}

fn pointer_edges_left(h: &HostHarness) -> usize {
    h.app.ctx.input(|i| {
        i.events
            .iter()
            .filter(|e| matches!(e, Event::PointerButton { .. } | Event::MouseWheel { .. }))
            .count()
    })
}

/// Review round 3: a host chord resolves at its own position, while the
/// palette still owns input. Cmd+B (a Normal chord) before the Escape that
/// closes the palette never acts; Cmd+Option+P is not the palette's chord.
#[test]
fn command_palette_normal_chord_waits_for_no_close() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(2);
    open_palette(&mut h);
    deliver(&mut h, vec![native_cmd(Key::P, Modifiers::ALT)]);
    assert!(
        h.app.command_palette_open(),
        "Cmd+Option+P is another global chord, not the palette's"
    );
    let sidebar = h.app.sidebar_visible;
    deliver(
        &mut h,
        vec![
            native_cmd(Key::B, Modifiers::NONE),
            press(Key::Escape, Modifiers::NONE),
        ],
    );
    assert!(!h.app.command_palette_open());
    assert_eq!(
        h.app.sidebar_visible, sidebar,
        "Cmd+B arrived while the palette owned input"
    );
}

/// Review round 3: closing consumes the rest of the session's delivery,
/// pointer edges included, whether Escape closes it or a deferred Enter
/// completes in logic.
#[test]
fn command_palette_closing_consumes_pointer_edges() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(2);
    let at = egui::pos2(640.0, 400.0);
    let click = |pressed| Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    open_palette(&mut h);
    deliver(
        &mut h,
        vec![
            press(Key::Escape, Modifiers::NONE),
            click(true),
            click(false),
        ],
    );
    assert!(!h.app.command_palette_open());
    assert_eq!(
        pointer_edges_left(&h),
        0,
        "Escape: the clicks were the palette's"
    );

    open_palette(&mut h);
    deliver(
        &mut h,
        vec![
            Event::Paste("reset text size".into()),
            press(Key::Enter, Modifiers::NONE),
        ],
    );
    deliver(&mut h, vec![click(true), click(false)]);
    assert!(!h.app.command_palette_open());
    assert_eq!(
        pointer_edges_left(&h),
        0,
        "closed in logic: the clicks were still the palette's"
    );
}

/// Stint 1391: in one delivery, text before the Cmd+P that opens the palette
/// is the terminal's, and text after it is the palette's query. Neither the
/// opening nor the closing Escape lets a key reach the terminal behind it.
#[test]
fn command_palette_opening_splits_the_delivery_at_its_chord() {
    let mut h = HostHarness::new();
    let term = h.add_focused_terminal();
    h.terminal_backend(term).enable_input_tap();
    h.focus_pane(term);
    h.run_frames(3);
    h.terminal_backend(term).take_input_tap();
    deliver(
        &mut h,
        vec![
            Event::Text("a".into()),
            press(Key::P, Modifiers::COMMAND),
            Event::Text("b".into()),
        ],
    );
    h.run_frames(2);
    assert!(h.app.command_palette_open());
    let query = h
        .app
        .host
        .picker(PickerId::Palette)
        .expect("palette picker")
        .query()
        .to_owned();
    assert_eq!(query, "b", "the text after Cmd+P is the query");
    deliver(
        &mut h,
        vec![press(Key::Escape, Modifiers::NONE), Event::Text("c".into())],
    );
    h.run_frames(2);
    assert!(!h.app.command_palette_open());
    assert_eq!(
        String::from_utf8_lossy(&h.terminal_backend(term).take_input_tap()),
        "a",
        "only the text before Cmd+P reached the terminal"
    );
}
