use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

fn routines_file() -> String {
    format!("{}/routines.toml", crate::config::workspace_channel_dir())
}

/// Machine-local runtime state beside `routines.toml` — currently just
/// last-fire timestamps, so interval routines survive a host restart without
/// re-firing (stint 0573). Safe to delete; a missing or corrupt file degrades
/// to firing once on next launch.
fn routine_state_file() -> String {
    format!(
        "{}/routine-state.toml",
        crate::config::workspace_channel_dir()
    )
}

/// Parsed `{workspace_channel_dir}/routines.toml` (e.g. `.plexi-alpha/routines.toml`
/// on the alpha channel).
#[derive(Deserialize, Default)]
pub(crate) struct RoutinesConfig {
    #[serde(default)]
    pub routine: Vec<Routine>,
}

/// On-disk shape of `routine-state.toml`: routine name → RFC 3339 last-fire.
#[derive(Deserialize, Serialize, Default)]
struct RoutineState {
    #[serde(default)]
    last_fire: HashMap<String, String>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct Routine {
    pub name: String,
    pub command: String,
    pub schedule: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub ephemeral: bool,
}

#[derive(Clone)]
pub(crate) enum ParsedSchedule {
    /// Run every N seconds
    Interval { secs: u64 },
    /// Run when this cron expression matches (checked at minute-level)
    Cron(CronExpr),
}

#[derive(Clone)]
pub(crate) struct CronExpr {
    pub minute: CronField,
    pub hour: CronField,
    pub day_of_month: CronField,
    pub month: CronField,
    pub day_of_week: CronField,
}

#[derive(Clone)]
pub(crate) enum CronField {
    Any,
    Single(u32),
    Range(u32, u32),
    List(Vec<u32>),
}

pub(crate) struct SchedulerEntry {
    pub routine: Routine,
    pub schedule: ParsedSchedule,
    pub last_fire: Option<chrono::DateTime<chrono::Local>>,
    pub source_root: PathBuf,
}

/// A routine whose spawned pane is still open with its command running.
/// Tracked so `tick_scheduler` can skip (never stack) overlapping runs.
pub(crate) struct LiveRun {
    pub pane_id: u64,
    /// One notification per skip streak: set after the first skip of this run
    /// is surfaced, cleared when the run is reaped (the streak ends with it).
    pub skip_notified: bool,
}

/// A user-visible problem discovered while (re)loading a root's routines,
/// returned only on the transition into that failure state — repeated reloads
/// observing the same failure return nothing (stint 0573).
pub(crate) struct RoutineLoadFailure {
    pub title: String,
    pub body: String,
}

pub(crate) struct Scheduler {
    pub entries: Vec<SchedulerEntry>,
    /// Per-workspace-root → last loaded entries (avoid re-parsing every tick)
    loaded_roots: HashMap<PathBuf, std::time::Instant>,
    /// Overlap guard: (source_root, routine name) → the run's spawned pane.
    /// Registered on fire, reaped when the pane closes (`reap_pane`) or when
    /// the run is observed finished at skip-decision time (`reap_run`).
    live_runs: HashMap<(PathBuf, String), LiveRun>,
    /// Failure states currently known (and already notified). Keyed by
    /// "{root}|{subject}|{kind}[|detail]" so each (routine, reason) pair
    /// notifies once on transition into failure, not on every observation.
    known_failures: HashSet<String>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            loaded_roots: HashMap::new(),
            live_runs: HashMap::new(),
            known_failures: HashSet::new(),
        }
    }

    /// Load (or reload) routines from `root/{workspace_channel_dir}/routines.toml`.
    /// Called once per context root. Re-reads if not yet loaded or 60s have passed.
    /// Returns newly-entered failure states for the caller to surface.
    pub fn load_from_root(&mut self, root: &Path) -> Vec<RoutineLoadFailure> {
        // Cache check first — avoids path.exists() syscall every second
        if let Some(last) = self.loaded_roots.get(root) {
            if last.elapsed().as_secs() < 60 {
                return Vec::new();
            }
        }
        self.loaded_roots
            .insert(root.to_path_buf(), std::time::Instant::now());

        let root_key = root.display();
        let path = root.join(routines_file());
        if !path.exists() {
            // Prune any entries and failure states that came from this root
            // (file was deleted — nothing about it can still be failing).
            self.entries.retain(|e| e.source_root != root);
            let prefix = format!("{root_key}|");
            self.known_failures.retain(|k| !k.starts_with(&prefix));
            return Vec::new();
        }
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("scheduler: failed to read {}: {e}", path.display());
                return self.note_load_failure(
                    format!("{root_key}|<file>|unreadable"),
                    "Routines file unreadable".to_string(),
                    format!("Could not read {}: {e}", path.display()),
                );
            }
        };
        let config: RoutinesConfig = match toml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("scheduler: failed to parse {}: {e}", path.display());
                return self.note_load_failure(
                    format!("{root_key}|<file>|malformed"),
                    "Routines file malformed".to_string(),
                    format!("Failed to parse {}: {e}", path.display()),
                );
            }
        };
        // The file read and parsed — those failure states (if any) are over.
        self.known_failures
            .remove(&format!("{root_key}|<file>|unreadable"));
        self.known_failures
            .remove(&format!("{root_key}|<file>|malformed"));

        // Preserve last_fire for routines that survive the reload (avoid re-firing
        // immediately); seed from the persisted state file where memory has nothing.
        // In-memory wins — a reload must never rewind a fire that already happened
        // this session.
        let mut last_fires: HashMap<String, Option<chrono::DateTime<chrono::Local>>> =
            HashMap::new();
        for e in &self.entries {
            if e.source_root == root {
                last_fires.insert(e.routine.name.clone(), e.last_fire);
            }
        }
        let persisted = load_routine_state(root);
        // Replace all entries from this root with the freshly parsed set
        self.entries.retain(|e| e.source_root != root);
        let count = config.routine.len();
        let mut new_failures = Vec::new();
        for routine in config.routine {
            // A schedule edit resolves the routine's previous bad-schedule
            // state whether or not the new string parses; a *different* bad
            // string is a new failure and notifies again.
            let bad_schedule_prefix = format!("{root_key}|{}|bad-schedule|", routine.name);
            let current_key = format!("{bad_schedule_prefix}{}", routine.schedule);
            let schedule = match parse_schedule(&routine.schedule) {
                Some(s) => {
                    self.known_failures
                        .retain(|k| !k.starts_with(&bad_schedule_prefix));
                    s
                }
                None => {
                    log::warn!(
                        "scheduler: routine '{}' has unparseable schedule '{}', skipping",
                        routine.name,
                        routine.schedule
                    );
                    self.known_failures
                        .retain(|k| !k.starts_with(&bad_schedule_prefix) || *k == current_key);
                    if self.known_failures.insert(current_key) {
                        new_failures.push(RoutineLoadFailure {
                            title: format!("Routine '{}'", routine.name),
                            body: format!(
                                "Schedule '{}' is not a recognized format — the routine will never fire. See `plexi routine --help` for accepted forms.",
                                routine.schedule
                            ),
                        });
                    }
                    continue;
                }
            };
            let last_fire = last_fires
                .get(&routine.name)
                .copied()
                .flatten()
                .or_else(|| persisted.get(&routine.name).copied());
            self.entries.push(SchedulerEntry {
                routine,
                schedule,
                last_fire,
                source_root: root.to_path_buf(),
            });
        }
        log::info!("scheduler: loaded {count} routines from {}", path.display());
        new_failures
    }

    /// Record a file-level load failure; returns it only on the transition in.
    fn note_load_failure(
        &mut self,
        key: String,
        title: String,
        body: String,
    ) -> Vec<RoutineLoadFailure> {
        if self.known_failures.insert(key) {
            vec![RoutineLoadFailure { title, body }]
        } else {
            Vec::new()
        }
    }

    /// Record a fire-time failure state (missing context, spawn failure).
    /// Returns true when this is a new state — i.e. the caller should notify.
    pub fn note_failure(&mut self, key: String) -> bool {
        self.known_failures.insert(key)
    }

    /// Clear a fire-time failure state so a future recurrence notifies again.
    pub fn clear_failure(&mut self, key: &str) {
        self.known_failures.remove(key);
    }

    /// Check which routines are due. Returns their indices.
    pub fn due_routines(&mut self, now: chrono::DateTime<chrono::Local>) -> Vec<usize> {
        let mut due = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            if is_due(&entry.schedule, now, entry.last_fire.as_ref()) {
                due.push(i);
            }
        }
        due
    }

    pub fn mark_fired(&mut self, idx: usize, now: chrono::DateTime<chrono::Local>) {
        if let Some(entry) = self.entries.get_mut(idx) {
            entry.last_fire = Some(now);
            let root = entry.source_root.clone();
            self.persist_state_for_root(&root);
        }
    }

    /// Write this root's last-fire map to `routine-state.toml`, atomically
    /// (temp + rename) and best-effort: a write failure is logged and never
    /// aborts the fire that triggered it.
    fn persist_state_for_root(&self, root: &Path) {
        let mut state = RoutineState::default();
        for e in &self.entries {
            if e.source_root == root {
                if let Some(lf) = e.last_fire {
                    state
                        .last_fire
                        .insert(e.routine.name.clone(), lf.to_rfc3339());
                }
            }
        }
        let path = root.join(routine_state_file());
        let toml_str = match toml::to_string(&state) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("scheduler: failed to serialize routine state for {root:?}: {e}");
                return;
            }
        };
        let tmp = path.with_extension("toml.tmp");
        if let Err(e) = std::fs::write(&tmp, &toml_str).and_then(|_| std::fs::rename(&tmp, &path)) {
            log::warn!(
                "scheduler: failed to persist routine state to {}: {e}",
                path.display()
            );
        }
    }

    // ── Overlap guard registry ───────────────────────────────────────────────

    pub fn register_run(&mut self, root: &Path, name: &str, pane_id: u64) {
        self.live_runs.insert(
            (root.to_path_buf(), name.to_string()),
            LiveRun {
                pane_id,
                skip_notified: false,
            },
        );
    }

    pub fn live_run(&self, root: &Path, name: &str) -> Option<&LiveRun> {
        self.live_runs.get(&(root.to_path_buf(), name.to_string()))
    }

    pub fn mark_skip_notified(&mut self, root: &Path, name: &str) {
        if let Some(run) = self
            .live_runs
            .get_mut(&(root.to_path_buf(), name.to_string()))
        {
            run.skip_notified = true;
        }
    }

    /// Drop the live-run record for a routine observed finished.
    pub fn reap_run(&mut self, root: &Path, name: &str) {
        self.live_runs
            .remove(&(root.to_path_buf(), name.to_string()));
    }

    /// Drop any live-run record whose pane just closed. Returns the reaped
    /// routine names. This is the destroy half of the register/reap coupling —
    /// called from the host's pane-close path (`close_tile`).
    pub fn reap_pane(&mut self, pane_id: u64) -> Vec<String> {
        let reaped: Vec<String> = self
            .live_runs
            .iter()
            .filter(|(_, run)| run.pane_id == pane_id)
            .map(|((_, name), _)| name.clone())
            .collect();
        self.live_runs.retain(|_, run| run.pane_id != pane_id);
        reaped
    }

    /// Number of live routine runs currently tracked (test observability).
    #[cfg(test)]
    pub fn live_run_count(&self) -> usize {
        self.live_runs.len()
    }

    /// Test-only: drop the 60s reload cache so the next `load_from_root`
    /// re-reads the file immediately.
    #[cfg(test)]
    pub fn clear_reload_cache(&mut self) {
        self.loaded_roots.clear();
    }
}

