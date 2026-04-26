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
#[derive(serde::Serialize)]
pub struct LedgerRow {
    pub ts: String,
    pub backend: String,
    pub billing: &'static str,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cost_usd: Option<f64>,
}

impl LedgerRow {
    pub fn new(
        backend_name: &str,
        billing: BillingModel,
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

        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            backend: backend_name.to_string(),
            billing: match billing {
                BillingModel::Metered => "metered",
                BillingModel::Subscription => "subscription",
            },
            input_tokens,
            output_tokens,
            cost_usd,
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
