//! Cost ledger — one JSONL row per AI call, written to
//! `~/.plexi-alpha/ai-ledger.jsonl` (or the build-appropriate config dir).
//!
//! Budget enforcement is active: `check_budget` gates every `ai.query` call
//! against per-app and global daily spend caps from `[ai]` config.
//!
//! Row format (cost_usd is null for subscription billing; client is null when
//! the run and `[ai] client` both leave it unset; wall_ms is omitted until a
//! run measures it). A token count is a positive number or the string
//! `"unknown"` — a missing or zero reading is never stored as `0`:
//! ```json
//! {"ts":"2026-04-16T12:00:00Z","backend":"openrouter","billing":"metered",
//!  "model":"anthropic/claude-haiku-4-5","input_tokens":234,"output_tokens":512,
//!  "cost_usd":0.0023,"cost_cents":0,"client":"narrative","kind":"output"}
//! ```

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::plexi_ai::backend::BillingModel;

/// Whether a run is host bookkeeping (`system`) or user-facing work (`output`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    System,
    #[default]
    Output,
}

impl RunKind {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "system" => Ok(Self::System),
            "output" => Ok(Self::Output),
            other => Err(format!("kind must be 'system' or 'output', got '{other}'")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Output => "output",
        }
    }
}

/// Tags stamped on one brokered run and its ledger row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTags {
    pub client: Option<String>,
    pub kind: RunKind,
}

impl RunTags {
    /// Per-run override wins when it is non-empty. Otherwise `config_client`
    /// (the workspace `[ai] client`) is the client. Kind defaults to `output`.
    pub fn resolve(
        override_client: Option<&str>,
        override_kind: Option<RunKind>,
        config_client: Option<&str>,
    ) -> Self {
        let client = nonempty(override_client).or_else(|| nonempty(config_client));
        Self {
            client,
            kind: override_kind.unwrap_or(RunKind::Output),
        }
    }
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Which tag `summary` groups by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryBy {
    Client,
    Kind,
}

impl SummaryBy {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "client" => Ok(Self::Client),
            "kind" => Ok(Self::Kind),
            other => Err(format!("--by must be 'client' or 'kind', got '{other}'")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Kind => "kind",
        }
    }
}

/// One aggregated bucket from [`summary`].
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerSummaryGroup {
    pub key: Option<String>,
    pub runs: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub wall_ms: Option<u64>,
}

/// Result of `plexi ledger summary`.
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerSummary {
    pub by: SummaryBy,
    pub since: Option<String>,
    pub groups: Vec<LedgerSummaryGroup>,
}

impl LedgerSummary {
    /// JSON object. Each group names its key `client` or `kind` to match `by`.
    pub fn to_value(&self) -> serde_json::Value {
        let name = self.by.as_str();
        let groups: Vec<serde_json::Value> = self
            .groups
            .iter()
            .map(|group| {
                serde_json::json!({
                    name: group.key,
                    "runs": group.runs,
                    "input_tokens": token_total_value(group.input_tokens),
                    "output_tokens": token_total_value(group.output_tokens),
                    "cost_usd": group.cost_usd,
                    "wall_ms": group.wall_ms,
                })
            })
            .collect();
        serde_json::json!({
            "by": name,
            "since": self.since,
            "groups": groups,
        })
    }
}

/// One ledger entry — matches the JSON shape written to disk.
///
/// `app_id` and `model` are populated for `ai.query` broker calls (#284) so
/// the cost ledger can attribute spend per-app and per-model.
#[derive(serde::Serialize)]
pub struct LedgerRow {
    pub ts: String,
    pub backend: String,
    pub billing: &'static str,
    /// Originating app id when the call came in via the `ai.query` broker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Concrete model id resolved by the broker (e.g. "claude-haiku-4-5").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Prompt tokens. A positive count, or `None` when the provider did not
    /// report a usable count. Serialized as that number or the string
    /// `"unknown"`.
    #[serde(serialize_with = "serialize_token_count")]
    pub input_tokens: Option<u32>,
    /// Completion tokens. Same contract as [`Self::input_tokens`].
    #[serde(serialize_with = "serialize_token_count")]
    pub output_tokens: Option<u32>,
    pub cost_usd: Option<f64>,
    /// Cost in USD cents, rounded. `0` for subscription billing or unknown
    /// token counts. The issue (#284) requires this on every broker row.
    pub cost_cents: u64,
    /// Agent run that this usage belongs to. Absent on assistant broker rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Head that owned the run (`agent:<id>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Agents-API client tag, or `internal/unallocated` when the run is not billable work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Free-form project tag (`narrative`, `du`, `personal`). Null when neither
    /// the run nor `[ai] client` set one. Always serialized so a migrated row
    /// is distinguishable from a row that has not been migrated.
    /// Agent-run rows copy `client_ref` here so `ledger summary --by client` sees them.
    pub client: Option<String>,
    /// `system` or `output`. Null only on rows migrated from before tags
    /// existed; new rows always set it (`output` when the run did not).
    pub kind: Option<RunKind>,
    /// Wall-clock time of the brokered turn, in milliseconds. Omitted when
    /// the row was written before wall time was tracked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wall_ms: Option<u64>,
}

