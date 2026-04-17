/// In-memory run registry for PGAP v3 `RunGet` / `RunComplete` wiring.
///
/// Runs are short-lived host-side records that track an in-flight app operation.
/// They surface in the Run palette (Cmd+R) and emit `RunUpdate` events back to the app.
/// Persisted to the event log in Layer 4; for now, in-memory only.

use std::collections::HashMap;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub enum RunStatus {
    Pending,
    Running,
    BlockedOnUser,
    Completed,
    Failed,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::BlockedOnUser => "blocked_on_user",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Run {
    pub run_id: String,
    pub app_id: String,
    pub intent: String,
    pub payload: serde_json::Value,
    pub status: RunStatus,
    pub created_at: SystemTime,
}

pub struct RunRegistry {
    runs: HashMap<String, Run>,
    next_id: u64,
}

impl RunRegistry {
    pub fn new() -> Self {
        Self {
            runs: HashMap::new(),
            next_id: 1,
        }
    }

    /// Allocate a new run for `app_id` with `intent` + `payload`.
    /// Returns the allocated run_id.
    pub fn allocate(&mut self, app_id: &str, intent: &str, payload: serde_json::Value) -> String {
        let run_id = format!("run-{}-{}", app_id, self.next_id);
        self.next_id += 1;
        self.runs.insert(run_id.clone(), Run {
            run_id: run_id.clone(),
            app_id: app_id.to_string(),
            intent: intent.to_string(),
            payload,
            status: RunStatus::Pending,
            created_at: SystemTime::now(),
        });
        run_id
    }

    /// Mark a run as complete and remove it from the registry.
    pub fn complete(&mut self, run_id: &str) {
        self.runs.remove(run_id);
    }

    /// List all active runs (for the Run palette).
    pub fn list_runs(&self) -> Vec<&Run> {
        self.runs.values().collect()
    }
}

impl Default for RunRegistry {
    fn default() -> Self {
        Self::new()
    }
}
