use super::*;

const THREE_STOPS: &[Stop] = &[
    Stop {
        id: "commands",
        label: "Commands",
        source: "commands",
        ranking: RankingPreset::Grouped,
        placeholder: "",
        empty_text: "",
        preview: false,
    },
    Stop {
        id: "files",
        label: "Files",
        source: "files",
        ranking: RankingPreset::Grouped,
        placeholder: "",
        empty_text: "",
        preview: false,
    },
    Stop {
        id: "agents",
        label: "Agents",
        source: "agents",
        ranking: RankingPreset::Grouped,
        placeholder: "",
        empty_text: "",
        preview: false,
    },
];

fn row(source: &str, id: &str, group_rank: u32, score: Option<i64>) -> PickerRow {
    PickerRow {
        key: RowKey::new(source, id),
        title: id.to_string(),
        secondary: String::new(),
        chips: Vec::new(),
        group: "",
        group_rank,
        score,
        pinned: false,
        unavailable: None,
        attention: None,
        inline_preview: None,
        actions: vec![RowAction {
            id: "run",
            label: "Run",
            chord: None,
        }],
        usage: None,
        modified: None,
        preview: None,
    }
}

fn opened(stops: &'static [Stop]) -> (PickerState, Vec<PickerEffect>) {
    let mut state = PickerState::new(stops);
    let effects = state.apply(PickerCommand::Open {
        stop: stops[0].id,
        query: String::new(),
    });
    (state, effects)
}

fn answer(state: &mut PickerState, rows: Vec<PickerRow>) {
    let generation = state.generation();
    state.apply(PickerCommand::Rows {
        generation,
        rows,
        done: true,
    });
}

fn queried_source(effects: &[PickerEffect]) -> Option<&'static str> {
    effects.iter().find_map(|e| match e {
        PickerEffect::Query { source, .. } => Some(*source),
        _ => None,
    })
}

fn hit(state: &PickerState, id: &str) -> Hit {
    Hit {
        stamp: state.rows_stamp(),
        key: RowKey::new("commands", id),
    }
}

fn rows(ids: &[&str]) -> Vec<PickerRow> {
    ids.iter().map(|id| row("commands", id, 0, None)).collect()
}

fn activated(effects: &[PickerEffect]) -> Vec<&str> {
    effects
        .iter()
        .filter_map(|e| match e {
            PickerEffect::Activate { row, .. } => Some(row.stable_id()),
            _ => None,
        })
        .collect()
}

fn deliver(state: &mut PickerState, ids: &[&str], done: bool) -> Vec<PickerEffect> {
    let generation = state.generation();
    state.apply(PickerCommand::Rows {
        generation,
        rows: rows(ids),
        done,
    })
}

fn selected(state: &PickerState) -> Option<&str> {
    state.selected().map(RowKey::as_str)
}

#[test]
fn tab_cycles_stops_and_shift_tab_goes_back() {
    let (mut state, effects) = opened(THREE_STOPS);
    assert_eq!(queried_source(&effects), Some("commands"));
    let order: Vec<_> = (0..3)
        .map(|_| {
            let effects = state.apply(PickerCommand::NextStop);
            (state.active_stop().id, queried_source(&effects))
        })
        .collect();
    assert_eq!(
        order,
        vec![
            ("files", Some("files")),
            ("agents", Some("agents")),
            ("commands", Some("commands")),
        ]
    );
    state.apply(PickerCommand::PrevStop);
    assert_eq!(state.active_stop().id, "agents");
    state.apply(PickerCommand::PrevStop);
    assert_eq!(state.active_stop().id, "files");
}

#[test]
fn one_stop_tab_is_a_no_op() {
    let (mut state, _) = opened(FILES_STOP);
    let generation = state.generation();
    assert!(state.apply(PickerCommand::NextStop).is_empty());
    assert!(state.apply(PickerCommand::PrevStop).is_empty());
    assert_eq!(state.generation(), generation);
}

#[test]
fn query_survives_stop_switch() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("rel".into()));
    let effects = state.apply(PickerCommand::NextStop);
    assert_eq!(state.query(), "rel");
    assert!(matches!(
        effects.as_slice(),
        [PickerEffect::Query { source: "files", text, .. }] if text == "rel"
    ));
    state.apply(PickerCommand::PrevStop);
    assert_eq!(state.query(), "rel");
}