/// Read the persisted last-fire map for a root. Missing or corrupt state is
/// not an error — it degrades to the routine firing once on next launch.
fn load_routine_state(root: &Path) -> HashMap<String, chrono::DateTime<chrono::Local>> {
    let path = root.join(routine_state_file());
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return HashMap::new(),
        Err(e) => {
            log::warn!(
                "scheduler: failed to read routine state {}: {e} — ignoring",
                path.display()
            );
            return HashMap::new();
        }
    };
    let state: RoutineState = match toml::from_str(&contents) {
        Ok(s) => s,
        Err(e) => {
            log::warn!(
                "scheduler: corrupt routine state {}: {e} — ignoring",
                path.display()
            );
            return HashMap::new();
        }
    };
    state
        .last_fire
        .iter()
        .filter_map(
            |(name, ts)| match chrono::DateTime::parse_from_rfc3339(ts) {
                Ok(t) => Some((name.clone(), t.with_timezone(&chrono::Local))),
                Err(e) => {
                    log::warn!(
                        "scheduler: bad last_fire timestamp for '{name}' in {}: {e} — ignoring",
                        path.display()
                    );
                    None
                }
            },
        )
        .collect()
}

fn is_due(
    schedule: &ParsedSchedule,
    now: chrono::DateTime<chrono::Local>,
    last_fire: Option<&chrono::DateTime<chrono::Local>>,
) -> bool {
    match schedule {
        ParsedSchedule::Interval { secs } => match last_fire {
            None => true,
            Some(lf) => (now - *lf).num_seconds() >= *secs as i64,
        },
        ParsedSchedule::Cron(expr) => {
            if !cron_matches(expr, &now) {
                return false;
            }
            // Only fire once per minute matching the expression
            match last_fire {
                None => true,
                Some(lf) => {
                    use chrono::Timelike;
                    // Different minute than last fire
                    let now_min = now.with_second(0).and_then(|t| t.with_nanosecond(0));
                    let lf_min = lf.with_second(0).and_then(|t| t.with_nanosecond(0));
                    match (now_min, lf_min) {
                        (Some(n), Some(l)) => n != l,
                        _ => true,
                    }
                }
            }
        }
    }
}

