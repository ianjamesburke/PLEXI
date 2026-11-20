//! Picker reducer: the one state machine behind every picker surface.
//!
//! Pure and egui-free. `HostModel` holds one [`PickerState`] per [`PickerId`];
//! [`PickerCommand`]s go in through `HostAction::Picker` and
//! [`PickerEffect`]s come out through `HostEffect::Picker`. Sources (host code
//! outside the model) answer `Query` with rows; `Activate` is resolved by the
//! stop's source, never here. Design: `docs/palette-picker.md`.
//!
//! Rules this module enforces:
//! - No key command edits the query. Only `QueryEdited`, sent by the text
//!   field, changes it.
//! - Selection is a row key, never an index. It follows the row across
//!   re-ranks; when the selected row disappears nothing is selected until the
//!   person moves.
//! - Rows from an older generation are dropped. Switching stops keeps the
//!   query and re-emits `Query`.
//! - Every command binds where it sits in the input stream: a pointer hit to
//!   the stamp of the rows it was painted from, a key to the state the
//!   commands before it left. An Enter waiting on its query's answer is
//!   frozen once and runs at most once.
//! - Actions never run here.

/// Which picker a command addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PickerId {
    /// The Cmd+P palette.
    Palette,
}

impl PickerId {
    fn stops(self) -> &'static [Stop] {
        match self {
            PickerId::Palette => PALETTE_STOPS,
        }
    }
}

/// How a stop orders the rows its source returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RankingPreset {
    /// Pinned rows first. Every group stays one contiguous run so a header
    /// sits on its own rows. An empty query keeps group order and source
    /// order inside a group. A typed query orders groups by their best
    /// member's score (group order breaks ties), then rows by score, so a
    /// direct match in a later group is never buried under an incidental
    /// match in an earlier one.
    Grouped,
}

/// One stop of a picker: data defined in code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stop {
    pub id: &'static str,
    /// Chip label.
    pub label: &'static str,
    /// Source that answers this stop's `Query`.
    pub source: &'static str,
    pub ranking: RankingPreset,
    /// Text field placeholder.
    pub placeholder: &'static str,
}

/// Source id of the palette's Commands stop.
pub(crate) const COMMANDS_SOURCE: &str = "commands";

/// The palette's stops, in Tab order. Files and Agents add themselves here.
pub(crate) const PALETTE_STOPS: &[Stop] = &[Stop {
    id: "commands",
    label: "Commands",
    source: COMMANDS_SOURCE,
    ranking: RankingPreset::Grouped,
    placeholder: "Jump to context or launch plugin...",
}];

/// A row's identity: `<source id>:<stable id>`, split on the first `:`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RowKey(String);

impl RowKey {
    pub(crate) fn new(source: &str, stable_id: &str) -> Self {
        Self(format!("{source}:{stable_id}"))
    }

    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn source(&self) -> &str {
        self.0.split_once(':').map_or(self.0.as_str(), |(s, _)| s)
    }

    pub(crate) fn stable_id(&self) -> &str {
        self.0.split_once(':').map_or("", |(_, id)| id)
    }
}

/// One thing a row can do. The first action of a row is Enter's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowAction {
    pub id: &'static str,
    pub label: &'static str,
}

/// A result row. Holds no egui types or host enums.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PickerRow {
    pub key: RowKey,
    pub title: String,
    pub secondary: String,
    pub chips: Vec<&'static str>,
    /// Section header text; rows of one group are kept contiguous.
    pub group: &'static str,
    /// Group order: lower first.
    pub group_rank: u32,
    /// Match score from `search::score_candidate`, higher is better. `None`
    /// for an empty query or a row that matched without a score.
    pub score: Option<i64>,
    /// Pinned rows lead the list whatever the query.
    pub pinned: bool,
    /// When set the row stays listed, shows the reason and refuses Enter.
    pub unavailable: Option<&'static str>,
    pub actions: Vec<RowAction>,
}

/// The snapshot a presented row belongs to. `generation` names the open
/// session, stop and committed query; `revision` names one actionable answer
/// within it. A pointer hit carries the stamp of the rows it was painted
/// from, so it can never act on rows the person did not see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Stamp {
    pub generation: u64,
    pub revision: u64,
}

/// One arrow (or Cmd+J/K) press. Moves are kept one by one: clamping makes
/// Up, Down differ from a summed delta of zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Up,
    Down,
}

/// A presented row under the pointer, bound when the pointer event was hit
/// tested against what the person saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hit {
    pub stamp: Stamp,
    pub key: RowKey,
}

