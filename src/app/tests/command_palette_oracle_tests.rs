//! Stint 1391: the production Cmd+P path against an independent sequential
//! oracle, over generated frame-cut histories.
//!
//! Production is the real host: `RawInput` frames through `App::logic` and
//! `App::ui`, so `claim_palette_boundary`, `route_palette_delivery`, the
//! ordinal host-shortcut policy, the presented hit map, the reducer and
//! owned-target activation all run as shipped. Only the Commands source is
//! scripted, so answers land when the history says. The oracle never calls
//! any of that: it keeps a text buffer, the literal presented rows and an
//! activation log, and applies the design's rules item by item. A mismatch
//! is shrunk to a minimal history before it is reported.

use crate::host::picker::PickerId;
use crate::overlays::command_palette::palette_test_log;
use crate::testing::HostHarness;
use egui::{Event, Key, Modifiers, RawInput};
use std::cell::Cell;
use std::rc::Rc;

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
    Final,
    /// The first half, not done.
    Partial,
    /// The full answer without its second row.
    Removed,
    Reranked,
    /// An answer to the previous generation.
    Stale,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Type(char),
    Paste(&'static str),
    ImeCommit(&'static str),
    SelectAll,
    Backspace,
    Down,
    Up,
    /// Native Cmd+J / Cmd+K (macOS sets `mac_cmd` with `command`).
    CmdJ,
    CmdK,
    Enter,
    Escape,
    /// Pointer at presented row `n` (or a free spot in the modal).
    PointerOver(usize),
    Press(usize),
    Release(usize),
    /// Press and release on the scrim.
    ScrimClick,
    /// Cmd+B, a Normal host chord: never acts while the palette owns input.
    Sidebar,
    /// Cmd+Option+P, a different global chord than the palette's own.
    CmdOptP,
    /// Cmd+P: opens a closed palette, closes an open one. Always last in
    /// its frame.
    CmdP,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    /// The source answer `App::logic` delivers at the start of the frame.
    deliver: Option<Answer>,
    inputs: Vec<Item>,
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    activations: Vec<String>,
    open: bool,
    query: String,
    selected: Option<String>,
    /// Frames the palette owned in which a Normal chord still acted.
    sidebar_toggles_while_owned: usize,
    /// Person input left for later consumers after frames the palette
    /// owned.
    leaked_events: usize,
}

fn native(key: Key, extra: Modifiers) -> Event {
    let mut modifiers = extra;
    modifiers.command = true;
    if cfg!(target_os = "macos") {
        modifiers.mac_cmd = true;
    } else {
        modifiers.ctrl = true;
    }
    press(key, modifiers)
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

fn button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

const SCRIM: egui::Pos2 = egui::pos2(4.0, 4.0);

fn is_person_input(event: &Event) -> bool {
    matches!(
        event,
        Event::Key { .. }
            | Event::Text(_)
            | Event::Paste(_)
            | Event::Copy
            | Event::Cut
            | Event::Ime(_)
            | Event::PointerButton { .. }
            | Event::MouseWheel { .. }
    )
}

// ── Production ─────────────────────────────────────────────────────────────

struct Production {
    h: HostHarness,
    next: Rc<Cell<Option<Answer>>>,
    free: egui::Pos2,
    activations: Vec<String>,
    sidebar_toggles_while_owned: usize,
    leaked_events: usize,
}

impl Production {
    fn new() -> Self {
        let mut h = HostHarness::new();
        let pane = h.add_test_pane();
        h.focus_pane(pane);
        h.run_frames(2);
        let next: Rc<Cell<Option<Answer>>> = Rc::new(Cell::new(None));
        let script = next.clone();
        palette_test_log::set_script(Some(Box::new(move |generation, query| {
            script.take().map(|answer| {
                let (rows, done) = shaped(answer, corpus_rows(query));
                let answered = if answer == Answer::Stale {
                    generation - 1
                } else {
                    generation
                };
                (
                    answered,
                    rows.into_iter().map(str::to_owned).collect(),
                    done,
                )
            })
        })));
        let mut production = Self {
            h,
            next,
            free: egui::Pos2::ZERO,
            activations: Vec::new(),
            sidebar_toggles_while_owned: 0,
            leaked_events: 0,
        };
        production.raw(vec![native(Key::P, Modifiers::NONE)]);
        production.raw(Vec::new());
        assert!(production.h.app.command_palette_open(), "Cmd+P opened");
        let (_, modal) = production
            .h
            .app
            .palette_input
            .presented_layout()
            .expect("the palette presented");
        production.free = modal.left_bottom() + egui::vec2(6.0, -6.0);
        palette_test_log::take_activations();
        production
    }

    fn raw(&mut self, events: Vec<Event>) {
        self.h.frame(RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        });
    }

    fn at(&self, index: usize) -> egui::Pos2 {
        self.h
            .app
            .palette_input
            .presented_layout()
            .and_then(|(rows, _)| rows.get(index).copied())
            .unwrap_or(self.free)
    }

    fn events(&self, item: Item) -> Vec<Event> {
        match item {
            Item::Type(c) => vec![Event::Text(c.to_string())],
            Item::Paste(text) => vec![Event::Paste(text.to_owned())],
            Item::ImeCommit(text) => vec![
                Event::Ime(egui::ImeEvent::Enabled),
                Event::Ime(egui::ImeEvent::Preedit(text.to_owned())),
                Event::Ime(egui::ImeEvent::Commit(text.to_owned())),
                Event::Ime(egui::ImeEvent::Disabled),
            ],
            Item::SelectAll => vec![native(Key::A, Modifiers::NONE)],
            Item::Backspace => vec![press(Key::Backspace, Modifiers::NONE)],
            Item::Down => vec![press(Key::ArrowDown, Modifiers::NONE)],
            Item::Up => vec![press(Key::ArrowUp, Modifiers::NONE)],
            Item::CmdJ => vec![native(Key::J, Modifiers::NONE)],
            Item::CmdK => vec![native(Key::K, Modifiers::NONE)],
            Item::Enter => vec![press(Key::Enter, Modifiers::NONE)],
            Item::Escape => vec![press(Key::Escape, Modifiers::NONE)],
            Item::PointerOver(i) => vec![Event::PointerMoved(self.at(i))],
            Item::Press(i) => vec![button(self.at(i), true)],
            Item::Release(i) => vec![button(self.at(i), false)],
            Item::ScrimClick => vec![button(SCRIM, true), button(SCRIM, false)],
            Item::Sidebar => vec![native(Key::B, Modifiers::NONE)],
            Item::CmdOptP => vec![native(Key::P, Modifiers::ALT)],
            Item::CmdP => vec![native(Key::P, Modifiers::NONE)],
        }
    }

    fn frame(&mut self, frame: &Frame) {
        let owned = self.h.app.command_palette_open();
        let sidebar = self.h.app.sidebar_visible;
        let events = frame
            .inputs
            .iter()
            .flat_map(|item| self.events(*item))
            .collect();
        self.next.set(frame.deliver);
        self.raw(events);
        self.next.set(None);
        if owned {
            self.sidebar_toggles_while_owned += usize::from(self.h.app.sidebar_visible != sidebar);
            self.leaked_events += self
                .h
                .app
                .ctx
                .input(|i| i.events.iter().filter(|e| is_person_input(e)).count());
        }
        self.activations.extend(
            palette_test_log::take_activations()
                .into_iter()
                .map(|key| key.trim_start_matches("commands:view/").to_owned()),
        );
    }

    fn outcome(&self) -> Outcome {
        let picker = self.h.app.host.picker(PickerId::Palette);
        Outcome {
            activations: self.activations.clone(),
            open: self.h.app.command_palette_open(),
            query: picker.map_or(String::new(), |p| p.query().to_owned()),
            selected: picker
                .and_then(|p| p.selected())
                .map(|key| key.as_str().trim_start_matches("commands:view/").to_owned()),
            sidebar_toggles_while_owned: self.sidebar_toggles_while_owned,
            leaked_events: self.leaked_events,
        }
    }
}

impl Drop for Production {
    fn drop(&mut self) {
        palette_test_log::set_script(None);
    }
}

fn run_production(history: &[Frame]) -> Outcome {
    let mut production = Production::new();
    for frame in history {
        production.frame(frame);
    }
    production.outcome()
}

// ── Oracle ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    Row(usize),
    Free,
    Scrim,
}

