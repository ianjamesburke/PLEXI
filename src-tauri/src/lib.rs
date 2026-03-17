mod session;

use session::{SessionManager, OpenSessionParams, SessionInput, SessionStartedMessage};
use std::sync::Mutex;

struct AppState {
    session_manager: Mutex<SessionManager>,
}

#[tauri::command]
fn open_session(
    state: tauri::State<'_, AppState>,
    params: OpenSessionParams,
) -> Result<SessionStartedMessage, String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.open_session(params)
}

#[tauri::command]
fn write_session(
    state: tauri::State<'_, AppState>,
    input: SessionInput,
) -> Result<(), String> {
    let manager = state.session_manager.lock().map_err(|_| "Lock poisoned")?;
    manager.write_session(input)
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