/// Commands are applied one at a time, in input order; each binds to the
/// state the commands before it left.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PickerCommand {
    /// Open on `stop` with an initial query.
    Open {
        stop: &'static str,
        query: String,
    },
    Close,
    NextStop,
    PrevStop,
    /// The field's committed text changed. Sent only by the text field.
    QueryEdited(String),
    Move(Step),
    Hover(Hit),
    /// Enter: the selected row's first action, now or once the current
    /// query is answered.
    Accept,
    /// Pointer click on a presented row: select it and accept it.
    Click(Hit),
    /// A source's answer to `PickerEffect::Query`.
    Rows {
        generation: u64,
        rows: Vec<PickerRow>,
        done: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PickerEffect {
    /// Ask the stop's source for rows.
    Query {
        source: &'static str,
        text: String,
        generation: u64,
    },
    /// Run `action` on `row` of the answer `stamp` names. Emitted before
    /// `Closed`, so the caller takes the row's target out of that answer
    /// before close cleanup releases it.
    Activate {
        stamp: Stamp,
        row: RowKey,
        action: &'static str,
    },
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    /// No answer for this query yet: select the first row once rows arrive,
    /// then apply these arrows in order.
    Initial(Vec<Step>),
    Key(RowKey),
    /// The selected row went away: nothing selected until the person moves.
    Cleared,
}

/// An Enter that arrived before its query was fully answered. Frozen once;
/// later input cannot rewrite it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingEnter {
    /// A partial answer showed this row selected.
    Key { key: RowKey, action: &'static str },
    /// No row existed yet: the initial-selection rule plus the arrows that
    /// preceded Enter. The one case without a concrete row id.
    Initial(Vec<Step>),
}

#[derive(Debug, Clone)]
pub(crate) struct PickerState {
    stops: &'static [Stop],
    active: usize,
    open: bool,
    query: String,
    /// The presented rows. While a new query loads they are the previous
    /// answer's, still carrying `rows_stamp`, and no longer actionable.
    rows: Vec<PickerRow>,
    rows_stamp: Stamp,
    selection: Selection,
    generation: u64,
    next_revision: u64,
    /// The current generation has an answer (`rows` belong to it).
    answered: bool,
    loading: bool,
    pending: Option<PendingEnter>,
    /// Bumped when the keyboard moves the selection, so the results painter
    /// scrolls the selected row into view exactly once.
    reveal: u64,
}

impl PickerState {
    pub(crate) fn new(stops: &'static [Stop]) -> Self {
        assert!(!stops.is_empty(), "a picker needs at least one stop");
        Self {
            stops,
            active: 0,
            open: false,
            query: String::new(),
            rows: Vec::new(),
            rows_stamp: Stamp {
                generation: 0,
                revision: 0,
            },
            selection: Selection::Initial(Vec::new()),
            generation: 0,
            next_revision: 1,
            answered: false,
            loading: false,
            pending: None,
            reveal: 0,
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn stops(&self) -> &'static [Stop] {
        self.stops
    }

    pub(crate) fn active_stop(&self) -> &'static Stop {
        &self.stops[self.active]
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn rows(&self) -> &[PickerRow] {
        &self.rows
    }

    /// The stamp of the presented rows, for the painter's hit map.
    pub(crate) fn rows_stamp(&self) -> Stamp {
        self.rows_stamp
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.loading
    }

    pub(crate) fn reveal_epoch(&self) -> u64 {
        self.reveal
    }

    pub(crate) fn selected(&self) -> Option<&RowKey> {
        match &self.selection {
            Selection::Key(key) => Some(key),
            Selection::Initial(_) | Selection::Cleared => None,
        }
    }

    pub(crate) fn apply(&mut self, command: PickerCommand) -> Vec<PickerEffect> {
        match command {
            PickerCommand::Open { stop, query } => {
                let Some(index) = self.stops.iter().position(|s| s.id == stop) else {
                    log::warn!("picker: open refused, unknown stop '{stop}'");
                    return Vec::new();
                };
                self.open = true;
                self.active = index;
                self.query = query;
                self.rows.clear();
                self.reveal += 1;
                log::info!("picker: opened on stop '{stop}'");
                vec![self.requery()]
            }
            PickerCommand::Close => self.close(),
            PickerCommand::NextStop => self.switch_stop((self.active + 1) % self.stops.len()),
            PickerCommand::PrevStop => {
                self.switch_stop((self.active + self.stops.len() - 1) % self.stops.len())
            }
            PickerCommand::QueryEdited(text) => {
                if !self.open || text == self.query {
                    return Vec::new();
                }
                self.query = text;
                self.reveal += 1;
                vec![self.requery()]
            }
            PickerCommand::Move(step) => {
                if self.open {
                    self.move_selection(step);
                }
                Vec::new()
            }
            PickerCommand::Hover(hit) => {
                if let Some(key) = self.current_hit(hit) {
                    self.selection = Selection::Key(key);
                }
                Vec::new()
            }
            PickerCommand::Accept => self.accept_enter(),
            PickerCommand::Click(hit) => {
                if self.pending.is_some() {
                    log::debug!("picker: click ignored, an Enter is pending");
                    return Vec::new();
                }
                let Some(key) = self.current_hit(hit) else {
                    return Vec::new();
                };
                self.selection = Selection::Key(key.clone());
                self.activate(&key, None)
            }
            PickerCommand::Rows {
                generation,
                rows,
                done,
            } => self.answer(generation, rows, done),
        }
    }

    fn close(&mut self) -> Vec<PickerEffect> {
        if !self.open {
            return Vec::new();
        }
        self.open = false;
        self.query.clear();
        self.rows.clear();
        self.selection = Selection::Initial(Vec::new());
        self.loading = false;
        self.answered = false;
        self.pending = None;
        // Answers and hits still in flight for this session are stale.
        self.generation += 1;
        log::info!("picker: closed");
        vec![PickerEffect::Closed]
    }

    fn switch_stop(&mut self, index: usize) -> Vec<PickerEffect> {
        if !self.open || index == self.active {
            return Vec::new();
        }
        self.active = index;
        self.rows.clear();
        self.reveal += 1;
        log::info!("picker: stop '{}'", self.active_stop().id);
        vec![self.requery()]
    }

    /// Start a new generation for the active stop and the current query. The
    /// previous rows stay presented with their own stamp but stop being
    /// actionable; a pending Enter is cancelled.
    fn requery(&mut self) -> PickerEffect {
        self.generation += 1;
        self.loading = true;
        self.answered = false;
        if self.pending.take().is_some() {
            log::info!("picker: pending Enter cancelled by a new query");
        }
        self.selection = Selection::Initial(Vec::new());
        PickerEffect::Query {
            source: self.active_stop().source,
            text: self.query.clone(),
            generation: self.generation,
        }
    }

    /// The key of a hit on the current answer, or `None` for a stale hit.
    fn current_hit(&self, hit: Hit) -> Option<RowKey> {
        if !self.open || !self.answered || hit.stamp != self.rows_stamp {
            log::debug!(
                "picker: dropped a pointer hit on {:?} (current {:?}, answered {})",
                hit.stamp,
                self.rows_stamp,
                self.answered
            );
            return None;
        }
        self.rows
            .iter()
            .any(|row| row.key == hit.key)
            .then_some(hit.key)
    }

    fn answer(&mut self, generation: u64, rows: Vec<PickerRow>, done: bool) -> Vec<PickerEffect> {
        if !self.open || generation != self.generation {
            log::debug!(
                "picker: dropped rows of generation {generation} (current {})",
                self.generation
            );
            return Vec::new();
        }
        let ranked = rank(self.active_stop().ranking, &self.query, rows);
        if !self.answered || identity(&ranked) != identity(&self.rows) {
            self.rows_stamp = Stamp {
                generation,
                revision: self.next_revision,
            };
            self.next_revision += 1;
        }
        self.rows = ranked;
        self.answered = true;
        self.loading = !done;
        self.reconcile_selection();
        if done {
            if let Some(pending) = self.pending.take() {
                return self.resolve_pending(pending);
            }
        }
        Vec::new()
    }

    fn resolve_pending(&mut self, pending: PendingEnter) -> Vec<PickerEffect> {
        let target = match pending {
            PendingEnter::Key { key, action } => Some((key, Some(action))),
            PendingEnter::Initial(moves) => {
                index_after(&self.rows, &moves).map(|i| (self.rows[i].key.clone(), None))
            }
        };
        match target {
            Some((key, action)) => self.activate(&key, action),
            None => {
                log::info!("picker: pending Enter found no row in the completed answer");
                Vec::new()
            }
        }
    }

    fn reconcile_selection(&mut self) {
        match &self.selection {
            Selection::Initial(moves) => {
                if let Some(index) = index_after(&self.rows, moves) {
                    self.selection = Selection::Key(self.rows[index].key.clone());
                }
            }
            Selection::Key(key) => {
                if !self.rows.iter().any(|row| row.key == *key) {
                    self.selection = Selection::Cleared;
                }
            }
            Selection::Cleared => {}
        }
    }

    fn move_selection(&mut self, step: Step) {
        self.reveal += 1;
        if !self.answered || self.rows.is_empty() {
            if let Selection::Initial(moves) = &mut self.selection {
                moves.push(step);
            }
            return;
        }
        let current = self
            .selected()
            .and_then(|key| self.rows.iter().position(|row| row.key == *key));
        let next = match current {
            Some(index) => clamped(index, step, self.rows.len()),
            // Nothing selected: the first move lands on the first row.
            None => 0,
        };
        self.selection = Selection::Key(self.rows[next].key.clone());
    }

    fn accept_enter(&mut self) -> Vec<PickerEffect> {
        if !self.open {
            return Vec::new();
        }
        if self.pending.is_some() {
            log::debug!("picker: repeated Enter ignored, one is pending");
            return Vec::new();
        }
        if self.answered && !self.loading {
            return match self.selection.clone() {
                Selection::Key(key) => self.activate(&key, None),
                Selection::Initial(_) | Selection::Cleared => Vec::new(),
            };
        }
        self.pending = match &self.selection {
            Selection::Key(key) => self
                .rows
                .iter()
                .find(|row| row.key == *key)
                .and_then(|row| row.actions.first())
                .map(|action| PendingEnter::Key {
                    key: key.clone(),
                    action: action.id,
                }),
            Selection::Initial(moves) => Some(PendingEnter::Initial(moves.clone())),
            Selection::Cleared => None,
        };
        if self.pending.is_some() {
            log::info!(
                "picker: Enter waits for generation {} to finish",
                self.generation
            );
        }
        Vec::new()
    }

    /// Activate `key` of the current answer: Enter's (first) action, which
    /// must equal `action` when a frozen Enter named one.
    fn activate(&mut self, key: &RowKey, action: Option<&'static str>) -> Vec<PickerEffect> {
        let Some(row) = self.rows.iter().find(|row| row.key == *key) else {
            log::info!("picker: activation found no row for its key");
            return Vec::new();
        };
        if let Some(reason) = row.unavailable {
            log::info!("picker: refused unavailable row ({reason})");
            return Vec::new();
        }
        let Some(first) = row.actions.first().map(|a| a.id) else {
            log::warn!("picker: row has no action");
            return Vec::new();
        };
        if action.is_some_and(|frozen| frozen != first) {
            log::info!("picker: pending Enter's action no longer on its row");
            return Vec::new();
        }
        log::info!(
            "picker: activate source '{}' action '{first}'",
            key.source()
        );
        let mut effects = vec![PickerEffect::Activate {
            stamp: self.rows_stamp,
            row: key.clone(),
            action: first,
        }];
        effects.extend(self.close());
        effects
    }
}

fn clamped(index: usize, step: Step, len: usize) -> usize {
    match step {
        Step::Up => index.saturating_sub(1),
        Step::Down => (index + 1).min(len - 1),
    }
}

/// The row the initial selection (the first row) reaches after `moves`.
/// `None` for no rows.
fn index_after(rows: &[PickerRow], moves: &[Step]) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    Some(
        moves
            .iter()
            .fold(0, |index, step| clamped(index, *step, rows.len())),
    )
}

/// What makes an answer actionable: membership, order, availability and
/// action identity. Labels, scores and pips are cosmetic.
fn identity(rows: &[PickerRow]) -> Vec<(&RowKey, bool, Vec<&'static str>)> {
    rows.iter()
        .map(|row| {
            (
                &row.key,
                row.unavailable.is_some(),
                row.actions.iter().map(|a| a.id).collect(),
            )
        })
        .collect()
}

fn rank(preset: RankingPreset, query: &str, mut rows: Vec<PickerRow>) -> Vec<PickerRow> {
    match preset {
        RankingPreset::Grouped => {
            // Lower sorts first; a row without a score ranks after every
            // scored row of its group.
            let row_rank = |row: &PickerRow| row.score.map_or(i64::MAX, |s| -s);
            if query.trim().is_empty() {
                rows.sort_by_key(|row| (!row.pinned, row.group_rank));
                return rows;
            }
            // Pinned rows lead regardless, so they never lift their group.
            let mut best: std::collections::HashMap<u32, i64> = std::collections::HashMap::new();
            for row in rows.iter().filter(|row| !row.pinned) {
                let entry = best.entry(row.group_rank).or_insert(i64::MAX);
                *entry = (*entry).min(row_rank(row));
            }
            rows.sort_by_cached_key(|row| {
                (
                    !row.pinned,
                    best.get(&row.group_rank).copied().unwrap_or(i64::MAX),
                    row.group_rank,
                    row_rank(row),
                )
            });
            rows
        }
    }
}

/// The per-picker store `HostModel` keeps.
#[derive(Debug, Default)]
pub(crate) struct Pickers {
    states: std::collections::HashMap<PickerId, PickerState>,
}

impl Pickers {
    pub(crate) fn get(&self, id: PickerId) -> Option<&PickerState> {
        self.states.get(&id)
    }

    pub(crate) fn apply(&mut self, id: PickerId, command: PickerCommand) -> Vec<PickerEffect> {
        self.states
            .entry(id)
            .or_insert_with(|| PickerState::new(id.stops()))
            .apply(command)
    }
}

#[cfg(test)]
mod tests;