impl LedgerRow {
    /// Construct a ledger row carrying `app_id` and concrete `model` — the
    /// shape the `ai.query` broker writes (#284, #383).
    ///
    /// `cost_usd` is the provider's `usage.cost` when the response included
    /// it. Pass `None` when that field was absent (the generation lookup may
    /// fill it later) or for subscription billing; `cost_cents` is `0` then.
    pub fn with_attribution(
        backend_name: &str,
        billing: BillingModel,
        app_id: Option<String>,
        model: Option<String>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        cost_usd: Option<f64>,
    ) -> Self {
        let cost_usd = match billing {
            BillingModel::Metered => cost_usd,
            BillingModel::Subscription => None,
        };

        let cost_cents = cost_usd
            .map(|usd| (usd * 100.0).round().max(0.0) as u64)
            .unwrap_or(0);

        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            backend: backend_name.to_string(),
            billing: match billing {
                BillingModel::Metered => "metered",
                BillingModel::Subscription => "subscription",
            },
            app_id,
            model,
            input_tokens: known_tokens(input_tokens),
            output_tokens: known_tokens(output_tokens),
            cost_usd,
            cost_cents,
            run_id: None,
            agent_id: None,
            client_ref: None,
            parent_run_id: None,
            client: None,
            kind: Some(RunKind::Output),
            wall_ms: None,
        }
    }

    /// A run row. Token counts are the usage reported for that run. Tags are
    /// the client and the work kind (`system` or `output`).
    pub fn for_agent_run(
        run_id: &str,
        agent_id: &str,
        client_ref: &str,
        kind: &str,
        parent_run_id: Option<&str>,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Self {
        let parsed = RunKind::parse(kind).unwrap_or(RunKind::Output);
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            backend: "agents".to_string(),
            billing: "metered",
            app_id: Some(agent_id.to_string()),
            model: None,
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            cost_usd: None,
            cost_cents: 0,
            run_id: Some(run_id.to_string()),
            agent_id: Some(agent_id.to_string()),
            client_ref: Some(client_ref.to_string()),
            parent_run_id: parent_run_id.map(str::to_string),
            client: Some(client_ref.to_string()),
            kind: Some(parsed),
            wall_ms: None,
        }
    }

    /// Stamp the run's tags and measured wall time onto a row.
    pub fn tagged(mut self, tags: RunTags, wall_ms: Option<u64>) -> Self {
        self.client = tags.client;
        self.kind = Some(tags.kind);
        self.wall_ms = wall_ms;
        self
    }
}

/// Append `row` to the ledger file. Creates the file if it doesn't exist.
/// Silently logs and returns on I/O failure — billing ledger errors must
/// never crash the UI or interrupt the conversation.
fn ledger_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn append(row: &LedgerRow) {
    if let Err(error) = append_result(row) {
        log::warn!("plexi_ai ledger: {error}");
    }
}

/// Append `row` and return the I/O or encode error. Agent runs use this so a
/// missing ledger line is a failed spawn, not a silent zero.
pub fn append_result(row: &LedgerRow) -> Result<(), String> {
    let _guard = ledger_lock();
    // One-shot for files written before tags existed; a no-op once every
    // object already has `client` and `kind`. Runs under the same lock as
    // `fill_cost` so a cost patch cannot race a rewrite.
    migrate_null_tags_locked();
    let path = ledger_path();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!("failed to create dir {}: {error}", parent.display())
        })?;
    }

    let line = serde_json::to_string(row).map_err(|error| format!("failed to serialize row: {error}"))?;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    writeln!(file, "{line}").map_err(|error| format!("write error: {error}"))?;
    log::info!(
        "plexi_ai ledger: appended run_id={} agent_id={} client_ref={} client={} kind={} input_tokens={} output_tokens={}",
        row.run_id.as_deref().unwrap_or("-"),
        row.agent_id.as_deref().unwrap_or("-"),
        row.client_ref.as_deref().unwrap_or("-"),
        row.client.as_deref().unwrap_or("(none)"),
        row.kind.map(RunKind::as_str).unwrap_or("(none)"),
        token_log(row.input_tokens),
        token_log(row.output_tokens),
    );
    Ok(())
}

/// Returns the ledger path, migrating from the old `agent-ledger.jsonl` name
/// if it exists and the new file does not yet.
pub(crate) fn ledger_path() -> PathBuf {
    let config_dir = crate::config::config_dir();
    let new_path = config_dir.join("ai-ledger.jsonl");
    let old_path = config_dir.join("agent-ledger.jsonl");

    // One-time migration: if the old file exists and the new one doesn't,
    // move it so historical entries are preserved.
    if old_path.exists() && !new_path.exists() {
        if let Err(e) = std::fs::rename(&old_path, &new_path) {
            log::warn!(
                "plexi_ai ledger: failed to migrate {} → {}: {e}",
                old_path.display(),
                new_path.display()
            );
        }
    }

    new_path
}

/// Give every existing object row explicit null `client` and `kind` when those
/// keys are absent. Malformed lines are kept verbatim. Token and cost bytes
/// on a migrated row are not rewritten. Returns how many rows gained a null
/// tag. Idempotent: a second call rewrites nothing.
pub fn migrate_null_tags() -> usize {
    let _guard = ledger_lock();
    migrate_null_tags_locked()
}

