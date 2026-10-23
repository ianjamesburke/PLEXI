//! Lineage labels for panes that live in a child context.
use super::PlexiApp;
use std::collections::HashMap;

impl PlexiApp {
    /// Lineage for the displayed tiles, resolved before the renderer borrows its window.
    pub(crate) fn pane_lineage(&self) -> HashMap<u64, String> {
        let win = &self.windows[self.active_window];
        let pane_parent_name = |parent: u64| {
            self.windows
                .iter()
                .find_map(|w| w.panes.get(&parent))
                .and_then(|p| match p {
                    crate::host::pane::Pane::Terminal(t) => t
                        .name
                        .clone()
                        .or_else(|| t.agent_role.clone()),
                    crate::host::pane::Pane::App(a) => Some(a.name.clone()),
                    crate::host::pane::Pane::Portal(_) => None,
                })
                .unwrap_or_else(|| parent.to_string())
        };
        win.panes
            .iter()
            .filter_map(|(&id, pane)| {
                let context_id = pane.portal_target().unwrap_or(win.context_id);
                let parent_context = self
                    .router
                    .iter()
                    .find(|c| c.context_id == context_id)?
                    .parent_id?;
                let parent = pane.as_terminal().and_then(|t| t.parent_pane_id).or_else(|| {
                    pane.portal_target().and_then(|target| {
                        let mut parents = self
                            .windows
                            .iter()
                            .filter(|w| w.context_id == target)
                            .flat_map(|w| w.panes.values())
                            .filter_map(|p| p.as_terminal()?.parent_pane_id);
                        let first = parents.next()?;
                        parents.all(|parent| parent == first).then_some(first)
                    })
                });
                let label = if let Some(parent) = parent {
                    pane_parent_name(parent)
                } else {
                    self.router
                        .iter()
                        .find(|c| c.context_id == parent_context)
                        .map(|c| c.name.to_string())
                        .unwrap_or_else(|| parent_context.to_string())
                };
                Some((id, label))
            })
            .collect()
    }

    pub(crate) fn visible_pane_rect(&self, pane_id: u64) -> Option<egui::Rect> {
        let (win, tile) = self.find_pane_in_any_window(pane_id)?;
        self.windows[win].tree.tiles.rect(tile)
    }
}
