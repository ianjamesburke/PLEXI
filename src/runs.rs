//! In-memory run registry for PGAP v3 `RunGet` / `RunComplete` wiring.
//!
//! Runs are short-lived host-side records that track an in-flight app operation.
//! They surface in the Run palette (Cmd+R) and emit `RunUpdate` events back to the app.

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
    /// User-facing prompt shown when status == BlockedOnUser.
    pub blocked_prompt: Option<String>,
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
            blocked_prompt: None,
        });
        run_id
    }

    /// Mark a run as complete and remove it from the registry.
    /// Emits RunCompleted to the event log.
    pub fn complete(&mut self, run_id: &str) {
        if let Some(run) = self.runs.remove(run_id) {
            crate::event_log::emit(crate::event_log::HostEvent::RunCompleted {
                run_id: run.run_id.clone(),
                status: RunStatus::Completed.as_str().to_string(),
                timestamp: crate::event_log::now_timestamp(),
            });
        }
    }

    /// Resume a blocked run — sets status back to Pending and emits RunUpdated.
    pub fn resume(&mut self, run_id: &str) {
        if let Some(run) = self.runs.get_mut(run_id) {
            run.status = RunStatus::Pending;
            run.blocked_prompt = None;
            crate::event_log::emit(crate::event_log::HostEvent::RunUpdated {
                run_id: run_id.to_string(),
                status: RunStatus::Pending.as_str().to_string(),
                timestamp: crate::event_log::now_timestamp(),
            });
        }
    }

    /// Block a run on user input — sets status to BlockedOnUser with a prompt.
    pub fn set_blocked(&mut self, run_id: &str, prompt: String) {
        if let Some(run) = self.runs.get_mut(run_id) {
            run.status = RunStatus::BlockedOnUser;
            run.blocked_prompt = Some(prompt);
            crate::event_log::emit(crate::event_log::HostEvent::RunUpdated {
                run_id: run_id.to_string(),
                status: RunStatus::BlockedOnUser.as_str().to_string(),
                timestamp: crate::event_log::now_timestamp(),
            });
        }
    }

    /// Unblock a run with a user response and emit RunUpdated.
    pub fn unblock(&mut self, run_id: &str, _response: String) {
        self.resume(run_id);
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
