//! Startup-only plugin policy and inert restore views. No plugin factory runs here.
use crate::app::view_trait::{View, ViewRenderContext};
use crate::host::pane::{Pane, ViewPane, ViewRuntime};
use crate::workspace::SavedPane;

pub(crate) fn unavailable(plugin: &str) -> String {
    format!("Plugin '{plugin}' is unavailable because core-only mode is on. Set core_only = false in the profile config and restart Plexi.")
}

/// Why routines do not run in core-only mode. Shared by the scheduler's
/// suspension trace and the `plexi routine run` refusal (stint 1242).
pub(crate) fn routines_suspended() -> String {
    "Routines are suspended because core-only mode is on. Set core_only = false in the profile config and restart Plexi.".to_string()
}

impl super::PlexiApp {
    pub(crate) fn require_plugin(&mut self, plugin: &str) -> Result<(), String> {
        if !self.config.core_only.unwrap_or(false) {
            return Ok(());
        }
        let message = unavailable(plugin);
        log::info!("plugins: launch refused plugin={plugin}: core-only mode is on");
        self.plugin_notice = Some((message.clone(), std::time::Instant::now()));
        Err(message)
    }

    /// The refusal notice expires after the kit notice dwell. Runs from
    /// `App::logic`, so a hidden window still clears it.
    pub(crate) fn expire_plugin_notice(&mut self) {
        if self
            .plugin_notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= crate::ui::style::TOAST_DWELL_NOTICE)
        {
            self.plugin_notice = None;
            log::info!(
                "plugins: core-only notice expired after {:?}",
                crate::ui::style::TOAST_DWELL_NOTICE
            );
        }
    }

    /// The refusal notice never takes keyboard focus and never claims Esc:
    /// the refused launch may be an agent's or script's request, and nothing
    /// an agent does may move the user's focus. It expires on its own after
    /// the kit notice dwell, or sooner with its Dismiss action.
    pub(crate) fn draw_plugin_notice(&mut self, ctx: &egui::Context) {
        let Some((message, at)) = self.plugin_notice.clone() else {
            return;
        };
        let remaining = crate::ui::style::TOAST_DWELL_NOTICE.saturating_sub(at.elapsed());
        if remaining.is_zero() {
            return;
        }
        let colors = self.colors;
        let response = crate::ui::toast::ToastShell::bottom("plugin_unavailable")
            .notice_action(ctx, &colors, &message, "Dismiss");
        #[cfg(test)]
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("plugin_notice_dismiss_rect"), response.rect)
        });
        if response.clicked() {
            self.plugin_notice = None;
            log::info!("plugins: core-only notice dismissed");
        }
        ctx.request_repaint_after(
            remaining + std::time::Duration::from_secs_f32(ctx.input(|i| i.predicted_dt)),
        );
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
        agent: None,
        slots: saved.slots.clone(),
        semantic_state: Default::default(),
    }))
}
