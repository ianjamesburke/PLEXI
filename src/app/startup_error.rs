//! The window a GUI launch shows when profile initialization fails, so the
//! user sees why Plexi did not open instead of watching the app bounce and quit.
//!
//! It runs before `PlexiApp` exists and owns no host state: a [`StartupErrorView`]
//! in, a process exit with status 1 out. Command-line launches keep their
//! stderr line and never open a window.
use std::path::Path;

use crate::config::ProfileInitError;
use crate::ui::button::ButtonKind;
use crate::ui::dialog::{ActionModal, DialogAction, DialogShortcut};
use crate::ui::text::{self, Side};
use crate::ui::theme::Colors;
use crate::ui::{style, typography};

/// The minimum height keeps the action row on screen: the modal scrolls its
/// body, so a window shorter than the content would hide Quit.
const WINDOW_SIZE: [f32; 2] = [640.0, 520.0];
const WINDOW_MIN_SIZE: [f32; 2] = [380.0, 480.0];

/// A GUI launch opens the window only when the process is the foreground host.
/// A command-line invocation reports on stderr, and a background host was
/// started by a command that already reported the failure.
pub(crate) fn shows_window(is_host_startup: bool, background: bool) -> bool {
    is_host_startup && !background
}

/// Plain-language content of the startup error: what failed, where, why, and
/// what to do. Holds no file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartupErrorView {
    pub(crate) title: String,
    pub(crate) plugin: Option<String>,
    pub(crate) file: Option<String>,
    pub(crate) reason: String,
    pub(crate) fix: Option<String>,
    pub(crate) reassurance: Option<String>,
    pub(crate) log_path: String,
}

impl StartupErrorView {
    pub(crate) fn from_error(error: &ProfileInitError, log_path: &Path) -> Self {
        let log_path = log_path.display().to_string();
        match error {
            ProfileInitError::Migration(failure) => Self {
                title: "Plexi could not update your plugins".into(),
                plugin: failure.package.clone(),
                file: failure.file.as_ref().map(|file| file.display().to_string()),
                reason: plain_text(&failure.plain_reason),
                fix: failure
                    .hint
                    .as_deref()
                    .map(|hint| sentence_case(&plain_text(hint))),
                reassurance: failure
                    .unchanged
                    .then(|| "Nothing was moved or changed.".to_owned()),
                log_path,
            },
            ProfileInitError::Other(message) => Self {
                title: "Plexi could not prepare its profile".into(),
                plugin: None,
                file: None,
                reason: plain_text(message),
                fix: None,
                reassurance: None,
                log_path,
            },
        }
    }

    /// The host could not take its profile lock at all (for example a
    /// profile on a file system without locks), so it cannot tell whether
    /// another Plexi already runs there.
    pub(crate) fn host_lock_failure(reason: &str, log_path: &Path) -> Self {
        Self {
            title: "Plexi could not check for another running copy".into(),
            plugin: None,
            file: None,
            reason: plain_text(reason),
            fix: Some(
                "Keep the Plexi profile on a local disk that supports file locks, then open Plexi again."
                    .into(),
            ),
            reassurance: Some("Nothing was moved or changed.".into()),
            log_path: log_path.display().to_string(),
        }
    }

    /// Everything on screen as plain text, for the Copy details action.
    pub(crate) fn details(&self) -> String {
        let mut lines = vec![self.title.clone()];
        if let Some(plugin) = &self.plugin {
            lines.push(format!("Plugin: {plugin}"));
        }
        if let Some(file) = &self.file {
            lines.push(format!("File: {file}"));
        }
        lines.push(format!("Reason: {}", self.reason));
        if let Some(fix) = &self.fix {
            lines.push(format!("Fix: {fix}"));
        }
        if let Some(reassurance) = &self.reassurance {
            lines.push(reassurance.clone());
        }
        lines.push(format!("Log: {}", self.log_path));
        lines.join("\n")
    }
}