#[test]
fn no_key_command_edits_the_query() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("abc".into()));
    answer(
        &mut state,
        vec![row("commands", "a", 0, None), row("commands", "b", 0, None)],
    );
    let b = hit(&state, "b");
    for command in [
        PickerCommand::Move(Step::Down),
        PickerCommand::Move(Step::Up),
        PickerCommand::NextStop,
        PickerCommand::PrevStop,
        PickerCommand::Hover(b),
    ] {
        state.apply(command);
        assert_eq!(state.query(), "abc");
    }
    // Enter and Esc close the picker; nothing they do types into the query.
    let mut accepted = state.clone();
    accepted.apply(PickerCommand::Accept(None));
    assert!(!accepted.query().contains(['\n', '\t']));
    state.apply(PickerCommand::Close);
    assert_eq!(state.query(), "");
}

#[test]
fn selection_follows_row_key_and_clears_on_removal() {
    let (mut state, _) = opened(THREE_STOPS);
    answer(
        &mut state,
        vec![
            row("commands", "a", 0, None),
            row("commands", "b", 0, None),
            row("commands", "c", 0, None),
        ],
    );
    assert_eq!(selected(&state), Some("commands:a"));
    state.apply(PickerCommand::Move(Step::Down));
    assert_eq!(selected(&state), Some("commands:b"));
    // Re-ranked: b moves to the end, selection stays on b.
    answer(
        &mut state,
        vec![
            row("commands", "c", 0, None),
            row("commands", "a", 0, None),
            row("commands", "b", 0, None),
        ],
    );
    assert_eq!(selected(&state), Some("commands:b"));
    // b disappears: nothing selected, and Enter does nothing.
    answer(
        &mut state,
        vec![row("commands", "c", 0, None), row("commands", "a", 0, None)],
    );
    assert_eq!(selected(&state), None);
    assert!(state.apply(PickerCommand::Accept(None)).is_empty());
    // b coming back does not reselect it behind the person's back.
    answer(
        &mut state,
        vec![row("commands", "b", 0, None), row("commands", "a", 0, None)],
    );
    assert_eq!(selected(&state), None);
    // Moving selects again.
    state.apply(PickerCommand::Move(Step::Down));
    assert_eq!(selected(&state), Some("commands:b"));
}

#[test]
fn stale_generation_rows_are_dropped() {
    let (mut state, _) = opened(THREE_STOPS);
    let old = state.generation();
    state.apply(PickerCommand::QueryEdited("x".into()));
    assert_ne!(state.generation(), old);
    state.apply(PickerCommand::Rows {
        generation: old,
        rows: vec![row("commands", "stale", 0, None)],
        done: true,
    });
    assert!(state.rows().is_empty());
    assert!(state.is_loading());
    answer(&mut state, vec![row("commands", "fresh", 0, None)]);
    assert_eq!(state.rows()[0].key.as_str(), "commands:fresh");
    assert!(!state.is_loading());
    // A stop switch makes the previous stop's answer stale too.
    let before_switch = state.generation();
    state.apply(PickerCommand::NextStop);
    state.apply(PickerCommand::Rows {
        generation: before_switch,
        rows: vec![row("commands", "late", 0, None)],
        done: true,
    });
    assert!(state.rows().is_empty());
}

#[test]
fn mouse_and_enter_resolve_the_same_action() {
    let rows = vec![row("commands", "a", 0, None), row("commands", "b", 0, None)];
    let (mut keyboard, _) = opened(THREE_STOPS);
    answer(&mut keyboard, rows.clone());
    keyboard.apply(PickerCommand::Move(Step::Down));
    let by_enter = keyboard.apply(PickerCommand::Accept(None));

    let (mut mouse, _) = opened(THREE_STOPS);
    answer(&mut mouse, rows);
    let by_click = mouse.apply(PickerCommand::Click(hit(&mouse, "b")));

    for effects in [&by_enter, &by_click] {
        assert!(matches!(
            effects.as_slice(),
            [PickerEffect::Activate { row, action: "run", .. }, PickerEffect::Closed]
                if row.stable_id() == "b"
        ));
    }
    assert!(!keyboard.is_open() && !mouse.is_open());
}