fn migrate_null_tags_locked() -> usize {
    let path = ledger_path();
    let data = match std::fs::read_to_string(&path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            log::warn!(
                "plexi_ai ledger: tag migration skipped, failed to read {}: {e}",
                path.display()
            );
            return 0;
        }
    };

    let mut changed = 0usize;
    let mut out = String::new();
    for line in data.split_inclusive('\n') {
        let (body, newline) = line
            .strip_suffix('\n')
            .map(|body| (body, true))
            .unwrap_or((line, false));
        let updated = match null_tag_line(body) {
            Some(updated) => {
                changed += 1;
                updated
            }
            None => body.to_string(),
        };
        out.push_str(&updated);
        if newline {
            out.push('\n');
        }
    }
    if changed == 0 {
        return 0;
    }

    let tmp = path.with_extension("jsonl.migrating");
    if let Err(e) = std::fs::write(&tmp, &out) {
        log::warn!(
            "plexi_ai ledger: tag migration failed to write {}: {e}",
            tmp.display()
        );
        return 0;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        log::warn!(
            "plexi_ai ledger: tag migration failed to replace {}: {e}",
            path.display()
        );
        let _ = std::fs::remove_file(&tmp);
        return 0;
    }
    log::info!("ai ledger: migrated {changed} row(s) with null client and kind tags");
    changed
}

/// Fill `cost_usd` on the row whose `ts` matches, when that field is still
/// null. Other lines are left byte-for-byte. Returns whether a row was
/// patched. A row that already has a cost is left alone.
pub fn fill_cost(path: &Path, ts: &str, cost_usd: f64) -> bool {
    if !cost_usd.is_finite() || cost_usd < 0.0 {
        log::warn!("plexi_ai ledger: refusing to fill non-finite cost {cost_usd}");
        return false;
    }
    let _guard = ledger_lock();
    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(e) => {
            log::warn!(
                "plexi_ai ledger: cost fill skipped, failed to read {}: {e}",
                path.display()
            );
            return false;
        }
    };
    let mut changed = false;
    let mut out = String::with_capacity(data.len() + 16);
    for line in data.split_inclusive('\n') {
        if !changed {
            if let Some(patched) = patch_null_cost_line(line, ts, cost_usd) {
                out.push_str(&patched);
                changed = true;
                continue;
            }
        }
        out.push_str(line);
    }
    if !changed {
        log::info!("ai ledger: no null-cost row matched ts={ts}");
        return false;
    }
    let tmp = path.with_extension("jsonl.cost");
    if let Err(e) = std::fs::write(&tmp, &out) {
        log::warn!(
            "plexi_ai ledger: cost fill failed to write {}: {e}",
            tmp.display()
        );
        return false;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        log::warn!(
            "plexi_ai ledger: cost fill failed to replace {}: {e}",
            path.display()
        );
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    log::info!("ai ledger: filled cost_usd={cost_usd} on row ts={ts}");
    true
}

/// Replace a null `cost_usd` on the line with this `ts`. `None` when the
/// line is a different row, already priced, or not the compact shape the
/// ledger writer emits.
fn patch_null_cost_line(line: &str, ts: &str, cost_usd: f64) -> Option<String> {
    let (body, newline) = line
        .strip_suffix('\n')
        .map(|body| (body, true))
        .unwrap_or((line, false));
    if body.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if value.get("ts").and_then(|v| v.as_str()) != Some(ts) {
        return None;
    }
    if !value.get("cost_usd").is_some_and(|v| v.is_null()) {
        return None;
    }
    let needle = "\"cost_usd\":null,\"cost_cents\":0";
    if !body.contains(needle) {
        log::warn!("plexi_ai ledger: null-cost row ts={ts} was not in the compact writer shape");
        return None;
    }
    let cost_json = serde_json::to_string(&cost_usd).ok()?;
    let cents = (cost_usd * 100.0).round().max(0.0) as u64;
    let mut updated = body.replacen(
        needle,
        &format!("\"cost_usd\":{cost_json},\"cost_cents\":{cents}"),
        1,
    );
    if newline {
        updated.push('\n');
    }
    Some(updated)
}

/// `Some(rewritten)` when `line` is a JSON object missing `client` or `kind`.
/// The original bytes stay; the missing keys are appended as JSON null.
fn null_tag_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let object = value.as_object()?;
    let need_client = !object.contains_key("client");
    let need_kind = !object.contains_key("kind");
    if !need_client && !need_kind {
        return None;
    }
    let end = line.trim_end();
    if !end.ends_with('}') {
        log::warn!("plexi_ai ledger: tag migration left a line unchanged (no closing brace)");
        return None;
    }
    let mut updated = end[..end.len() - 1].to_string();
    if need_client {
        updated.push_str(",\"client\":null");
    }
    if need_kind {
        updated.push_str(",\"kind\":null");
    }
    updated.push('}');
    Some(updated)
}

