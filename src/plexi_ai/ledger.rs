//! Cost ledger — one JSONL row per AI call, written to
//! `~/.plexi-alpha/ai-ledger.jsonl` (or the build-appropriate config dir).
//!
//! Stage 1: append-only, no gates. Stage 5 will light up budget enforcement
//! (spec §10 risk #10).
//!
//! Row format (all fields present; cost_usd is null for subscription billing):
//! ```json
//! {"ts":"2026-04-16T12:00:00Z","backend":"openrouter","billing":"metered",
//!  "model":"anthropic/claude-haiku-4-5","input_tokens":234,"output_tokens":512,
//!  "cost_usd":0.0023,"cost_cents":0}
//! ```

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
        assert!(row.cost_usd.is_none(), "missing cost must produce null cost_usd");
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
}
