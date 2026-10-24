//! Per-pane record store for the Program Status Protocol (OSC 7501).
use std::collections::BTreeMap;

/// Records per terminal; the least recently updated one is evicted past this.
pub const MAX_RECORDS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Working => "working",
            State::Done => "done",
            State::Blocked => "blocked",
            State::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Permission,
    Question,
    Auth,
}

/// One record: exactly the keys of the last report that used its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub state: State,
    pub kind: Option<Kind>,
    pub progress: Option<u8>,
    pub msg: Option<String>,
    pub title: Option<String>,
    /// As reported; see [`ProgramStatusStore::effective_app`] for inheritance.
    pub app: Option<String>,
    /// The user has seen this `done` / `error` record.
    pub seen: bool,
    /// Monotonic update stamp (store-local), used for eviction order.
    pub updated_at: u64,
}

/// A validated report ready to apply to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// `id` is empty for the root record.
    Set { id: String, record: Record },
    /// `id` is empty to clear every record.
    Clear { id: String },
}

#[derive(Debug, Default)]
pub struct ProgramStatusStore {
    records: BTreeMap<String, Record>,
    tick: u64,
}

impl ProgramStatusStore {
    /// Apply a report. Returns the previous state of a replaced record, if any.
    pub fn apply(&mut self, report: Report) -> Option<State> {
        match report {
            Report::Clear { id } => {
                self.clear(&id);
                None
            }
            Report::Set { id, mut record } => {
                self.tick += 1;
                record.updated_at = self.tick;
                record.seen = false;
                let previous = self.records.get(&id).map(|r| r.state);
                if previous.is_none() && self.records.len() >= MAX_RECORDS {
                    if let Some(oldest) = self
                        .records
                        .iter()
                        .min_by_key(|(_, r)| r.updated_at)
                        .map(|(k, _)| k.clone())
                    {
                        self.records.remove(&oldest);
                    }
                }
                self.records.insert(id, record);
                previous
            }
        }
    }

    /// Remove `id` and everything beneath it; an empty id removes all.
    pub fn clear(&mut self, id: &str) {
        if id.is_empty() {
            self.records.clear();
            return;
        }
        let prefix = format!("{id}/");
        self.records
            .retain(|k, _| k != id && !k.starts_with(&prefix));
    }

    /// Process exit: drop `working` and `blocked`; `done` / `error` survive.
    pub fn on_process_exit(&mut self) {
        self.records
            .retain(|_, r| !matches!(r.state, State::Working | State::Blocked));
    }

    /// New shell prompt (OSC 133;A): same lifetime rule as process exit.
    pub fn on_prompt_start(&mut self) {
        self.on_process_exit();
    }

    /// RIS: remove every record.
    pub fn reset(&mut self) {
        self.records.clear();
    }

    /// The user has seen the results: flag `done` / `error` records and drop them.
    pub fn mark_seen(&mut self) {
        for r in self.records.values_mut() {
            if matches!(r.state, State::Done | State::Error) {
                r.seen = true;
            }
        }
        self.records.retain(|_, r| !r.seen);
    }

    pub fn get(&self, id: &str) -> Option<&Record> {
        self.records.get(id)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Records in id order (root first).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Record)> {
        self.records.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// `app` of the record, or of its nearest ancestor that has one.
    pub fn effective_app(&self, id: &str) -> Option<&str> {
        let mut cur = id;
        loop {
            if let Some(app) = self.records.get(cur).and_then(|r| r.app.as_deref()) {
                return Some(app);
            }
            if cur.is_empty() {
                return None;
            }
            cur = cur.rfind('/').map_or("", |i| &cur[..i]);
        }
    }
}