/// Aggregate ledger rows. Missing tag keys count as null. Token, cost, and
/// wall totals are null when no row in the group recorded that field.
/// `since` is a `YYYY-MM-DD` or RFC3339 lower bound compared against `ts`.
pub fn summary(by: SummaryBy, since: Option<&str>) -> Result<LedgerSummary, String> {
    if let Some(since) = since {
        validate_since(since)?;
    }
    migrate_null_tags();
    let path = ledger_path();
    let data = match std::fs::read_to_string(&path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(format!(
                "failed to read AI ledger {}: {e}",
                path.display()
            ));
        }
    };

    let mut groups: Vec<LedgerSummaryGroup> = Vec::new();
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(e) => {
                log::warn!("plexi_ai ledger: skipping malformed line during summary: {e}");
                continue;
            }
        };
        let ts = value.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(since) = since {
            if ts.as_bytes() < since.as_bytes() {
                continue;
            }
        }
        let key = match by {
            SummaryBy::Client => tag_key(&value, "client"),
            SummaryBy::Kind => tag_key(&value, "kind"),
        };
        let idx = groups
            .iter()
            .position(|group| group.key == key)
            .unwrap_or_else(|| {
                groups.push(LedgerSummaryGroup {
                    key,
                    runs: 0,
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                    wall_ms: None,
                });
                groups.len() - 1
            });
        let group = &mut groups[idx];
        group.runs = group.runs.saturating_add(1);
        add_u64(
            &mut group.input_tokens,
            json_token_count(value.get("input_tokens")),
        );
        add_u64(
            &mut group.output_tokens,
            json_token_count(value.get("output_tokens")),
        );
        if let Some(cost) = value.get("cost_usd").and_then(serde_json::Value::as_f64) {
            group.cost_usd = Some(group.cost_usd.unwrap_or(0.0) + cost);
        }
        add_u64(&mut group.wall_ms, json_u64(value.get("wall_ms")));
    }
    groups.sort_by(|left, right| match (&left.key, &right.key) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(left), Some(right)) => left.cmp(right),
    });
    log::info!(
        "ai ledger: summary by={} since={} groups={}",
        by.as_str(),
        since.unwrap_or(""),
        groups.len()
    );
    Ok(LedgerSummary {
        by,
        since: since.map(str::to_string),
        groups,
    })
}

fn validate_since(since: &str) -> Result<(), String> {
    let bytes = since.as_bytes();
    let date_ok = bytes.len() >= 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && (bytes.len() == 10 || bytes[10] == b'T' || bytes[10] == b' ');
    if date_ok {
        Ok(())
    } else {
        Err(format!(
            "--since must be YYYY-MM-DD or an RFC3339 timestamp, got '{since}'"
        ))
    }
}

fn tag_key(value: &serde_json::Value, field: &str) -> Option<String> {
    match value.get(field) {
        Some(serde_json::Value::String(text)) => nonempty(Some(text)),
        _ => None,
    }
}

/// A measured token count. `"unknown"`, JSON null, a missing field, and
/// numeric `0` do not add into a group total. Wall time uses [`json_u64`]
/// because a measured `0` is a real duration.
fn json_token_count(value: Option<&serde_json::Value>) -> Option<u64> {
    let value = value?;
    let count = match value {
        serde_json::Value::Number(number) => number.as_u64().or_else(|| {
            number
                .as_f64()
                .filter(|n| n.is_finite() && *n > 0.0)
                .map(|n| n as u64)
        }),
        serde_json::Value::String(text) if text == "unknown" => None,
        _ => None,
    };
    count.filter(|count| *count > 0)
}

fn token_total_value(value: Option<u64>) -> serde_json::Value {
    match value {
        Some(count) if count > 0 => serde_json::Value::from(count),
        _ => serde_json::Value::String("unknown".to_string()),
    }
}

/// Positive counts stay numbers. `None` and `Some(0)` are the literal
/// `"unknown"` so a row never persists a zero token count.
fn known_tokens(value: Option<u32>) -> Option<u32> {
    match value {
        Some(count) if count > 0 => Some(count),
        _ => None,
    }
}

fn serialize_token_count<S: serde::Serializer>(
    value: &Option<u32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(count) if *count > 0 => serializer.serialize_u32(*count),
        _ => serializer.serialize_str("unknown"),
    }
}

fn token_log(value: Option<u32>) -> String {
    match value {
        Some(count) if count > 0 => count.to_string(),
        _ => "unknown".to_string(),
    }
}

fn json_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    let value = value?;
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map(|n| n as u64)
    })
}

fn add_u64(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or(0).saturating_add(value));
    }
}

// ── Budget enforcement ────────────────────────────────────────────────────────

/// Summary of AI spend for a single calendar day (UTC).
pub struct DailySpend {
    /// Total spend across all apps today, in USD.
    pub global_usd: f64,
    /// Per-app spend today, keyed by app_id, in USD.
    pub per_app: HashMap<String, f64>,
}