#[test]
fn unavailable_row_stays_listed_and_refuses_enter() {
    let (mut state, _) = opened(THREE_STOPS);
    let mut parked = row("commands", "parked", 0, None);
    parked.unavailable = Some("no live window");
    answer(&mut state, vec![parked]);
    assert_eq!(state.rows().len(), 1);
    assert!(state.apply(PickerCommand::Accept(None)).is_empty());
    assert!(state
        .apply(PickerCommand::Click(hit(&state, "parked")))
        .is_empty());
    assert!(state.is_open());
}

#[test]
fn grouped_ranking_keeps_groups_contiguous_and_pins_first() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("q".into()));
    let mut pinned = row("commands", "update", 0, None);
    pinned.pinned = true;
    answer(
        &mut state,
        vec![
            row("commands", "g0-weak", 0, Some(1)),
            row("commands", "g1-strong", 1, Some(9)),
            row("commands", "g1-weak", 1, Some(2)),
            pinned,
            row("commands", "g0-unscored", 0, None),
        ],
    );
    let keys: Vec<_> = state.rows().iter().map(|r| r.key.stable_id()).collect();
    assert_eq!(
        keys,
        vec!["update", "g1-strong", "g1-weak", "g0-weak", "g0-unscored"]
    );
    // Empty query: group order, then source order.
    state.apply(PickerCommand::QueryEdited(String::new()));
    answer(
        &mut state,
        vec![
            row("commands", "g1-a", 1, None),
            row("commands", "g0-a", 0, None),
            row("commands", "g1-b", 1, None),
        ],
    );
    let keys: Vec<_> = state.rows().iter().map(|r| r.key.stable_id()).collect();
    assert_eq!(keys, vec!["g0-a", "g1-a", "g1-b"]);
}

#[test]
fn row_key_splits_on_the_first_colon() {
    let key = RowKey::new("commands", "note:/tmp/a:b.md");
    assert_eq!(key.source(), "commands");
    assert_eq!(key.stable_id(), "note:/tmp/a:b.md");
}

#[test]
fn closed_picker_ignores_rows_and_edits() {
    let (mut state, _) = opened(THREE_STOPS);
    let generation = state.generation();
    state.apply(PickerCommand::Close);
    state.apply(PickerCommand::Rows {
        generation,
        rows: vec![row("commands", "a", 0, None)],
        done: true,
    });
    assert!(state.rows().is_empty());
    assert!(state
        .apply(PickerCommand::QueryEdited("x".into()))
        .is_empty());
}

#[test]
fn enter_while_rows_load_accepts_the_new_answer() {
    let (mut state, _) = opened(THREE_STOPS);
    answer(&mut state, vec![row("commands", "old", 0, None)]);
    // Paste and Enter in one frame: the field's edit lands first, then Enter.
    state.apply(PickerCommand::QueryEdited("new".into()));
    assert!(
        state.apply(PickerCommand::Accept(None)).is_empty(),
        "Enter waits for the edited query's rows"
    );
    assert!(state.is_open());
    let generation = state.generation();
    let effects = state.apply(PickerCommand::Rows {
        generation,
        rows: vec![row("commands", "new", 0, None)],
        done: true,
    });
    assert_eq!(activated(&effects), vec!["new"]);
    assert_eq!(effects.last(), Some(&PickerEffect::Closed));
}

#[test]
fn pinned_row_never_lifts_its_group() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("q".into()));
    let mut pinned = row("commands", "update", 0, Some(100));
    pinned.pinned = true;
    answer(
        &mut state,
        vec![
            pinned,
            row("commands", "weak-context", 0, Some(1)),
            row("commands", "strong-command", 4, Some(50)),
        ],
    );
    let keys: Vec<_> = state.rows().iter().map(|r| r.key.stable_id()).collect();
    assert_eq!(keys, vec!["update", "strong-command", "weak-context"]);
}

#[test]
fn enter_and_arrows_bind_in_arrival_order() {
    let ab = ["a", "b", "c"];
    let (mut state, _) = opened(THREE_STOPS);
    deliver(&mut state, &ab, true);
    let effects = state.apply(PickerCommand::Accept(None));
    state.apply(PickerCommand::Move(Step::Down));
    assert_eq!(activated(&effects), vec!["a"], "Enter, Down runs A");

    let (mut state, _) = opened(THREE_STOPS);
    deliver(&mut state, &ab, true);
    state.apply(PickerCommand::Move(Step::Down));
    assert_eq!(
        activated(&state.apply(PickerCommand::Accept(None))),
        vec!["b"],
        "Down, Enter runs B"
    );

    // Clamped moves count one by one: Up at the top stays, then Down.
    let (mut state, _) = opened(THREE_STOPS);
    deliver(&mut state, &ab, true);
    state.apply(PickerCommand::Move(Step::Up));
    state.apply(PickerCommand::Move(Step::Down));
    assert_eq!(
        activated(&state.apply(PickerCommand::Accept(None))),
        vec!["b"]
    );
}

