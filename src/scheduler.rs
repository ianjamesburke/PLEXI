use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const ROUTINES_FILE: &str = ".plexi/routines.toml";

/// Parsed `.plexi/routines.toml`
#[derive(Deserialize, Default)]
pub(crate) struct RoutinesConfig {
    #[serde(default)]
    pub routine: Vec<Routine>,
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
}

pub(crate) struct Scheduler {
    pub entries: Vec<SchedulerEntry>,
    /// Per-workspace-root → last loaded entries (avoid re-parsing every tick)
    loaded_roots: HashMap<PathBuf, std::time::Instant>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { entries: Vec::new(), loaded_roots: HashMap::new() }
    }

    /// Load (or reload) routines from `root/.plexi/routines.toml`.
    /// Called once per context root. Re-reads if not yet loaded or file changed.
    pub fn load_from_root(&mut self, root: &Path) {
        let path = root.join(ROUTINES_FILE);
        if !path.exists() {
            return;
        }
        // Only reload every 60 seconds per root
        if let Some(last) = self.loaded_roots.get(root) {
            if last.elapsed().as_secs() < 60 {
                return;
            }
        }
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("scheduler: failed to read {}: {e}", path.display());
                return;
            }
        };
        let config: RoutinesConfig = match toml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("scheduler: failed to parse {}: {e}", path.display());
                return;
            }
        };
        // Remove old entries from this root (they may have been re-configured)
        // For simplicity: remove all entries whose routine names are in the new config,
        // then re-add them. Since we don't track source root per entry, just reload all.
        // This is fine — routines.toml is small.
        let new_names: std::collections::HashSet<&str> = config.routine.iter().map(|r| r.name.as_str()).collect();
        self.entries.retain(|e| !new_names.contains(e.routine.name.as_str()));
        let count = config.routine.len();
        for routine in config.routine {
            let schedule = match parse_schedule(&routine.schedule) {
                Some(s) => s,
                None => {
                    log::warn!("scheduler: routine '{}' has unparseable schedule '{}', skipping", routine.name, routine.schedule);
                    continue;
                }
            };
            self.entries.push(SchedulerEntry { routine, schedule, last_fire: None });
        }
        self.loaded_roots.insert(root.to_path_buf(), std::time::Instant::now());
        log::info!("scheduler: loaded {count} routines from {}", path.display());
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
        }
    }
}

fn is_due(schedule: &ParsedSchedule, now: chrono::DateTime<chrono::Local>, last_fire: Option<&chrono::DateTime<chrono::Local>>) -> bool {
    match schedule {
        ParsedSchedule::Interval { secs } => {
            match last_fire {
                None => true,
                Some(lf) => (now - *lf).num_seconds() >= *secs as i64,
            }
        }
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
    use chrono::{Datelike, Timelike};
    let minute = now.minute();
    let hour = now.hour();
    let day = now.day();
    let month = now.month();
    // chrono: weekday().num_days_from_monday() gives 0=Mon..6=Sun; cron 1=Mon..7=Sun
    let weekday = now.weekday().num_days_from_monday() + 1;
    field_matches(&expr.minute, minute)
        && field_matches(&expr.hour, hour)
        && field_matches(&expr.day_of_month, day)
        && field_matches(&expr.month, month)
        && field_matches(&expr.day_of_week, weekday)
}

fn field_matches(field: &CronField, value: u32) -> bool {
    match field {
        CronField::Any => true,
        CronField::Single(n) => *n == value,
        CronField::Range(a, b) => value >= *a && value <= *b,
        CronField::List(vals) => vals.contains(&value),
    }
}

/// Parse a schedule string into a `ParsedSchedule`.
/// Returns `None` if the string cannot be parsed.
pub(crate) fn parse_schedule(s: &str) -> Option<ParsedSchedule> {
    let s = s.trim();

    // "every 30m" / "every 2h" / "every 5s"
    if let Some(rest) = s.strip_prefix("every ").or_else(|| s.strip_prefix("Every ")) {
        let rest = rest.trim();
        if let Some(n_str) = rest.strip_suffix('m') {
            let n: u64 = n_str.trim().parse().ok()?;
            return Some(ParsedSchedule::Interval { secs: n * 60 });
        }
        if let Some(n_str) = rest.strip_suffix('h') {
            let n: u64 = n_str.trim().parse().ok()?;
            return Some(ParsedSchedule::Interval { secs: n * 3600 });
        }
        if let Some(n_str) = rest.strip_suffix('s') {
            let n: u64 = n_str.trim().parse().ok()?;
            return Some(ParsedSchedule::Interval { secs: n });
        }
        // "every minute" / "every hour"
        if rest.eq_ignore_ascii_case("minute") { return Some(ParsedSchedule::Interval { secs: 60 }); }
        if rest.eq_ignore_ascii_case("hour") { return Some(ParsedSchedule::Interval { secs: 3600 }); }
        return None;
    }

    // "daily at 9am" / "daily at 10:30am"
    if let Some(rest) = s.strip_prefix("daily at ").or_else(|| s.strip_prefix("Daily at ")) {
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
    if let Some(rest) = s.strip_prefix("weekdays at ").or_else(|| s.strip_prefix("Weekdays at ")) {
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
    if let Some(rest) = s.strip_prefix("weekends at ").or_else(|| s.strip_prefix("Weekends at ")) {
        let (hour, minute) = parse_time(rest)?;
        return Some(ParsedSchedule::Cron(CronExpr {
            minute: CronField::Single(minute),
            hour: CronField::Single(hour),
            day_of_month: CronField::Any,
            month: CronField::Any,
            day_of_week: CronField::List(vec![6, 7]), // Sat-Sun
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
        if pm && h != 12 { h += 12; }
        if !pm && h == 12 { h = 0; }
        return Some((h, m));
    }
    // Just hour
    let mut h: u32 = s.parse().ok()?;
    if pm && h != 12 { h += 12; }
    if !pm && h == 12 { h = 0; }
    Some((h, 0))
}

fn parse_cron_field(s: &str) -> Option<CronField> {
    if s == "*" { return Some(CronField::Any); }
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

/// Parse day-of-week field, supporting mon-fri names
fn parse_cron_field_dow(s: &str) -> Option<CronField> {
    let s_lower = s.to_lowercase();
    // Named day ranges like "mon-fri"
    let day_names = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
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
    // Numeric
    parse_cron_field(s)
}

/// Compute human-readable next-fire description for CLI display.
pub(crate) fn next_fire_description(schedule: &ParsedSchedule, last_fire: Option<&chrono::DateTime<chrono::Local>>) -> String {
    let now = chrono::Local::now();
    match schedule {
        ParsedSchedule::Interval { secs } => {
            match last_fire {
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
            }
        }
        ParsedSchedule::Cron(expr) => {
            // Find the next minute that matches
            use chrono::Duration;
            let mut t = now + Duration::minutes(1);
            for _ in 0..1500 {  // search up to ~24h
                if cron_matches(expr, &t) {
                    use chrono::Timelike;
                    return format!("at {:02}:{:02}", t.hour(), t.minute());
                }
                t = t + Duration::minutes(1);
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
            other => panic!("expected Interval(1800), got {:?}", other.as_ref().map(|_| "Some")),
        }
    }

    #[test]
    fn parse_interval_hours() {
        match parse_schedule("every 2h") {
            Some(ParsedSchedule::Interval { secs: 7200 }) => {}
            other => panic!("expected Interval(7200), got {:?}", other.as_ref().map(|_| "Some")),
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
    }
}
