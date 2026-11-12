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
        // Own the keyboard in this frame, so Esc/Enter reach the notice
        // rather than the pane under it.
        self.sync_plugin_notice_focus();
        Err(message)
    }

    /// The notice owns a focus layer exactly while `plugin_notice` is set.
    pub(crate) fn sync_plugin_notice_focus(&mut self) {
        self.reconcile_focus_layer(
            crate::app::FocusKind::PluginNotice,
            self.plugin_notice.is_some(),
        );
    }

    pub(crate) fn plugin_notice_handle_key(
        &mut self,
        input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
            || input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
        {
            self.dismiss_plugin_notice("keyboard");
        }
        crate::app::view_trait::KeyDisposition::Consumed
    }

    fn dismiss_plugin_notice(&mut self, via: &str) {
        if self.plugin_notice.take().is_some() {
            log::info!("plugins: core-only notice dismissed via {via}");
        }
        self.sync_plugin_notice_focus();
    }

    pub(crate) fn draw_plugin_notice(&mut self, ctx: &egui::Context) {
        let Some(message) = self.plugin_notice.clone() else {
            return;
        };
        let colors = self.colors;
        let response = crate::ui::toast::ToastShell::bottom("plugin_unavailable")
            .notice_action(ctx, &colors, &message, "Dismiss");
        if response.clicked() {
            self.dismiss_plugin_notice("button");
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
