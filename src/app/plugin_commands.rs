//! Plugin manifest commands (`[[contributes.commands]]`) in discovery and dispatch.
//! A command is offered only while its plugin has a view open in the invoking
//! context, and its handler runs on that live view. Starting a plugin headless
//! to run a handler with no view open is later work.

use super::PlexiApp;
use crate::host::keys::{plugin_view_commands, DiscoveredCommand, PluginCommandTarget};
use crate::host::picker::InputOrigin;

impl PlexiApp {
    /// Commands offered by plugin views open in `context_id`. When a plugin has
    /// several views open, the focused pane wins, then the lowest pane ID.
    pub(crate) fn context_plugin_commands(
        &mut self,
        context_id: u64,
        focused_pane: Option<u64>,
    ) -> Vec<DiscoveredCommand> {
        let mut views: Vec<(u64, String, std::path::PathBuf)> = self
            .windows
            .iter()
            .filter(|window| window.context_id == context_id)
            .flat_map(|window| window.panes.values())
            .filter_map(|pane| pane.as_view())
            .map(|view| {
                (
                    view.id,
                    view.manifest_id.clone(),
                    view.workspace_root.clone(),
                )
            })
            .collect();
        views.sort_by_key(|(pane_id, ..)| (Some(*pane_id) != focused_pane, *pane_id));
        let mut seen = std::collections::HashSet::new();
        let mut commands = Vec::new();
        for (pane_id, plugin_id, root) in views {
            if !seen.insert(plugin_id.clone()) {
                continue;
            }
            if let Some(plugin) = self.registries.view_for_root(&root).get(&plugin_id) {
                commands.extend(plugin_view_commands(plugin, pane_id));
            }
        }
        commands
    }

    /// Deliver a plugin command's handler to the view captured at discovery.
    /// Refuses when that pane closed or now runs another plugin, and refuses
    /// approval handlers from synthetic input.
    pub(crate) fn run_plugin_command(&mut self, target: PluginCommandTarget, origin: InputOrigin) {
        let id = crate::host::guest_text::shown(&target.command);
        let pane_id = target.pane_id;
        let refusal = if matches!(origin, InputOrigin::Synthetic(_))
            && super::plugin_call::is_approval_action(&target.handler)
        {
            Err("Approval commands need real keyboard input.".to_string())
        } else {
            match self
                .windows
                .iter_mut()
                .find_map(|window| window.panes.get_mut(&pane_id))
                .and_then(|pane| pane.as_view_mut())
                .filter(|view| view.manifest_id == target.plugin)
            {
                None => Err(format!(
                    "{id} is unavailable: its view closed. Reopen the command palette to try again."
                )),
                Some(view) => view
                    .runtime
                    .send_plugin_action(target.handler.clone(), Vec::new()),
            }
        };
        match refusal {
            Ok(()) => log::info!("command: plugin dispatch id={id} pane={pane_id}"),
            Err(reason) => {
                log::info!("command: rejected id={id} pane={pane_id} reason={reason}");
                self.attention_queue.transient_hint = Some((reason, std::time::Instant::now()));
            }
        }
    }
}
