mod session;

use session::{SessionManager, OpenSessionParams, SessionInput, SessionStartedMessage, FocusParams, SessionStatusInfo};
use std::sync::Mutex;

struct AppState {
    session_manager: Mutex<SessionManager>,
}

#[tauri::command]
fn open_session(
    state: tauri::State<'_, AppState>,
    panel_id: String,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<SessionStartedMessage, String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.open_session(OpenSessionParams {
        panel_id,
        cwd,
        cols,
        rows,
    })
}

#[tauri::command]
fn write_session(
    state: tauri::State<'_, AppState>,
    panel_id: String,
    data: String,
) -> Result<(), String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.write_session(SessionInput {
        panel_id,
        data,
    })
}

#[tauri::command]
fn resize_session(
    state: tauri::State<'_, AppState>,
    panel_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.resize_session(panel_id, cols, rows)
}

#[tauri::command]
fn close_session(state: tauri::State<'_, AppState>, panel_id: String) -> Result<(), String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.close_session(panel_id)
}

#[tauri::command]
fn get_sessions(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.get_all_sessions()
}

/// Focus a panel and return buffered output history
#[tauri::command]
fn focus_panel(
    state: tauri::State<'_, AppState>,
    panel_id: String,
) -> Result<String, String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.focus_panel(&panel_id)
}

/// Unfocus a panel (stop streaming, start buffering)
#[tauri::command]
fn unfocus_panel(
    state: tauri::State<'_, AppState>,
    panel_id: String,
) -> Result<(), String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.unfocus_panel(&panel_id)
}

/// Get status of a session (for debugging)
#[tauri::command]
fn get_session_status(
    state: tauri::State<'_, AppState>,
    panel_id: String,
) -> Result<SessionStatusInfo, String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.get_session_status(&panel_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            session_manager: Mutex::new(SessionManager::new()),
        })
        .invoke_handler(tauri::generate_handler![
            open_session,
            write_session,
            resize_session,
            close_session,
            get_sessions,
            focus_panel,
            unfocus_panel,
            get_session_status,
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle()
                    .plugin(tauri_plugin_log::Builder::default().build())?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
