use crate::host::command::{HostCommand, OpenPaneRequest, PaneRuntimeKind};
use crate::host::effect::HostEffect;
use crate::host::services::HostServices;
use crate::tiling::PaneId;

#[derive(Debug, Clone)]
pub struct HostPane {
    pub id: PaneId,
    pub kind: PaneRuntimeKind,
}

#[derive(Debug, Clone, Default)]
pub struct HostContext {
    pub panes: Vec<HostPane>,
    pub focused_pane: Option<PaneId>,
}

#[derive(Debug, Default)]
pub struct HostModel {
    contexts: Vec<HostContext>,
    active_context: usize,
    next_pane_id: PaneId,
}

impl HostModel {
    pub fn new() -> Self {
        let mut model = Self {
            contexts: vec![HostContext::default()],
            active_context: 0,
            next_pane_id: 1,
        };
        let initial = HostPane {
            id: 0,
            kind: PaneRuntimeKind::Terminal,
        };
        model.context_mut().panes.push(initial);
        model.context_mut().focused_pane = Some(0);
        model
    }

    pub fn active_context(&self) -> usize {
        self.active_context
    }

    pub fn pane_count(&self) -> usize {
        self.context().panes.len()
    }

    pub fn focused_pane(&self) -> Option<&HostPane> {
        let focused = self.context().focused_pane?;
        self.context().panes.iter().find(|p| p.id == focused)
    }

    pub fn handle_command(
        &mut self,
        command: HostCommand,
        _services: &mut HostServices,
    ) -> Vec<HostEffect> {
        match command {
            HostCommand::OpenPane(req) => self.open_pane(req),
            HostCommand::CloseFocusedPane => self.close_focused_pane(),
            HostCommand::FocusPane(pane_id) => self.focus_pane(pane_id),
            HostCommand::FocusNext => self.focus_next(),
            HostCommand::FocusPrev => self.focus_prev(),
        }
    }

    fn open_pane(&mut self, req: OpenPaneRequest) -> Vec<HostEffect> {
        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        self.context_mut().panes.push(HostPane {
            id: new_id,
            kind: req.runtime.clone(),
        });
        self.context_mut().focused_pane = Some(new_id);
        vec![
            HostEffect::PaneOpened {
                pane_id: new_id,
                kind: req.runtime,
            },
            HostEffect::FocusChanged {
                pane_id: Some(new_id),
            },
        ]
    }

    fn close_focused_pane(&mut self) -> Vec<HostEffect> {
        let Some(focused) = self.context().focused_pane else {
            return Vec::new();
        };
        let idx = self.context().panes.iter().position(|p| p.id == focused);
        let Some(idx) = idx else {
            return Vec::new();
        };
        self.context_mut().panes.remove(idx);
        let new_focus = self
            .context()
            .panes
            .get(idx.saturating_sub(1))
            .map(|p| p.id);
        self.context_mut().focused_pane = new_focus;
        vec![
            HostEffect::PaneClosed { pane_id: focused },
            HostEffect::FocusChanged { pane_id: new_focus },
        ]
    }

    fn focus_pane(&mut self, pane_id: PaneId) -> Vec<HostEffect> {
        if self.context().panes.iter().any(|p| p.id == pane_id) {
            self.context_mut().focused_pane = Some(pane_id);
            vec![HostEffect::FocusChanged {
                pane_id: Some(pane_id),
            }]
        } else {
            Vec::new()
        }
    }

    fn focus_next(&mut self) -> Vec<HostEffect> {
        let panes = &self.context().panes;
        if panes.is_empty() {
            return Vec::new();
        }
        let cur = self.context().focused_pane.unwrap_or(panes[0].id);
        let idx = panes.iter().position(|p| p.id == cur).unwrap_or(0);
        let next = panes[(idx + 1) % panes.len()].id;
        self.context_mut().focused_pane = Some(next);
        vec![HostEffect::FocusChanged {
            pane_id: Some(next),
        }]
    }

    fn focus_prev(&mut self) -> Vec<HostEffect> {
        let panes = &self.context().panes;
        if panes.is_empty() {
            return Vec::new();
        }
        let cur = self.context().focused_pane.unwrap_or(panes[0].id);
        let idx = panes.iter().position(|p| p.id == cur).unwrap_or(0);
        let prev = panes[(idx + panes.len() - 1) % panes.len()].id;
        self.context_mut().focused_pane = Some(prev);
        vec![HostEffect::FocusChanged {
            pane_id: Some(prev),
        }]
    }

    fn context(&self) -> &HostContext {
        &self.contexts[self.active_context]
    }

    fn context_mut(&mut self) -> &mut HostContext {
        &mut self.contexts[self.active_context]
    }
}

#[cfg(test)]
mod tests {
    use crate::host::command::{
        HostCommand, OpenPaneRequest, PaneRuntimeKind, Placement, ShareRatio,
    };
    use crate::host::model::HostModel;
    use crate::host::services::HostServices;

    #[test]
    fn host_model_constructs_without_egui() {
        let model = HostModel::new();
        assert_eq!(model.pane_count(), 1);
        assert_eq!(
            model.focused_pane().map(|p| p.kind.clone()),
            Some(PaneRuntimeKind::Terminal)
        );
    }

    #[test]
    fn host_model_open_focus_close_cycle() {
        let mut model = HostModel::new();
        let mut services = HostServices::new();

        let req = OpenPaneRequest {
            runtime: PaneRuntimeKind::App {
                app_id: "todo".to_string(),
            },
            placement: Placement::Right,
            share: ShareRatio::new(3.0, 1.0).expect("ratio"),
        };
        let open_fx = model.handle_command(HostCommand::OpenPane(req), &mut services);
        assert_eq!(open_fx.len(), 2);
        assert_eq!(model.pane_count(), 2);
        assert_eq!(
            model.focused_pane().map(|p| p.kind.clone()),
            Some(PaneRuntimeKind::App {
                app_id: "todo".to_string()
            })
        );

        let close_fx = model.handle_command(HostCommand::CloseFocusedPane, &mut services);
        assert_eq!(close_fx.len(), 2);
        assert_eq!(model.pane_count(), 1);
        assert_eq!(
            model.focused_pane().map(|p| p.kind.clone()),
            Some(PaneRuntimeKind::Terminal)
        );
    }
}
