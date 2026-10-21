use crate::app::app_trait::{App, AppRenderContext, KeyDisposition};
use crate::ui::button::{self, ButtonKind};
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::{labels, row, style};
use crate::workspace::secrets::manager::{self, Mutation, SecretEntry, SecretTarget};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

struct PendingImport {
    target: SecretTarget,
    receiver: mpsc::Receiver<crate::host::services::FilePickOutcome>,
    ready: Arc<AtomicBool>,
}

pub struct SecretsApp {
    entries: Vec<SecretEntry>,
    selected: usize,
    cwd: PathBuf,
    target: SecretTarget,
    editing: bool,
    form_open: bool,
    new_key: String,
    new_value: zeroize::Zeroizing<String>,
    focus_value: bool,
    pending_delete: bool,
    pick_requested: bool,
    pending_import: Option<PendingImport>,
    picker: Arc<dyn crate::host::services::PickerService>,
    terminal_inject: Vec<String>,
    status_msg: String,
}

impl SecretsApp {
    pub fn new(cwd: PathBuf) -> Self {
        let mut app = Self {
            entries: Vec::new(),
            selected: 0,
            target: SecretTarget::Global,
            cwd,
            editing: false,
            form_open: false,
            new_key: String::new(),
            new_value: zeroize::Zeroizing::new(String::new()),
            focus_value: false,
            pending_delete: false,
            pick_requested: false,
            pending_import: None,
            picker: crate::host::services::default_picker_service(),
            terminal_inject: Vec::new(),
            status_msg: String::new(),
        };
        match manager::workspace_target(&app.cwd) {
            Ok(Some(target)) => app.target = target,
            Ok(None) => {}
            Err(error) => app.report_error(error),
        }
        app.refresh();
        log::info!(
            "secrets_manager: opened cwd={} count={}",
            app.cwd.display(),
            app.entries.len()
        );
        app
    }

    fn report_error(&mut self, error: String) {
        log::warn!("secrets_manager: {error}");
        self.status_msg = error;
    }

    fn refresh(&mut self) {
        match manager::secret_inventory(&self.cwd, false) {
            Ok(entries) => self.entries = entries,
            Err(error) => self.report_error(error),
        }
        self.terminal_inject.clear();
        match manager::workspace_target(&self.cwd) {
            Ok(Some(SecretTarget::Workspace { root, .. })) => {
                match crate::workspace::secrets::WorkspaceSecrets::load(&root) {
                    Ok(Some(router)) => self.terminal_inject = router.terminal.env.inject,
                    Ok(None) => {}
                    Err(error) => self.report_error(error),
                }
            }
            Ok(_) => {}
            Err(error) => self.report_error(error),
        }
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    fn begin_add(&mut self) {
        self.new_key.clear();
        self.new_value.clear();
        self.editing = false;
        self.form_open = true;
        self.focus_value = false;
        self.pending_delete = false;
        self.status_msg.clear();
    }

    fn begin_edit(&mut self) {
        if let Some(entry) = self.entries.get(self.selected) {
            self.target = entry.target.clone();
            self.new_key = entry.name.clone();
            self.new_value.clear();
            self.editing = true;
            self.form_open = true;
            self.focus_value = true;
            self.pending_delete = false;
            self.status_msg = "Enter a replacement value. The stored value stays hidden.".into();
        }
    }

    fn close_form(&mut self) {
        self.form_open = false;
        self.new_value.clear();
    }

    fn store_for(target: &SecretTarget) -> &'static dyn crate::workspace::secrets::SecretStore {
        match target {
            SecretTarget::Folder(_) => crate::workspace::secrets::folder_store(),
            _ => crate::workspace::secrets::system_store(),
        }
    }

    fn commit_add(&mut self) {
        let name = self.new_key.trim().to_string();
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let result = manager::mutate_secret(
            Self::store_for(&self.target),
            &monitor,
            &self.target,
            &name,
            Mutation::Set(&self.new_value),
        );
        match result {
            Ok(()) => {
                self.close_form();
                self.refresh();
                self.selected = self
                    .entries
                    .iter()
                    .position(|entry| entry.name == name && entry.target == self.target)
                    .unwrap_or(0);
                self.status_msg = format!("Saved {name}.");
            }
            Err(error) => self.report_error(error),
        }
    }

