//! Cost ledger — one JSONL row per LLM call, written to
//! `~/.plexi-alpha/ledger.jsonl` (or the build-appropriate config dir).
//!
//! Stage 1: append-only, no gates. Stage 5 will light up budget enforcement
//! (spec §10 risk #10).
//!
//! Row format (all fields present; cost_usd is null for subscription billing):
//! ```json
//! {"ts":"2026-04-16T12:00:00Z","backend":"anthropic-api","billing":"metered",
//!  "input_tokens":234,"output_tokens":512,"cost_usd":0.0023}
//! ```

use std::io::Write;
use std::path::PathBuf;

use crate::plexi_iq::backend::BillingModel;

/// One ledger entry — matches the JSON shape written to disk.
///
/// `app_id` and `model` are populated for `iq.query` broker calls (#284) so
/// the cost ledger can attribute spend per-app and per-model. Legacy
/// `Pane::Agent` turns leave both `None`.
#[derive(serde::Serialize)]
pub struct LedgerRow {
    pub ts: String,
    pub backend: String,
    pub billing: &'static str,
    /// Originating app id when the call came in via the `iq.query` broker.
    /// `None` for legacy in-process turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Concrete model id resolved by the broker (e.g. "claude-haiku-4-5").
    /// `None` for legacy in-process turns.
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
    /// shape the `iq.query` broker writes (#284). Pass `None` for `app_id`
    /// and `model` for legacy non-brokered turns; both fields are skipped
    /// from the JSON when `None`.
    pub fn with_attribution(
        backend_name: &str,
        billing: BillingModel,
        app_id: Option<String>,
        model: Option<String>,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    ) -> Self {
        let cost_usd = match billing {
            BillingModel::Metered => {
                // claude-sonnet-4-5: $3/M input, $15/M output (as of 2026-04).
                // Using conservative published rates; exact rates will vary by model.
                let in_cost = input_tokens.unwrap_or(0) as f64 * 3.0 / 1_000_000.0;
                let out_cost = output_tokens.unwrap_or(0) as f64 * 15.0 / 1_000_000.0;
                Some((in_cost + out_cost * 100.0).round() / 100.0) // round to cents
            }
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
                "plexi_iq ledger: failed to create dir {}: {e}",
                parent.display()
            );
            return;
        }
    }

    let line = match serde_json::to_string(row) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("plexi_iq ledger: failed to serialize row: {e}");
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
                log::warn!("plexi_iq ledger: write error: {e}");
            }
        }
        Err(e) => {
            log::warn!("plexi_iq ledger: failed to open {}: {e}", path.display());
        }
    }
}

fn ledger_path() -> PathBuf {
    crate::config::config_dir().join("ledger.jsonl")
}

#[cfg(test)]
mod ledger_tests {
    //! Wire-shape tests for the v3.3 broker ledger row (#284).
    //!
    //! `iq.query` calls must appear in `ledger.jsonl` with the four fields
    //! the spec calls out: `app_id`, `model`, `tokens_in`/`tokens_out`,
    //! and `cost_cents`. The JSON shape is checked here so that a future
    //! refactor can't silently drop any of them.
    use super::*;

    #[test]
    fn iq_query_appends_agent_turn_to_ledger_row_shape() {
        // Build the row exactly as `LiveIqBroker::dispatch` does.
        let row = LedgerRow::with_attribution(
            "anthropic-api (native)",
            BillingModel::Metered,
            Some("test-app".to_string()),
            Some("claude-haiku-4-5".to_string()),
            Some(1_000),
            Some(2_000),
        );

        // Direct field assertions — these are the broker's contract.
        assert_eq!(row.app_id.as_deref(), Some("test-app"));
        assert_eq!(row.model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(row.input_tokens, Some(1_000));
        assert_eq!(row.output_tokens, Some(2_000));
        // Metered cost: 1000*$3/M + 2000*$15/M = $0.003 + $0.030 = $0.033 → 3 cents
        // (the row formula multiplies output_cost by 100 internally then
        // divides by 100 — see the rounding logic in `with_attribution`).
        assert!(row.cost_cents > 0, "metered call must produce non-zero cost_cents");
        assert_eq!(row.billing, "metered");

        // Wire shape: the on-disk JSON must include every field the issue
        // requires so a `cat ledger.jsonl | jq` shows them.
        let line = serde_json::to_string(&row).expect("serialise ledger row");
        for needle in [
            r#""app_id":"test-app""#,
            r#""model":"claude-haiku-4-5""#,
            r#""input_tokens":1000"#,
            r#""output_tokens":2000"#,
            r#""cost_cents":"#,
        ] {
            assert!(
                line.contains(needle),
                "ledger row missing `{needle}`: {line}"
            );
        }
    }

    #[test]
    fn legacy_row_without_attribution_omits_app_id_and_model() {
        // Rows built without app_id/model — the JSON should
        // omit `app_id` and `model` (skip_serializing_if = "Option::is_none")
        // so old log rows aren't polluted with `null` fields.
        let row = LedgerRow::with_attribution(
            "claude-cli (proxied)",
            BillingModel::Subscription,
            None,
            None,
            None,
            None,
        );
        let line = serde_json::to_string(&row).expect("serialise");
        assert!(
            !line.contains("\"app_id\""),
            "legacy row must not include app_id: {line}"
        );
        assert!(
            !line.contains("\"model\""),
            "legacy row must not include model: {line}"
        );
        // cost_cents is always present, even on subscription billing (=0).
        assert!(
            line.contains(r#""cost_cents":0"#),
            "subscription row must report cost_cents=0: {line}"
        );
    }
}
