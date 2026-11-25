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
//! - A preview answer for a row that is no longer selected is dropped.
//! - An answer is cut to its best `MAX_SOURCE_ROWS` rows after ranking.

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
    /// Filename match rank, then match score, usage (picks, then latest
    /// pick), and newest modified. An empty query scores nothing, so usage leads.
    Usage,
    /// Waiting on you first, then most recent interaction.
    Attention,
}

/// The order `RankingPreset::Usage` sorts by, as a key: lower sorts first.
/// A source that must cut its answer to `MAX_SEARCH_HITS` cuts by this same
/// key, so the reducer and the source never disagree on the top rows.
pub(crate) fn usage_rank_key(
    file_match: Option<files::FileMatchRank>,
    score: Option<i64>,
    usage: Option<usage::UsageEntry>,
    modified: Option<u64>,
) -> impl Ord {
    use std::cmp::Reverse;
    (
        Reverse(file_match),
        Reverse(score),
        Reverse(usage.map(|u| (u.count, u.last_pick))),
        Reverse(modified),
    )
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
    /// Shown when an answer has no rows.
    pub empty_text: &'static str,
    /// The stop shows the selected row's preview beside the results.
    pub preview: bool,
}

/// Source id of the palette's Commands stop.
pub(crate) const COMMANDS_SOURCE: &str = "commands";

/// Source id of the palette's Files stop.
pub(crate) const FILES_SOURCE: &str = "files";

/// Source id of the palette's Agents stop.
pub(crate) const AGENTS_SOURCE: &str = "agents";

/// The palette's stops, in Tab order.
pub(crate) const PALETTE_STOPS: &[Stop] = &[
    Stop {
        id: "commands",
        label: "Commands",
        source: COMMANDS_SOURCE,
        ranking: RankingPreset::Grouped,
        placeholder: "Jump to context or launch plugin...",
        empty_text: "No matching contexts or plugins",
        preview: false,
    },
    Stop {
        id: "files",
        label: "Files",
        source: FILES_SOURCE,
        ranking: RankingPreset::Usage,
        placeholder: "Find a file in this context...",
        empty_text: "No matching files",
        preview: true,
    },
    Stop {
        id: AGENTS_SOURCE,
        label: "Agents",
        source: AGENTS_SOURCE,
        ranking: RankingPreset::Attention,
        placeholder: "Search agents...",
        empty_text: "No matching agents",
        preview: true,
    },
];

/// The most rows one answer shows; ranking decides which.
pub(crate) const MAX_SOURCE_ROWS: usize = crate::search::MAX_SEARCH_HITS;

/// A row's identity: `<source id>:<stable id>`, split on the first `:`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RowKey(String);

impl RowKey {
    pub(crate) fn new(source: &str, stable_id: &str) -> Self {
        Self(format!("{source}:{stable_id}"))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn source(&self) -> &str {
        self.0.split_once(':').map_or(self.0.as_str(), |(s, _)| s)
    }

    /// The id within its source; tests read activations by it.
    #[cfg(test)]
    pub(crate) fn stable_id(&self) -> &str {
        self.0.split_once(':').map_or("", |(_, id)| id)
    }
}

/// One thing a row can do. The first action of a row is Enter's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowAction {
    pub id: &'static str,
    pub label: &'static str,
    pub chord: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct AttentionRank {
    pub waiting_on_you: bool,
    pub last_interaction_at: u64,
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
    /// Filename and depth preferences used by the Files stop.
    pub file_match: Option<files::FileMatchRank>,
    /// Pinned rows lead the list whatever the query.
    pub pinned: bool,
    /// When set the row stays listed, shows the reason and refuses Enter.
    pub unavailable: Option<&'static str>,
    pub actions: Vec<RowAction>,
    pub attention: Option<AttentionRank>,
    /// How often and how lately this row was picked (`RankingPreset::Usage`).
    pub usage: Option<usage::UsageEntry>,
    /// Last modification, unix seconds (`RankingPreset::Usage`).
    pub modified: Option<u64>,
    /// What the preview half shows for this row, on a stop with a preview.
    pub preview: Option<PreviewRef>,
    /// Ready content from the source's snapshot, refreshed even when selection stays on this row.
    pub inline_preview: Option<PreviewBody>,
}

/// A row's preview reference: the kind its source loads. The painter draws
/// by kind and never guesses from a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreviewRef {
    Text,
    Image,
    /// No preview, with the one dim line that says why.
    Unsupported(&'static str),
}

/// A loaded preview. An image's pixels stay with the host that uploaded
/// them; the reducer keeps only what it needs to lay them out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreviewBody {
    Text {
        text: String,
        truncated: bool,
    },
    Image {
        width: u32,
        height: u32,
    },
    /// One dim line in place of a body: binary, too large, timed out, or a
    /// failed read.
    Note(String),
}