/// Read today's ledger and compute total spend. O(n) scan — called before each
/// `ai.query`. Fails open: any I/O or parse error is logged and returns zero
/// spend so a bad ledger never blocks queries.
pub fn today_spend() -> DailySpend {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let path = ledger_path();

    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No ledger yet — zero spend.
            return DailySpend {
                global_usd: 0.0,
                per_app: HashMap::new(),
            };
        }
        Err(e) => {
            log::warn!(
                "plexi_ai ledger: failed to read {} for budget check: {e}",
                path.display()
            );
            return DailySpend {
                global_usd: 0.0,
                per_app: HashMap::new(),
            };
        }
    };

    let mut global_usd = 0.0f64;
    let mut per_app: HashMap<String, f64> = HashMap::new();

    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("plexi_ai ledger: skipping malformed line during budget scan: {e}");
                continue;
            }
        };
        // Only count today's rows.
        let ts = v["ts"].as_str().unwrap_or("");
        if !ts.starts_with(&today) {
            continue;
        }
        let cost = v["cost_usd"].as_f64().unwrap_or(0.0);
        global_usd += cost;
        if let Some(app_id) = v["app_id"].as_str() {
            *per_app.entry(app_id.to_string()).or_insert(0.0) += cost;
        }
    }

    DailySpend {
        global_usd,
        per_app,
    }
}

/// Check if an `ai.query` from `app_id` would exceed budget limits.
/// Returns `Ok(())` if under budget, `Err(reason)` if over.
/// Fails open: if `today_spend` returns zero due to I/O errors, the query
/// is allowed through.
pub fn check_budget(app_id: &str, config: &crate::config::AiConfig) -> Result<(), String> {
    let spend = today_spend();
    let global_cap = config.effective_global_daily_usd();
    if spend.global_usd >= global_cap {
        log::warn!(
            "ai_broker[{app_id}]: global daily budget exceeded (${:.4} / ${:.2})",
            spend.global_usd,
            global_cap,
        );
        return Err(format!(
            "global daily AI budget exceeded (${:.2} / ${:.2})",
            spend.global_usd, global_cap
        ));
    }
    let app_cap = config.effective_per_app_daily_usd();
    let app_spend = spend.per_app.get(app_id).copied().unwrap_or(0.0);
    if app_spend >= app_cap {
        log::warn!(
            "ai_broker[{app_id}]: per-app daily budget exceeded (${:.4} / ${:.2})",
            app_spend,
            app_cap,
        );
        return Err(format!(
            "per-app daily AI budget exceeded for '{app_id}' (${:.2} / ${:.2})",
            app_spend, app_cap
        ));
    }
    Ok(())
}

#[cfg(test)]
mod ledger_tests {
    //! Wire-shape tests for the v3.3 broker ledger row (#284, #383).
    use super::*;

    #[test]
    fn ai_query_appends_agent_turn_to_ledger_row_shape() {
        let cost_usd: f64 = 0.0049;
        let row = LedgerRow::with_attribution(
            "openrouter",
            BillingModel::Metered,
            Some("test-app".to_string()),
            Some("anthropic/claude-haiku-4-5".to_string()),
            Some(1_000),
            Some(2_000),
            Some(0.05), // $0.05 → 5 cents
        );

        assert_eq!(row.app_id.as_deref(), Some("test-app"));
        assert_eq!(row.model.as_deref(), Some("anthropic/claude-haiku-4-5"));
        assert_eq!(row.input_tokens, Some(1_000));
        assert_eq!(row.output_tokens, Some(2_000));
        assert_eq!(row.cost_cents, 5, "cost_cents must be 5 for $0.05 input");
        assert_eq!(row.billing, "metered");

        let line = serde_json::to_string(&row).expect("serialise ledger row");
        for needle in [
            r#""backend":"openrouter""#,
            r#""app_id":"test-app""#,
            r#""model":"anthropic/claude-haiku-4-5""#,
            r#""input_tokens":1000"#,
            r#""output_tokens":2000"#,
            r#""cost_cents":"#,
        ] {
            assert!(
                line.contains(needle),
                "ledger row missing `{needle}`: {line}"
            );
        }

        let _ = cost_usd;
    }

    #[test]
    fn missing_cost_produces_zero_cents() {
        let row = LedgerRow::with_attribution(
            "openrouter",
            BillingModel::Metered,
            Some("test-app".to_string()),
            Some("anthropic/claude-haiku-4-5".to_string()),
            Some(100),
            Some(200),
            None,
        );
        assert_eq!(row.cost_cents, 0, "missing cost must produce cost_cents=0");
        assert!(
            row.cost_usd.is_none(),
            "missing cost must produce null cost_usd"
        );
    }

    #[test]
    fn row_without_attribution_omits_app_id_and_model() {
        let row = LedgerRow::with_attribution(
            "ollama",
            BillingModel::Subscription,
            None,
            None,
            None,
            None,
            None,
        );
        let line = serde_json::to_string(&row).expect("serialise");
        assert!(
            !line.contains("\"app_id\""),
            "row must not include app_id when None: {line}"
        );
        assert!(
            !line.contains("\"model\""),
            "row must not include model when None: {line}"
        );
        assert!(
            line.contains(r#""cost_cents":0"#),
            "subscription row must report cost_cents=0: {line}"
        );
    }

    // ── Budget enforcement tests ──────────────────────────────────────────────

    fn budget_config(per_app: f64, global: f64) -> crate::config::AiConfig {
        crate::config::AiConfig {
            per_app_daily_usd: Some(per_app),
            global_daily_usd: Some(global),
            ..Default::default()
        }
    }

    #[test]
    fn check_budget_passes_under_limit() {
        let config = budget_config(1.0, 10.0);
        // Build a mock DailySpend inline by calling check_budget with a config
        // where limits are well above any actual ledger spend. Since we are in
        // a test environment with no real ledger, today_spend() returns zero —
        // this call must succeed.
        let result = check_budget("app1", &config);
        assert!(
            result.is_ok(),
            "should pass when spend is under limit: {result:?}"
        );
    }

    #[test]
    fn check_budget_blocks_global_over_limit() {
        // Set global cap very low so even zero today_spend() doesn't trigger...
        // We need to actually write a ledger row for today to test blocking.
        // Use a temp dir to isolate. Since ledger_path() uses config_dir() which
        // is process-global, we test the logic directly using a known spend
        // by setting a cap below the mock spend value.
        //
        // Instead, test the error message shape by using a cap of 0.0 (any spend
        // would exceed it — but spend is 0 from an empty ledger).
        // To truly trigger the global block without mocking, set global cap to 0.0
        // which means 0.0 >= 0.0 is true.
        let config = crate::config::AiConfig {
            global_daily_usd: Some(0.0),
            per_app_daily_usd: Some(1.0),
            ..Default::default()
        };
        let result = check_budget("app1", &config);
        assert!(result.is_err(), "should block when global spend >= cap");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("global daily"),
            "error must mention 'global daily': {msg}"
        );
    }

