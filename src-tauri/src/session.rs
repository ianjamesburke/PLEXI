use crate::pty::{resize as resize_pty, write_input, PtySession};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const OUTPUT_EVENT: &str = "plexi://session-output";
const EXIT_EVENT: &str = "plexi://session-exit";
const POLL_READ_CHUNK_SIZE: usize = 16 * 1024;

fn detect_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if std::path::Path::new(&shell).exists() {
            return shell;
        }
    }

    for shell in [
        "/bin/zsh",
        "/usr/bin/zsh",
        "/bin/bash",
        "/usr/bin/bash",
        "/bin/sh",
    ] {
        if std::path::Path::new(shell).exists() {
            return shell.to_string();
        }
    }

    "/bin/sh".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartedMessage {
    pub panel_id: String,
    pub cwd: String,
    pub home_dir: String,
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

#[derive(Debug, Serialize)]
pub struct SessionStatusInfo {
    pub panel_id: String,
    pub cols: u16,
    pub rows: u16,
    pub output_seq: u32,
}

struct SessionRecord {
    panel_id: String,
    writer: Arc<Mutex<pty_process::OwnedWritePty>>,
    child: Arc<Mutex<Child>>,
    reader_task: JoinHandle<()>,
    cols: u16,
    rows: u16,
    output_seq: Arc<Mutex<u32>>,
}

pub struct SessionManager {
    sessions: Mutex<HashMap<String, SessionRecord>>,
    zdotdir: Option<String>,
}

impl SessionManager {
    pub fn new(zdotdir: Option<String>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            zdotdir,
        }
    }

    pub async fn open_session<R: Runtime>(
        &self,
        app: AppHandle<R>,
        params: OpenSessionParams,
    ) -> Result<SessionStartedMessage, String> {
        let mut sessions = self.sessions.lock().await;

        // If a stale session exists (e.g. webview reloaded), clean it up first.
        if let Some(old) = sessions.remove(&params.panel_id) {
            old.reader_task.abort();
            let mut child = old.child.lock().await;
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        let shell_path = detect_shell();
        let shell_name = std::path::Path::new(&shell_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string();

        let session =
            PtySession::spawn_shell(&shell_path, params.cwd.as_deref(), self.zdotdir.as_deref(), params.cols, params.rows)
                .map_err(|e| format!("Failed to spawn PTY: {e}"))?;

        let output_seq = Arc::new(Mutex::new(0u32));
        let writer = Arc::new(Mutex::new(session.writer));
        let child = Arc::new(Mutex::new(session.child));
        let reader_task = spawn_reader_loop(
            app,
            params.panel_id.clone(),
            session.reader,
            Arc::clone(&child),
            Arc::clone(&output_seq),
        );

        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let started = SessionStartedMessage {
            panel_id: params.panel_id.clone(),
            cwd: session.cwd,
            home_dir,
            backend: "pty-process".to_string(),
            platform: std::env::consts::OS.to_string(),
            shell_path,
            shell_name,
            cols: params.cols,
            rows: params.rows,
        };

        sessions.insert(
            params.panel_id.clone(),
            SessionRecord {
                panel_id: params.panel_id,
                writer,
                child,
                reader_task,
                cols: params.cols,
                rows: params.rows,
                output_seq,
            },
        );

        Ok(started)
    }

    pub async fn write_session(&self, input: SessionInput) -> Result<(), String> {
        let writer = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(&input.panel_id)
                .map(|session| Arc::clone(&session.writer))
                .ok_or_else(|| "Session not found".to_string())?
        };

        let mut writer = writer.lock().await;
        write_input(&mut writer, input.data.as_bytes())
            .await
            .map_err(|e| format!("Failed to write to PTY: {e}"))
    }

    pub async fn resize_session(
        &self,
        panel_id: String,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        let writer = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(&panel_id)
                .ok_or_else(|| "Session not found".to_string())?;
            session.cols = cols;
            session.rows = rows;
            Arc::clone(&session.writer)
        };

        let writer = writer.lock().await;
        resize_pty(&writer, cols, rows).map_err(|e| format!("Failed to resize PTY: {e}"))
    }

    pub async fn close_session(&self, panel_id: String) -> Result<(), String> {
        let record = {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(&panel_id)
        };

        if let Some(record) = record {
            record.reader_task.abort();
            terminate_child(&record.child).await;
        }

        Ok(())
    }

    pub async fn close_all_sessions(&self) {
        let sessions = {
            let mut sessions = self.sessions.lock().await;
            sessions
                .drain()
                .map(|(_, session)| session)
                .collect::<Vec<_>>()
        };

        for session in sessions {
            session.reader_task.abort();
            terminate_child(&session.child).await;
        }
    }

    pub async fn get_all_sessions(&self) -> Vec<String> {
        let sessions = self.sessions.lock().await;
        sessions.keys().cloned().collect()
    }

    pub async fn get_session_status(&self, panel_id: &str) -> Result<SessionStatusInfo, String> {
        let (panel_id, cols, rows, output_seq) = {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(panel_id)
                .ok_or_else(|| "Session not found".to_string())?;
            (
                session.panel_id.clone(),
                session.cols,
                session.rows,
                Arc::clone(&session.output_seq),
            )
        };

        let output_seq = *output_seq.lock().await;

        Ok(SessionStatusInfo {
            panel_id,
            cols,
            rows,
            output_seq,
        })
    }
}

fn spawn_reader_loop<R: Runtime>(
    app: AppHandle<R>,
    panel_id: String,
    mut reader: pty_process::OwnedReadPty,
    child: Arc<Mutex<Child>>,
    output_seq: Arc<Mutex<u32>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = vec![0u8; POLL_READ_CHUNK_SIZE];

        loop {
            match reader.read(&mut buf).await {
                Ok(0) => {
                    if let Some(exit_code) = try_wait_exit_code(&child).await {
                        emit_exit(&app, &panel_id, Some(exit_code));
                        break;
                    }
                }
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let seq = {
                        let mut seq = output_seq.lock().await;
                        *seq += 1;
                        *seq
                    };

                    let payload = SessionOutputMessage {
                        panel_id: panel_id.clone(),
                        data,
                        seq,
                    };

                    if app.emit(OUTPUT_EVENT, payload).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    log::warn!("PTY read loop failed for {}: {}", panel_id, error);
                    let exit_code = try_wait_exit_code(&child).await;
                    emit_exit(&app, &panel_id, exit_code);
                    break;
                }
            }
        }
    })
}

async fn try_wait_exit_code(child: &Arc<Mutex<Child>>) -> Option<i32> {
    let mut child = child.lock().await;
    match child.try_wait() {
        Ok(Some(status)) => status.code(),
        Ok(None) => None,
        Err(error) => {
            log::warn!("Failed to check child exit status: {}", error);
            Some(1)
        }
    }
}

async fn terminate_child(child: &Arc<Mutex<Child>>) {
    let mut child = child.lock().await;
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn emit_exit<R: Runtime>(app: &AppHandle<R>, panel_id: &str, exit_code: Option<i32>) {
    let _ = app.emit(
        EXIT_EVENT,
        SessionExitMessage {
            panel_id: panel_id.to_string(),
            exit_code,
        },
    );
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(None)
    }
}
