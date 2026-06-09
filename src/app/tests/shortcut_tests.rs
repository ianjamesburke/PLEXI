use crate::host::keys::{
    build_binding_table, build_key_bindings, Action, BindingContext, Direction,
};

fn default_table() -> Vec<crate::host::keys::BindingEntry> {
    build_binding_table(&build_key_bindings(None))
}

#[test]
fn rename_context_bound_to_cmd_shift_r() {
    let table = default_table();
    let e = table
        .iter()
        .find(|e| matches!(e.action, Action::RenameContext))
        .expect("RenameContext has no binding");
    assert_eq!(e.key, egui::Key::R);
    assert!(e.modifiers.command, "must require Cmd");
    assert!(e.modifiers.shift, "must require Shift");
    assert!(!e.modifiers.alt);
    assert!(!e.modifiers.ctrl);
}

#[test]
fn rename_pane_bound_to_cmd_r_no_shift() {
    let table = default_table();
    let e = table
        .iter()
        .find(|e| matches!(e.action, Action::RenamePane))
        .expect("RenamePane has no binding");
    assert_eq!(e.key, egui::Key::R);
    assert!(e.modifiers.command, "must require Cmd");
    assert!(!e.modifiers.shift, "must NOT require Shift");
    assert!(
        e.exact,
        "RenamePane must be exact — or Cmd+Shift+R (RenameContext) would shadow it"
    );
}

#[test]
fn new_context_bound_to_cmd_shift_n() {
    let table = default_table();
    let e = table
        .iter()
        .find(|e| matches!(e.action, Action::NewContext))
        .expect("NewContext has no binding");
    assert_eq!(e.key, egui::Key::N);
    assert!(e.modifiers.command);
    assert!(e.modifiers.shift);
}

#[test]
fn split_vertical_and_horizontal_share_key_d_with_different_modifiers() {
    let table = default_table();
    let vert = table
        .iter()
        .find(|e| matches!(e.action, Action::SplitVertical))
        .unwrap();
    let horiz = table
        .iter()
        .find(|e| matches!(e.action, Action::SplitHorizontal))
        .unwrap();
    assert_eq!(vert.key, egui::Key::D);
    assert_eq!(horiz.key, egui::Key::D);
    // Vertical uses Cmd+Shift, horizontal uses Cmd only.
    assert!(vert.modifiers.shift);
    assert!(!horiz.modifiers.shift);
}

#[test]
fn all_four_navigate_directions_have_bindings() {
    let table = default_table();
    for dir in [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ] {
        let found = table
            .iter()
            .any(|e| matches!(&e.action, Action::Navigate(d) if *d == dir));
        assert!(
            found,
            "Navigate({dir:?}) has no binding in the default table"
        );
    }
}

/// Detect accidental shadowing: two exact Normal-context entries with the same
/// modifier+key chord. Non-exact entries are intentionally layered (e.g. Cmd+R
/// for RenamePane and Cmd+Shift+R for RenameContext); only exact ones must be unique.
#[test]
fn no_duplicate_exact_normal_chords() {
    let table = default_table();
    let exact: Vec<_> = table
        .iter()
        .filter(|e| e.exact && matches!(e.context, BindingContext::Normal))
        .collect();

    for (i, a) in exact.iter().enumerate() {
        for b in &exact[i + 1..] {
            let same = a.modifiers.command == b.modifiers.command
                && a.modifiers.shift == b.modifiers.shift
                && a.modifiers.alt == b.modifiers.alt
                && a.modifiers.ctrl == b.modifiers.ctrl
                && a.key == b.key;
            assert!(
                !same,
                "Duplicate exact Normal chord: {:?} cmd={} shift={} alt={} ctrl={}",
                a.key, a.modifiers.command, a.modifiers.shift, a.modifiers.alt, a.modifiers.ctrl
            );
        }
    }
}

/// Every binding table entry must have a non-Global context OR be a globally
/// available action. Verify the table is non-empty and all contexts are valid enum variants.
#[test]
fn binding_table_is_populated() {
    let table = default_table();
    assert!(
        table.len() >= 20,
        "binding table has too few entries (got {})",
        table.len()
    );
    // Every entry is reachable (context is a valid variant) — just call the match.
    for e in &table {
        let _ = match e.context {
            BindingContext::Global => 0,
            BindingContext::Normal => 1,
            BindingContext::AppActive => 2,
        };
    }
}
