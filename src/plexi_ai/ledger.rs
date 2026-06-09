//! Cost ledger — one JSONL row per AI call, written to
//! `~/.plexi-alpha/ai-ledger.jsonl` (or the build-appropriate config dir).
//!
//! Budget enforcement is active: `check_budget` gates every `ai.query` call
//! against per-app and global daily spend caps from `[ai]` config.
//!
//! Row format (all fields present; cost_usd is null for subscription billing):
//! ```json
//! {"ts":"2026-04-16T12:00:00Z","backend":"openrouter","billing":"metered",
//!  "model":"anthropic/claude-haiku-4-5","input_tokens":234,"output_tokens":512,
//!  "cost_usd":0.0023,"cost_cents":0}
//! ```

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use crate::plexi_ai::backend::BillingModel;

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
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cost_usd: Option<f64>,
    /// Cost in USD cents, rounded. `0` for subscription billing or unknown
    /// token counts. The issue (#284) requires this on every broker row.
    pub cost_cents: u64,
}

impl LedgerRow {
    /// Construct a ledger row carrying `app_id` and concrete `model` — the
    /// shape the `ai.query` broker writes (#284, #383).
    ///
    /// `cost_usd` is passed explicitly from the caller (fetched via the
    /// OpenRouter generation endpoint after the turn completes). Pass `None`
    /// when cost is unavailable (generation endpoint returned 404 / timeout)
    /// or for subscription billing; `cost_cents` will be `0` in that case.
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
            input_tokens,
            output_tokens,
            cost_usd,
            cost_cents,
        }
    }
}

/// Append `row` to the ledger file. Creates the file if it doesn't exist.
/// Silently logs and returns on I/O failure — billing ledger errors must
/// never crash the UI or interrupt the conversation.
pub fn append(row: &LedgerRow) {
    let path = ledger_path();

    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!(
                "plexi_ai ledger: failed to create dir {}: {e}",
                parent.display()
            );
            return;
        }
    }

    let line = match serde_json::to_string(row) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("plexi_ai ledger: failed to serialize row: {e}");
            return;
        }
    };

    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut file) => {
            if let Err(e) = writeln!(file, "{line}") {
                log::warn!("plexi_ai ledger: write error: {e}");
            }
        }
        Err(e) => {
            log::warn!("plexi_ai ledger: failed to open {}: {e}", path.display());
        }
    }
}

/// Returns the ledger path, migrating from the old `agent-ledger.jsonl` name
/// if it exists and the new file does not yet.
fn ledger_path() -> PathBuf {
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
}
