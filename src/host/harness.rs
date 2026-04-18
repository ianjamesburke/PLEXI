use crate::host::command::{HostCommand, OpenPaneRequest, PaneRuntimeKind, Placement, ShareRatio};
use crate::host::effect::HostEffect;
use crate::host::model::HostModel;
use crate::host::services::HostServices;

pub struct HostHarness {
    pub model: HostModel,
    pub services: HostServices,
    pub effects: Vec<HostEffect>,
}

impl HostHarness {
    pub fn new() -> Self {
        Self {
            model: HostModel::new(),
            services: HostServices::new(),
            effects: Vec::new(),
        }
    }

    pub fn press(&mut self, chord: &str) {
        let command = match chord {
            "cmd+w" => Some(HostCommand::CloseFocusedPane),
            "tab" => Some(HostCommand::FocusNext),
            "shift+tab" => Some(HostCommand::FocusPrev),
            _ => None,
        };
        if let Some(command) = command {
            self.effects
                .extend(self.model.handle_command(command, &mut self.services));
        }
    }

    pub fn launch_fake_app(&mut self, app_id: &str) {
        let request = OpenPaneRequest {
            runtime: PaneRuntimeKind::App {
                app_id: app_id.to_string(),
            },
            placement: Placement::Right,
            share: ShareRatio::new(3.0, 1.0).expect("positive ratio"),
        };
        self.effects.extend(
            self.model
                .handle_command(HostCommand::OpenPane(request), &mut self.services),
        );
    }

    pub fn assert_pane_count(&self, expected: usize) {
        assert_eq!(self.model.pane_count(), expected);
    }

    pub fn assert_focus_kind(&self, expected: PaneRuntimeKind) {
        assert_eq!(self.model.focused_pane().map(|p| p.kind.clone()), Some(expected));
    }
}

#[cfg(test)]
mod tests {
    use crate::host::command::PaneRuntimeKind;
    use crate::host::harness::HostHarness;

    #[test]
    fn harness_supports_keyboard_and_launch_flow() {
        let mut h = HostHarness::new();
        h.launch_fake_app("todo");
        h.assert_pane_count(2);
        h.assert_focus_kind(PaneRuntimeKind::App {
            app_id: "todo".to_string(),
        });

        h.press("cmd+w");
        h.assert_pane_count(1);
        h.assert_focus_kind(PaneRuntimeKind::Terminal);
    }
}
