/// Session management for PTY terminals
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartedMessage {
    pub panel_id: String,
    pub backend: String,
    pub platform: String,
    pub shell_path: String,
    pub shell_name: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOutputMessage {
    pub panel_id: String,
    pub data: String,
    pub seq: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionExitMessage {
    pub panel_id: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpenSessionParams {
    pub panel_id: String,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionInput {
    pub panel_id: String,
    pub data: String,
}

pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, SessionRecord>>>,
}

struct SessionRecord {
    panel_id: String,
    // In real implementation, would hold PTY handle
    // For now, placeholder
    active: bool,
}

impl SessionManager {
    pub fn new() -> Self {
        SessionManager {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn open_session(&self, params: OpenSessionParams) -> Result<SessionStartedMessage, String> {
        let mut sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;

        if sessions.contains_key(&params.panel_id) {
            return Err("Session already exists".to_string());
        }

        // Placeholder: in real implementation, spawn PTY here
        sessions.insert(
            params.panel_id.clone(),
            SessionRecord {
                panel_id: params.panel_id.clone(),
                active: true,
            },
        );

        Ok(SessionStartedMessage {
            panel_id: params.panel_id,
            backend: "pty-process".to_string(),
            platform: std::env::consts::OS.to_string(),
            shell_path: "/bin/zsh".to_string(), // TODO: detect shell
            shell_name: "zsh".to_string(),
            cols: params.cols,
            rows: params.rows,
        })
    }

    pub fn write_session(&self, input: SessionInput) -> Result<(), String> {
        let sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        if !sessions.contains_key(&input.panel_id) {
            return Err("Session not found".to_string());
        }
        // Placeholder: in real implementation, write to PTY
        Ok(())
    }

    pub fn resize_session(&self, panel_id: String, cols: u16, rows: u16) -> Result<(), String> {
        let sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        if !sessions.contains_key(&panel_id) {
            return Err("Session not found".to_string());
        }
        // Placeholder: resize PTY
        Ok(())
    }

    pub fn close_session(&self, panel_id: String) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        sessions.remove(&panel_id);
        Ok(())
    }

    pub fn get_all_sessions(&self) -> Result<Vec<String>, String> {
        let sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        Ok(sessions.keys().cloned().collect())
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