#[test]
fn moves_before_the_first_answer_apply_to_it_in_order() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("q".into()));
    state.apply(PickerCommand::Move(Step::Up));
    state.apply(PickerCommand::Move(Step::Down));
    state.apply(PickerCommand::Move(Step::Down));
    deliver(&mut state, &["a", "b", "c"], true);
    assert_eq!(selected(&state), Some("commands:c"));
}

/// Paste, Down, Enter, Down before any rows: the second row, once, on done.
#[test]
fn pending_enter_freezes_the_moves_before_it() {
    let (mut state, _) = opened(THREE_STOPS);
    deliver(&mut state, &["old"], true);
    state.apply(PickerCommand::QueryEdited("q".into()));
    state.apply(PickerCommand::Move(Step::Down));
    assert!(state.apply(PickerCommand::Accept(None)).is_empty());
    state.apply(PickerCommand::Move(Step::Down));
    assert!(
        state.apply(PickerCommand::Accept(None)).is_empty(),
        "no duplicate"
    );
    // A partial answer resolves nothing.
    assert!(deliver(&mut state, &["a", "b"], false).is_empty());
    let effects = deliver(&mut state, &["a", "b", "c"], true);
    assert_eq!(activated(&effects), vec!["b"]);
    assert!(!state.is_open());
    // A duplicate final and a stale final after reopen activate nothing.
    let old_generation = state.generation() - 1;
    state.apply(PickerCommand::Open {
        stop: "commands",
        query: String::new(),
    });
    for generation in [old_generation, old_generation + 1] {
        assert!(state
            .apply(PickerCommand::Rows {
                generation,
                rows: rows(&["a", "b", "c"]),
                done: true,
            })
            .is_empty());
    }
    assert!(state.is_open());
}

#[test]
fn pending_enter_on_a_partial_answer_freezes_its_key() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("q".into()));
    deliver(&mut state, &["a", "b"], false);
    state.apply(PickerCommand::Move(Step::Down));
    assert!(state.apply(PickerCommand::Accept(None)).is_empty());
    // A later move changes the live selection, never the frozen key.
    state.apply(PickerCommand::Move(Step::Up));
    let effects = deliver(&mut state, &["c", "b", "a"], true);
    assert_eq!(activated(&effects), vec!["b"], "found by identity");

    // A frozen key that vanishes in the final answer runs nothing.
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("q".into()));
    deliver(&mut state, &["a", "b"], false);
    state.apply(PickerCommand::Move(Step::Down));
    state.apply(PickerCommand::Accept(None));
    assert!(deliver(&mut state, &["a", "c"], true).is_empty());
    // The intent is consumed: a later refresh does not resurrect it.
    assert!(deliver(&mut state, &["a", "b"], true).is_empty());
    assert!(state.is_open());
}

#[test]
fn empty_final_answer_consumes_a_pending_enter() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("q".into()));
    state.apply(PickerCommand::Accept(None));
    assert!(deliver(&mut state, &[], true).is_empty());
    assert!(
        deliver(&mut state, &["a"], true).is_empty(),
        "a live refresh after the empty answer runs nothing"
    );
    assert!(state.is_open());
}

#[test]
fn new_query_and_close_cancel_a_pending_enter() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("q".into()));
    state.apply(PickerCommand::Accept(None));
    state.apply(PickerCommand::QueryEdited("qr".into()));
    assert!(deliver(&mut state, &["a"], true).is_empty());

    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("q".into()));
    state.apply(PickerCommand::Accept(None));
    let generation = state.generation();
    state.apply(PickerCommand::Close);
    state.apply(PickerCommand::Open {
        stop: "commands",
        query: "q".into(),
    });
    assert!(state
        .apply(PickerCommand::Rows {
            generation,
            rows: rows(&["a"]),
            done: true,
        })
        .is_empty());
    assert!(
        deliver(&mut state, &["a"], true).is_empty(),
        "reopen is a new session"
    );
}

