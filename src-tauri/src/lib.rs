mod pty;
mod session;

use session::{
    OpenSessionParams, SessionInput, SessionManager, SessionStartedMessage, SessionStatusInfo,
};
use tauri::Manager;

struct AppState {
    session_manager: SessionManager,
}

#[tauri::command]
async fn open_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    panel_id: String,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<SessionStartedMessage, String> {
    state
        .session_manager
        .open_session(
            app,
            OpenSessionParams {
                panel_id,
                cwd,
                cols,
                rows,
            },
        )
        .await
}

#[tauri::command]
async fn write_session(
    state: tauri::State<'_, AppState>,
    panel_id: String,
    data: String,
) -> Result<(), String> {
    state
        .session_manager
        .write_session(SessionInput { panel_id, data })
        .await
}

#[tauri::command]
async fn resize_session(
    state: tauri::State<'_, AppState>,
    panel_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state
        .session_manager
        .resize_session(panel_id, cols, rows)
        .await
}

#[tauri::command]
async fn close_session(state: tauri::State<'_, AppState>, panel_id: String) -> Result<(), String> {
    state.session_manager.close_session(panel_id).await
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
async fn get_sessions(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.session_manager.get_all_sessions().await)
}

#[tauri::command]
async fn get_session_status(
    state: tauri::State<'_, AppState>,
    panel_id: String,
) -> Result<SessionStatusInfo, String> {
    state.session_manager.get_session_status(&panel_id).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .menu(tauri::menu::Menu::default)
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            session_manager: SessionManager::new(),
        })
        .invoke_handler(tauri::generate_handler![
            open_session,
            write_session,
            resize_session,
            close_session,
            get_sessions,
            get_session_status,
            quit_app,
        ])
        .on_window_event(|window, event| {
            if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    app.state::<AppState>()
                        .session_manager
                        .close_all_sessions()
                        .await;
                });
            }
        })
        .setup(|_app| Ok(()))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
