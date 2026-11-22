//! Stint 1538: unsaved editor text survives a host that ends without asking
//! (Dock Quit, logout, a quit Apple event, a crash, Force Quit) and is offered
//! back on the next launch.

use std::path::Path;

use crate::app::text_editor_view::TextEditorView;
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;

const ON_DISK: &str = "first draft\n";
const TYPED: &str = "first draft\nwork the person never saved\n";

fn open_editor(h: &mut HostHarness, path: &Path) -> PaneId {
    let before: std::collections::HashSet<PaneId> =
        h.app.windows[0].panes.keys().copied().collect();
    h.app.open_builtin_view_pane(
        Box::new(TextEditorView::new(path.to_path_buf())),
        crate::app::permissions::PluginPermissions::builtin(),
        path.parent().expect("file has a parent").to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane = *h.app.windows[0]
        .panes
        .keys()
        .find(|id| !before.contains(id))
        .expect("editor pane must have been inserted");
    h.focus_pane(pane);
    h.run_frames(3);
    pane
}

fn editor_in(h: &mut HostHarness, path: &Path) -> PaneId {
    h.app
        .find_open_text_editor_pane_any_window(path)
        .expect("the workspace restores the editor pane")
}

fn journal_files() -> usize {
    match std::fs::read_dir(crate::host::unsaved_buffers::test_store_dir()) {
        Ok(listing) => listing.count(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => panic!("journal dir unreadable: {e}"),
    }
}

/// Runs frames until `done` holds, failing with `what` at the deadline.
fn run_frames_until(h: &mut HostHarness, what: &str, done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now()
        + crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        h.run_frames(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// A host with `path` open in an editor holding `TYPED` unsaved.
fn dirty_host(name: &str) -> (HostHarness, std::path::PathBuf, PaneId) {
    let mut h = HostHarness::new();
    let path = h.workspace_root().join(name);
    std::fs::write(&path, ON_DISK).unwrap();
    let pane = open_editor(&mut h, &path);
    h.text_editor_mut(pane).test_set_unsaved(TYPED);
    (h, path, pane)
}

/// Dock Quit, logout, restart, shut down and the quit Apple event all reach
/// only `on_exit`, which asks nothing and ends in `exit_host` without dropping
/// a view. The next launch offers the text; the file is untouched until the
/// person restores and saves.
#[test]
fn a_system_quit_keeps_unsaved_text_and_the_next_launch_offers_it() {
    let (mut h, path, _pane) = dirty_host("draft.txt");
    let profile = h.profile_dir().to_path_buf();
    // `on_exit`'s steps before `exit_host`.
    h.app.flush_unsaved_buffers_now();
    let _ = h
        .app
        .persist_for_teardown(std::time::Duration::from_millis(1));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ON_DISK);

    let mut next = HostHarness::from_profile_dir(&profile);
    next.run_frames(3);
    let pane = editor_in(&mut next, &path);
    let editor = next.text_editor_mut(pane);
    assert_eq!(editor.test_recovered_text(), Some(TYPED), "offered");
    assert_eq!(
        editor.open_buffer().text,
        ON_DISK,
        "nothing restored unasked"
    );
    let state = crate::app::view_trait::View::semantic_state(&*editor).unwrap();
    assert_eq!(state["recovered_unsaved"]["bytes"], TYPED.len());
    assert_eq!(state["recovered_unsaved"]["file_changed_since"], false);

    editor.test_restore_recovered();
    let buffer = editor.open_buffer();
    assert_eq!(buffer.text, TYPED, "Restore puts the text back");
    assert!(
        buffer.dirty,
        "restored text is unsaved until the person saves"
    );
    assert_eq!(editor.test_recovered_text(), None);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ON_DISK);
    drop(next);
    drop(h);
}

/// A crash or Force Quit reaches no quit path at all: the journal written in
/// the background while the person typed is what the next launch offers.
#[test]
fn text_journaled_in_the_background_survives_a_host_that_never_quit() {
    let (mut h, path, _pane) = dirty_host("crash.txt");
    let profile = h.profile_dir().to_path_buf();
    run_frames_until(&mut h, "the journal write", || journal_files() == 1);
    assert!(h.app.save_workspace_now(), "workspace must save");

    let mut next = HostHarness::from_profile_dir(&profile);
    next.run_frames(3);
    let pane = editor_in(&mut next, &path);
    assert_eq!(
        next.text_editor_mut(pane).test_recovered_text(),
        Some(TYPED)
    );
    drop(next);
    drop(h);
}

/// A system quit the person never answered leaves text offered on every
/// launch until it is restored or discarded; Discard ends it.
#[test]
fn recovered_text_is_offered_until_discarded() {
    let (mut h, path, _pane) = dirty_host("ignored.txt");
    let profile = h.profile_dir().to_path_buf();
    h.app.flush_unsaved_buffers_now();
    let _ = h
        .app
        .persist_for_teardown(std::time::Duration::from_millis(1));

    let mut second = HostHarness::from_profile_dir(&profile);
    second.run_frames(3);
    let pane = editor_in(&mut second, &path);
    assert_eq!(
        second.text_editor_mut(pane).test_recovered_text(),
        Some(TYPED)
    );
    // Ignored: this launch ends too.
    second.app.flush_unsaved_buffers_now();

    let mut third = HostHarness::from_profile_dir(&profile);
    third.run_frames(3);
    let pane = editor_in(&mut third, &path);
    assert_eq!(
        third.text_editor_mut(pane).test_recovered_text(),
        Some(TYPED),
        "unresolved text is offered again"
    );
    third.text_editor_mut(pane).test_discard_recovered();
    third.app.flush_unsaved_buffers_now();
    assert_eq!(journal_files(), 0, "Discard removes the journal");
    assert_eq!(third.text_editor_mut(pane).open_buffer().text, ON_DISK);

    let mut fourth = HostHarness::from_profile_dir(&profile);
    fourth.run_frames(3);
    let pane = editor_in(&mut fourth, &path);
    assert_eq!(fourth.text_editor_mut(pane).test_recovered_text(), None);
    drop(fourth);
    drop(third);
    drop(second);
    drop(h);
}

/// A buffer that becomes clean (saved or discarded) leaves nothing behind.
#[test]
fn a_clean_buffer_is_dropped_from_the_journal() {
    let (mut h, _path, pane) = dirty_host("clean.txt");
    run_frames_until(&mut h, "the journal write", || journal_files() == 1);
    h.text_editor_mut(pane).test_discard_unsaved();
    run_frames_until(&mut h, "the journal removal", || journal_files() == 0);
}