/// Round 5: once another surface takes input above the picker, an Enter
/// it froze never activates; the picker stays open and Enter works again.
#[test]
fn yield_cancels_a_pending_enter() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("q".into()));
    state.apply(PickerCommand::Accept(None));
    assert!(state.apply(PickerCommand::Yield).is_empty());
    assert!(deliver(&mut state, &["a"], true).is_empty());
    assert!(state.is_open(), "yielding keeps the picker open");
    assert_eq!(
        activated(&state.apply(PickerCommand::Accept(None))),
        vec!["a"]
    );
}

/// Round 2: a hover or click from rows painted for an older query is
/// dropped, even when the same key survives into the new answer.
#[test]
fn stale_pointer_hits_are_dropped() {
    let (mut state, _) = opened(THREE_STOPS);
    deliver(&mut state, &["a", "b"], true);
    let old_b = hit(&state, "b");
    state.apply(PickerCommand::QueryEdited("q".into()));
    state.apply(PickerCommand::Hover(old_b.clone()));
    state.apply(PickerCommand::Accept(None));
    state.apply(PickerCommand::Hover(old_b.clone()));
    let effects = deliver(&mut state, &["c", "b"], true);
    assert_eq!(activated(&effects), vec!["c"], "B survived second; C runs");

    let (mut state, _) = opened(THREE_STOPS);
    deliver(&mut state, &["a", "b"], true);
    let old_b = hit(&state, "b");
    state.apply(PickerCommand::QueryEdited("q".into()));
    deliver(&mut state, &["c", "b"], true);
    assert!(
        state.apply(PickerCommand::Click(old_b)).is_empty(),
        "a stale click alone runs nothing"
    );
    let current_b = hit(&state, "b");
    state.apply(PickerCommand::Hover(current_b));
    assert_eq!(
        activated(&state.apply(PickerCommand::Accept(None))),
        vec!["b"]
    );
}

#[test]
fn identical_refresh_keeps_the_revision_and_reorder_advances_it() {
    let (mut state, _) = opened(THREE_STOPS);
    deliver(&mut state, &["a", "b"], true);
    let stamp = state.rows_stamp();
    let mut cosmetic = rows(&["a", "b"]);
    cosmetic[0].secondary = "pip changed".into();
    let generation = state.generation();
    state.apply(PickerCommand::Rows {
        generation,
        rows: cosmetic,
        done: true,
    });
    assert_eq!(
        state.rows_stamp(),
        stamp,
        "a cosmetic refresh keeps hits valid"
    );
    let b = hit(&state, "b");
    deliver(&mut state, &["b", "a"], true);
    assert_ne!(state.rows_stamp(), stamp);
    assert!(state.apply(PickerCommand::Click(b)).is_empty());
}

// ── Files stop: usage ranking, preview, refused answers (stint 1392) ──

const FILES_STOP: &[Stop] = &[Stop {
    id: "files",
    label: "Files",
    source: "files",
    ranking: RankingPreset::Usage,
    placeholder: "",
    empty_text: "",
    preview: true,
}];

fn file(id: &str, score: Option<i64>, picks: Option<(u32, u64)>, modified: u64) -> PickerRow {
    PickerRow {
        usage: picks.map(|(count, last_pick)| usage::UsageEntry { count, last_pick }),
        modified: Some(modified),
        preview: Some(PreviewRef::Text),
        ..row("files", id, 0, score)
    }
}

fn order(state: &PickerState) -> Vec<&str> {
    state.rows().iter().map(|row| row.key.stable_id()).collect()
}

#[test]
fn empty_query_orders_files_by_usage() {
    let (mut state, _) = opened(FILES_STOP);
    answer(
        &mut state,
        vec![
            file("new-unused", None, None, 900),
            file("old-unused", None, None, 100),
            file("picked-once", None, Some((1, 50)), 10),
            file("picked-often", None, Some((5, 20)), 5),
            file("picked-once-later", None, Some((1, 60)), 1),
        ],
    );
    assert_eq!(
        order(&state),
        [
            "picked-often",
            "picked-once-later",
            "picked-once",
            "new-unused",
            "old-unused"
        ],
        "usage first (picks, then latest pick), then newest modified"
    );
}

