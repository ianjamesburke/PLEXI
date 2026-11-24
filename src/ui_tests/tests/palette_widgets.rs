//! Stint 1391, V3 steps 2 and 3: the palette's field and rows are egui's own
//! widgets. Editing, selection and clicks behave as in every other field and
//! list, through the production path (`PlexiUiHarness::step`).

use super::*;
use crate::overlays::command_palette::palette_test_log;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use std::cell::RefCell;
use std::rc::Rc;

fn deliver(h: &mut PlexiUiHarness, events: Vec<Event>) {
    h.harness().input_mut().events.extend(events);
    h.step();
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

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    }
}

fn palette_open(h: &PlexiUiHarness) -> bool {
    h.with_app(|app| app.command_palette_open())
}

fn open_palette(h: &mut PlexiUiHarness) {
    if palette_open(h) {
        deliver(h, vec![press(Key::Escape, Modifiers::NONE)]);
        h.run_steps(2);
    }
    h.harness().key_press_modifiers(Modifiers::COMMAND, Key::P);
    h.run_steps(3);
    assert!(palette_open(h), "Cmd+P opened the palette");
    palette_test_log::take_activations();
}

fn draft(h: &PlexiUiHarness) -> String {
    h.with_app(|app| app.palette_input.draft.clone())
}

fn query(h: &PlexiUiHarness) -> String {
    h.with_app(|app| {
        app.host
            .picker(crate::host::picker::PickerId::Palette)
            .map(|picker| picker.query().to_owned())
            .unwrap_or_default()
    })
}

fn field_rect(h: &mut PlexiUiHarness) -> egui::Rect {
    h.harness()
        .ctx
        .read_response(egui::Id::new("palette_search"))
        .expect("the palette field drew")
        .rect
}

/// Type `text` and let its answer land, so the modal has settled where
/// the person sees it.
fn typed(h: &mut PlexiUiHarness, text: &str) {
    deliver(h, vec![Event::Text(text.into())]);
    h.run_steps(4);
}

/// A click at `at` with `modifiers` held, as one delivery.
fn click(h: &mut PlexiUiHarness, at: Pos2, modifiers: Modifiers) {
    h.harness().input_mut().modifiers = modifiers;
    deliver(
        h,
        vec![
            Event::PointerMoved(at),
            button(at, true, modifiers),
            button(at, false, modifiers),
        ],
    );
    h.harness().input_mut().modifiers = Modifiers::NONE;
}

fn palette_harness() -> PlexiUiHarness {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    add_focused_pane(&mut h);
    h.step();
    h
}

/// Round 6 finding 4 and V3 step 2: double click selects a word, triple
/// click the line, Shift-click extends from the caret, and typing replaces
/// the selection, as in every egui text field.
#[test]
fn palette_field_selects_by_pointer_like_every_text_field() {
    let mut h = palette_harness();

    open_palette(&mut h);
    typed(&mut h, "reset text size");
    let rect = field_rect(&mut h);
    let end = egui::pos2(rect.right() - 12.0, rect.center().y);
    click(&mut h, end, Modifiers::NONE);
    click(&mut h, end, Modifiers::NONE);
    h.run_steps(1);
    typed(&mut h, "font");
    assert_eq!(draft(&h), "reset text font", "double click selected a word");

    open_palette(&mut h);
    typed(&mut h, "reset text size");
    let rect = field_rect(&mut h);
    let end = egui::pos2(rect.right() - 12.0, rect.center().y);
    click(&mut h, end, Modifiers::NONE);
    click(&mut h, end, Modifiers::NONE);
    click(&mut h, end, Modifiers::NONE);
    h.run_steps(1);
    typed(&mut h, "x");
    assert_eq!(draft(&h), "x", "triple click selected the line");

    open_palette(&mut h);
    typed(&mut h, "abc");
    let rect = field_rect(&mut h);
    let end = egui::pos2(rect.right() - 12.0, rect.center().y);
    let start = egui::pos2(rect.left() + 2.0, rect.center().y);
    click(&mut h, start, Modifiers::NONE);
    h.run_steps(30);
    click(&mut h, end, Modifiers::SHIFT);
    h.run_steps(1);
    typed(&mut h, "x");
    assert_eq!(draft(&h), "x", "Shift-click extended the selection");
    assert_eq!(query(&h), "x", "the committed query is the field's text");
}

/// Round 6 finding 4 and V3 step 2: platform keys, Backspace and undo are
/// egui's own.
#[test]
fn palette_field_keys_behave_like_every_text_field() {
    let mut h = palette_harness();

    // The harness reports a generic OS; Control+A is a macOS finding.
    h.harness().ctx.set_os(egui::os::OperatingSystem::Mac);
    open_palette(&mut h);
    typed(&mut h, "abc");
    deliver(&mut h, vec![press(Key::A, Modifiers::CTRL)]);
    typed(&mut h, "x");
    assert_eq!(draft(&h), "xabc", "macOS Control+A moves to the line start");

    open_palette(&mut h);
    deliver(&mut h, vec![Event::Paste("    ".into())]);
    h.run_steps(1);
    deliver(&mut h, vec![press(Key::Backspace, Modifiers::NONE)]);
    h.run_steps(1);
    assert_eq!(draft(&h), "   ", "Backspace removes one space");

    open_palette(&mut h);
    typed(&mut h, "ab");
    deliver(&mut h, vec![press(Key::Z, Modifiers::COMMAND)]);
    h.run_steps(2);
    assert_ne!(draft(&h), "ab", "Cmd+Z undoes the typing");
    assert_eq!(query(&h), draft(&h), "the undo is committed");
}