    fn delete_selected(&mut self) {
        self.pending_delete = false;
        let Some(entry) = self.entries.get(self.selected).cloned() else {
            return;
        };
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        match manager::mutate_secret(
            Self::store_for(&entry.target),
            &monitor,
            &entry.target,
            &entry.name,
            Mutation::Delete,
        ) {
            Ok(()) => {
                self.refresh();
                self.status_msg = format!("Deleted {}.", entry.name);
            }
            Err(error) => self.report_error(error),
        }
    }

    fn import_selected(&mut self, target: &SecretTarget, path: &Path) {
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let result = manager::import_env(Self::store_for(target), &monitor, target, path);
        self.refresh();
        match result {
            Ok(count) => {
                self.status_msg =
                    format!("Imported {count} secrets. The .env file was not changed.")
            }
            Err(error) => self.report_error(error),
        }
    }

    fn start_picker(&mut self, context: &egui::Context) {
        self.pick_requested = false;
        if self.pending_import.is_some() {
            return;
        }
        let service = self.picker.clone();
        let (sender, receiver) = mpsc::channel();
        let ready = Arc::new(AtomicBool::new(false));
        let completed = ready.clone();
        let wake = context.clone();
        let result = std::thread::Builder::new()
            .name("secrets-import-picker".into())
            .spawn(move || {
                let result = service.pick(&crate::host::services::FilePickRequest {
                    filter: Vec::new(),
                    multiple: false,
                    mode: crate::protocol::FilePickerMode::Open,
                });
                if sender.send(result).is_ok() {
                    completed.store(true, Ordering::Release);
                    wake.request_repaint();
                }
            });
        match result {
            Ok(_) => {
                self.pending_import = Some(PendingImport {
                    target: self.target.clone(),
                    receiver,
                    ready,
                });
                self.status_msg = "Choose a .env file to import into the selected scope.".into();
            }
            Err(error) => self.report_error(format!("Could not start file picker: {error}")),
        }
    }

    fn scope_picker(&mut self, ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) {
        labels::description_label(ui, "Scope", colors);
        ui.add_enabled_ui(!self.editing && self.pending_import.is_none(), |ui| {
            egui::ComboBox::from_id_salt("secret_scope")
                .selected_text(self.target.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.target, SecretTarget::Global, "Global");
                    if let Ok(Some(target)) = manager::workspace_target(&self.cwd) {
                        ui.selectable_value(&mut self.target, target, "Workspace");
                    }
                    if crate::release::feature_enabled(
                        crate::release::ReleaseFeature::FolderSecrets,
                    ) {
                        ui.selectable_value(
                            &mut self.target,
                            SecretTarget::Folder(crate::platform::path::canonical_or_self(
                                &self.cwd,
                            )),
                            "Folder (current directory)",
                        );
                    }
                });
        });
    }

    fn form(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;
        labels::section_header(
            ui,
            if self.editing {
                "Edit Secret"
            } else {
                "New Secret"
            },
            colors,
        );
        self.scope_picker(ui, colors);
        let key_id = ui.make_persistent_id("secrets_new_key");
        let value_id = ui.make_persistent_id("secrets_new_value");
        labels::description_label(ui, "Name", colors);
        ui.add_enabled_ui(!self.editing, |ui| {
            let response = crate::ui::text_field::TextField::singleline(
                key_id,
                "e.g. OPENAI_API_KEY",
            )
            .show(ui, &mut self.new_key, colors);
            crate::ui::focus::register_text_surface(
                ui.ctx(),
                crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
                key_id,
            );
            if response.has_focus() {
                self.focus_value = false;
            }
            if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Tab)) {
                self.focus_value = true;
            }
        });
        labels::description_label(ui, "Value (hidden)", colors);
        let response = crate::ui::text_field::TextField::singleline(value_id, "secret value")
            .password(true)
            .show(ui, &mut self.new_value, colors);
        crate::ui::focus::register_text_surface(
            ui.ctx(),
            crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
            value_id,
        );
        if response.has_focus() {
            self.focus_value = true;
        }
        let save = response.has_focus()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        crate::ui::focus::claim_text_surface(
            ui.ctx(),
            crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
            if self.focus_value { value_id } else { key_id },
        );
        ui.add_space(style::SPACE_SM);
        ui.horizontal(|ui| {
            if button::chrome_button(ui, "Save", ButtonKind::Accent, colors, 64.0).clicked() || save
            {
                self.commit_add();
            }
            if button::chrome_button(ui, "Cancel", ButtonKind::Secondary, colors, 64.0).clicked() {
                self.close_form();
            }
        });
        HintBar::new(&[
            HintGroup::new(&["Tab"], "next field"),
            HintGroup::new(&["Enter"], "save"),
            HintGroup::new(&["Esc"], "cancel"),
        ])
        .show(ui, colors);
    }

    fn toggle_terminal_env(&mut self, name: &str, root: &Path) {
        let enabled = !self.terminal_inject.iter().any(|key| key == name);
        match crate::workspace::secrets::write_terminal_env_inject(root, name, enabled) {
            Ok(()) => {
                self.refresh();
                self.status_msg = format!(
                    "{name}: human terminal environment {}. Agent panes remain excluded.",
                    if enabled { "on" } else { "off" }
                );
            }
            Err(error) => self.report_error(error),
        }
    }
}