/// The window draws plain text, so Markdown code marks are dropped. The CLI
/// and log keep the original strings.
fn plain_text(text: &str) -> String {
    text.replace('`', "")
}

fn sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StartupAction {
    Quit,
}

pub(crate) struct StartupErrorApp {
    view: StartupErrorView,
    colors: Colors,
}

impl StartupErrorApp {
    pub(crate) fn new(view: StartupErrorView, colors: Colors) -> Self {
        Self { view, colors }
    }

    /// Paint one frame. Returns the action the user chose, if any; Copy details
    /// is handled here because it needs no process decision.
    pub(crate) fn frame(&mut self, ui: &mut egui::Ui) -> Option<StartupAction> {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(self.colors.bg_darkest))
            .show_inside(ui, |_| {});
        let actions =
            [
                DialogAction::new("copy", "Copy details", ButtonKind::Ghost),
                DialogAction::new("quit", "Quit", ButtonKind::Primary).shortcut(
                    DialogShortcut::new(&["Esc"], egui::Modifiers::NONE, egui::Key::Escape),
                ),
            ];
        let view = &self.view;
        let colors = &self.colors;
        let response = ActionModal::new("startup_error", &view.title, &actions)
            .button_row()
            .default_action("quit")
            .show(&ctx, colors, |ui| {
                if let Some(plugin) = &view.plugin {
                    field(ui, "Plugin", plugin, false, colors);
                }
                if let Some(file) = &view.file {
                    field(ui, "File", file, true, colors);
                }
                field(ui, "What went wrong", &view.reason, false, colors);
                if let Some(fix) = &view.fix {
                    field(ui, "What to do", fix, false, colors);
                }
                if let Some(reassurance) = &view.reassurance {
                    typography::body_strong(ui, reassurance, colors);
                    ui.add_space(style::SPACE_SM);
                }
                field(ui, "Log", &view.log_path, true, colors);
            });
        if response.selected == Some("copy") {
            ctx.copy_text(self.view.details());
        }
        (response.selected == Some("quit") || response.dismissed || close_requested(&ctx))
            .then_some(StartupAction::Quit)
    }
}

fn close_requested(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.viewport().close_requested())
}

/// A caption over its value. Paths elide from the left so the leaf stays
/// visible; the full text is on hover and in Copy details.
fn field(ui: &mut egui::Ui, label: &str, value: &str, is_path: bool, colors: &Colors) {
    typography::caption(ui, label, colors);
    if is_path {
        let font = egui::FontId::proportional(style::TEXT_HINT);
        let shown = text::elide(ui, value, font, ui.available_width(), Side::Leading);
        typography::body(ui, shown, colors).on_hover_text(value);
    } else {
        typography::body(ui, value, colors);
    }
    ui.add_space(style::SPACE_SM);
}

impl eframe::App for StartupErrorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.frame(ui) == Some(StartupAction::Quit) {
            log::info!("startup_error: window dismissed, exiting with status 1");
            crate::cli::trace::exit(1);
        }
    }
}

/// Show the window until the user dismisses it. Dismissal exits the process
/// with status 1 from inside the app; this returns only if the window could
/// not be created, which the caller reports on stderr.
pub(crate) fn show(view: StartupErrorView, colors: Colors) -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size(WINDOW_MIN_SIZE)
        .with_title(env!("PLEXI_APP_TITLE"));
    let options = eframe::NativeOptions {
        run_and_return: false,
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        env!("CARGO_PKG_NAME"),
        options,
        Box::new(move |cc| {
            crate::ui::theme::setup_fonts(&cc.egui_ctx);
            crate::ui::theme::setup_style(&cc.egui_ctx, &colors);
            Ok(Box::new(StartupErrorApp::new(view, colors)))
        }),
    )
}

