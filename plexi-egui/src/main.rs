#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod keys;
#[cfg(target_os = "macos")]
mod macos_menu;
mod pane;
mod shell;
mod theme;
mod tiling;

fn main() -> eframe::Result {
    env_logger::init();

    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../../src-tauri/icons/icon.png"))
            .expect("failed to load app icon");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([400.0, 300.0])
            .with_title("Plexi")
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "plexi",
        native_options,
        Box::new(|cc| Ok(Box::new(app::PlexiApp::new(cc)))),
    )
}
