/// Per-app LLM cost tracking.
///
/// Records cost_report events from external apps, accumulates session totals
/// in memory, and appends each report to `~/.plexi-alpha/costs.jsonl` on disk.

use crate::config;
use std::io::Write;

pub struct CostTracker {
    app_id: String,
    session_total_usd: f64,
}

impl CostTracker {
    pub fn new(app_id: &str) -> Self {
        Self {
            app_id: app_id.to_string(),
            session_total_usd: 0.0,
        }
    }

    /// Record a cost report: accumulate in memory and append to disk.
    pub fn record(
        &mut self,
        service: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        operation_id: Option<&str>,
        timestamp: Option<&str>,
    ) {
        self.session_total_usd += cost_usd;

        let ts = timestamp
            .map(|s| s.to_string())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

        let entry = serde_json::json!({
            "app_id": self.app_id,
            "service": service,
            "model": model,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cost_usd": cost_usd,
            "operation_id": operation_id,
            "timestamp": ts,
        });

        if let Err(e) = self.append_to_disk(&entry) {
            log::warn!("CostTracker: failed to write costs.jsonl: {e}");
        }

        log::info!(
            "app::{} cost: ${:.4} ({} {} in:{} out:{})",
            self.app_id, cost_usd, service, model, input_tokens, output_tokens
        );
    }

    /// Total cost accumulated in this session.
    pub fn session_total_usd(&self) -> f64 {
        self.session_total_usd
    }

    fn append_to_disk(&self, entry: &serde_json::Value) -> Result<(), std::io::Error> {
        let path = config::config_dir().join("costs.jsonl");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let mut line = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        line.push('\n');
        file.write_all(line.as_bytes())
    }
}
