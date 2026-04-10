// ─── Reserved Plexi shortcuts (apps must NOT consume these) ───────────────
//
// Cmd+D / Cmd+Shift+D  — split horizontal / vertical
// Cmd+W                 — close pane
// Cmd+H/J/K/L          — navigate panes
// Cmd+T                 — new tab
// Cmd+] / Cmd+[         — cycle tabs
// Cmd+Q                 — quit
// Cmd+B                 — toggle sidebar
// Cmd+Enter             — toggle zoom
// Cmd+/                 — toggle shortcuts overlay
// Cmd+P                 — command palette
// Cmd+Shift+R           — rename pane
// Cmd+N                 — new context
// Cmd+Up / Cmd+Down     — scroll
// Cmd+= / Cmd+-         — font size
// Cmd+E                 — file browser
// Cmd+0                 — quick note
// Cmd+1–9               — switch context
// Escape (app active)   — close app
// Tab (app active)      — navigate to linked terminal
//
// Apps should use Cmd+S, Cmd+Shift+<key>, Ctrl+<key>, or unmodified keys.
// Always guard with `!input.modifiers.command` before consuming Enter, H, J,
// K, L, Backspace, or other keys that Plexi uses with Cmd modifier.
// ───────────────────────────────────────────────────────────────────────────

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
    SwitchContext(usize),
    NextTab,
    PrevTab,
    Quit,
    ToggleSidebar,
    ToggleShortcuts,
    ToggleZoom,
    ToggleCommandPalette,
    RenamePane,
    NewContext,
    IncreasePaneFontSize,
    DecreasePaneFontSize,
    ScrollUp,
    ScrollDown,
    /// Dismiss the active app surface and return to full terminal.
    CloseApp,
    /// Toggle keyboard focus between app surface and terminal command bar.
    ToggleAppFocus,
    /// Open the file browser app in the focused terminal.
    OpenFileBrowser,
    /// Open the quick note app (full pane, no terminal split).
    OpenQuickNote,
    /// Open the audio player app.
    OpenAudioPlayer,
}

/// Poll global keyboard actions. `app_active` indicates whether the focused
/// pane currently has an app surface open (affects Escape and Tab handling).
pub fn poll_actions(ctx: &egui::Context, app_active: bool) -> Vec<Action> {
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

        // Toggle zoom (Cmd+Enter)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter) {
            actions.push(Action::ToggleZoom);
        }

        // Toggle shortcuts overlay (Cmd+/)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Slash) {
            actions.push(Action::ToggleShortcuts);
        }

        // Command palette (Cmd+P)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::P) {
            actions.push(Action::ToggleCommandPalette);
        }

        // Rename pane (Cmd+Shift+R)
        if input.consume_key(cmd_shift, egui::Key::R) {
            actions.push(Action::RenamePane);
        }

        // New context (Cmd+N)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::N) {
            actions.push(Action::NewContext);
        }

        // Scrollback (Cmd+Up / Cmd+Down)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowUp) {
            actions.push(Action::ScrollUp);
        }
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::ArrowDown) {
            actions.push(Action::ScrollDown);
        }

        // Per-pane font size (Cmd+= / Cmd+-)
        // Use exact modifier checks to avoid matching Cmd+Shift variants.
        let cmd_only = egui::Modifiers::COMMAND;
        if !input.modifiers.shift && input.consume_key(cmd_only, egui::Key::Equals) {
            actions.push(Action::IncreasePaneFontSize);
        }
        if !input.modifiers.shift && input.consume_key(cmd_only, egui::Key::Minus) {
            actions.push(Action::DecreasePaneFontSize);
        }

        // App surface: Escape closes app, Tab toggles terminal split.
        // These are only intercepted at the global level when an app is active,
        // so that Escape and Tab work normally in a plain terminal.
        if app_active {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                actions.push(Action::CloseApp);
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                actions.push(Action::ToggleAppFocus);
            }
        }

        // Open file browser (Cmd+E)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::E) {
            actions.push(Action::OpenFileBrowser);
        }

        // Open quick note (Cmd+0)
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0) {
            actions.push(Action::OpenQuickNote);
        }

        // Open audio player (Cmd+Shift+A)
        if input.consume_key(cmd_shift, egui::Key::A) {
            actions.push(Action::OpenAudioPlayer);
        }

        // Switch context (Cmd+1 through Cmd+9)
        let num_keys = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        for (i, key) in num_keys.into_iter().enumerate() {
            if input.consume_key(egui::Modifiers::COMMAND, key) {
                actions.push(Action::SwitchContext(i));
            }
        }
    });

    actions
}
