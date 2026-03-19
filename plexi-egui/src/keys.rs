#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

pub enum Action {
    SplitHorizontal,
    SplitVertical,
    Navigate(Direction),
    ClosePane,
    NewTab,
    NextTab,
    PrevTab,
    Quit,
    ToggleSidebar,
    ToggleShortcuts,
}

pub fn poll_actions(ctx: &egui::Context) -> Vec<Action> {
    let mut actions = Vec::new();
    let cmd_shift = egui::Modifiers {
        shift: true,
        ..egui::Modifiers::COMMAND
    };

    ctx.input_mut(|input| {
        // Check Cmd+Shift+D before Cmd+D (more specific first)
        if input.consume_key(cmd_shift, egui::Key::D) {
            actions.push(Action::SplitVertical);
        } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::D) {
            actions.push(Action::SplitHorizontal);
        }

        // Close pane (Ctrl+W on Linux; on macOS, Cmd+W goes through close_requested)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::W) {
            actions.push(Action::ClosePane);
        }

        // Focus navigation (Cmd+HJKL)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::H) {
            actions.push(Action::Navigate(Direction::Left));
        }
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::J) {
            actions.push(Action::Navigate(Direction::Down));
        }
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::K) {
            actions.push(Action::Navigate(Direction::Up));
        }
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::L) {
            actions.push(Action::Navigate(Direction::Right));
        }

        // New tab (Cmd+T)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::T) {
            actions.push(Action::NewTab);
        }

        // Cycle tabs (Cmd+] / Cmd+[)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::CloseBracket) {
            actions.push(Action::NextTab);
        }
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::OpenBracket) {
            actions.push(Action::PrevTab);
        }

        // Quit (Cmd+Q)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Q) {
            actions.push(Action::Quit);
        }

        // Toggle sidebar (Cmd+B)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::B) {
            actions.push(Action::ToggleSidebar);
        }

        // Toggle shortcuts overlay (Cmd+/)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Slash) {
            actions.push(Action::ToggleShortcuts);
        }
    });

    actions
}
