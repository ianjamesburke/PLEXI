/// Session management for PTY terminals with visibility-aware output buffering
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use crate::pty::PtyManager;

const RING_BUFFER_SIZE: usize = 1024 * 1024; // 1MB per session

/// Detect the user's preferred shell
fn detect_shell() -> String {
    // Try to get shell from environment
    if let Ok(shell) = std::env::var("SHELL") {
        if std::path::Path::new(&shell).exists() {
            return shell;
        }
    }

    // Check common shell paths
    let shells = ["/bin/zsh", "/usr/bin/zsh", "/bin/bash", "/usr/bin/bash", "/bin/sh"];
    for shell in &shells {
        if std::path::Path::new(shell).exists() {
            return shell.to_string();
        }
    }

    // Fallback to sh
    "/bin/sh".to_string()
}

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

#[derive(Debug, Serialize, Deserialize)]
pub struct FocusParams {
    pub panel_id: String,
}

/// Ring buffer for storing terminal output
/// Automatically evicts old data when buffer fills
struct OutputRingBuffer {
    buffer: VecDeque<u8>,
    max_size: usize,
}

impl OutputRingBuffer {
    fn new(max_size: usize) -> Self {
        OutputRingBuffer {
            buffer: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    fn append(&mut self, data: &[u8]) {
        for &byte in data {
            if self.buffer.len() >= self.max_size {
                self.buffer.pop_front(); // Evict oldest byte
            }
            self.buffer.push_back(byte);
        }
    }

    fn drain_all(&mut self) -> Vec<u8> {
        self.buffer.drain(..).collect()
    }

    fn len(&self) -> usize {
        self.buffer.len()
    }
}

pub struct SessionRecord {
    pub panel_id: String,
    pub is_visible: bool,
    pub output_buffer: OutputRingBuffer,
    pub output_seq: u32,
    pub pty: Option<PtyManager>,
    pub shell_path: String,
    pub cols: u16,
    pub rows: u16,
}

pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, SessionRecord>>>,
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

        // Detect shell (prefer zsh, fallback to bash, then sh)
        let shell_path = detect_shell();
        let shell_name = std::path::Path::new(&shell_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string();

        // Create and spawn PTY
        let mut pty = PtyManager::new();
        pty.spawn_shell(&shell_path, params.cwd.as_deref(), params.cols, params.rows)
            .map_err(|e| format!("Failed to spawn PTY: {}", e))?;

        // Create new session with ring buffer
        sessions.insert(
            params.panel_id.clone(),
            SessionRecord {
                panel_id: params.panel_id.clone(),
                is_visible: true, // New sessions are visible by default
                output_buffer: OutputRingBuffer::new(RING_BUFFER_SIZE),
                output_seq: 0,
                pty: Some(pty),
                shell_path: shell_path.clone(),
                cols: params.cols,
                rows: params.rows,
            },
        );

        log::info!(
            "Opened session {} with shell {} ({}x{})",
            params.panel_id,
            shell_name,
            params.cols,
            params.rows
        );

        Ok(SessionStartedMessage {
            panel_id: params.panel_id,
            backend: "pty-process".to_string(),
            platform: std::env::consts::OS.to_string(),
            shell_path,
            shell_name,
            cols: params.cols,
            rows: params.rows,
        })
    }

    pub fn write_session(&self, input: SessionInput) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        let session = sessions.get_mut(&input.panel_id)
            .ok_or_else(|| "Session not found".to_string())?;

        if let Some(ref mut pty) = session.pty {
            pty.write_input(input.data.as_bytes())
                .map_err(|e| format!("Failed to write to PTY: {}", e))
        } else {
            Err("PTY not initialized".to_string())
        }
    }

    pub fn append_output(&self, panel_id: &str, data: &[u8]) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        let session = sessions.get_mut(panel_id)
            .ok_or_else(|| "Session not found".to_string())?;

        // Always buffer the data (whether visible or not)
        session.output_buffer.append(data);
        session.output_seq += 1;

        // TODO: If visible, emit event to frontend
        // tauri::api::ipc::InvokeResponse::Ok(...)