fn cron_matches(expr: &CronExpr, now: &chrono::DateTime<chrono::Local>) -> bool {
    cron_date_matches(expr, now.date_naive()) && cron_time_matches(expr, now.time())
}

fn cron_date_matches(expr: &CronExpr, date: chrono::NaiveDate) -> bool {
    use chrono::Datelike;
    // chrono: weekday().num_days_from_monday() gives 0=Mon..6=Sun; cron 1=Mon..7=Sun
    let weekday = date.weekday().num_days_from_monday() + 1;
    field_matches(&expr.day_of_month, date.day())
        && field_matches(&expr.month, date.month())
        && field_matches(&expr.day_of_week, weekday)
}

fn cron_time_matches(expr: &CronExpr, time: chrono::NaiveTime) -> bool {
    use chrono::Timelike;
    field_matches(&expr.minute, time.minute()) && field_matches(&expr.hour, time.hour())
}

fn field_matches(field: &CronField, value: u32) -> bool {
    match field {
        CronField::Any => true,
        CronField::Single(n) => *n == value,
        CronField::Range(a, b) => value >= *a && value <= *b,
        CronField::List(vals) => vals.contains(&value),
    }
}

/// Case-insensitive ASCII prefix strip.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Seconds per one of the given interval unit: short suffix or full word,
/// singular or plural. Unit-token matching — never strip single characters,
/// so `"30 second"` can never be produced from `"30 seconds"`.
fn interval_unit_secs(unit: &str) -> Option<u64> {
    match unit.to_ascii_lowercase().as_str() {
        "s" | "second" | "seconds" => Some(1),
        "m" | "minute" | "minutes" => Some(60),
        "h" | "hour" | "hours" => Some(3600),
        _ => None,
    }
}