/// The preview of one row: loading, or what loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreviewStatus {
    Loading,
    Ready(PreviewBody),
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
    SetStop(&'static str),
    /// The field's committed text changed. Sent only by the text field.
    QueryEdited(String),
    Move(Step),
    Hover(Hit),
    /// Enter: the selected row's first action, now or once the current
    /// query is answered.
    Accept(Option<&'static str>),
    ShowActions,
    Back,
    ClickAction(Hit, u64, &'static str),
    /// Pointer click on a presented row: select it and accept it.
    Click(Hit),
    /// Another surface took input above the open picker: an Enter it froze
    /// was meant for the rows the person saw, never for later ones.
    Yield,
    /// A source's answer to `PickerEffect::Query`.
    Rows {
        generation: u64,
        rows: Vec<PickerRow>,
        done: bool,
    },
    /// A source's answer to `PickerEffect::LoadPreview`.
    Preview {
        row: RowKey,
        body: PreviewBody,
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
    /// Load the preview of `row`, selected on a stop with a preview.
    LoadPreview {
        row: RowKey,
        kind: PreviewRef,
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
struct ActionsStep {
    filter: String,
    selected: Option<&'static str>,
    opening: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct PickerState {
    stops: &'static [Stop],
    active: usize,
    open: bool,
    query: String,
    actions: Option<ActionsStep>,
    /// Selection at Cmd+Enter, kept until the current query settles.
    pending_actions: Option<Selection>,
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
    /// The preview of the last row selected on a preview stop. It is kept
    /// while a new query loads, so reselecting that row loads nothing.
    preview: Option<(RowKey, PreviewStatus)>,
}

impl PickerState {
    pub(crate) fn new(stops: &'static [Stop]) -> Self {
        assert!(!stops.is_empty(), "a picker needs at least one stop");
        Self {
            stops,
            active: 0,
            open: false,
            query: String::new(),
            actions: None,
            pending_actions: None,
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
            preview: None,
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

    pub(crate) fn showing_actions(&self) -> bool {
        self.actions.is_some()
    }

    pub(crate) fn actions_opening(&self) -> Option<u64> {
        self.actions.as_ref().map(|step| step.opening)
    }

    pub(crate) fn field_text(&self) -> &str {
        self.actions
            .as_ref()
            .map_or(&self.query, |step| &step.filter)
    }

    pub(crate) fn selected_row(&self) -> Option<&PickerRow> {
        let key = self.selected()?;
        self.rows.iter().find(|row| row.key == *key)
    }

    pub(crate) fn visible_actions(&self) -> Vec<&RowAction> {
        let Some(step) = &self.actions else {
            return Vec::new();
        };
        let filter = step.filter.to_lowercase();
        self.selected_row()
            .into_iter()
            .filter(|row| row.unavailable.is_none())
            .flat_map(|row| &row.actions)
            .filter(|action| action.label.to_lowercase().contains(&filter))
            .collect()
    }

    pub(crate) fn selected_action(&self) -> Option<&RowAction> {
        let selected = self.actions.as_ref()?.selected?;
        self.visible_actions()
            .into_iter()
            .find(|action| action.id == selected)
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

    /// The presented rows are the current query's, and more may follow.
    pub(crate) fn is_partial(&self) -> bool {
        self.answered && self.loading
    }

    pub(crate) fn reveal_epoch(&self) -> u64 {
        self.reveal
    }

    /// The selected row's preview. `None` when the stop has no preview or
    /// nothing is selected.
    pub(crate) fn preview(&self) -> Option<&PreviewStatus> {
        let selected = self.selected()?;
        self.preview
            .as_ref()
            .filter(|(key, _)| key == selected)
            .map(|(_, status)| status)
    }

    pub(crate) fn selected(&self) -> Option<&RowKey> {
        match &self.selection {
            Selection::Key(key) => Some(key),
            Selection::Initial(_) | Selection::Cleared => None,
        }
    }

    pub(crate) fn apply(&mut self, command: PickerCommand) -> Vec<PickerEffect> {
        let mut effects = self.apply_command(command);
        // Before `Activate` and `Closed` would be wrong: a closing picker
        // loads nothing, and `sync_preview` sees it closed.
        if let Some(load) = self.sync_preview() {
            effects.push(load);
        }
        effects
    }

    /// Follow the selection with the preview: a newly selected row on a
    /// preview stop asks its source for its preview, once.
    fn sync_preview(&mut self) -> Option<PickerEffect> {
        if !self.open || !self.active_stop().preview {
            self.preview = None;
            return None;
        }
        let row = self.selected_row()?;
        let key = row.key.clone();
        if let Some(body) = &row.inline_preview {
            if !matches!(
                &self.preview,
                Some((shown, PreviewStatus::Ready(current))) if shown == &key && current == body
            ) {
                self.preview = Some((key, PreviewStatus::Ready(body.clone())));
            }
            return None;
        }
        if self
            .preview
            .as_ref()
            .is_some_and(|(shown, _)| *shown == key)
        {
            return None;
        }
        let (status, load) = match row.preview {
            Some(kind @ (PreviewRef::Text | PreviewRef::Image)) => (
                PreviewStatus::Loading,
                Some(PickerEffect::LoadPreview {
                    row: key.clone(),
                    kind,
                }),
            ),
            Some(PreviewRef::Unsupported(reason)) => (
                PreviewStatus::Ready(PreviewBody::Note(reason.to_owned())),
                None,
            ),
            None => (
                PreviewStatus::Ready(PreviewBody::Note("No preview".to_owned())),
                None,
            ),
        };
        self.preview = Some((key, status));
        load
    }

    fn apply_command(&mut self, command: PickerCommand) -> Vec<PickerEffect> {
        match command {
            PickerCommand::Open { stop, query } => {
                let Some(index) = self.stops.iter().position(|s| s.id == stop) else {
                    log::warn!("picker: open refused, unknown stop '{stop}'");
                    return Vec::new();
                };
                self.open = true;
                self.active = index;
                self.query = query;
                self.actions = None;
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
            PickerCommand::SetStop(stop) => self
                .stops
                .iter()
                .position(|s| s.id == stop)
                .map_or_else(Vec::new, |index| self.switch_stop(index)),
            PickerCommand::ShowActions => {
                self.show_actions();
                Vec::new()
            }
            PickerCommand::Back => {
                let was_waiting = self.pending_actions.take().is_some();
                if self.actions.take().is_some() || was_waiting {
                    log::info!("picker: returned to results");
                    self.reveal += 1;
                    Vec::new()
                } else {
                    self.close()
                }
            }
            PickerCommand::QueryEdited(text) => {
                if let Some(step) = &mut self.actions {
                    if step.filter != text {
                        step.filter = text;
                        let selected = self.visible_actions().first().map(|action| action.id);
                        if let Some(step) = &mut self.actions {
                            step.selected = selected;
                        }
                        self.reveal += 1;
                    }
                    return Vec::new();
                }
                if !self.open || text == self.query {
                    return Vec::new();
                }
                self.query = text;
                self.reveal += 1;
                vec![self.requery()]
            }
            PickerCommand::Move(step) => {
                if self.showing_actions() {
                    self.move_action_selection(step);
                } else if self.open {
                    self.move_selection(step);
                }
                Vec::new()
            }
            PickerCommand::Hover(hit) => {
                if let Some(key) = self.result_hit(hit) {
                    self.selection = Selection::Key(key);
                }
                Vec::new()
            }
            PickerCommand::Accept(Some(action)) => self.accept_action(action),
            PickerCommand::Accept(None) => {
                if self.showing_actions() {
                    self.selected_action()
                        .map(|action| action.id)
                        .map_or_else(Vec::new, |id| self.accept_action(id))
                } else {
                    self.accept_enter()
                }
            }
            PickerCommand::ClickAction(hit, opening, action) => {
                if self.actions_opening() == Some(opening)
                    && self.selected() == Some(&hit.key)
                    && self.current_hit(hit).is_some()
                {
                    self.accept_action(action)
                } else {
                    Vec::new()
                }
            }
            PickerCommand::Yield => {
                self.pending_actions = None;
                if self.pending.take().is_some() {
                    log::info!("picker: pending Enter cancelled; input moved to another surface");
                }
                Vec::new()
            }
            PickerCommand::Click(hit) => {
                if self.pending.is_some() {
                    log::debug!("picker: click ignored, an Enter is pending");
                    return Vec::new();
                }
                let Some(key) = self.result_hit(hit) else {
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
            PickerCommand::Preview { row, body } => {
                match &mut self.preview {
                    Some((key, status @ PreviewStatus::Loading)) if *key == row => {
                        *status = PreviewStatus::Ready(body);
                    }
                    _ => log::debug!("picker: dropped a preview for a row no longer selected"),
                }
                Vec::new()
            }
        }
    }

    fn close(&mut self) -> Vec<PickerEffect> {
        if !self.open {
            return Vec::new();
        }
        self.open = false;
        self.query.clear();
        self.actions = None;
        self.pending_actions = None;
        self.rows.clear();
        self.selection = Selection::Initial(Vec::new());
        self.loading = false;
        self.answered = false;
        self.pending = None;
        self.preview = None;
        // Answers and hits still in flight for this session are stale.
        self.generation += 1;
        log::info!("picker: closed");
        vec![PickerEffect::Closed]
    }

    fn switch_stop(&mut self, index: usize) -> Vec<PickerEffect> {
        if !self.open
            || self.showing_actions()
            || self.pending_actions.is_some()
            || index == self.active
        {
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
        self.pending_actions = None;
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

    fn result_hit(&self, hit: Hit) -> Option<RowKey> {
        if self.showing_actions() {
            return None;
        }
        self.current_hit(hit)
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
        let mut ranked = rank(self.active_stop().ranking, &self.query, rows);
        if ranked.len() > MAX_SOURCE_ROWS {
            log::info!(
                "picker: showing the best {MAX_SOURCE_ROWS} of {} rows from '{}'",
                ranked.len(),
                self.active_stop().source
            );
            ranked.truncate(MAX_SOURCE_ROWS);
        }
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
        if self.selected().is_none() {
            self.actions = None;
        }
        if done {
            if let Some(selection) = self.pending_actions.take() {
                let key = match selection {
                    Selection::Key(key) => Some(key),
                    Selection::Initial(moves) => {
                        index_after(&self.rows, &moves).map(|i| self.rows[i].key.clone())
                    }
                    Selection::Cleared => None,
                };
                if let Some(key) = key {
                    self.show_actions_for(key);
                }
            }
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
            Some((key, action)) => {
                if action.is_some_and(|frozen| {
                    self.rows
                        .iter()
                        .find(|row| row.key == key)
                        .and_then(|row| row.actions.first())
                        .map(|action| action.id)
                        != Some(frozen)
                }) {
                    log::info!("picker: pending Enter's action no longer first on its row");
                    Vec::new()
                } else {
                    self.activate(&key, action)
                }
            }
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

    fn show_actions(&mut self) {
        if !self.open
            || self.showing_actions()
            || self.pending.is_some()
            || self.pending_actions.is_some()
        {
            return;
        }
        if self.loading {
            if self.selection != Selection::Cleared {
                log::info!("picker: Actions waits for the completed answer");
                self.pending_actions = Some(self.selection.clone());
            }
        } else if let Some(key) = self.selected().cloned() {
            self.show_actions_for(key);
        }
    }

    fn show_actions_for(&mut self, key: RowKey) {
        let Some(first) = self
            .rows
            .iter()
            .find(|row| row.key == key)
            .filter(|row| row.unavailable.is_none())
            .and_then(|row| row.actions.first())
            .map(|action| action.id)
        else {
            log::info!("picker: requested row is gone or has no available actions");
            return;
        };
        self.selection = Selection::Key(key);
        self.reveal += 1;
        self.actions = Some(ActionsStep {
            selected: Some(first),
            opening: self.reveal,
            filter: String::new(),
        });
        log::info!("picker: opened actions on '{}'", self.active_stop().id);
    }

    fn move_action_selection(&mut self, step: Step) {
        let actions = self.visible_actions();
        if actions.is_empty() {
            return;
        }
        let current = self.actions.as_ref().and_then(|step| step.selected);
        let index = actions
            .iter()
            .position(|action| Some(action.id) == current)
            .map_or(0, |index| clamped(index, step, actions.len()));
        let id = actions[index].id;
        if let Some(step) = &mut self.actions {
            step.selected = Some(id);
            self.reveal += 1;
        }
    }

    fn accept_enter(&mut self) -> Vec<PickerEffect> {
        if !self.open || self.pending_actions.is_some() {
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

    fn accept_action(&mut self, id: &'static str) -> Vec<PickerEffect> {
        if self.visible_actions().iter().any(|action| action.id == id) {
            if let Some(key) = self.selected().cloned() {
                return self.activate(&key, Some(id));
            }
        }
        Vec::new()
    }

    /// Activate an action by id, or the row's first action for Enter.
    fn activate(&mut self, key: &RowKey, action: Option<&'static str>) -> Vec<PickerEffect> {
        let Some(row) = self.rows.iter().find(|row| row.key == *key) else {
            log::info!("picker: activation found no row for its key");
            return Vec::new();
        };
        if let Some(reason) = row.unavailable {
            log::info!("picker: refused unavailable row ({reason})");
            return Vec::new();
        }
        let chosen = match action {
            Some(id) => row.actions.iter().find(|action| action.id == id),
            None => row.actions.first(),
        };
        let Some(action) = chosen.map(|action| action.id) else {
            log::warn!("picker: row has no matching action");
            return Vec::new();
        };
        log::info!(
            "picker: activate source '{}' action '{action}'",
            key.source()
        );
        let mut effects = vec![PickerEffect::Activate {
            stamp: self.rows_stamp,
            row: key.clone(),
            action,
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
        RankingPreset::Attention => {
            rows.sort_by(|a, b| {
                b.attention
                    .cmp(&a.attention)
                    .then_with(|| a.title.cmp(&b.title))
            });
            rows
        }
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
        RankingPreset::Usage => {
            rows.sort_by_cached_key(|row| {
                usage_rank_key(row.file_match, row.score, row.usage, row.modified)
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

pub(crate) mod files;
pub(crate) mod preview;
pub(crate) mod usage;

#[cfg(test)]
mod tests;
