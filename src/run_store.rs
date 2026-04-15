use std::collections::HashMap;
use std::path::PathBuf;
use std::fs::OpenOptions;
use std::io::Write;
use crate::app_protocol::{Run, RunStatus, RunOutcome, Caller, RunScope};

pub struct RunStore {
    runs: HashMap<String, Run>,
    log_path: PathBuf,
    counter: u64,
}

impl RunStore {
    pub fn new(log_path: PathBuf) -> Self {
        Self { runs: HashMap::new(), log_path, counter: 0 }
    }

    pub fn create(
        &mut self,
        head_task: String,
        payload: serde_json::Value,
        initiator: Caller,
        parent_run_id: Option<String>,
    ) -> String {
        self.counter += 1;
        let id = format!("run_{:016x}", self.counter);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let run = Run {
            id: id.clone(),
            created_at: now,
            updated_at: now,
            status: RunStatus::Pending,
            head_task,
            initiator,
            scope: RunScope::Global,
            notification_id: None,
            parent_run_id,
            payload,
        };
        self.append_log(&run);
        self.runs.insert(id.clone(), run);
        id
    }

    pub fn update(
        &mut self,
        run_id: &str,
        status: RunStatus,
        head_task: Option<String>,
        payload: Option<serde_json::Value>,
    ) -> bool {
        if let Some(run) = self.runs.get_mut(run_id) {
            run.status = status;
            if let Some(ht) = head_task {
                run.head_task = ht;
            }
            if let Some(p) = payload {
                run.payload = p;
            }
            run.updated_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let run_clone = run.clone();
            self.append_log(&run_clone);
            true
        } else {
            false
        }
    }

    pub fn complete(&mut self, run_id: &str, outcome: RunOutcome) -> bool {
        let status = match &outcome {
            RunOutcome::Success => RunStatus::Complete,
            RunOutcome::Failed { error } => RunStatus::Failed { error: error.clone() },
            RunOutcome::Cancelled => RunStatus::Cancelled,
        };
        self.update(run_id, status, None, None)
    }

    pub fn get(&self, run_id: &str) -> Option<&Run> {
        self.runs.get(run_id)
    }

    pub fn list_active(&self) -> Vec<&Run> {
        self.runs.values().filter(|r| !matches!(
            r.status,
            RunStatus::Complete | RunStatus::Failed { .. } | RunStatus::Cancelled
        )).collect()
    }

    fn append_log(&self, run: &Run) {
        if let Ok(line) = serde_json::to_string(run) {
            match OpenOptions::new().create(true).append(true).open(&self.log_path) {
                Ok(mut f) => { let _ = writeln!(f, "{}", line); }
                Err(e) => log::warn!("run_store: failed to write log: {e}"),
            }
        }
    }
}