/// Four scripted rows, answered on every logic pass from `rows`.
fn scripted(h: &mut PlexiUiHarness) -> Rc<RefCell<Vec<String>>> {
    let rows: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(
        ["one", "two", "three", "four"]
            .iter()
            .map(|id| (*id).to_owned())
            .collect(),
    ));
    let source = rows.clone();
    palette_test_log::set_script(Some(Box::new(move |generation, _query| {
        Some((generation, source.borrow().clone(), true))
    })));
    open_palette(h);
    rows
}

fn row_center(h: &mut PlexiUiHarness, title: &str) -> Pos2 {
    h.harness().get_by_label(title).rect().center()
}

/// Round 6 finding 2 and V3 step 3: an accessibility Click on the second
/// named result runs exactly that row, once.
#[test]
fn palette_accessibility_click_runs_the_named_row() {
    let mut h = palette_harness();
    scripted(&mut h);
    h.harness().get_by_label("two").click_accesskit();
    h.run_steps(3);
    assert_eq!(
        palette_test_log::take_activations(),
        vec!["commands:view/two".to_owned()]
    );
    palette_test_log::set_script(None);
}

/// Round 6 finding 3 and V3 step 3: a short still click runs a row; a
/// two-second hold and a drag out and back run nothing.
#[test]
fn palette_row_click_needs_a_short_still_press() {
    let mut h = palette_harness();
    scripted(&mut h);
    let at = row_center(&mut h, "two");

    deliver(
        &mut h,
        vec![Event::PointerMoved(at), button(at, true, Modifiers::NONE)],
    );
    deliver(
        &mut h,
        vec![Event::PointerMoved(at + egui::vec2(0.0, 60.0))],
    );
    deliver(&mut h, vec![Event::PointerMoved(at)]);
    deliver(&mut h, vec![button(at, false, Modifiers::NONE)]);
    h.run_steps(2);
    assert!(
        palette_test_log::take_activations().is_empty(),
        "a drag out and back runs nothing"
    );

    deliver(&mut h, vec![button(at, true, Modifiers::NONE)]);
    h.run_steps(130);
    deliver(&mut h, vec![button(at, false, Modifiers::NONE)]);
    h.run_steps(2);
    assert!(
        palette_test_log::take_activations().is_empty(),
        "a two-second hold runs nothing"
    );
    assert!(palette_open(&h));

    deliver(&mut h, vec![button(at, true, Modifiers::NONE)]);
    deliver(&mut h, vec![button(at, false, Modifiers::NONE)]);
    h.run_steps(2);
    assert_eq!(
        palette_test_log::take_activations(),
        vec!["commands:view/two".to_owned()],
        "control: a short still click runs the row"
    );
    palette_test_log::set_script(None);
}

/// V3 step 3: rows re-ranked between a press and its release, or between an
/// accessibility lookup and its Click, never run the row that now holds the
/// place the person acted on.
#[test]
fn palette_rerank_never_runs_the_new_occupant() {
    let mut h = palette_harness();
    let rows = scripted(&mut h);
    let at = row_center(&mut h, "two");
    deliver(
        &mut h,
        vec![Event::PointerMoved(at), button(at, true, Modifiers::NONE)],
    );
    rows.borrow_mut().reverse();
    h.run_steps(1);
    assert_eq!(
        row_center(&mut h, "three"),
        at,
        "three now holds two's place"
    );
    deliver(&mut h, vec![button(at, false, Modifiers::NONE)]);
    h.run_steps(2);
    assert!(
        palette_test_log::take_activations().is_empty(),
        "the press on two ran neither two nor three"
    );

    let node = h.harness().get_by_label("three");
    rows.borrow_mut().reverse();
    node.click_accesskit();
    h.run_steps(3);
    assert!(
        palette_test_log::take_activations().is_empty(),
        "a Click aimed at three before the re-rank ran nothing"
    );
    assert!(palette_open(&h));

    deliver(&mut h, vec![button(at, true, Modifiers::NONE)]);
    deliver(&mut h, vec![button(at, false, Modifiers::NONE)]);
    h.run_steps(2);
    assert_eq!(
        palette_test_log::take_activations(),
        vec!["commands:view/two".to_owned()],
        "control: a fresh click runs the row now there"
    );
    palette_test_log::set_script(None);
}

/// V3 step 3: arrowing past the fold keeps the selected row inside the
/// list's visible band.
#[test]
fn palette_arrowing_past_the_fold_keeps_the_selection_visible() {
    let mut h = palette_harness();
    open_palette(&mut h);
    let rows = h.with_app(|app| {
        app.host
            .picker(crate::host::picker::PickerId::Palette)
            .map_or(0, |picker| picker.rows().len())
    });
    assert!(rows > 20, "the list overflows: {rows} rows");
    let field = field_rect(&mut h);
    for _ in 0..(rows - 2) {
        deliver(&mut h, vec![press(Key::ArrowDown, Modifiers::NONE)]);
    }
    h.run_steps(2);
    let footer = h.harness().get_by_label("dismiss").rect().top();
    let selected = h
        .harness()
        .query_all(
            egui_kittest::kittest::by()
                .predicate(|node| node.toggled() == Some(egui::accesskit::Toggled::True)),
        )
        .next()
        .map(|node| node.rect())
        .expect("a selected row");
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        selected.top() >= field.bottom() && selected.bottom() <= footer,
        "the selected row {selected:?} is between the field {field:?} and the footer at {footer}"
    );
    assert_eq!(
        h.with_app(|app| app.ctx.memory(|m| m.focused())),
        Some(egui::Id::new("palette_search")),
        "the field kept focus through the arrows"
    );
}
