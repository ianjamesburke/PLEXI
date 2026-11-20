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
    /// Sent only by the text field.
    QueryEdited(String),
    Move(i32),
    Hover(RowKey),
    /// Enter: the selected row's first action.
    Accept,
    /// Pointer click: select the row, then accept it as Enter would.
    Click(RowKey),
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
    /// Run `action` on `row`, resolved by the row's source.
    Activate {
        row: RowKey,
        action: &'static str,
    },
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    /// Select the first row once rows arrive (fresh open, query or stop).
    First,
    Key(RowKey),
    /// The selected row went away: nothing selected until the person moves.
    Cleared,
}

#[derive(Debug, Clone)]
pub(crate) struct PickerState {
    stops: &'static [Stop],
    active: usize,
    open: bool,
    query: String,
    rows: Vec<PickerRow>,
    selection: Selection,
    generation: u64,
    loading: bool,
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
            selection: Selection::First,
            generation: 0,
            loading: false,
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
            Selection::First | Selection::Cleared => None,
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
            PickerCommand::Close => {
                if !self.open {
                    return Vec::new();
                }
                self.open = false;
                self.query.clear();
                self.rows.clear();
                self.selection = Selection::First;
                self.loading = false;
                // Answers still in flight for this session are stale.
                self.generation += 1;
                log::info!("picker: closed");
                vec![PickerEffect::Closed]
            }
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
            PickerCommand::Move(delta) => {
                self.move_selection(delta);
                Vec::new()
            }
            PickerCommand::Hover(key) => {
                if self.rows.iter().any(|row| row.key == key) {
                    self.selection = Selection::Key(key);
                }
                Vec::new()
            }
            PickerCommand::Accept => match self.selection.clone() {
                Selection::Key(key) => self.accept(key),
                Selection::First | Selection::Cleared => Vec::new(),
            },
            PickerCommand::Click(key) => {
                if !self.rows.iter().any(|row| row.key == key) {
                    return Vec::new();
                }
                self.selection = Selection::Key(key.clone());
                self.accept(key)
            }
            PickerCommand::Rows {
                generation,
                rows,
                done,
            } => {
                if !self.open || generation != self.generation {
                    log::debug!(
                        "picker: dropped rows of generation {generation} (current {})",
                        self.generation
                    );
                    return Vec::new();
                }
                self.rows = rank(self.active_stop().ranking, &self.query, rows);
                self.loading = !done;
                self.reconcile_selection();
                Vec::new()
            }
        }
    }

    fn switch_stop(&mut self, index: usize) -> Vec<PickerEffect> {
        if !self.open {
            return Vec::new();
        }
        if index == self.active {
            return Vec::new();
        }
        self.active = index;
        self.rows.clear();
        self.reveal += 1;
        log::info!("picker: stop '{}'", self.active_stop().id);
        vec![self.requery()]
    }

    /// Start a new generation for the active stop and the current query.
    fn requery(&mut self) -> PickerEffect {
        self.generation += 1;
        self.loading = true;
        self.selection = Selection::First;
        PickerEffect::Query {
            source: self.active_stop().source,
            text: self.query.clone(),
            generation: self.generation,
        }
    }

    fn reconcile_selection(&mut self) {
        match &self.selection {
            Selection::First => {
                if let Some(first) = self.rows.first() {
                    self.selection = Selection::Key(first.key.clone());
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

    fn move_selection(&mut self, delta: i32) {
        if self.rows.is_empty() || delta == 0 {
            return;
        }
        let current = self
            .selected()
            .and_then(|key| self.rows.iter().position(|row| row.key == *key));
        let next = match current {
            Some(index) => {
                let last = self.rows.len() as i64 - 1;
                (index as i64 + i64::from(delta)).clamp(0, last) as usize
            }
            None => 0,
        };
        self.selection = Selection::Key(self.rows[next].key.clone());
        self.reveal += 1;
    }

    fn accept(&mut self, key: RowKey) -> Vec<PickerEffect> {
        let Some(row) = self.rows.iter().find(|row| row.key == key) else {
            return Vec::new();
        };
        if let Some(reason) = row.unavailable {
            log::info!("picker: refused unavailable row ({reason})");
            return Vec::new();
        }
        let Some(action) = row.actions.first().map(|a| a.id) else {
            log::warn!("picker: row has no action");
            return Vec::new();
        };
        log::info!(
            "picker: activate source '{}' action '{action}'",
            key.source()
        );
        let mut effects = self.apply(PickerCommand::Close);
        effects.push(PickerEffect::Activate { row: key, action });
        effects
    }
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
            let mut best: std::collections::HashMap<u32, i64> = std::collections::HashMap::new();
            for row in &rows {
                let entry = best.entry(row.group_rank).or_insert(i64::MAX);
                *entry = (*entry).min(row_rank(row));
            }
            rows.sort_by_cached_key(|row| {
                (
                    !row.pinned,
                    best[&row.group_rank],
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