#[test]
fn usage_breaks_ties_after_score() {
    let (mut state, _) = opened(FILES_STOP);
    state.apply(PickerCommand::QueryEdited("main".into()));
    answer(
        &mut state,
        vec![
            file("weak-but-used", Some(10), Some((9, 9)), 9),
            file("strong", Some(50), None, 1),
            file("tied-unused", Some(30), None, 5),
            file("tied-used", Some(30), Some((1, 1)), 1),
        ],
    );
    assert_eq!(
        order(&state),
        ["strong", "tied-used", "tied-unused", "weak-but-used"],
        "score leads; usage only breaks ties"
    );
}

#[test]
fn preview_for_a_moved_selection_is_dropped() {
    let (mut state, _) = opened(FILES_STOP);
    let generation = state.generation();
    let effects = state.apply(PickerCommand::Rows {
        generation,
        rows: vec![file("a", None, None, 2), file("b", None, None, 1)],
        done: true,
    });
    let a = RowKey::new("files", "a");
    let b = RowKey::new("files", "b");
    assert_eq!(
        effects,
        [PickerEffect::LoadPreview {
            row: a.clone(),
            kind: PreviewRef::Text
        }],
        "the first selected row asks for its preview"
    );
    let moved = state.apply(PickerCommand::Move(Step::Down));
    assert_eq!(
        moved,
        [PickerEffect::LoadPreview {
            row: b.clone(),
            kind: PreviewRef::Text
        }]
    );
    // a's read finishes after the selection moved on: dropped.
    state.apply(PickerCommand::Preview {
        row: a,
        body: PreviewBody::Note("late".into()),
    });
    assert_eq!(state.preview(), Some(&PreviewStatus::Loading));
    let body = PreviewBody::Text {
        text: "b".into(),
        truncated: false,
    };
    state.apply(PickerCommand::Preview {
        row: b,
        body: body.clone(),
    });
    assert_eq!(state.preview(), Some(&PreviewStatus::Ready(body)));
}

#[test]
fn reselecting_a_previewed_row_loads_nothing() {
    let (mut state, _) = opened(FILES_STOP);
    answer(&mut state, vec![file("a", None, None, 1)]);
    // A new query re-ranks the same row first: its preview stays.
    state.apply(PickerCommand::QueryEdited("a".into()));
    let generation = state.generation();
    let effects = state.apply(PickerCommand::Rows {
        generation,
        rows: vec![file("a", Some(1), None, 1)],
        done: true,
    });
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn unsupported_rows_say_why_without_loading() {
    let (mut state, _) = opened(FILES_STOP);
    let generation = state.generation();
    let effects = state.apply(PickerCommand::Rows {
        generation,
        rows: vec![PickerRow {
            preview: Some(PreviewRef::Unsupported("Too large to preview")),
            ..file("big.png", None, None, 1)
        }],
        done: true,
    });
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        state.preview(),
        Some(&PreviewStatus::Ready(PreviewBody::Note(
            "Too large to preview".into()
        )))
    );
}

#[test]
fn oversized_source_answer_is_truncated() {
    // Files: the best MAX_SOURCE_ROWS by usage order, newest first here.
    let (mut state, _) = opened(FILES_STOP);
    let rows: Vec<PickerRow> = (0..MAX_SOURCE_ROWS as u64 + 5)
        .map(|i| file(&format!("f{i}"), None, None, i))
        .collect();
    answer(&mut state, rows);
    assert_eq!(state.rows().len(), MAX_SOURCE_ROWS);
    let newest = format!("f{}", MAX_SOURCE_ROWS + 4);
    assert_eq!(state.rows()[0].key.stable_id(), newest);
    assert_eq!(
        state.selected().map(RowKey::stable_id),
        Some(newest.as_str())
    );

    // Commands: an over-cap default list still shows its first rows.
    let (mut state, _) = opened(THREE_STOPS);
    let rows: Vec<PickerRow> = (0..MAX_SOURCE_ROWS + 5)
        .map(|i| row("commands", &format!("c{i}"), 0, None))
        .collect();
    answer(&mut state, rows);
    assert_eq!(state.rows().len(), MAX_SOURCE_ROWS);
    assert_eq!(state.rows()[0].key.stable_id(), "c0");
    assert_eq!(
        state.rows()[MAX_SOURCE_ROWS - 1].key.stable_id(),
        format!("c{}", MAX_SOURCE_ROWS - 1)
    );
    let effects = state.apply(PickerCommand::Accept(None));
    assert!(
        matches!(&effects[..], [PickerEffect::Activate { row, .. }, PickerEffect::Closed] if row.stable_id() == "c0"),
        "{effects:?}"
    );
}