/// Parse a schedule string into a `ParsedSchedule`.
/// Returns `None` if the string cannot be parsed.
pub(crate) fn parse_schedule(s: &str) -> Option<ParsedSchedule> {
    let s = s.trim();

    // "every 30s" / "every 30 seconds" / "every 5 minutes" / "every 2 hours"
    if let Some(rest) = strip_prefix_ci(s, "every ") {
        let rest = rest.trim();
        // "every minute" / "every hour"
        if rest.eq_ignore_ascii_case("minute") {
            return Some(ParsedSchedule::Interval { secs: 60 });
        }
        if rest.eq_ignore_ascii_case("hour") {
            return Some(ParsedSchedule::Interval { secs: 3600 });
        }
        // Number and unit as two tokens ("30 seconds") or glued ("30s").
        let (n_str, unit) = match rest.split_once(char::is_whitespace) {
            Some((n, u)) => (n.trim(), u.trim()),
            None => rest.split_at(rest.find(|c: char| !c.is_ascii_digit())?),
        };
        let n: u64 = n_str.parse().ok()?;
        let secs = n.checked_mul(interval_unit_secs(unit)?)?;
        return Some(ParsedSchedule::Interval { secs });
    }

    // "daily at 9am" / "daily at 10:30am"
    if let Some(rest) = strip_prefix_ci(s, "daily at ") {
        let (hour, minute) = parse_time(rest)?;
        return Some(ParsedSchedule::Cron(CronExpr {
            minute: CronField::Single(minute),
            hour: CronField::Single(hour),
            day_of_month: CronField::Any,
            month: CronField::Any,
            day_of_week: CronField::Any,
        }));
    }

    // "weekdays at 9am"
    if let Some(rest) = strip_prefix_ci(s, "weekdays at ") {
        let (hour, minute) = parse_time(rest)?;
        return Some(ParsedSchedule::Cron(CronExpr {
            minute: CronField::Single(minute),
            hour: CronField::Single(hour),
            day_of_month: CronField::Any,
            month: CronField::Any,
            day_of_week: CronField::Range(1, 5), // Mon-Fri
        }));
    }

    // "weekends at 9am"
    if let Some(rest) = strip_prefix_ci(s, "weekends at ") {
        let (hour, minute) = parse_time(rest)?;
        return Some(ParsedSchedule::Cron(CronExpr {
            minute: CronField::Single(minute),
            hour: CronField::Single(hour),
            day_of_month: CronField::Any,
            month: CronField::Any,
            day_of_week: CronField::List(vec![6, 7]), // Sat-Sun
        }));
    }

    // "weekly on monday at 09:00" / "weekly on mon at 9am"
    if let Some(rest) = strip_prefix_ci(s, "weekly on ") {
        let (day_str, time_str) = rest.split_once(" at ")?;
        let dow = parse_day_name(day_str)?;
        let (hour, minute) = parse_time(time_str)?;
        return Some(ParsedSchedule::Cron(CronExpr {
            minute: CronField::Single(minute),
            hour: CronField::Single(hour),
            day_of_month: CronField::Any,
            month: CronField::Any,
            day_of_week: CronField::Single(dow),
        }));
    }

    // "monthly on 1 at 08:00"
    if let Some(rest) = strip_prefix_ci(s, "monthly on ") {
        let (day_str, time_str) = rest.split_once(" at ")?;
        let day: u32 = day_str.trim().parse().ok()?;
        if !(1..=31).contains(&day) {
            return None;
        }
        let (hour, minute) = parse_time(time_str)?;
        return Some(ParsedSchedule::Cron(CronExpr {
            minute: CronField::Single(minute),
            hour: CronField::Single(hour),
            day_of_month: CronField::Single(day),
            month: CronField::Any,
            day_of_week: CronField::Any,
        }));
    }

    // Raw 5-field cron: "0 9 * * 1-5"
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() == 5 {
        let minute = parse_cron_field(parts[0])?;
        let hour = parse_cron_field(parts[1])?;
        let dom = parse_cron_field(parts[2])?;
        let month = parse_cron_field(parts[3])?;
        let dow = parse_cron_field_dow(parts[4])?;
        return Some(ParsedSchedule::Cron(CronExpr {
            minute,
            hour,
            day_of_month: dom,
            month,
            day_of_week: dow,
        }));
    }

    None
}

