//! In-memory run registry for PGAP v3 `RunGet` / `RunComplete` wiring.
//!
//! Runs are short-lived host-side records that track an in-flight app operation.
//! They surface in the Run palette (Cmd+R) and emit `RunUpdate` events back to the app.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum RunStatus {
    Pending,
    Completed,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Pending => "pending",
            RunStatus::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Run {
    pub run_id: String,
    pub app_id: String,
    pub status: RunStatus,
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

    /// Allocate a new run for `app_id`. Returns the allocated run_id.
    pub fn allocate(&mut self, app_id: &str) -> String {
        let run_id = format!("run-{}-{}", app_id, self.next_id);
        self.next_id += 1;
        self.runs.insert(
            run_id.clone(),
            Run {
                run_id: run_id.clone(),
                app_id: app_id.to_string(),
                status: RunStatus::Pending,
            },
        );
        run_id
    }

    /// Return the `app_id` of the app that originated `run_id`, or `None`
    /// if the run does not exist (already completed or never registered).
    /// Must be called BEFORE `complete()` since complete removes the entry.
    pub fn originator_of(&self, run_id: &str) -> Option<&str> {
        self.runs.get(run_id).map(|r| r.app_id.as_str())
    }

    /// Mark a run as complete and remove it from the registry.
    /// Emits RunCompleted to the event log.
    pub fn complete(&mut self, run_id: &str) {
        if let Some(run) = self.runs.remove(run_id) {
            crate::host::event_log::emit(crate::host::event_log::HostEvent::RunCompleted {
                run_id: run.run_id.clone(),
                status: RunStatus::Completed.as_str().to_string(),
                timestamp: crate::host::event_log::now_timestamp(),
            });
        }
    }

    /// Resume a blocked run — sets status back to Pending and emits RunUpdated.
    pub fn resume(&mut self, run_id: &str) {
        if let Some(run) = self.runs.get_mut(run_id) {
            run.status = RunStatus::Pending;
            crate::host::event_log::emit(crate::host::event_log::HostEvent::RunUpdated {
                run_id: run_id.to_string(),
                status: RunStatus::Pending.as_str().to_string(),
                timestamp: crate::host::event_log::now_timestamp(),
            });
        }
    }
}

impl Default for RunRegistry {
    fn default() -> Self {
        Self::new()
    }
}