#[test]
fn stops_without_a_preview_never_load_one() {
    let (mut state, _) = opened(THREE_STOPS);
    let generation = state.generation();
    let effects = state.apply(PickerCommand::Rows {
        generation,
        rows: vec![row("commands", "a", 0, None)],
        done: true,
    });
    assert!(effects.is_empty());
    assert_eq!(state.preview(), None);
}

#[test]
fn cmd_enter_lists_actions_and_keeps_the_query() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("original".into()));
    let mut result = row("commands", "agent", 0, None);
    result.actions.push(RowAction {
        id: "close",
        label: "Close pane",
        chord: None,
    });
    answer(&mut state, vec![result]);
    let before = state.selected().cloned();
    assert!(state.apply(PickerCommand::ShowActions).is_empty());
    assert_eq!(state.visible_actions().len(), 2);
    assert_eq!(state.field_text(), "");
    assert!(state
        .apply(PickerCommand::QueryEdited("close".into()))
        .is_empty());
    assert_eq!(state.visible_actions()[0].id, "close");
    state.apply(PickerCommand::NextStop);
    assert_eq!(state.active_stop().id, "commands");
    assert!(state.apply(PickerCommand::Back).is_empty());
    assert_eq!(state.field_text(), "original");
    assert_eq!(state.selected(), before.as_ref());
    state.apply(PickerCommand::ShowActions);
    assert!(matches!(
        state.apply(PickerCommand::Accept(Some("close"))).as_slice(),
        [PickerEffect::Activate { row, action: "close", .. }, PickerEffect::Closed]
            if Some(row) == before.as_ref()
    ));
}

#[test]
fn cmd_enter_before_first_answer_uses_the_initial_selection_at_the_chord() {
    let (mut state, _) = opened(THREE_STOPS);
    state.apply(PickerCommand::QueryEdited("new query".into()));
    state.apply(PickerCommand::ShowActions);
    state.apply(PickerCommand::Move(Step::Down));
    answer(&mut state, rows(&["a", "b"]));

    assert!(state.showing_actions());
    assert_eq!(selected(&state), Some("commands:a"));
}

#[test]
fn cmd_enter_during_files_walk_keeps_the_requested_row() {
    let (mut state, _) = opened(FILES_STOP);
    let files = || vec![row("files", "a", 0, None), row("files", "b", 0, None)];
    state.apply(PickerCommand::Rows {
        generation: state.generation(),
        rows: files(),
        done: false,
    });
    let requested = state.selected().cloned().expect("visible file");

    state.apply(PickerCommand::ShowActions);
    state.apply(PickerCommand::Move(Step::Down));
    assert_ne!(state.selected(), Some(&requested));
    answer(&mut state, files());

    assert!(state.showing_actions());
    assert_eq!(
        state.selected(),
        Some(&requested),
        "Cmd+Enter keeps its file"
    );
    assert_eq!(
        activated(&state.apply(PickerCommand::Accept(None))),
        vec![requested.stable_id()]
    );
}

#[test]
fn cmd_enter_during_files_walk_cancels_when_the_requested_row_disappears() {
    let (mut state, _) = opened(FILES_STOP);
    state.apply(PickerCommand::Rows {
        generation: state.generation(),
        rows: vec![row("files", "a", 0, None), row("files", "b", 0, None)],
        done: false,
    });

    state.apply(PickerCommand::ShowActions);
    state.apply(PickerCommand::Move(Step::Down));
    answer(&mut state, vec![row("files", "b", 0, None)]);

    assert!(
        !state.showing_actions(),
        "Cmd+Enter cannot retarget a missing file"
    );
    assert_eq!(selected(&state), Some("files:b"));
}

#[test]
fn agents_waiting_on_you_sort_first() {
    let mut waiting = row("agents", "waiting", 0, Some(1));
    waiting.attention = Some(AttentionRank {
        waiting_on_you: true,
        last_interaction_at: 1,
    });
    let mut recent = row("agents", "recent", 0, Some(100));
    recent.attention = Some(AttentionRank {
        waiting_on_you: false,
        last_interaction_at: 10,
    });
    let mut state = PickerState::new(PALETTE_STOPS);
    state.apply(PickerCommand::Open {
        stop: "agents",
        query: "match".into(),
    });
    answer(&mut state, vec![recent, waiting]);
    assert_eq!(selected(&state), Some("agents:waiting"));
}