#[derive(Debug, Clone, PartialEq)]
enum Selection {
    Initial(Vec<i8>),
    Key(&'static str),
    Cleared,
}

#[derive(Debug, Clone)]
enum Pending {
    Key(&'static str),
    Initial(Vec<i8>),
}

type Tag = (u64, u64);

struct Oracle {
    open: bool,
    text: Vec<char>,
    anchor: usize,
    caret: usize,
    version: u64,
    identity: u64,
    /// Answered rows of the current version, and whether they are done.
    answer: Option<(Vec<&'static str>, bool)>,
    tag: Tag,
    /// The rows on screen and the tag they carry.
    shown: (Vec<&'static str>, Tag),
    presented: Option<(Vec<&'static str>, Tag)>,
    pointer: Option<Spot>,
    pressed: Option<(Spot, Option<(&'static str, Tag)>, bool)>,
    selection: Selection,
    pending: Option<Pending>,
    activations: Vec<String>,
}

fn walk(len: usize, moves: &[i8]) -> Option<usize> {
    (len > 0).then(|| {
        moves.iter().fold(0usize, |i, step| {
            if *step < 0 {
                i.saturating_sub(1)
            } else {
                (i + 1).min(len - 1)
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
            identity: 0,
            answer: None,
            tag: (0, 0),
            shown: (Vec::new(), (0, 0)),
            presented: None,
            pointer: None,
            pressed: None,
            selection: Selection::Initial(Vec::new()),
            pending: None,
            activations: Vec::new(),
        };
        oracle.open();
        // Production draws one frame after opening before the history.
        oracle.present();
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
        self.shown.0.clear();
        self.presented = None;
        self.pressed = None;
        self.new_version();
    }

    fn close(&mut self) {
        self.open = false;
        self.text.clear();
        self.answer = None;
        self.shown.0.clear();
        self.pending = None;
        self.presented = None;
        self.pressed = None;
        self.selection = Selection::Initial(Vec::new());
        self.version += 1;
    }

    fn new_version(&mut self) {
        self.version += 1;
        self.answer = None;
        self.pending = None;
        self.selection = Selection::Initial(Vec::new());
    }

    fn present(&mut self) {
        self.presented = Some(self.shown.clone());
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

    fn spot(&self, index: usize) -> Spot {
        match &self.presented {
            Some((rows, _)) if index < rows.len() => Spot::Row(index),
            _ => Spot::Free,
        }
    }

    fn presented_row(&self, spot: Spot) -> Option<(&'static str, Tag)> {
        let Spot::Row(index) = spot else {
            return None;
        };
        let (rows, tag) = self.presented.as_ref()?;
        Some((rows[index], *tag))
    }

    /// A presented row the current answer accepts.
    fn live(&self, hit: Option<(&'static str, Tag)>) -> Option<&'static str> {
        let (id, tag) = hit?;
        (self.answer.is_some() && tag == self.tag && self.rows().contains(&id)).then_some(id)
    }

    fn move_to(&mut self, spot: Spot) {
        if self.pointer == Some(spot) {
            return;
        }
        self.pointer = Some(spot);
        if let Some((origin, _, dragged)) = self.pressed.as_mut() {
            *dragged |= *origin != spot;
        }
        if let Some(id) = self.live(self.presented_row(spot)) {
            self.selection = Selection::Key(id);
        }
    }

    fn step_move(&mut self, step: i8) {
        if self.answer.is_some() && !self.rows().is_empty() {
            let rows = self.rows().to_vec();
            let at = match &self.selection {
                Selection::Key(id) => rows.iter().position(|r| r == id),
                _ => None,
            };
            let next = match at {
                Some(i) if step < 0 => i.saturating_sub(1),
                Some(i) => (i + 1).min(rows.len() - 1),
                None => 0,
            };
            self.selection = Selection::Key(rows[next]);
        } else if let Selection::Initial(moves) = &mut self.selection {
            moves.push(step);
        }
    }

    /// One input while the palette owns it.
    fn step(&mut self, item: Item) {
        match item {
            Item::Type(c) => self.edit(&c.to_string(), false),
            Item::Paste(text) | Item::ImeCommit(text) => self.edit(text, false),
            Item::Backspace => self.edit("", true),
            Item::SelectAll => {
                self.anchor = 0;
                self.caret = self.text.len();
            }
            Item::Down | Item::CmdJ => self.step_move(1),
            Item::Up | Item::CmdK => self.step_move(-1),
            Item::Enter => {
                if self.pending.is_some() {
                    return;
                }
                match (&self.answer, self.selection.clone()) {
                    (Some((_, true)), Selection::Key(id)) => self.activate(id),
                    (Some((_, true)), _) => {}
                    (_, Selection::Key(id)) => self.pending = Some(Pending::Key(id)),
                    (_, Selection::Initial(moves)) => self.pending = Some(Pending::Initial(moves)),
                    (_, Selection::Cleared) => {}
                }
            }
            Item::Escape | Item::CmdP => self.close(),
            Item::PointerOver(i) => self.move_to(self.spot(i)),
            Item::Press(i) => {
                let spot = self.spot(i);
                self.pointer = Some(spot);
                self.pressed = Some((spot, self.presented_row(spot), false));
            }
            Item::Release(i) => {
                let spot = self.spot(i);
                self.pointer = Some(spot);
                let Some((origin, down, dragged)) = self.pressed.take() else {
                    return;
                };
                let up = self.presented_row(spot);
                if dragged || origin != spot || down.is_none() || down != up {
                    return;
                }
                if self.pending.is_none() {
                    if let Some(id) = self.live(up) {
                        self.selection = Selection::Key(id);
                        self.activate(id);
                    }
                }
            }
            Item::ScrimClick => {
                self.pointer = Some(Spot::Scrim);
                self.pressed = None;
                // Before the first presentation there is no modal frame to
                // be outside of.
                if self.presented.is_some() {
                    self.close();
                }
            }
            Item::Sidebar | Item::CmdOptP => {}
        }
    }

    fn deliver(&mut self, answer: Answer) {
        if answer == Answer::Stale {
            return;
        }
        let (rows, done) = shaped(answer, corpus_rows(&self.query()));
        if self.answer.as_ref().map(|(r, _)| r) != Some(&rows) {
            self.identity += 1;
        }
        self.tag = (self.version, self.identity);
        self.shown = (rows.clone(), self.tag);
        self.answer = Some((rows.clone(), done));
        self.selection = match std::mem::replace(&mut self.selection, Selection::Cleared) {
            Selection::Initial(moves) => match walk(rows.len(), &moves) {
                Some(i) => Selection::Key(rows[i]),
                None => Selection::Initial(moves),
            },
            Selection::Key(id) if rows.contains(&id) => Selection::Key(id),
            Selection::Key(_) | Selection::Cleared => Selection::Cleared,
        };
        if done {
            match self.pending.take() {
                Some(Pending::Key(id)) if rows.contains(&id) => self.activate(id),
                Some(Pending::Initial(moves)) => {
                    if let Some(i) = walk(rows.len(), &moves) {
                        self.activate(rows[i]);
                    }
                }
                _ => {}
            }
        }
    }

    fn frame(&mut self, frame: &Frame) {
        if !self.open {
            // Input goes to the host; only Cmd+P (last in the frame) matters.
            if frame.inputs.last() == Some(&Item::CmdP) {
                self.open();
            }
            return;
        }
        if let Some(answer) = frame.deliver {
            self.deliver(answer);
        }
        for item in &frame.inputs {
            if !self.open {
                // The rest of a closed session's delivery is consumed.
                return;
            }
            self.step(*item);
        }
        if self.open {
            self.present();
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
                Selection::Key(id) if self.open => Some((*id).to_owned()),
                _ => None,
            },
            sidebar_toggles_while_owned: 0,
            leaked_events: 0,
        }
    }
}

fn run_oracle(history: &[Frame]) -> Outcome {
    let mut oracle = Oracle::new();
    for frame in history {
        oracle.frame(frame);
    }
    oracle.outcome()
}

// ── Generation and shrinking ───────────────────────────────────────────────

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

fn generate(rng: &mut Rng) -> Vec<Frame> {
    const WORDS: &[&str] = &["al", "bet", "a", "pex", "gamma"];
    const ANSWERS: &[Answer] = &[
        Answer::Final,
        Answer::Final,
        Answer::Partial,
        Answer::Removed,
        Answer::Reranked,
        Answer::Stale,
    ];
    let frames = 1 + rng.below(8);
    (0..frames)
        .map(|_| {
            let deliver = (rng.below(5) < 2).then(|| ANSWERS[rng.below(ANSWERS.len())]);
            let count = rng.below(5);
            let mut inputs: Vec<Item> = (0..count)
                .map(|_| match rng.below(19) {
                    0 => Item::Type(['a', 'b', 'e', 'l', 'p', 't'][rng.below(6)]),
                    1 => Item::Paste(WORDS[rng.below(WORDS.len())]),
                    2 => Item::ImeCommit(WORDS[rng.below(WORDS.len())]),
                    3 => Item::SelectAll,
                    4 => Item::Backspace,
                    5 => Item::Down,
                    6 => Item::Up,
                    7 => Item::CmdJ,
                    8 => Item::CmdK,
                    9 | 10 => Item::Enter,
                    11 => Item::Escape,
                    12 => Item::PointerOver(rng.below(4)),
                    13 => Item::Press(rng.below(4)),
                    14 => Item::Release(rng.below(4)),
                    15 => Item::ScrimClick,
                    16 => Item::Sidebar,
                    17 => Item::CmdOptP,
                    _ => Item::Down,
                })
                .collect();
            if rng.below(6) == 0 {
                inputs.push(Item::CmdP);
            }
            Frame { deliver, inputs }
        })
        .collect()
}

fn diverges(history: &[Frame]) -> bool {
    run_production(history) != run_oracle(history)
}

/// Drop frames, then items, then deliveries, while the divergence persists.
fn shrink(mut history: Vec<Frame>) -> Vec<Frame> {
    loop {
        let mut candidates: Vec<Vec<Frame>> = Vec::new();
        for f in 0..history.len() {
            let mut c = history.clone();
            c.remove(f);
            candidates.push(c);
            for i in 0..history[f].inputs.len() {
                let mut c = history.clone();
                c[f].inputs.remove(i);
                candidates.push(c);
            }
            if history[f].deliver.is_some() {
                let mut c = history.clone();
                c[f].deliver = None;
                candidates.push(c);
            }
        }
        match candidates.into_iter().find(|c| diverges(c)) {
            Some(smaller) => history = smaller,
            None => return history,
        }
    }
}

/// The design's main property: for every generated history, the shipped
/// Cmd+P path runs the same rows in the same order and number, leaves the
/// same query and selection, lets no Normal chord act and hands no person
/// input past the palette in frames it owns.
#[test]
fn command_palette_production_path_matches_sequential_oracle() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..250 {
        let history = generate(&mut rng);
        if diverges(&history) {
            let minimal = shrink(history);
            panic!(
                "case {case}: production diverges from the oracle\n\
                 history: {minimal:#?}\n\
                 oracle:     {:?}\n\
                 production: {:?}",
                run_oracle(&minimal),
                run_production(&minimal)
            );
        }
    }
}
