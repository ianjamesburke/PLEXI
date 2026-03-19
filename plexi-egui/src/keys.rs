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
            log::info!("Key: Cmd+Shift+D → SplitVertical");
            actions.push(Action::SplitVertical);
        } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::D) {
            log::info!("Key: Cmd+D → SplitHorizontal");
            actions.push(Action::SplitHorizontal);
        }

        // Close pane (Ctrl+W on Linux; on macOS, Cmd+W goes through close_requested)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::W) {
            log::info!("Key: Cmd+W → ClosePane");
            actions.push(Action::ClosePane);
        }

        // Focus navigation
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
    });

    actions
}