/// Parse "9am", "10pm", "9:30am", "14:00" → (hour_24, minute)
fn parse_time(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    // Strip am/pm
    let (s, pm) = if let Some(stripped) = s.strip_suffix("pm") {
        (stripped.trim(), true)
    } else if let Some(stripped) = s.strip_suffix("am") {
        (stripped.trim(), false)
    } else {
        (s, false)
    };
    // Try "H:MM"
    if let Some((h_str, m_str)) = s.split_once(':') {
        let mut h: u32 = h_str.parse().ok()?;
        let m: u32 = m_str.parse().ok()?;
        if pm && h != 12 {
            h += 12;
        }
        if !pm && h == 12 {
            h = 0;
        }
        return Some((h, m));
    }
    // Just hour
    let mut h: u32 = s.parse().ok()?;
    if pm && h != 12 {
        h += 12;
    }
    if !pm && h == 12 {
        h = 0;
    }
    Some((h, 0))
}

fn parse_cron_field(s: &str) -> Option<CronField> {
    if s == "*" {
        return Some(CronField::Any);
    }
    if let Some((a, b)) = s.split_once('-') {
        let a: u32 = a.parse().ok()?;
        let b: u32 = b.parse().ok()?;
        return Some(CronField::Range(a, b));
    }
    if s.contains(',') {
        let vals: Option<Vec<u32>> = s.split(',').map(|v| v.parse().ok()).collect();
        return Some(CronField::List(vals?));
    }
    let n: u32 = s.parse().ok()?;
    Some(CronField::Single(n))
}

/// Short day names, indexed sun=0..sat=6 (converted to cron 1=Mon..7=Sun).
const DAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const DAY_NAMES_FULL: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

/// Parse a day name — short (`mon`) or full (`monday`) spelling — into the
/// cron day-of-week number (1=Mon..7=Sun).
fn parse_day_name(s: &str) -> Option<u32> {
    let s = s.trim().to_ascii_lowercase();
    let idx = DAY_NAMES
        .iter()
        .position(|d| *d == s)
        .or_else(|| DAY_NAMES_FULL.iter().position(|d| *d == s))?;
    Some(if idx == 0 { 7 } else { idx as u32 })
}

/// Parse day-of-week field, supporting mon-fri names
fn parse_cron_field_dow(s: &str) -> Option<CronField> {
    let s_lower = s.to_lowercase();
    // Named day ranges like "mon-fri"
    let day_names = DAY_NAMES;
    let s_lo = s_lower.as_str();
    // Check for named range
    if let Some((a_str, b_str)) = s_lo.split_once('-') {
        let a_idx = day_names.iter().position(|&d| d == a_str);
        let b_idx = day_names.iter().position(|&d| d == b_str);
        if let (Some(a), Some(b)) = (a_idx, b_idx) {
            // Convert sun=0 to sun=7 for standard cron (1=Mon..7=Sun)
            let a = if a == 0 { 7 } else { a as u32 };
            let b = if b == 0 { 7 } else { b as u32 };
            return Some(CronField::Range(a, b));
        }
    }
    // Named single day
    if let Some(idx) = day_names.iter().position(|&d| d == s_lo) {
        let n = if idx == 0 { 7 } else { idx as u32 };
        return Some(CronField::Single(n));
    }
    // Numeric — normalize 0 to 7 (both mean Sunday; chrono gives 7 for Sunday via num_days_from_monday+1)
    let field = parse_cron_field(s)?;
    Some(match field {
        CronField::Single(0) => CronField::Single(7),
        CronField::Range(0, b) => CronField::Range(7, b),
        CronField::Range(a, 0) => CronField::Range(a, 7),
        other => other,
    })
}

/// Compute human-readable next-fire description for CLI display.
pub(crate) fn next_fire_description(
    schedule: &ParsedSchedule,
    last_fire: Option<&chrono::DateTime<chrono::Local>>,
) -> String {
    next_fire_description_at(schedule, last_fire, chrono::Local::now())
}