/// Report a failed profile initialization and exit with status 1. Startup
/// failures happen before the logger exists, so this brings it up, replays the
/// buffered failure into `plexi.log`, then prints the CLI line and, for a GUI
/// launch, opens the window.
pub(crate) fn exit_on_profile_init_failure(error: &ProfileInitError, show_window: bool) -> ! {
    let config = crate::config::PlexiConfig::load();
    let log_config = config.log.clone().unwrap_or_default();
    crate::platform::logging::init(
        log_config.level_filter().unwrap_or(log::LevelFilter::Info),
        log_config.retention_days.unwrap_or(30),
        !show_window,
    );
    crate::app::plugin_migration::replay_early_traces();
    log::error!(
        "startup: profile initialization failed, exiting with status 1 (window={show_window})"
    );
    log::logger().flush();
    eprintln!("profile initialization: {error}");
    if show_window {
        let view = StartupErrorView::from_error(error, &crate::platform::logging::log_path());
        let colors = crate::ui::theme::colors_from_config(&config);
        if let Err(window_error) = show(view, colors) {
            log::error!("startup_error: cannot open the error window: {window_error}");
            log::logger().flush();
            eprintln!("cannot open the error window: {window_error}");
        }
    }
    crate::cli::trace::exit(1)
}

/// Report a startup failure once logging is running: log at error, flush,
/// print the CLI line, open the window for a GUI launch, exit with status 1.
pub(crate) fn exit_with_startup_error(view: StartupErrorView, show_window: bool) -> ! {
    log::error!(
        "startup: {}: {} (window={show_window})",
        view.title,
        view.reason
    );
    log::logger().flush();
    eprintln!("{}: {}", view.title, view.reason);
    if show_window {
        let colors = crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::load());
        if let Err(window_error) = show(view, colors) {
            log::error!("startup_error: cannot open the error window: {window_error}");
            log::logger().flush();
            eprintln!("cannot open the error window: {window_error}");
        }
    }
    crate::cli::trace::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::plugin_migration::MigrationFailure;

    const MISSING_TYPE: &str =
        "schema_version = 1\n[app]\nid = 'brokenpkg'\nname = 'Broken'\nentry = 'main.py'\n";

    /// Critic on PR #3133: a profile lock that cannot be taken is shown with
    /// what failed and what to do.
    #[test]
    fn host_lock_failure_names_the_reason_and_the_fix() {
        let view = StartupErrorView::host_lock_failure(
            "cannot check for a running Plexi host: lock host.lock: no locks available",
            Path::new("/tmp/plexi.log"),
        );
        assert!(
            view.title.contains("another running copy"),
            "{}",
            view.title
        );
        assert!(
            view.reason.contains("no locks available"),
            "{}",
            view.reason
        );
        assert!(view
            .fix
            .as_deref()
            .is_some_and(|fix| fix.contains("file locks")));
        assert_eq!(view.log_path, "/tmp/plexi.log");
    }

    fn migration_error() -> ProfileInitError {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("apps/brokenpkg");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.toml"), MISSING_TYPE).unwrap();
        let failure: Box<MigrationFailure> = crate::app::plugin_migration::migrate(root.path())
            .expect_err("manifest without a type cannot migrate");
        ProfileInitError::Migration(failure)
    }

    #[test]
    fn window_opens_only_for_a_foreground_host_launch() {
        assert!(shows_window(true, false));
        assert!(!shows_window(true, true), "background host");
        assert!(!shows_window(false, false), "command-line invocation");
        assert!(!shows_window(false, true));
    }

    #[test]
    fn view_names_package_file_reason_and_fix_and_reassures() {
        let view = StartupErrorView::from_error(&migration_error(), Path::new("/p/plexi.log"));
        assert_eq!(view.plugin.as_deref(), Some("brokenpkg"));
        let file = view.file.expect("file");
        assert!(file.ends_with("apps/brokenpkg/manifest.toml"), "{file}");
        assert_eq!(
            view.reason,
            "This plugin's manifest has no type, so Plexi cannot tell how to convert it."
        );
        let fix = view.fix.expect("fix");
        assert!(
            fix.starts_with("Add type = \"app\" to [app] in the named manifest"),
            "sentence case: {fix}"
        );
        assert_eq!(
            view.reassurance.as_deref(),
            Some("Nothing was moved or changed.")
        );
        assert_eq!(view.log_path, "/p/plexi.log");
    }

    fn error_for_manifest(manifest: &str) -> ProfileInitError {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("apps/pkg");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.toml"), manifest).unwrap();
        ProfileInitError::Migration(crate::app::plugin_migration::migrate(root.path()).unwrap_err())
    }

    /// The window draws plain text: no Markdown code marks and no internal
    /// vocabulary, for every failure class a user can reach.
    #[test]
    fn no_text_in_the_window_has_backticks_or_internal_wording() {
        let app = "[app]\nid = 'pkg'\ntype = 'app'\nentry = 'main.py'\n";
        let mut errors: Vec<ProfileInitError> = [
            "schema_version = 1\n[app\n".to_owned(),
            "name = 'x'\n".to_owned(),
            "schema_version = 1\n".to_owned(),
            "schema_version = 1\napp = 3\n".to_owned(),
            "schema_version = 1\n[app]\nname = 'B'\ntype = 'app'\nentry = 'main.py'\n".into(),
            MISSING_TYPE.to_owned(),
            "schema_version = 1\n[app]\nid = 'pkg'\ntype = 'rust'\nentry = 'main.py'\n".into(),
            "schema_version = 1\n[app]\nid = 'pkg'\ntype = 'app'\n".into(),
            format!("schema_version = 1\nruntime = 3\n{app}"),
            format!("schema_version = 1\n{app}[runtime]\nkind = 7\n"),
        ]
        .iter()
        .map(|manifest| error_for_manifest(manifest))
        .collect();
        for setup in [
            |root: &Path| std::fs::write(root.join("apps.toml"), "schema_version = 7\n").unwrap(),
            |root: &Path| std::fs::write(root.join("apps.toml"), "[[app\n").unwrap(),
            |root: &Path| {
                std::fs::create_dir(root.join("apps")).unwrap();
                std::fs::create_dir(root.join("plugins")).unwrap();
            },
        ] {
            let root = tempfile::tempdir().unwrap();
            setup(root.path());
            errors.push(ProfileInitError::Migration(
                crate::app::plugin_migration::migrate(root.path()).unwrap_err(),
            ));
        }
        for error in &errors {
            let view = StartupErrorView::from_error(error, Path::new("/p/plexi.log"));
            let details = view.details();
            let shown = [
                Some(view.title.as_str()),
                view.plugin.as_deref(),
                view.file.as_deref(),
                Some(view.reason.as_str()),
                view.fix.as_deref(),
                view.reassurance.as_deref(),
                Some(details.as_str()),
            ];
            for text in shown.into_iter().flatten() {
                assert!(!text.contains('`'), "backtick in window text: {text}");
            }
            assert!(
                !view.reason.to_lowercase().contains("legacy"),
                "internal wording: {}",
                view.reason
            );
            assert!(
                view.fix.is_some(),
                "every class names a fix: {}",
                view.reason
            );
        }
    }

    #[test]
    fn non_migration_failure_has_no_plugin_file_or_reassurance() {
        let view = StartupErrorView::from_error(
            &ProfileInitError::Other("create /x: denied".into()),
            Path::new("/p/plexi.log"),
        );
        assert_eq!(view.title, "Plexi could not prepare its profile");
        assert!(view.plugin.is_none() && view.file.is_none() && view.reassurance.is_none());
        assert_eq!(view.reason, "create /x: denied");
    }

    #[test]
    fn copy_details_carries_every_line_shown() {
        let view = StartupErrorView::from_error(&migration_error(), Path::new("/p/plexi.log"));
        let details = view.details();
        for expected in [
            "Plugin: brokenpkg",
            "Reason: This plugin's manifest has no type, so Plexi cannot tell how to convert it.",
            "Fix: Add type = \"app\"",
            "Nothing was moved or changed.",
            "Log: /p/plexi.log",
        ] {
            assert!(details.contains(expected), "{expected}\n{details}");
        }
        assert!(details.contains("File: "), "{details}");
    }

    fn harness(
        size: egui::Vec2,
        view: StartupErrorView,
    ) -> egui_kittest::Harness<'static, StartupErrorApp> {
        let colors = crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default());
        let mut initialized = false;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(size)
            .build_ui_state(
                move |ui, app: &mut StartupErrorApp| {
                    if !initialized {
                        // `ui-medium` binds on the frame after the fonts are set.
                        crate::ui::theme::setup_fonts(ui.ctx());
                        crate::ui::theme::setup_style(ui.ctx(), &app.colors);
                        initialized = true;
                        return;
                    }
                    app.frame(ui);
                },
                StartupErrorApp::new(view, colors),
            );
        harness.run();
        harness.run();
        harness
    }

    #[test]
    fn modal_shows_every_field_and_both_actions() {
        use egui_kittest::kittest::Queryable;
        let view = StartupErrorView::from_error(&migration_error(), Path::new("/p/plexi.log"));
        let h = harness(egui::vec2(720.0, 520.0), view);
        h.get_by_label("Plexi could not update your plugins");
        h.get_by_label("brokenpkg");
        h.get_by_label(
            "This plugin's manifest has no type, so Plexi cannot tell how to convert it.",
        );
        h.get_by_label("Nothing was moved or changed.");
        h.get_by_label_contains("Copy details");
        h.get_by_label_contains("Quit");
    }

    /// Run the window under a harness, apply one interaction, and return what
    /// the app asked the process to do.
    fn drive(interact: impl FnOnce(&mut egui_kittest::Harness<'_>)) -> Option<StartupAction> {
        let view = StartupErrorView::from_error(&migration_error(), Path::new("/p/plexi.log"));
        let colors = crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default());
        let mut app = StartupErrorApp::new(view, colors);
        let seen = std::cell::RefCell::new(None);
        let mut initialized = false;
        {
            let mut h = egui_kittest::Harness::builder()
                .with_size(egui::vec2(720.0, 560.0))
                .build_ui(|ui| {
                    if !initialized {
                        crate::ui::theme::setup_fonts(ui.ctx());
                        crate::ui::theme::setup_style(ui.ctx(), &app.colors);
                        initialized = true;
                        return;
                    }
                    if let Some(action) = app.frame(ui) {
                        *seen.borrow_mut() = Some(action);
                    }
                });
            h.run();
            h.run();
            assert!(seen.borrow().is_none(), "no action before any interaction");
            interact(&mut h);
            h.run();
        }
        seen.into_inner()
    }

    #[test]
    fn quit_button_requests_exit() {
        use egui_kittest::kittest::Queryable;
        let action = drive(|h| h.get_by_label_contains("Quit").click());
        assert_eq!(action, Some(StartupAction::Quit));
    }

    #[test]
    fn escape_requests_exit() {
        let action = drive(|h| h.key_press(egui::Key::Escape));
        assert_eq!(action, Some(StartupAction::Quit));
    }

    fn shot(size: egui::Vec2, test_name: &str) {
        let view = StartupErrorView::from_error(
            &migration_error(),
            Path::new("/Users/someone/.plexi-beta/plexi.log"),
        );
        let mut h = harness(size, view);
        h.render()
            .expect("render")
            .save(crate::ui_tests::evidence_png_path(test_name, ""))
            .expect("save screenshot");
    }

    #[test]
    fn screenshot_startup_error_normal_width() {
        shot(
            egui::vec2(720.0, 520.0),
            "screenshot_startup_error_normal_width",
        );
    }

    #[test]
    fn screenshot_startup_error_narrow_width() {
        // The window's minimum size: the narrowest and shortest a user can make it.
        shot(
            egui::vec2(WINDOW_MIN_SIZE[0], WINDOW_MIN_SIZE[1]),
            "screenshot_startup_error_narrow_width",
        );
    }
}