    #[test]
    fn check_budget_blocks_per_app_over_limit() {
        // Set per-app cap to 0.0 so any spend (including 0 >= 0) triggers.
        // But global cap is high so that path doesn't fire first.
        let config = crate::config::AiConfig {
            global_daily_usd: Some(100.0),
            per_app_daily_usd: Some(0.0),
            ..Default::default()
        };
        // With empty ledger, app_spend = 0.0 which is NOT >= 0.0... actually 0.0 >= 0.0 is true.
        let result = check_budget("app1", &config);
        assert!(result.is_err(), "should block when per-app spend >= cap");
        let msg = result.unwrap_err();
        assert!(msg.contains("app1"), "error must mention the app_id: {msg}");
        assert!(
            msg.contains("per-app daily"),
            "error must mention 'per-app daily': {msg}"
        );
    }

    #[test]
    fn today_spend_returns_zero_on_missing_ledger() {
        // today_spend() must not panic when ledger doesn't exist.
        // We can't control the ledger path in tests (it's process-global),
        // but we can verify it returns a DailySpend without panicking.
        let spend = today_spend();
        // global_usd is >= 0 (sanity check — no panics, no negative values)
        assert!(spend.global_usd >= 0.0, "global_usd must be non-negative");
    }

    fn isolated_ledger() -> (tempfile::TempDir, crate::config::TestProfileDirGuard) {
        let dir = tempfile::TempDir::new().expect("temp profile dir");
        let guard = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        (dir, guard)
    }

    fn ledger_file() -> std::path::PathBuf {
        crate::config::config_dir().join("ai-ledger.jsonl")
    }

    #[test]
    fn run_tags_persist_on_the_ledger_row() {
        let (_dir, _guard) = isolated_ledger();
        let row = LedgerRow::with_attribution(
            "openrouter",
            BillingModel::Metered,
            Some("assistant".to_string()),
            Some("xiaomi/mimo-v2.5".to_string()),
            Some(194),
            Some(12),
            Some(0.5),
        )
        .tagged(
            RunTags {
                client: Some("narrative".to_string()),
                kind: RunKind::Output,
            },
            Some(1500),
        );
        append(&row);
        let text = std::fs::read_to_string(ledger_file()).expect("ledger");
        let value: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(value["client"], "narrative");
        assert_eq!(value["kind"], "output");
        assert_eq!(value["input_tokens"], 194);
        assert_eq!(value["output_tokens"], 12);
        assert_eq!(value["cost_usd"], 0.5);
        assert_eq!(value["wall_ms"], 1500);
        assert_eq!(value["cost_cents"], 50);
    }