fn next_fire_description_at(
    schedule: &ParsedSchedule,
    last_fire: Option<&chrono::DateTime<chrono::Local>>,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    match schedule {
        ParsedSchedule::Interval { secs } => match last_fire {
            None => format!("in {}s (never fired)", secs),
            Some(lf) => {
                let elapsed = (now - *lf).num_seconds().max(0) as u64;
                if elapsed >= *secs {
                    "now (overdue)".to_string()
                } else {
                    let remaining = secs - elapsed;
                    if remaining >= 3600 {
                        format!("in {}h {}m", remaining / 3600, (remaining % 3600) / 60)
                    } else if remaining >= 60 {
                        format!("in {}m {}s", remaining / 60, remaining % 60)
                    } else {
                        format!("in {}s", remaining)
                    }
                }
            }
        },
        ParsedSchedule::Cron(expr) => {
            // Walk days with the date half of the fire matcher, then take
            // the earliest matching wall-clock slot of the first matching
            // day — the same `cron_date_matches`/`cron_time_matches` pair
            // `cron_matches` fires on, so the display can never disagree
            // with an actual fire. The horizon must cover the sparsest
            // reachable schedule: a leap-day cron across the 2100-style
            // century gap (Feb 29 2096 → Feb 29 2104) is eight years out,
            // so nine years of days bounds it. Anything unmatched by then
            // is genuinely unreachable (e.g. `0 8 30 2 *`, Feb 30).
            use chrono::{Duration, Timelike};
            let day_slots: Vec<(u32, u32)> = (0..24u32)
                .filter(|h| field_matches(&expr.hour, *h))
                .flat_map(|h| {
                    (0..60u32)
                        .filter(|m| field_matches(&expr.minute, *m))
                        .map(move |m| (h, m))
                })
                .collect();
            if day_slots.is_empty() {
                return "unknown".to_string();
            }
            const MAX_DAYS: i64 = 366 * 9;
            for d in 0..=MAX_DAYS {
                let date = now.date_naive() + Duration::days(d);
                if !cron_date_matches(expr, date) {
                    continue;
                }
                // Today only counts slots strictly after the current minute.
                let slot = if d == 0 {
                    day_slots
                        .iter()
                        .copied()
                        .find(|&(h, m)| (h, m) > (now.hour(), now.minute()))
                } else {
                    day_slots.first().copied()
                };
                let Some((h, m)) = slot else { continue };
                let when = if d == 0 {
                    String::new()
                } else if d == 1 {
                    "tomorrow ".to_string()
                } else {
                    format!("{date} ")
                };
                return format!("{when}at {h:02}:{m:02}");
            }
            "unknown".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_interval_minutes() {
        match parse_schedule("every 30m") {
            Some(ParsedSchedule::Interval { secs: 1800 }) => {}
            other => panic!(
                "expected Interval(1800), got {:?}",
                other.as_ref().map(|_| "Some")
            ),
        }
    }

    #[test]
    fn parse_interval_hours() {
        match parse_schedule("every 2h") {
            Some(ParsedSchedule::Interval { secs: 7200 }) => {}
            other => panic!(
                "expected Interval(7200), got {:?}",
                other.as_ref().map(|_| "Some")
            ),
        }
    }

    #[test]
    fn parse_daily_at() {
        let s = parse_schedule("daily at 9am").expect("should parse");
        match s {
            ParsedSchedule::Cron(expr) => {
                assert!(matches!(expr.hour, CronField::Single(9)));
                assert!(matches!(expr.minute, CronField::Single(0)));
                assert!(matches!(expr.day_of_week, CronField::Any));
            }
            _ => panic!("expected Cron"),
        }
    }

    #[test]
    fn parse_weekdays() {
        let s = parse_schedule("weekdays at 9am").expect("should parse");
        match s {
            ParsedSchedule::Cron(expr) => {
                assert!(matches!(expr.day_of_week, CronField::Range(1, 5)));
            }
            _ => panic!("expected Cron"),
        }
    }

    #[test]
    fn parse_raw_cron() {
        let s = parse_schedule("0 9 * * 1-5").expect("should parse");
        match s {
            ParsedSchedule::Cron(expr) => {
                assert!(matches!(expr.minute, CronField::Single(0)));
                assert!(matches!(expr.hour, CronField::Single(9)));
                assert!(matches!(expr.day_of_week, CronField::Range(1, 5)));
            }
            _ => panic!("expected Cron"),
        }
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(parse_schedule("garbage schedule").is_none());
        assert!(parse_schedule("").is_none());
        assert!(parse_schedule("every 30 parsecs").is_none());
        assert!(parse_schedule("every fast").is_none());
        assert!(parse_schedule("weekly on funday at 9am").is_none());
        assert!(parse_schedule("weekly on monday").is_none());
        assert!(parse_schedule("monthly on x at 8am").is_none());
        assert!(parse_schedule("monthly on 32 at 8am").is_none());
        assert!(parse_schedule("monthly on 0 at 8am").is_none());
    }

    fn interval_secs(s: &str) -> u64 {
        match parse_schedule(s) {
            Some(ParsedSchedule::Interval { secs }) => secs,
            _ => panic!("expected Interval for {s:?}"),
        }
    }

    /// Every row of the documented schedule table (website cli.md), both
    /// spellings where two exist (stint 0571).
    #[test]
    fn parse_documented_interval_forms() {
        assert_eq!(interval_secs("every 30 seconds"), 30);
        assert_eq!(interval_secs("every 1 second"), 1);
        assert_eq!(interval_secs("every 5 minutes"), 300);
        assert_eq!(interval_secs("every 1 minute"), 60);
        assert_eq!(interval_secs("every 2 hours"), 7200);
        assert_eq!(interval_secs("every 1 hour"), 3600);
        // Short forms and bare units keep working.
        assert_eq!(interval_secs("every 30s"), 30);
        assert_eq!(interval_secs("every 30m"), 1800);
        assert_eq!(interval_secs("every 2h"), 7200);
        assert_eq!(interval_secs("every minute"), 60);
        assert_eq!(interval_secs("every hour"), 3600);
        assert_eq!(interval_secs("Every 5 minutes"), 300);
    }

    #[test]
    fn parse_weekly_on_day() {
        for s in ["weekly on monday at 09:00", "weekly on mon at 9am"] {
            match parse_schedule(s).unwrap_or_else(|| panic!("should parse {s:?}")) {
                ParsedSchedule::Cron(expr) => {
                    assert!(matches!(expr.hour, CronField::Single(9)), "{s}");
                    assert!(matches!(expr.minute, CronField::Single(0)), "{s}");
                    assert!(matches!(expr.day_of_week, CronField::Single(1)), "{s}");
                    assert!(matches!(expr.day_of_month, CronField::Any), "{s}");
                }
                _ => panic!("expected Cron for {s:?}"),
            }
        }
        // Sunday maps to cron 7, both spellings.
        for s in ["weekly on sun at 8am", "weekly on sunday at 8am"] {
            match parse_schedule(s).unwrap_or_else(|| panic!("should parse {s:?}")) {
                ParsedSchedule::Cron(expr) => {
                    assert!(matches!(expr.day_of_week, CronField::Single(7)), "{s}");
                }
                _ => panic!("expected Cron for {s:?}"),
            }
        }
    }

    #[test]
    fn parse_monthly_on_day() {
        match parse_schedule("monthly on 1 at 08:00").expect("should parse") {
            ParsedSchedule::Cron(expr) => {
                assert!(matches!(expr.hour, CronField::Single(8)));
                assert!(matches!(expr.minute, CronField::Single(0)));
                assert!(matches!(expr.day_of_month, CronField::Single(1)));
                assert!(matches!(expr.day_of_week, CronField::Any));
            }
            _ => panic!("expected Cron"),
        }
    }

    #[test]
    fn parse_weekends() {
        match parse_schedule("weekends at 10:30am").expect("should parse") {
            ParsedSchedule::Cron(expr) => {
                assert!(matches!(expr.hour, CronField::Single(10)));
                assert!(matches!(expr.minute, CronField::Single(30)));
                assert!(matches!(expr.day_of_week, CronField::List(_)));
            }
            _ => panic!("expected Cron"),
        }
    }

    #[test]
    fn parse_daily_at_24h_time() {
        match parse_schedule("daily at 09:00").expect("should parse") {
            ParsedSchedule::Cron(expr) => {
                assert!(matches!(expr.hour, CronField::Single(9)));
                assert!(matches!(expr.minute, CronField::Single(0)));
            }
            _ => panic!("expected Cron"),
        }
    }

    // ── last_fire persistence (stint 0573) ──────────────────────────────────

    fn write_routines(root: &Path, body: &str) {
        let dir = root.join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&dir).expect("create channel dir");
        std::fs::write(dir.join("routines.toml"), body).expect("write routines.toml");
    }

    const ONE_ROUTINE: &str = r#"
        [[routine]]
        name = "sync"
        command = "true"
        schedule = "every 2 hours"
    "#;

    #[test]
    fn last_fire_survives_scheduler_rebuild() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        write_routines(root, ONE_ROUTINE);

        let now = chrono::Local::now();
        let mut s1 = Scheduler::new();
        assert!(s1.load_from_root(root).is_empty());
        assert_eq!(s1.due_routines(now).len(), 1, "fresh routine is due");
        s1.mark_fired(0, now);
        assert!(s1.due_routines(now).is_empty());

        // Simulated restart: a brand-new Scheduler seeds from the state file
        // and must NOT re-fire inside the interval.
        let mut s2 = Scheduler::new();
        assert!(s2.load_from_root(root).is_empty());
        assert!(
            s2.due_routines(chrono::Local::now()).is_empty(),
            "persisted last_fire must suppress the startup fire"
        );
        // …but a last_fire older than the interval is due again.
        let mut s3 = Scheduler::new();
        s3.load_from_root(root);
        assert_eq!(
            s3.due_routines(now + chrono::Duration::hours(3)).len(),
            1,
            "overdue routine fires once on next launch"
        );
    }

    #[test]
    fn in_memory_last_fire_wins_over_persisted_on_reload() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        write_routines(root, ONE_ROUTINE);

        let mut s = Scheduler::new();
        s.load_from_root(root);
        let session_fire = chrono::Local::now();
        s.mark_fired(0, session_fire);

        // Rewind the *persisted* value to 3 hours ago — a reload must not let
        // it win over the fire that already happened this session.
        let stale = (session_fire - chrono::Duration::hours(3)).to_rfc3339();
        let state_path = root
            .join(crate::config::workspace_channel_dir())
            .join("routine-state.toml");
        std::fs::write(&state_path, format!("[last_fire]\nsync = \"{stale}\"\n"))
            .expect("write state");

        s.clear_reload_cache();
        s.load_from_root(root);
        assert!(
            s.due_routines(chrono::Local::now()).is_empty(),
            "reload must not rewind an in-session fire"
        );
    }

    #[test]
    fn corrupt_state_file_degrades_to_startup_fire() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        write_routines(root, ONE_ROUTINE);
        std::fs::write(
            root.join(crate::config::workspace_channel_dir())
                .join("routine-state.toml"),
            "not [ valid { toml",
        )
        .expect("write corrupt state");

        let mut s = Scheduler::new();
        assert!(
            s.load_from_root(root).is_empty(),
            "corrupt state is not a load failure"
        );
        assert_eq!(s.due_routines(chrono::Local::now()).len(), 1);
    }

    #[test]
    fn state_write_failure_does_not_abort_the_fire() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        write_routines(root, ONE_ROUTINE);

        let mut s = Scheduler::new();
        s.load_from_root(root);
        // Remove the channel dir so the state write fails.
        std::fs::remove_dir_all(root.join(crate::config::workspace_channel_dir()))
            .expect("remove channel dir");
        let now = chrono::Local::now();
        s.mark_fired(0, now);
        assert_eq!(
            s.entries[0].last_fire,
            Some(now),
            "in-memory last_fire must be stamped even when persistence fails"
        );
    }

    // ── load-failure transitions (stint 0573) ───────────────────────────────

    #[test]
    fn bad_schedule_notifies_once_per_distinct_schedule() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let bad = r#"
            [[routine]]
            name = "sync"
            command = "true"
            schedule = "every fortnight"
        "#;
        write_routines(root, bad);

        let mut s = Scheduler::new();
        let first = s.load_from_root(root);
        assert_eq!(first.len(), 1, "transition into failure notifies");
        s.clear_reload_cache();
        assert!(
            s.load_from_root(root).is_empty(),
            "same failure must not re-notify on reload"
        );

        // A *different* bad schedule is a new failure state.
        write_routines(
            root,
            r#"
            [[routine]]
            name = "sync"
            command = "true"
            schedule = "every blue moon"
        "#,
        );
        s.clear_reload_cache();
        assert_eq!(s.load_from_root(root).len(), 1);

        // Fixing the schedule clears the state; breaking it again re-notifies.
        write_routines(root, ONE_ROUTINE);
        s.clear_reload_cache();
        assert!(s.load_from_root(root).is_empty());
        write_routines(root, bad);
        s.clear_reload_cache();
        assert_eq!(s.load_from_root(root).len(), 1, "recurrence re-notifies");
    }

    #[test]
    fn malformed_file_notifies_once() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        write_routines(root, "not [ valid { toml");

        let mut s = Scheduler::new();
        assert_eq!(s.load_from_root(root).len(), 1);
        s.clear_reload_cache();
        assert!(s.load_from_root(root).is_empty());
    }

    // ── overlap-guard registry (stint 0573) ─────────────────────────────────

    #[test]
    fn reap_pane_drops_matching_runs() {
        let mut s = Scheduler::new();
        let root = PathBuf::from("/w");
        s.register_run(&root, "a", 7);
        s.register_run(&root, "b", 9);
        assert_eq!(s.live_run_count(), 2);
        let reaped = s.reap_pane(7);
        assert_eq!(reaped, vec!["a".to_string()]);
        assert_eq!(s.live_run_count(), 1);
        assert!(s.live_run(&root, "a").is_none());
        assert!(s.live_run(&root, "b").is_some());
    }

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Local> {
        use chrono::TimeZone;
        chrono::Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .expect("unambiguous local time")
    }

    fn next_desc(schedule: &str, now: chrono::DateTime<chrono::Local>) -> String {
        let s = parse_schedule(schedule).expect("schedule parses");
        next_fire_description_at(&s, None, now)
    }

    #[test]
    fn next_fire_same_day_omits_date() {
        assert_eq!(
            next_desc("daily at 09:00", local(2026, 7, 28, 8, 0)),
            "at 09:00"
        );
    }

    #[test]
    fn next_fire_next_day_says_tomorrow() {
        assert_eq!(
            next_desc("daily at 09:00", local(2026, 7, 28, 12, 0)),
            "tomorrow at 09:00"
        );
    }

    #[test]
    fn next_fire_monthly_resolves_a_full_cycle_out() {
        // 31 days out — beyond the old 7-day search window that returned
        // "unknown" for `monthly on N` most of every month.
        assert_eq!(
            next_desc("monthly on 28 at 08:00", local(2026, 7, 28, 12, 0)),
            "2026-08-28 at 08:00"
        );
    }

    #[test]
    fn next_fire_monthly_on_31_reaches_across_february() {
        assert_eq!(
            next_desc("monthly on 31 at 08:00", local(2026, 2, 1, 12, 0)),
            "2026-03-31 at 08:00"
        );
    }

    #[test]
    fn next_fire_weekly_shows_the_date() {
        // 2026-07-28 is a Tuesday; next Monday is 2026-08-03.
        assert_eq!(
            next_desc("weekly on monday at 09:00", local(2026, 7, 28, 12, 0)),
            "2026-08-03 at 09:00"
        );
    }

    #[test]
    fn next_fire_leap_day_cron_resolves_across_years() {
        let s = parse_schedule("0 8 29 2 *").expect("leap-day cron parses");
        assert_eq!(
            next_fire_description_at(&s, None, local(2026, 7, 28, 12, 0)),
            "2028-02-29 at 08:00"
        );
    }

    #[test]
    fn next_fire_unreachable_cron_stays_unknown() {
        // February 30th never exists.
        let s = parse_schedule("0 8 30 2 *").expect("raw cron parses");
        assert_eq!(
            next_fire_description_at(&s, None, local(2026, 7, 28, 12, 0)),
            "unknown"
        );
    }
}
