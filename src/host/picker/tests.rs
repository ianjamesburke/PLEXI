use super::*;

const THREE_STOPS: &[Stop] = &[
    Stop {
        id: "commands",
        label: "Commands",
        source: "commands",
        ranking: RankingPreset::Grouped,
        placeholder: "",
    },
    Stop {
        id: "files",
        label: "Files",
        source: "files",
        ranking: RankingPreset::Grouped,
        placeholder: "",
    },
    Stop {
        id: "agents",
        label: "Agents",
        source: "agents",
        ranking: RankingPreset::Grouped,
        placeholder: "",
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
        actions: vec![RowAction {
            id: "run",
            label: "Run",
        }],
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
    let (mut state, _) = opened(PALETTE_STOPS);
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
    for command in [
        PickerCommand::Move(1),
        PickerCommand::Move(-1),
        PickerCommand::NextStop,
        PickerCommand::PrevStop,
        PickerCommand::Hover(RowKey::new("commands", "b")),
    ] {
        state.apply(command);
        assert_eq!(state.query(), "abc");
    }
    // Enter and Esc close the picker; nothing they do types into the query.
    let mut accepted = state.clone();
    accepted.apply(PickerCommand::Accept);
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
    state.apply(PickerCommand::Move(1));
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
    assert!(state.apply(PickerCommand::Accept).is_empty());
    // b coming back does not reselect it behind the person's back.
    answer(
        &mut state,
        vec![row("commands", "b", 0, None), row("commands", "a", 0, None)],
    );
    assert_eq!(selected(&state), None);
    // Moving selects again.
    state.apply(PickerCommand::Move(1));
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
    keyboard.apply(PickerCommand::Move(1));
    let by_enter = keyboard.apply(PickerCommand::Accept);

    let (mut mouse, _) = opened(THREE_STOPS);
    answer(&mut mouse, rows);
    let by_click = mouse.apply(PickerCommand::Click(RowKey::new("commands", "b")));

    let expected = vec![
        PickerEffect::Closed,
        PickerEffect::Activate {
            row: RowKey::new("commands", "b"),
            action: "run",
        },
    ];
    assert_eq!(by_enter, expected);
    assert_eq!(by_click, expected);
    assert!(!keyboard.is_open() && !mouse.is_open());
}

#[test]
fn unavailable_row_stays_listed_and_refuses_enter() {
    let (mut state, _) = opened(THREE_STOPS);
    let mut parked = row("commands", "parked", 0, None);
    parked.unavailable = Some("no live window");
    answer(&mut state, vec![parked]);
    assert_eq!(state.rows().len(), 1);
    assert!(state.apply(PickerCommand::Accept).is_empty());
    assert!(state
        .apply(PickerCommand::Click(RowKey::new("commands", "parked")))
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