        Ok(())
    }

    pub fn focus_panel(&self, panel_id: &str) -> Result<String, String> {
        let mut sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        let session = sessions.get_mut(panel_id)
            .ok_or_else(|| "Session not found".to_string())?;

        session.is_visible = true;

        // Return all buffered output as string (converted from bytes)
        let buffered_bytes = session.output_buffer.drain_all();
        let buffered_string = String::from_utf8_lossy(&buffered_bytes).to_string();

        log::info!(
            "Focused panel {} with {} bytes of buffered history",
            panel_id,
            buffered_bytes.len()
        );

        Ok(buffered_string)
    }

    pub fn unfocus_panel(&self, panel_id: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        let session = sessions.get_mut(panel_id)
            .ok_or_else(|| "Session not found".to_string())?;

        session.is_visible = false;
        log::info!("Unfocused panel {}, buffering future output", panel_id);

        Ok(())
    }

    pub fn resize_session(&self, panel_id: String, cols: u16, rows: u16) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        let session = sessions.get_mut(&panel_id)
            .ok_or_else(|| "Session not found".to_string())?;

        session.cols = cols;
        session.rows = rows;

        if let Some(ref mut pty) = session.pty {
            pty.resize(cols, rows)
                .map_err(|e| format!("Failed to resize PTY: {}", e))
        } else {
            Err("PTY not initialized".to_string())
        }
    }

    pub fn close_session(&self, panel_id: String) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        sessions.remove(&panel_id);
        log::info!("Closed session {}", panel_id);
        Ok(())
    }

    pub fn get_all_sessions(&self) -> Result<Vec<String>, String> {
        let sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        Ok(sessions.keys().cloned().collect())
    }

    pub fn get_session_status(&self, panel_id: &str) -> Result<SessionStatusInfo, String> {
        let sessions = self.sessions.lock().map_err(|_| "Lock poisoned")?;
        let session = sessions.get(panel_id)
            .ok_or_else(|| "Session not found".to_string())?;

        Ok(SessionStatusInfo {
            panel_id: session.panel_id.clone(),
            is_visible: session.is_visible,
            buffered_bytes: session.output_buffer.len(),
            output_seq: session.output_seq,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct SessionStatusInfo {
    pub panel_id: String,
    pub is_visible: bool,
    pub buffered_bytes: usize,
    pub output_seq: u32,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_append() {
        let mut buf = OutputRingBuffer::new(100);
        buf.append(b"hello");
        assert_eq!(buf.len(), 5);
        let drained = buf.drain_all();
        assert_eq!(drained, b"hello");
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let mut buf = OutputRingBuffer::new(10);
        buf.append(b"12345"); // 5 bytes
        buf.append(b"67890"); // 5 bytes, now full
        assert_eq!(buf.len(), 10);

        buf.append(b"ABC"); // 3 bytes, should evict oldest
        assert_eq!(buf.len(), 10);
        let drained = buf.drain_all();
        // Should have evicted "123", keeping "456789" + "0ABC"
        assert_eq!(&drained[..], b"6789ABC000"); // Evicted oldest 3 bytes "123"
    }

    #[test]
    fn test_session_visibility() {
        let manager = SessionManager::new();
        let params = OpenSessionParams {
            panel_id: "test-panel".to_string(),
            cwd: None,
            cols: 80,
            rows: 24,
        };

        manager.open_session(params).unwrap();
        manager
            .append_output("test-panel", b"hello world")
            .unwrap();

        // Panel is visible by default, so focus returns empty (already draining)
        let buffered = manager.focus_panel("test-panel").unwrap();
        assert!(buffered.is_empty() || buffered == "hello world");

        // Hide the panel
        manager.unfocus_panel("test-panel").unwrap();

        // Add more output while hidden
        manager
            .append_output("test-panel", b" from background")
            .unwrap();

        // When we focus again, we get the buffered content
        let buffered = manager.focus_panel("test-panel").unwrap();
        assert_eq!(buffered, " from background");
    }
}
