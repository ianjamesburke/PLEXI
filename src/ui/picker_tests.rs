//! Stint 1391: the production input adapter plus reducer, compared against an
//! independent sequential oracle over generated histories.
//!
//! The oracle never calls `PickerInput::route`, `PickerState::apply` or the
//! hit test. It keeps a plain text buffer, the literal presented rows and an
//! activation log, and applies the design's rules one history item at a
//! time. A mismatch is shrunk to a minimal history before it is reported.

use super::*;
use crate::host::picker::{PickerEffect, PickerRow, RowAction, PALETTE_STOPS};

/// Rows overlap across queries so a key can survive a query change.
const CORPUS: &[&str] = &["alpha", "alpine", "apex", "beta", "bet", "abet", "gamma"];

fn corpus_rows(query: &str) -> Vec<&'static str> {
    CORPUS
        .iter()
        .copied()
        .filter(|id| id.contains(query))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// The current query's full answer.
    Final,
    /// Its first half, not done.
    Partial,
    /// The full answer without its second row.
    Removed,
    /// The full answer in reverse source order.
    Reranked,
    /// An answer to an older generation, if one is outstanding.
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Type(char),
    Paste(&'static str),
    ImeCommit(&'static str),
    SelectAll,
    Backspace,
    Down,
    Up,
    Enter,
    Escape,
    /// Pointer over presented row `n`, press on it, release on it.
    PointerOver(usize),
    Press(usize),
    Release(usize),
    /// A source answer lands (logic pass).
    Deliver(Answer),
    /// The painter presents the current rows (end of a drawn pass).
    Present,
    /// Cmd+P on a closed palette.
    Reopen,
}

fn row_rect(index: usize) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(10.0, 100.0 + 20.0 * index as f32),
        egui::vec2(200.0, 18.0),
    )
}

fn key(id: &str) -> RowKey {
    RowKey::new("commands", id)
}

fn picker_row(id: &str) -> PickerRow {
    PickerRow {
        key: key(id),
        title: id.to_string(),
        secondary: String::new(),
        chips: Vec::new(),
        group: "",
        group_rank: 0,
        score: None,
        pinned: false,
        unavailable: None,
        actions: vec![RowAction {
            id: "run",
            label: "run",
        }],
    }
}

fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// The input events one history item stands for.
fn events(item: &Item) -> Vec<egui::Event> {
    use egui::{Event, Key, Modifiers};
    let pointer_button = |index: usize, pressed: bool| Event::PointerButton {
        pos: row_rect(index).center(),
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    match item {
        Item::Type(c) => vec![Event::Text(c.to_string())],
        Item::Paste(text) => vec![Event::Paste((*text).to_string())],
        Item::ImeCommit(text) => vec![
            Event::Ime(egui::ImeEvent::Enabled),
            Event::Ime(egui::ImeEvent::Preedit((*text).to_string())),
            Event::Ime(egui::ImeEvent::Commit((*text).to_string())),
            Event::Ime(egui::ImeEvent::Disabled),
        ],
        Item::SelectAll => vec![press(Key::A, Modifiers::COMMAND)],
        Item::Backspace => vec![press(Key::Backspace, Modifiers::NONE)],
        Item::Down => vec![press(Key::ArrowDown, Modifiers::NONE)],
        Item::Up => vec![press(Key::ArrowUp, Modifiers::NONE)],
        Item::Enter => vec![press(Key::Enter, Modifiers::NONE)],
        Item::Escape => vec![press(Key::Escape, Modifiers::NONE)],
        Item::PointerOver(index) => vec![Event::PointerMoved(row_rect(*index).center())],
        Item::Press(index) => vec![pointer_button(*index, true)],
        Item::Release(index) => vec![pointer_button(*index, false)],
        Item::Deliver(_) | Item::Present | Item::Reopen => Vec::new(),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    activations: Vec<String>,
    open: bool,
    query: String,
    selected: Option<String>,
}

// ── Production: the adapter and reducer, driven the way the app drives them.

struct Production {
    input: PickerInput,
    state: PickerState,
    /// Generations asked for and their query text, oldest first.
    outstanding: Vec<(u64, String)>,
    activations: Vec<String>,
    field: FieldGeometry,
}

impl Production {
    fn new(field: FieldGeometry) -> Self {
        let mut production = Self {
            input: PickerInput::default(),
            state: PickerState::new(PALETTE_STOPS),
            outstanding: Vec::new(),
            activations: Vec::new(),
            field,
        };
        production.open();
        production
    }

    fn open(&mut self) {
        self.input.reset("");
        self.apply(PickerCommand::Open {
            stop: PALETTE_STOPS[0].id,
            query: String::new(),
        });
    }

    fn apply(&mut self, command: PickerCommand) {
        for effect in self.state.apply(command) {
            match effect {
                PickerEffect::Query {
                    generation, text, ..
                } => self.outstanding.push((generation, text)),
                PickerEffect::Activate { row, .. } => {
                    self.activations.push(row.stable_id().to_owned())
                }
                PickerEffect::Closed => self.input.reset(""),
            }
        }
    }

    /// One frame of input events, routed in order.
    fn frame(&mut self, events: Vec<egui::Event>) {
        for event in events {
            if !self.state.is_open() {
                continue;
            }
            if let Routed::Command(command) = self.input.route(&event, &mut |_| {}) {
                self.apply(command);
            }
        }
    }

    fn deliver(&mut self, answer: Answer) {
        let current = self.state.generation();
        let target = match answer {
            Answer::Stale => self
                .outstanding
                .iter()
                .rev()
                .find(|(generation, _)| *generation != current)
                .cloned(),
            _ => self
                .outstanding
                .iter()
                .find(|(generation, _)| *generation == current)
                .cloned(),
        };
        let Some((generation, text)) = target else {
            return;
        };
        let (rows, done) = shaped(answer, corpus_rows(&text));
        self.apply(PickerCommand::Rows {
            generation,
            rows: rows.iter().map(|id| picker_row(id)).collect(),
            done,
        });
    }

    fn present(&mut self) {
        let rows = self
            .state
            .rows()
            .iter()
            .enumerate()
            .map(|(index, row)| (row_rect(index), row.key.clone()))
            .collect();
        self.input.present(Presented {
            stamp: self.state.rows_stamp(),
            rows,
            modal: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0)),
            field: self.field.clone(),
        });
    }

    fn outcome(&self) -> Outcome {
        Outcome {
            activations: self.activations.clone(),
            open: self.state.is_open(),
            query: self.state.query().to_owned(),
            selected: self.state.selected().map(|k| k.stable_id().to_owned()),
        }
    }
}

fn shaped(answer: Answer, mut rows: Vec<&'static str>) -> (Vec<&'static str>, bool) {
    match answer {
        Answer::Final | Answer::Stale => (rows, true),
        Answer::Partial => {
            rows.truncate(rows.len().div_ceil(2));
            (rows, false)
        }
        Answer::Removed => {
            if rows.len() > 1 {
                rows.remove(1);
            }
            (rows, true)
        }
        Answer::Reranked => {
            rows.reverse();
            (rows, true)
        }
    }
}

/// Run `history`, routing the inputs between two non-input items as one
/// frame, or each input as its own frame.
fn run_production(history: &[Item], field: &FieldGeometry, one_frame: bool) -> Outcome {
    let mut production = Production::new(field.clone());
    let mut frame = Vec::new();
    for item in history {
        let item_events = events(item);
        if !item_events.is_empty() {
            frame.extend(item_events);
            if !one_frame {
                production.frame(std::mem::take(&mut frame));
            }
            continue;
        }
        production.frame(std::mem::take(&mut frame));
        match item {
            Item::Deliver(answer) => production.deliver(*answer),
            Item::Present => production.present(),
            Item::Reopen if !production.state.is_open() => production.open(),
            _ => {}
        }
    }
    production.frame(frame);
    production.outcome()
}

// ── Oracle: the design's rules over literal state.

