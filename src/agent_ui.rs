use crate::agent_mode::{AgentMode, AgentModeState, MessageRole};
use crate::theme::Colors;

/// Render the agent mode UI into the given Ui region.
/// Returns true if the agent mode wants to deactivate (user pressed Escape).
pub fn render_agent_mode(
    ui: &mut egui::Ui,
    agent: &mut AgentMode,
    colors: &Colors,
) -> bool {
    let mut wants_deactivate = false;

    // Consume Escape to deactivate
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            wants_deactivate = true;
        }
    });

    if wants_deactivate {
        return true;
    }

    // Drain any LLM responses that arrived since the last frame. If something
    // changed, make sure we render another frame promptly. Also keep polling
    // while a request is in flight so we pick up the reply as soon as the
    // worker thread produces it.
    let changed = agent.poll_llm();
    if changed || agent.state == crate::agent_mode::AgentModeState::Processing {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }

    let available = ui.available_rect_before_wrap();

    // Background fill
    ui.painter()
        .rect_filled(available, 0.0, colors.terminal_bg);

    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(available), |ui| {
        ui.set_min_size(available.size());
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 4.0);

        egui::Frame::new()
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                // Header bar: directory scope + state indicator
                draw_status_bar(ui, agent, colors);

                ui.add_space(8.0);

                // Conversation history
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(available.height() - 120.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        draw_conversation(ui, agent, colors);
                    });

                ui.add_space(8.0);

                // Input area (only when waiting for input)
                if agent.state == AgentModeState::WaitingForInput {
                    draw_input(ui, agent, colors);
                } else if agent.state == AgentModeState::Processing {
                    ui.horizontal(|ui| {
                        ui.colored_label(colors.text_dim, "Thinking...");
                    });
                }
            });
    });

    false
}

fn draw_status_bar(ui: &mut egui::Ui, agent: &AgentMode, colors: &Colors) {
    ui.horizontal(|ui| {
        // Accent-colored agent indicator
        ui.colored_label(colors.accent, "AGENT");
        ui.add_space(8.0);

        // Directory scope
        let scope_str = crate::app::PlexiApp::abbreviate_home_path(&agent.directory_scope);
        ui.colored_label(colors.text_dim, &scope_str);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let state_label = match agent.state {
                AgentModeState::Inactive => "inactive",
                AgentModeState::WaitingForInput => "ready",
                AgentModeState::Processing => "thinking",
                AgentModeState::Responding => "responding",
            };
            ui.colored_label(colors.text_dim, state_label);
        });
    });

    // Separator
    let rect = ui.available_rect_before_wrap();
    let y = rect.top();
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), y),
            egui::pos2(rect.right(), y),
        ],
        egui::Stroke::new(1.0, colors.border),
    );
    ui.add_space(2.0);
}

fn draw_conversation(ui: &mut egui::Ui, agent: &AgentMode, colors: &Colors) {
    if agent.conversation.is_empty() {
        ui.colored_label(
            colors.text_dim,
            "Type a message to start. Press Escape to return to shell.",
        );
        return;
    }

    for msg in &agent.conversation {
        let (prefix, color) = match msg.role {
            MessageRole::User => ("> ", colors.accent),
            MessageRole::Agent => ("  ", colors.text_primary),
            MessageRole::System => ("# ", colors.text_dim),
        };

        ui.horizontal_wrapped(|ui| {
            ui.colored_label(color, prefix);
            ui.colored_label(color, &msg.content);
        });
        ui.add_space(4.0);
    }
}

fn draw_input(ui: &mut egui::Ui, agent: &mut AgentMode, colors: &Colors) {
    ui.horizontal(|ui| {
        ui.colored_label(colors.accent, "> ");

        let response = ui.add(
            egui::TextEdit::singleline(&mut agent.input_buffer)
                .desired_width(ui.available_width() - 20.0)
                .text_color(colors.text_primary)
                .frame(false)
                .hint_text("Ask the agent..."),
        );

        // Auto-focus the input field
        if !response.has_focus() {
            response.request_focus();
        }

        // Submit on Enter — agent.submit() now dispatches to the LLM worker
        // thread. The response lands via agent.poll_llm() on a later frame.
        if response.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            let _ = agent.submit();
        }
    });
}
