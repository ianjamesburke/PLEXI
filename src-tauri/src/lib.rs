mod config;
mod pty;
mod session;
mod shell_integration;

use session::{
    OpenSessionParams, SessionInput, SessionManager, SessionStartedMessage, SessionStatusInfo,
};
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager};

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

fn create_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    let pkg_name = app.package_info().name.clone();

    let app_menu = SubmenuBuilder::new(app, &pkg_name)
        .about(None)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let new_terminal = MenuItemBuilder::with_id("shell-new-terminal", "New Terminal")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let close_terminal = MenuItemBuilder::with_id("shell-close-terminal", "Close Terminal")
        .accelerator("CmdOrCtrl+W")
        .build(app)?;
    let save_workspace = MenuItemBuilder::with_id("shell-save-workspace", "Save Workspace")
        .accelerator("CmdOrCtrl+S")
        .build(app)?;
    let save_workspace_as = MenuItemBuilder::with_id("shell-save-workspace-as", "Save Workspace As…")
        .enabled(false)
        .build(app)?;
    let load_workspace = MenuItemBuilder::with_id("shell-load-workspace", "Load Workspace…")
        .enabled(false)
        .build(app)?;

    let shell_menu = SubmenuBuilder::new(app, "Shell")
        .item(&new_terminal)
        .item(&close_terminal)
        .separator()
        .item(&save_workspace)
        .item(&save_workspace_as)
        .item(&load_workspace)
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let zoom_in = MenuItemBuilder::with_id("view-zoom-in", "Increase Font Size")
        .accelerator("CmdOrCtrl+=")
        .build(app)?;
    let zoom_out = MenuItemBuilder::with_id("view-zoom-out", "Decrease Font Size")
        .accelerator("CmdOrCtrl+-")
        .build(app)?;
    let toggle_sidebar = MenuItemBuilder::with_id("view-toggle-sidebar", "Toggle Sidebar")
        .accelerator("CmdOrCtrl+B")
        .build(app)?;
    let toggle_minimap = MenuItemBuilder::with_id("view-toggle-minimap", "Toggle Minimap")
        .accelerator("CmdOrCtrl+M")
        .build(app)?;
    let show_shortcuts = MenuItemBuilder::with_id("view-show-shortcuts", "Keyboard Reference")
        .accelerator("CmdOrCtrl+/")
        .build(app)?;

    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&zoom_in)
        .item(&zoom_out)
        .separator()
        .item(&toggle_sidebar)
        .item(&toggle_minimap)
        .separator()
        .item(&show_shortcuts)
        .separator()
        .fullscreen()
        .build()?;

    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .build()?;

    MenuBuilder::new(app)
        .items(&[&app_menu, &shell_menu, &edit_menu, &view_menu, &window_menu])
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .menu(create_menu)
        .on_menu_event(|app, event| {
            let cmd = match event.id().as_ref() {
                "shell-new-terminal" => Some("new-node-right"),
                "shell-close-terminal" => Some("close-terminal"),
                "shell-save-workspace" => Some("save-workspace"),
                "view-zoom-in" => Some("zoom-in"),
                "view-zoom-out" => Some("zoom-out"),
                "view-toggle-sidebar" => Some("toggle-sidebar"),
                "view-toggle-minimap" => Some("toggle-minimap"),
                "view-show-shortcuts" => Some("toggle-shortcuts"),
                _ => None,
            };
            if let Some(cmd) = cmd {
                let _ = app.emit("plexi-menu-command", cmd);
            }
        })
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            session_manager: SessionManager::new(
                shell_integration::ensure_shell_integration()
                    .ok()
                    .and_then(|p| p.to_str().map(str::to_string)),
            ),
        })
        .invoke_handler(tauri::generate_handler![
            open_session,
            write_session,
            resize_session,
            close_session,
            get_sessions,
            get_session_status,
            quit_app,
            config::get_plexi_paths,
            config::read_config,
            config::write_config,
            config::read_workspace,
            config::write_workspace,
            config::backup_workspace,
            config::list_workspaces,
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