#[derive(Debug, Clone, PartialEq)]
enum OracleSelection {
    Initial(Vec<i8>),
    Key(&'static str),
    Cleared,
}

#[derive(Debug, Clone)]
enum OraclePending {
    Key(&'static str),
    Initial(Vec<i8>),
}

struct Oracle {
    open: bool,
    text: Vec<char>,
    anchor: usize,
    caret: usize,
    /// Bumped by every query change; answers name the version they answer.
    version: u64,
    /// Answered rows of the current version, and whether they are done.
    answer: Option<(Vec<&'static str>, bool)>,
    /// The rows on screen and the identity tag they were presented with.
    shown: Vec<&'static str>,
    shown_tag: (u64, u64),
    /// Tag of the current answer: (version, identity counter).
    tag: (u64, u64),
    identity: u64,
    presented: Option<(Vec<&'static str>, (u64, u64))>,
    pointer: Option<usize>,
    pressed: Option<(&'static str, (u64, u64))>,
    selection: OracleSelection,
    pending: Option<OraclePending>,
    activations: Vec<String>,
}

fn walk(len: usize, moves: &[i8]) -> Option<usize> {
    (len > 0).then(|| {
        moves.iter().fold(0usize, |index, step| {
            if *step < 0 {
                index.saturating_sub(1)
            } else {
                (index + 1).min(len - 1)
            }
        })
    })
}

impl Oracle {
    fn new() -> Self {
        let mut oracle = Self {
            open: false,
            text: Vec::new(),
            anchor: 0,
            caret: 0,
            version: 0,
            answer: None,
            shown: Vec::new(),
            shown_tag: (0, 0),
            tag: (0, 0),
            identity: 0,
            presented: None,
            pointer: None,
            pressed: None,
            selection: OracleSelection::Initial(Vec::new()),
            pending: None,
            activations: Vec::new(),
        };
        oracle.open();
        oracle
    }

    fn query(&self) -> String {
        self.text.iter().collect()
    }

    fn open(&mut self) {
        self.open = true;
        self.text.clear();
        self.anchor = 0;
        self.caret = 0;
        self.shown.clear();
        self.presented = None;
        self.pressed = None;
        self.new_version();
    }

    fn close(&mut self) {
        self.open = false;
        self.text.clear();
        self.answer = None;
        self.shown.clear();
        self.pending = None;
        self.presented = None;
        self.pressed = None;
        self.selection = OracleSelection::Initial(Vec::new());
        self.version += 1;
    }

    fn new_version(&mut self) {
        self.version += 1;
        self.answer = None;
        self.pending = None;
        self.selection = OracleSelection::Initial(Vec::new());
    }

    fn edit(&mut self, insert: &str, backspace: bool) {
        let before = self.text.clone();
        let (start, end) = (self.anchor.min(self.caret), self.anchor.max(self.caret));
        if start != end {
            self.text.drain(start..end);
            self.caret = start;
        } else if backspace && self.caret > 0 {
            self.caret -= 1;
            self.text.remove(self.caret);
        }
        for c in insert.chars() {
            self.text.insert(self.caret, c);
            self.caret += 1;
        }
        self.anchor = self.caret;
        if self.text != before {
            self.new_version();
        }
    }

    fn current(&self) -> bool {
        self.answer.is_some()
    }

    fn rows(&self) -> &[&'static str] {
        self.answer
            .as_ref()
            .map_or(&[], |(rows, _)| rows.as_slice())
    }

    fn activate(&mut self, id: &'static str) {
        self.activations.push(id.to_owned());
        self.close();
    }

    fn step(&mut self, item: &Item) {
        if !self.open {
            if *item == Item::Reopen {
                self.open();
            }
            return;
        }
        match item {
            Item::Type(c) => self.edit(&c.to_string(), false),
            Item::Paste(text) | Item::ImeCommit(text) => self.edit(text, false),
            Item::Backspace => self.edit("", true),
            Item::SelectAll => {
                self.anchor = 0;
                self.caret = self.text.len();
            }
            Item::Down | Item::Up => {
                let step = if *item == Item::Down { 1 } else { -1 };
                if self.current() && !self.rows().is_empty() {
                    let rows = self.rows().to_vec();
                    let at = match &self.selection {
                        OracleSelection::Key(id) => rows.iter().position(|r| r == id),
                        _ => None,
                    };
                    let next = match at {
                        Some(i) if step < 0 => i.saturating_sub(1),
                        Some(i) => (i + 1).min(rows.len() - 1),
                        None => 0,
                    };
                    self.selection = OracleSelection::Key(rows[next]);
                } else if let OracleSelection::Initial(moves) = &mut self.selection {
                    moves.push(step);
                }
            }
            Item::Enter => {
                if self.pending.is_some() {
                    return;
                }
                match (&self.answer, self.selection.clone()) {
                    (Some((_, true)), OracleSelection::Key(id)) => self.activate(id),
                    (Some((_, true)), _) => {}
                    (_, OracleSelection::Key(id)) => self.pending = Some(OraclePending::Key(id)),
                    (_, OracleSelection::Initial(moves)) => {
                        self.pending = Some(OraclePending::Initial(moves))
                    }
                    (_, OracleSelection::Cleared) => {}
                }
            }
            Item::Escape => self.close(),
            Item::PointerOver(index) => {
                if self.pointer == Some(*index) {
                    return;
                }
                self.pointer = Some(*index);
                if let Some(id) = self.presented_hit(*index) {
                    self.selection = OracleSelection::Key(id);
                }
            }
            Item::Press(index) => {
                self.pointer = Some(*index);
                self.pressed = self
                    .presented
                    .as_ref()
                    .and_then(|(rows, tag)| rows.get(*index).map(|id| (*id, *tag)));
            }
            Item::Release(index) => {
                self.pointer = Some(*index);
                let down = self.pressed.take();
                let up = self
                    .presented
                    .as_ref()
                    .and_then(|(rows, tag)| rows.get(*index).map(|id| (*id, *tag)));
                if let (Some(down), Some(up)) = (down, up) {
                    if down == up
                        && self.pending.is_none()
                        && self.current()
                        && up.1 == self.tag
                        && self.rows().contains(&up.0)
                    {
                        self.selection = OracleSelection::Key(up.0);
                        self.activate(up.0);
                    }
                }
            }
            Item::Deliver(answer) => self.deliver(*answer),
            Item::Present => {
                let tag = if self.current() {
                    self.tag
                } else {
                    self.shown_tag
                };
                self.presented = Some((self.shown.clone(), tag));
            }
            Item::Reopen => {}
        }
    }

    /// A hover hit on presented row `index` that the current answer accepts.
    fn presented_hit(&self, index: usize) -> Option<&'static str> {
        let (rows, tag) = self.presented.as_ref()?;
        let id = *rows.get(index)?;
        (self.current() && *tag == self.tag && self.rows().contains(&id)).then_some(id)
    }

    fn deliver(&mut self, answer: Answer) {
        if answer == Answer::Stale {
            return;
        }
        let query = self.query();
        let (rows, done) = shaped(answer, corpus_rows(&query));
        if self.answer.as_ref().map(|(r, _)| r) != Some(&rows) || !self.current() {
            self.identity += 1;
        }
        self.tag = (self.version, self.identity);
        self.shown = rows.clone();
        self.shown_tag = self.tag;
        self.answer = Some((rows.clone(), done));
        self.selection = match std::mem::replace(&mut self.selection, OracleSelection::Cleared) {
            OracleSelection::Initial(moves) => match walk(rows.len(), &moves) {
                Some(i) => OracleSelection::Key(rows[i]),
                None => OracleSelection::Initial(moves),
            },
            OracleSelection::Key(id) if rows.contains(&id) => OracleSelection::Key(id),
            OracleSelection::Key(_) | OracleSelection::Cleared => OracleSelection::Cleared,
        };
        if done {
            match self.pending.take() {
                Some(OraclePending::Key(id)) if rows.contains(&id) => self.activate(id),
                Some(OraclePending::Initial(moves)) => {
                    if let Some(i) = walk(rows.len(), &moves) {
                        self.activate(rows[i]);
                    }
                }
                _ => {}
            }
        }
    }

    fn outcome(&self) -> Outcome {
        Outcome {
            activations: self.activations.clone(),
            open: self.open,
            query: if self.open {
                self.query()
            } else {
                String::new()
            },
            selected: match &self.selection {
                OracleSelection::Key(id) if self.open => Some((*id).to_owned()),
                _ => None,
            },
        }
    }
}

fn run_oracle(history: &[Item]) -> Outcome {
    let mut oracle = Oracle::new();
    for item in history {
        oracle.step(item);
    }
    oracle.outcome()
}

// ── Generation and shrinking.

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn generate(rng: &mut Rng) -> Vec<Item> {
    const WORDS: &[&str] = &["al", "bet", "a", "pex", "gamma"];
    const ANSWERS: &[Answer] = &[
        Answer::Final,
        Answer::Final,
        Answer::Partial,
        Answer::Removed,
        Answer::Reranked,
        Answer::Stale,
    ];
    let len = 1 + rng.below(24);
    (0..len)
        .map(|_| match rng.below(16) {
            0 => Item::Type(['a', 'b', 'e', 'l', 'p', 't'][rng.below(6)]),
            1 => Item::Paste(WORDS[rng.below(WORDS.len())]),
            2 => Item::ImeCommit(WORDS[rng.below(WORDS.len())]),
            3 => Item::SelectAll,
            4 => Item::Backspace,
            5 => Item::Down,
            6 => Item::Up,
            7 => Item::Enter,
            8 => Item::Escape,
            9 => Item::PointerOver(rng.below(4)),
            10 => Item::Press(rng.below(4)),
            11 => Item::Release(rng.below(4)),
            12 | 13 => Item::Deliver(ANSWERS[rng.below(ANSWERS.len())]),
            14 => Item::Present,
            _ => Item::Reopen,
        })
        .collect()
}

fn diverges(history: &[Item], field: &FieldGeometry) -> bool {
    let expected = run_oracle(history);
    run_production(history, field, false) != expected
        || run_production(history, field, true) != expected
}

/// Remove items while the divergence persists.
fn shrink(mut history: Vec<Item>, field: &FieldGeometry) -> Vec<Item> {
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..history.len() {
            let mut candidate = history.clone();
            candidate.remove(i);
            if diverges(&candidate, field) {
                history = candidate;
                changed = true;
                break;
            }
        }
    }
    history
}

/// The field painted by the production painter, parked away from the rows.
fn field_geometry() -> FieldGeometry {
    let ctx = egui::Context::default();
    let mut geometry = None;
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        geometry = Some(OrderedLineField::new("").show(
            ui,
            egui::Id::new("oracle_field"),
            "",
            crate::ui::focus::SurfaceKey::Overlay(crate::app::input_owner::OverlaySurface::Layer(
                crate::app::FocusKind::CommandPalette,
            )),
            &Colors::from_config(&crate::config::ThemeConfig::default()),
        ));
    });
    let mut geometry = geometry.expect("field painted");
    geometry.rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 24.0));
    geometry
}

/// Round 1 and 2 as one property: for every generated history the
/// production adapter and reducer run the same rows, in the same order and
/// number, and leave the same query and selection, as the sequential oracle;
/// routing a delivery's inputs as one frame or one per frame changes nothing.
#[test]
fn command_palette_input_matches_sequential_oracle() {
    let field = field_geometry();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..3000 {
        let history = generate(&mut rng);
        if diverges(&history, &field) {
            let minimal = shrink(history, &field);
            panic!(
                "case {case}: production diverges from the oracle\n\
                 history: {minimal:?}\n\
                 oracle:     {:?}\n\
                 production: {:?}",
                run_oracle(&minimal),
                run_production(&minimal, &field, false)
            );
        }
    }
}
