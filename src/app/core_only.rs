//! Startup-only plugin policy and inert restore views. No plugin factory runs here.
use crate::app::view_trait::{View, ViewRenderContext};
use crate::host::pane::{Pane, ViewPane, ViewRuntime};
use crate::workspace::SavedPane;

pub(crate) fn unavailable(plugin: &str) -> String {
    format!("Plugin '{plugin}' is unavailable because core-only mode is on. Set core_only = false in the profile config and restart Plexi.")
}

impl super::PlexiApp {
    pub(crate) fn require_plugin(&mut self, plugin: &str) -> Result<(), String> {
        if !self.config.core_only.unwrap_or(false) {
            return Ok(());
        }
        let message = unavailable(plugin);
        log::info!("plugins: launch refused plugin={plugin}: core-only mode is on");
        self.plugin_notice = Some(message.clone());
        Err(message)
    }

    pub(crate) fn draw_plugin_notice(&mut self, ctx: &egui::Context) {
        let Some(message) = self.plugin_notice.clone() else {
            return;
        };
        let mut dismiss = false;
        crate::ui::toast::ToastShell::bottom("plugin_unavailable").show(ctx, &self.colors, |ui| {
            ui.label(message);
            dismiss = ui
                .add(crate::ui::toast::ToastShell::button(
                    "Dismiss",
                    &self.colors,
                ))
                .clicked();
        });
        if dismiss || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.plugin_notice = None;
        }
    }

    pub(crate) fn plugin_blocked(&mut self, plugin: &str) -> bool {
        self.require_plugin(plugin).is_err()
    }
}

struct DisabledPluginView {
    original: SavedPane,
}

impl View for DisabledPluginView {
    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn type_id(&self) -> &'static str {
        "disabled-plugin-view"
    }

    fn display_name(&self) -> String {
        self.original
            .name
            .clone()
            .unwrap_or_else(|| "Disabled plugin".into())
    }

    fn disabled_plugin_record(&self) -> Option<&SavedPane> {
        Some(&self.original)
    }

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &ViewRenderContext<'_>,
        _pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        ui.vertical_centered(|ui| {
            ui.add_space(crate::ui::style::SPACE_XL);
            ui.heading("Core-only mode");
            ui.add_space(crate::ui::style::SPACE_MD);
            ui.label(
                egui::RichText::new(format!(
                    "Plugin '{}' is disabled.",
                    self.original.plugin_id.as_deref().unwrap_or("unknown")
                ))
                .color(ctx.colors.text_primary),
            );
            ui.label("This view's saved state is retained.");
            ui.label("Turn off core_only and restart to restore it.");
        });
    }
}

pub(crate) fn disabled_plugin_pane(saved: &SavedPane) -> Pane {
    log::info!(
        "plugins: preserving disabled view pane_id={} plugin={:?}",
        saved.id,
        saved.plugin_id
    );
    Pane::View(Box::new(ViewPane {
        pip_status: None,
        id: saved.id,
        runtime: ViewRuntime::Builtin(Box::new(DisabledPluginView {
            original: saved.clone(),
        })),
        workspace_root: saved.cwd.clone(),
        permissions: super::permissions::PluginPermissions::default(),
        manifest_id: saved.plugin_id.clone().unwrap_or_default(),
        name: saved
            .name
            .clone()
            .unwrap_or_else(|| "Disabled plugin".into()),
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: None,
        hidden: saved.hidden,
        agent: None,
        slots: saved.slots.clone(),
        semantic_state: Default::default(),
    }))
}
