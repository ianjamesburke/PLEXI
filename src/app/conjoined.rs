//! A parent remains in its owning context while its child context pins a live view.
use super::PlexiApp;
use crate::agent::fleet::{AgentRow, FleetFilter};
use std::collections::HashMap;

impl PlexiApp {
    pub(crate) fn conjoined_parent(&self) -> Option<u64> {
        self.conjoined_contexts
            .get(&self.windows[self.active_window].context_id)
            .copied()
            .filter(|id| self.find_pane_in_any_window(*id).is_some())
    }

    pub(crate) fn visible_pane_rect(&self, pane_id: u64) -> Option<egui::Rect> {
        if self.conjoined_parent() == Some(pane_id) {
            if let Some((id, rect)) = self.conjoined_parent_rect {
                if id == pane_id {
                    return Some(rect);
                }
            }
        }
        let (win, tile) = self.find_pane_in_any_window(pane_id)?;
        self.windows[win].tree.tiles.rect(tile)
    }

    pub(crate) fn child_agent_strips(&self) -> HashMap<u64, Vec<AgentRow>> {
        let mut strips: HashMap<u64, Vec<AgentRow>> = HashMap::new();
        for row in self
            .agents_view(FleetFilter::All)
            .tasks
            .into_iter()
            .flat_map(|t| t.rows)
        {
            if let Some(parent) = row.parent_pane_id {
                strips.entry(parent).or_default().push(row);
            }
        }
        strips
    }

    pub(crate) fn dive_into_agent_children(&mut self, parent: u64) -> bool {
        let Some(ctx_id) = self.conjoined_contexts.iter().find_map(|(&ctx, &id)| {
            (id == parent
                && self
                    .windows
                    .iter()
                    .any(|w| w.context_id == ctx && !w.panes.is_empty()))
            .then_some(ctx)
        }) else {
            return false;
        };
        let Some(index) = self.router.position(|c| c.context_id == ctx_id) else {
            return false;
        };
        if self.router.active().context_id != ctx_id {
            let win = &self.windows[self.active_window];
            self.router
                .push_depth(win.context_id, win.window_id, win.focused_pane);
            self.switch_workspace(index);
        }
        self.conjoined_parent_focus = Some(parent);
        log::info!("agent_conjoined: dive parent={parent} context={ctx_id}");
        true
    }

    pub(crate) fn render_conjoined_parent(
        &mut self,
        ui: &mut egui::Ui,
        input: &mut Option<crate::render::terminal_pane::TerminalInput>,
    ) -> Option<egui::Rect> {
        let parent = self.conjoined_parent()?;
        let (window, tile) = self.find_pane_in_any_window(parent)?;
        if window == self.active_window {
            return None;
        }
        let rect = ui.max_rect();
        let gap = self.config.pane_gap.unwrap_or(4.0).clamp(0.0, 20.0);
        let split = rect.left() + rect.width() * 0.35;
        let left = egui::Rect::from_min_max(rect.min, egui::pos2(split - gap / 2.0, rect.bottom()));
        self.conjoined_parent_rect = Some((parent, left));
        let right = egui::Rect::from_min_max(egui::pos2(split + gap / 2.0, rect.top()), rect.max);
        if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(left) {
            self.conjoined_parent_focus = Some(parent);
        }
        let focused = self.visual_owner_pane() == Some(parent);
        let pane_input = if focused {
            input.take().unwrap_or_default()
        } else {
            Default::default()
        };
        let names = self.windows[window]
            .panes
            .iter()
            .filter_map(|(&id, p)| p.as_terminal()?.name.clone().map(|n| (id, n)))
            .collect();
        let root = self
            .router
            .iter()
            .find(|c| c.context_id == self.windows[window].context_id)
            .map(|c| c.root.clone())
            .unwrap_or_default();
        let pane = self.windows[window].panes.get_mut(&parent);
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt(("conjoined-parent", parent))
                .max_rect(left),
            |ui| {
                ui.set_clip_rect(left);
                if let Some(pane) = pane {
                    if let Some(terminal) = pane.as_terminal_mut() {
                        crate::render::terminal_pane::render(
                            ui,
                            terminal,
                            tile,
                            &parent,
                            focused,
                            &self.theme,
                            &self.colors,
                            &names,
                            &HashMap::new(),
                            &HashMap::new(),
                            &HashMap::new(),
                            Some(&root),
                            self.config.pane_title_font_size.unwrap_or(12.0),
                            pane_input,
                        );
                    } else if let Some(app) = pane.as_app_mut() {
                        crate::render::app_pane::render(ui, app, &self.colors, false, None, &root);
                    }
                }
            },
        );
        if focused {
            ui.painter().rect_stroke(
                left,
                0.0,
                egui::Stroke::new(gap.max(1.0), self.colors.accent),
                egui::StrokeKind::Inside,
            );
        }
        Some(right)
    }
}