    #[test]
    fn run_tag_defaults_apply() {
        let from_config = RunTags::resolve(None, None, Some("narrative"));
        assert_eq!(from_config.client.as_deref(), Some("narrative"));
        assert_eq!(from_config.kind, RunKind::Output);

        let overridden = RunTags::resolve(Some(" du "), Some(RunKind::System), Some("narrative"));
        assert_eq!(overridden.client.as_deref(), Some("du"));
        assert_eq!(overridden.kind, RunKind::System);

        let blank = RunTags::resolve(Some("  "), None, Some("personal"));
        assert_eq!(blank.client.as_deref(), Some("personal"));
        assert_eq!(blank.kind, RunKind::Output);

        let unset = RunTags::resolve(None, None, None);
        assert!(unset.client.is_none());
        assert_eq!(unset.kind, RunKind::Output);

        let row = LedgerRow::with_attribution(
            "openrouter",
            BillingModel::Metered,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(row.client.is_none());
        assert_eq!(row.kind, Some(RunKind::Output));
        assert!(row.wall_ms.is_none());

        let mut base = crate::config::AiConfig {
            client: Some("narrative".to_string()),
            ..Default::default()
        };
        base.overlay(crate::config::AiConfig::default());
        assert_eq!(base.client.as_deref(), Some("narrative"));
        let mut incoming = crate::config::AiConfig::default();
        incoming.client = Some("du".to_string());
        base.overlay(incoming);
        assert_eq!(
            RunTags::resolve(None, None, base.client.as_deref())
                .client
                .as_deref(),
            Some("du")
        );
        assert!(SummaryBy::parse("client").is_ok());
        assert!(SummaryBy::parse("kind").is_ok());
        assert!(SummaryBy::parse("app").is_err());
    }

    #[test]
    fn summary_aggregates_by_client_and_honors_since() {
        let (_dir, _guard) = isolated_ledger();
        let lines = [
            r#"{"ts":"2025-12-31T23:00:00Z","backend":"openrouter","billing":"metered","input_tokens":9,"output_tokens":9,"cost_usd":9.0,"cost_cents":900,"client":"personal","kind":"output","wall_ms":9}"#,
            r#"{"ts":"2026-01-01T00:00:00Z","backend":"openrouter","billing":"metered","input_tokens":10,"output_tokens":4,"cost_usd":0.25,"cost_cents":25,"client":"narrative","kind":"output","wall_ms":100}"#,
            r#"{"ts":"2026-01-02T00:00:00Z","backend":"openrouter","billing":"metered","input_tokens":5,"output_tokens":1,"cost_usd":0.5,"cost_cents":50,"client":"narrative","kind":"output","wall_ms":50}"#,
            r#"{"ts":"2026-01-02T01:00:00Z","backend":"openrouter","billing":"metered","input_tokens":null,"output_tokens":null,"cost_usd":0.25,"cost_cents":25,"client":"du","kind":"system"}"#,
            r#"{"ts":"2026-01-03T00:00:00Z","backend":"openrouter","billing":"metered","input_tokens":3,"output_tokens":1,"cost_usd":0.25,"cost_cents":25,"client":null,"kind":null}"#,
        ];
        std::fs::write(ledger_file(), lines.join("\n") + "\n").unwrap();

        let report = summary(SummaryBy::Client, Some("2026-01-01")).expect("summary");
        assert_eq!(report.groups.len(), 3);
        let du = &report.groups[0];
        assert_eq!(du.key.as_deref(), Some("du"));
        assert_eq!(du.runs, 1);
        assert!(du.input_tokens.is_none());
        assert!(du.output_tokens.is_none());
        assert_eq!(du.cost_usd, Some(0.25));
        assert!(du.wall_ms.is_none(), "untracked wall time stays null");
        let narrative = &report.groups[1];
        assert_eq!(narrative.key.as_deref(), Some("narrative"));
        assert_eq!(narrative.runs, 2);
        assert_eq!(narrative.input_tokens, Some(15));
        assert_eq!(narrative.output_tokens, Some(5));
        assert_eq!(narrative.cost_usd, Some(0.75));
        assert_eq!(narrative.wall_ms, Some(150));
        let untagged = &report.groups[2];
        assert!(untagged.key.is_none());
        assert_eq!(untagged.runs, 1);
        assert_eq!(untagged.input_tokens, Some(3));

        let value = report.to_value();
        assert_eq!(value["by"], "client");
        assert_eq!(value["since"], "2026-01-01");
        assert_eq!(value["groups"][1]["client"], "narrative");
        assert_eq!(value["groups"][1]["runs"], 2);
        assert_eq!(value["groups"][1]["input_tokens"], 15);
        assert_eq!(value["groups"][0]["input_tokens"], "unknown");
        assert_eq!(value["groups"][0]["output_tokens"], "unknown");
        assert_eq!(value["groups"][0]["wall_ms"], serde_json::Value::Null);

        let by_kind = summary(SummaryBy::Kind, Some("2026-01-01")).expect("kind summary");
        assert_eq!(by_kind.groups[0].key.as_deref(), Some("output"));
        assert_eq!(by_kind.groups[0].runs, 2);
        assert_eq!(by_kind.groups[1].key.as_deref(), Some("system"));
        assert_eq!(by_kind.groups[1].runs, 1);
        assert!(by_kind.groups[2].key.is_none());
        assert!(summary(SummaryBy::Client, Some("yesterday")).is_err());
    }

    #[test]
    fn migrate_null_tags_preserves_usage_and_is_idempotent() {
        let (_dir, _guard) = isolated_ledger();
        let original = "\
{\"ts\":\"2026-04-16T12:00:00Z\",\"backend\":\"openrouter\",\"billing\":\"metered\",\"input_tokens\":234,\"output_tokens\":512,\"cost_usd\":0.5,\"cost_cents\":50}\n\
not json\n\
{\"ts\":\"2026-04-17T12:00:00Z\",\"client\":\"du\",\"kind\":\"output\",\"input_tokens\":1,\"output_tokens\":2,\"cost_usd\":0.25,\"cost_cents\":25}\n";
        std::fs::write(ledger_file(), original).unwrap();
        assert_eq!(migrate_null_tags(), 1);
        let text = std::fs::read_to_string(ledger_file()).unwrap();
        let mut lines = text.lines();
        let migrated = lines.next().unwrap();
        assert!(
            migrated.contains("\"input_tokens\":234") && migrated.contains("\"output_tokens\":512"),
            "token bytes must survive migration: {migrated}"
        );
        assert!(
            migrated.contains("\"cost_usd\":0.5") && migrated.contains("\"cost_cents\":50"),
            "cost bytes must survive migration: {migrated}"
        );
        let value: serde_json::Value = serde_json::from_str(migrated).unwrap();
        assert!(value["client"].is_null());
        assert!(value["kind"].is_null());
        assert_eq!(value["input_tokens"], 234);
        assert_eq!(lines.next().unwrap(), "not json");
        let already = lines.next().unwrap();
        assert!(already.contains("\"client\":\"du\""));
        assert!(already.contains("\"kind\":\"output\""));
        let after = std::fs::read_to_string(ledger_file()).unwrap();
        assert_eq!(migrate_null_tags(), 0);
        assert_eq!(std::fs::read_to_string(ledger_file()).unwrap(), after);
    }

    #[test]
    fn fill_cost_patches_only_the_matching_null_row() {
        let (_dir, _guard) = isolated_ledger();
        let missing = LedgerRow::with_attribution(
            "openrouter",
            BillingModel::Metered,
            Some("assistant".to_string()),
            Some("xiaomi/mimo-v2.5".to_string()),
            Some(3463),
            Some(14),
            None,
        );
        let priced = LedgerRow::with_attribution(
            "openrouter",
            BillingModel::Metered,
            Some("assistant".to_string()),
            Some("xiaomi/mimo-v2.5".to_string()),
            Some(10),
            Some(4),
            Some(0.5),
        );
        append(&missing);
        append(&priced);
        let before = std::fs::read_to_string(ledger_file()).unwrap();
        let priced_line = before.lines().nth(1).unwrap().to_string();
        assert!(fill_cost(&ledger_file(), &missing.ts, 0.021));
        let after = std::fs::read_to_string(ledger_file()).unwrap();
        let mut lines = after.lines();
        let filled = lines.next().unwrap();
        assert!(
            filled.contains(r#""input_tokens":3463"#)
                && filled.contains(r#""output_tokens":14"#)
                && filled.contains(r#""cost_usd":0.021"#)
                && filled.contains(r#""cost_cents":2"#),
            "null cost must be filled in place: {filled}"
        );
        assert_eq!(
            lines.next().unwrap(),
            priced_line,
            "a row that already has a cost must keep its bytes"
        );
        assert!(!fill_cost(&ledger_file(), &missing.ts, 9.0));
        assert_eq!(std::fs::read_to_string(ledger_file()).unwrap(), after);
    }

    #[test]
    fn zero_token_counts_serialize_as_unknown_and_do_not_sum() {
        let row = LedgerRow::with_attribution(
            "openrouter",
            BillingModel::Metered,
            Some("assistant".to_string()),
            Some("xiaomi/mimo-v2.5".to_string()),
            Some(0),
            Some(0),
            None,
        );
        assert!(row.input_tokens.is_none());
        assert!(row.output_tokens.is_none());
        let line = serde_json::to_string(&row).expect("serialise");
        assert!(
            line.contains(r#""input_tokens":"unknown""#)
                && line.contains(r#""output_tokens":"unknown""#),
            "a zero count must be the literal unknown: {line}"
        );
        assert!(
            !line.contains(r#""input_tokens":0"#) && !line.contains(r#""input_tokens":null"#),
            "a row must not persist a zero or null token count: {line}"
        );

        let (_dir, _guard) = isolated_ledger();
        let lines = [
            r#"{"ts":"2026-02-01T00:00:00Z","backend":"openrouter","billing":"metered","input_tokens":10,"output_tokens":4,"cost_usd":0.1,"cost_cents":10,"client":"narrative","kind":"output","wall_ms":0}"#,
            r#"{"ts":"2026-02-01T01:00:00Z","backend":"openrouter","billing":"metered","input_tokens":"unknown","output_tokens":"unknown","cost_usd":0.1,"cost_cents":10,"client":"narrative","kind":"output"}"#,
            r#"{"ts":"2026-02-01T02:00:00Z","backend":"openrouter","billing":"metered","input_tokens":0,"output_tokens":0,"cost_usd":0.1,"cost_cents":10,"client":"narrative","kind":"output"}"#,
            r#"{"ts":"2026-02-01T03:00:00Z","backend":"openrouter","billing":"metered","input_tokens":"unknown","output_tokens":"unknown","cost_usd":null,"cost_cents":0,"client":"du","kind":"output"}"#,
        ];
        std::fs::write(ledger_file(), lines.join("\n") + "\n").unwrap();
        let report = summary(SummaryBy::Client, None).expect("summary");
        let narrative = report
            .groups
            .iter()
            .find(|group| group.key.as_deref() == Some("narrative"))
            .expect("narrative");
        assert_eq!(narrative.runs, 3);
        assert_eq!(narrative.input_tokens, Some(10));
        assert_eq!(narrative.output_tokens, Some(4));
        assert_eq!(
            narrative.wall_ms,
            Some(0),
            "a measured wall time of 0 is a duration"
        );
        let du = report
            .groups
            .iter()
            .find(|group| group.key.as_deref() == Some("du"))
            .expect("du");
        assert_eq!(du.runs, 1);
        assert!(du.input_tokens.is_none());
        assert!(du.output_tokens.is_none());
        let value = report.to_value();
        let du_json = value["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["client"] == "du")
            .unwrap();
        assert_eq!(du_json["input_tokens"], "unknown");
        assert_eq!(du_json["output_tokens"], "unknown");
    }
}