impl App for SecretsApp {
    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn type_id(&self) -> &'static str {
        "secrets_manager"
    }
    fn display_name(&self) -> String {
        "Secrets".into()
    }
    fn sync_cwd(&mut self, cwd: &Path) {
        if self.cwd != cwd {
            self.cwd = cwd.into();
            if !self.form_open && self.pending_import.is_none() {
                match manager::workspace_target(cwd) {
                    Ok(target) => self.target = target.unwrap_or(SecretTarget::Global),
                    Err(error) => self.report_error(error),
                }
            }
            self.refresh();
        }
    }
    fn background_tick(&mut self) {
        let Some(pending) = self.pending_import.as_ref() else {
            return;
        };
        match pending.receiver.try_recv() {
            Ok(outcome) => {
                let target = pending.target.clone();
                self.pending_import = None;
                match outcome {
                    crate::host::services::FilePickOutcome::Picked(paths) => {
                        if let Some(path) = paths.first() {
                            self.import_selected(&target, path);
                        }
                    }
                    crate::host::services::FilePickOutcome::Cancelled => {
                        self.status_msg = "Import cancelled.".into()
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending_import = None;
                self.report_error("File picker stopped before returning a file.".into());
            }
        }
    }
    fn needs_background_tick(&self) -> bool {
        self.pending_import
            .as_ref()
            .is_some_and(|pending| pending.ready.load(Ordering::Acquire))
    }
    fn submit_on_focused_text_input_enter(&self) -> bool {
        self.form_open
    }
    fn handle_key(&mut self, input: &crate::app::input_router::PlexiInput) -> KeyDisposition {
        if self.form_open {
            if input.modifiers().is_none() && input.key_pressed(egui::Key::Enter) {
                self.commit_add();
                return KeyDisposition::Consumed;
            }
            if input.key_pressed(egui::Key::Escape) {
                self.close_form();
                return KeyDisposition::Consumed;
            }
            return KeyDisposition::Passthrough;
        }
        if input.modifiers().command || input.modifiers().alt {
            return KeyDisposition::Passthrough;
        }
        let pressed = |key, character: &str| {
            input.key_pressed(key)
                || input
                    .events()
                    .iter()
                    .any(|event| matches!(event, egui::Event::Text(text) if text == character))
        };
        if input.key_pressed(egui::Key::Escape) && self.pending_delete {
            self.pending_delete = false;
            return KeyDisposition::Consumed;
        }
        if pressed(egui::Key::J, "j") || input.key_pressed(egui::Key::ArrowDown) {
            self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
            self.pending_delete = false;
        } else if pressed(egui::Key::K, "k") || input.key_pressed(egui::Key::ArrowUp) {
            self.selected = self.selected.saturating_sub(1);
            self.pending_delete = false;
        } else if pressed(egui::Key::A, "a") || pressed(egui::Key::N, "n") {
            self.begin_add();
        } else if input.key_pressed(egui::Key::Enter) {
            self.begin_edit();
        } else if pressed(egui::Key::D, "d") && !self.entries.is_empty() {
            if self.pending_delete {
                self.delete_selected();
            } else {
                self.pending_delete = true;
            }
        } else if pressed(egui::Key::Y, "y") && self.pending_delete {
            self.delete_selected();
        } else if pressed(egui::Key::I, "i") && self.entries.is_empty() {
            self.pick_requested = true;
        } else if pressed(egui::Key::R, "r") {
            self.refresh();
            self.pending_delete = false;
        } else {
            return KeyDisposition::Passthrough;
        }
        KeyDisposition::Consumed
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &AppRenderContext<'_>,
        _pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        let colors = ctx.colors;
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, colors.terminal_bg);
        if self.pick_requested {
            self.start_picker(ui.ctx());
        }
        egui::Frame::NONE.inner_margin(egui::Margin::same(style::SPACE_MD as i8)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Secrets");
                if button::chrome_button(ui, "Add", ButtonKind::Accent, colors, 64.0).clicked() { self.begin_add(); }
                if button::chrome_button(ui, "Refresh", ButtonKind::Secondary, colors, 80.0).clicked() { self.refresh(); }
            });
            if !self.form_open {
                HintBar::new(&[HintGroup::new(&["j", "k"], "select"), HintGroup::new(&["Enter"], "edit"), HintGroup::new(&["a"], "add"), HintGroup::new(&["d"], "delete")]).show(ui, colors);
            }
            // Fixed status space keeps async import completion from moving rows.
            labels::description_label(ui, if self.status_msg.is_empty() { "Values stay hidden. Permissions apply per caller." } else { &self.status_msg }, colors);
            ui.separator();
            egui::ScrollArea::vertical().id_salt("secrets_body").show(ui, |ui| {
                if self.form_open { self.form(ui, ctx); return; }
                if self.entries.is_empty() {
                    labels::section_header(ui, "No secrets stored", colors);
                    labels::description_label(ui, "Add a secret, or import literal NAME=value entries from a .env file.", colors);
                    self.scope_picker(ui, colors);
                    labels::description_label(ui, "The source file is never changed. Import replaces matching names in this scope.", colors);
                    ui.add_enabled_ui(self.pending_import.is_none(), |ui| {
                        if button::chrome_button(ui, "Import .env…", ButtonKind::Secondary, colors, 128.0).clicked() { self.pick_requested = true; }
                    });
                    HintBar::new(&[HintGroup::new(&["i"], "import .env")]).show(ui, colors);
                    return;
                }
                if self.pending_delete {
                    if let Some(entry) = self.entries.get(self.selected) { ui.label(format!("Delete {} from {}?", entry.name, entry.scope)); }
                    ui.horizontal(|ui| {
                        if button::chrome_button(ui, "Delete", ButtonKind::Accent, colors, 80.0).clicked() { self.delete_selected(); }
                        if button::chrome_button(ui, "Cancel", ButtonKind::Secondary, colors, 80.0).clicked() { self.pending_delete = false; }
                    });
                    HintBar::new(&[HintGroup::new(&["d", "y"], "confirm"), HintGroup::new(&["Esc"], "cancel")]).show(ui, colors);
                }
                let mut select = None;
                let mut edit = false;
                let mut toggle = None;
                for (index, entry) in self.entries.iter().enumerate() {
                    let (response, ()) = row::selectable_row(ui, index == self.selected, colors, |ui| {
                        ui.label(&entry.name);
                        labels::description_label(ui, &entry.scope, colors);
                        let last = entry.last_used.and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0)).map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_else(|| "unknown".into());
                        labels::description_label(ui, &format!("Last used: {last}"), colors);
                        let grant = if entry.grant.is_empty() { "Ask on agent/app read".into() } else { entry.grant.join("; ") };
                        labels::description_label(ui, &format!("Grants: {grant}"), colors);
                        if let SecretTarget::Workspace { root, .. } = &entry.target {
                            let enabled = self.terminal_inject.contains(&entry.name);
                            if button::chrome_button(ui, if enabled { "Human terminal env: on" } else { "Human terminal env: off" }, ButtonKind::Secondary, colors, 180.0).clicked() { toggle = Some((entry.name.clone(), root.clone())); }
                        }
                    });
                    if response.clicked() { select = Some(index); }
                    if response.double_clicked() { select = Some(index); edit = true; }
                }
                if let Some(index) = select { self.selected = index; self.pending_delete = false; }
                if edit { self.begin_edit(); }
                if let Some((name, root)) = toggle { self.toggle_terminal_env(&name, &root); }
            });
        });
    }
    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({"selected": self.selected}))
    }
    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(selected) = state.get("selected").and_then(|v| v.as_u64()) {
            self.selected = (selected as usize).min(self.entries.len().saturating_sub(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_app_mutations_use_shared_audited_handlers() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let cwd = tempfile::tempdir().unwrap();
        let cfg = crate::workspace::secrets::init_workspace(
            cwd.path(),
            &crate::config::workspace_channel_dir(),
        )
        .unwrap();
        let mut app = SecretsApp::new(cwd.path().into());
        app.begin_add();
        app.new_key = "APP_TEST_TOKEN".into();
        *app.new_value = "canary-app-first".into();
        app.commit_add();
        assert!(!app.form_open, "{}", app.status_msg);
        app.begin_edit();
        assert!(
            app.new_value.is_empty(),
            "edit must not fetch the stored value"
        );
        *app.new_value = "canary-app-second".into();
        app.commit_add();
        let account = crate::workspace::secrets::keychain_workspace_name(&cfg.id, "APP_TEST_TOKEN");
        assert_eq!(
            crate::workspace::secrets::system_store()
                .get(&account)
                .unwrap()
                .as_str(),
            "canary-app-second"
        );
        app.delete_selected();
        assert!(crate::workspace::secrets::system_store()
            .get(&account)
            .is_none());
        let fixture = cwd.path().join("fixture.env");
        let bytes = b"IMPORTED_APP_TOKEN=canary-app-import\n";
        std::fs::write(&fixture, bytes).unwrap();
        app.import_selected(&app.target.clone(), &fixture);
        assert_eq!(std::fs::read(&fixture).unwrap(), bytes);
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(profile.path());
        let rows = monitor.audit_records();
        assert_eq!(
            rows.iter()
                .filter(|r| r.kind == "secret_mutation" && r.decision == "ok")
                .count(),
            4
        );
        let audit = std::fs::read_to_string(profile.path().join("permission-audit.jsonl")).unwrap();
        assert!(audit.contains("APP_TEST_TOKEN"));
        assert!(!audit.contains("canary-app-"));
    }

    #[test]
    fn picker_import_keeps_chosen_scope_and_does_not_change_source() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().into());
        let root = tempfile::tempdir().unwrap();
        let cfg = crate::workspace::secrets::init_workspace(
            root.path(),
            &crate::config::workspace_channel_dir(),
        )
        .unwrap();
        let fixture = root.path().join("fixture.env");
        let bytes = b"PICKER_0858_TOKEN=picker-secret-canary\n";
        std::fs::write(&fixture, bytes).unwrap();
        let mut app = SecretsApp::new(root.path().into());
        app.picker = Arc::new(crate::host::services::ScriptedPickerService::from_outcomes(
            vec![crate::host::services::FilePickOutcome::Picked(vec![
                fixture.clone(),
            ])],
        ));
        app.start_picker(&egui::Context::default());
        app.target = SecretTarget::Global;
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
        while app.pending_import.is_some() && std::time::Instant::now() < deadline {
            app.background_tick();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.pending_import.is_none());
        assert!(
            app.status_msg.starts_with("Imported 1"),
            "{}",
            app.status_msg
        );
        assert_eq!(std::fs::read(&fixture).unwrap(), bytes);
        let account =
            crate::workspace::secrets::keychain_workspace_name(&cfg.id, "PICKER_0858_TOKEN");
        assert_eq!(
            crate::workspace::secrets::system_store()
                .get(&account)
                .unwrap()
                .as_str(),
            "picker-secret-canary"
        );
    }

    #[test]
    fn screenshot_secrets_scoped_inventory() {
        use crate::app::permissions::AppPermissions;
        let mut h = crate::ui_tests::PlexiUiHarness::new_sized_ppp(1000.0, 700.0, 1.0);
        let _experiment =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
        let root = tempfile::tempdir().unwrap();
        let cfg = crate::workspace::secrets::init_workspace(
            root.path(),
            &crate::config::workspace_channel_dir(),
        )
        .unwrap();
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let global = SecretTarget::Global;
        let workspace = SecretTarget::Workspace {
            id: cfg.id,
            root: root.path().into(),
        };
        let folder = SecretTarget::Folder(crate::platform::path::canonical_or_self(root.path()));
        for (name, target) in [
            ("OPENAI_0858_SHOT_KEY", &global),
            ("GITHUB_TOKEN", &workspace),
            ("DEPLOY_TOKEN", &folder),
        ] {
            manager::mutate_secret(
                SecretsApp::store_for(target),
                &monitor,
                target,
                name,
                Mutation::Set("screenshot-canary-value"),
            )
            .unwrap();
        }
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            app.open_builtin_app_pane(
                Box::new(crate::file_browser::FileBrowserApp::new(root.path().into())),
                AppPermissions::builtin(),
                root.path().into(),
                None,
                None,
                None,
            );
        });
        h.run_steps(3);
        h.harness().key_press_modifiers(
            egui::Modifiers {
                command: true,
                mac_cmd: true,
                shift: true,
                ..Default::default()
            },
            egui::Key::S,
        );
        h.run_steps(3);
        h.with_app_mut(|app| {
            let secrets = app.windows[app.active_window]
                .panes
                .values_mut()
                .find_map(|pane| {
                    let crate::host::pane::AppRuntime::Builtin(builtin) =
                        &mut pane.as_app_mut()?.runtime
                    else {
                        return None;
                    };
                    builtin.as_any_mut().downcast_mut::<SecretsApp>()
                })
                .expect("hotkey opens Secrets");
            assert!(secrets
                .entries
                .iter()
                .any(|entry| entry.name == "OPENAI_0858_SHOT_KEY" && entry.target == global));
            assert!(secrets
                .entries
                .iter()
                .any(|entry| entry.name == "GITHUB_TOKEN" && entry.target == workspace));
            assert!(secrets
                .entries
                .iter()
                .any(|entry| entry.name == "DEPLOY_TOKEN" && entry.target == folder));
        });
        h.save_screenshot(&crate::ui_tests::evidence_png!())
            .unwrap();
        h.harness().set_size(egui::vec2(520.0, 650.0));
        h.run_steps(3);
        h.save_screenshot(&crate::ui_tests::evidence_png!("_narrow"))
            .unwrap();
        h.harness().key_press(egui::Key::Enter);
        h.run_steps(3);
        h.save_screenshot(&crate::ui_tests::evidence_png!("_edit"))
            .unwrap();
        for (name, target) in [
            ("OPENAI_0858_SHOT_KEY", &global),
            ("GITHUB_TOKEN", &workspace),
            ("DEPLOY_TOKEN", &folder),
        ] {
            manager::mutate_secret(
                SecretsApp::store_for(target),
                &monitor,
                target,
                name,
                Mutation::Delete,
            )
            .unwrap();
        }
    }
}
