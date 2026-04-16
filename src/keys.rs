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
// Cmd+Shift+N           — notification palette
// Cmd+Up / Cmd+Down     — scroll
// Cmd+= / Cmd+-         — font size
// Cmd+E                 — file browser
// Cmd+Shift+E           — depth tree
// Cmd+Escape            — ascend depth (Z-axis)
// Cmd+0                 — quick note
// Cmd+1–9               — switch context
// Ctrl+/                — toggle agent mode
// Cmd+Z                 — undo (app active)
// Cmd+Shift+Z           — redo (app active)
// Escape (app active)   — close app
// Tab (app active)      — navigate to linked terminal
//
// Apps should use Cmd+S, Cmd+Shift+<key>, Ctrl+<key>, or unmodified keys.
// Always guard with `!input.modifiers.command` before consuming Enter, H, J,
// K, L, Backspace, or other keys that Plexi uses with Cmd modifier.
//
// GOTCHA: consume_key(Modifiers::NONE, Key) does NOT mean "key with no
// modifiers" — it matches the key regardless of modifiers. To distinguish
// plain Enter from Shift+Enter, check `input.modifiers.shift` BEFORE
// calling consume_key.
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug)]
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
    /// Open the depth tree browser in the focused terminal.
    OpenDepthTree,
    /// Open the quick note app (full pane, no terminal split).
    OpenQuickNote,
    /// Open the audio player app.
    OpenAudioPlayer,
    /// Open config file in the text editor.
    OpenConfig,
    /// Open the secrets manager (read-only vault viewer).
    OpenSecretsManager,
    /// Toggle agent mode on the focused pane.
    ToggleAgentMode,
    /// Undo last action in the focused app.
    AppUndo,
    /// Redo last undone action in the focused app.
    AppRedo,
    /// Open the notification palette (shows unread notifications).
    ToggleNotificationPalette,
    /// Open the run palette (shows active and recent runs).
    ToggleRunPalette,
    /// Ascend one depth level in the Z-axis (Cmd+Escape).
    AscendDepth,
    /// Toggle lock on the focused pane (prevents Cmd+W close).
    LockPane,
}

/// Guard condition for a key binding.
#[derive(Clone, Copy)]
enum Guard {
    /// Always active.
    Always,
    /// Only fires when an app surface is open in the focused pane.
    AppActive,
    /// Only fires when no app surface is open in the focused pane.
    AppInactive,
}

struct Binding {
    key: egui::Key,
    modifiers: egui::Modifiers,
    guard: Guard,
    action: fn() -> Action,
}

/// Count how many modifier flags are set, for specificity sorting.
#[inline]
fn modifier_count(m: egui::Modifiers) -> u32 {
    (m.alt as u32) + (m.ctrl as u32) + (m.shift as u32) + (m.mac_cmd as u32)
}

/// Poll global keyboard actions. `app_active` indicates whether the focused
/// pane currently has an app surface open (affects Escape and Tab handling).
pub fn poll_actions(ctx: &egui::Context, app_active: bool) -> Vec<Action> {
    // Modifier shorthands
    let cmd = egui::Modifiers::COMMAND;
    let cmd_shift = egui::Modifiers {
        shift: true,
        ..egui::Modifiers::COMMAND
    };
    let ctrl = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let none = egui::Modifiers::NONE;

    // Binding table. Bindings with the same key are checked in descending
    // modifier-count order (more-specific first), so Cmd+Shift+X is consumed
    // before Cmd+X and shadowing is impossible.
    //
    // Cmd+W appears twice: CloseApp (AppActive) and ClosePane (AppInactive).
    // They share the key but have mutually exclusive guards, so exactly one
    // fires per frame.
    let mut bindings: Vec<Binding> = vec![
        // Split
        Binding { key: egui::Key::D, modifiers: cmd_shift, guard: Guard::Always, action: || Action::SplitVertical },
        Binding { key: egui::Key::D, modifiers: cmd, guard: Guard::Always, action: || Action::SplitHorizontal },
        // Pane management
        Binding { key: egui::Key::W, modifiers: cmd, guard: Guard::AppActive, action: || Action::CloseApp },
        Binding { key: egui::Key::W, modifiers: cmd, guard: Guard::AppInactive, action: || Action::ClosePane },
        Binding { key: egui::Key::L, modifiers: cmd_shift, guard: Guard::Always, action: || Action::LockPane },
        // Navigation
        Binding { key: egui::Key::H, modifiers: cmd, guard: Guard::Always, action: || Action::Navigate(Direction::Left) },
        Binding { key: egui::Key::J, modifiers: cmd, guard: Guard::Always, action: || Action::Navigate(Direction::Down) },
        Binding { key: egui::Key::K, modifiers: cmd, guard: Guard::Always, action: || Action::Navigate(Direction::Up) },
        Binding { key: egui::Key::L, modifiers: cmd, guard: Guard::Always, action: || Action::Navigate(Direction::Right) },
        // Tabs
        Binding { key: egui::Key::T, modifiers: cmd, guard: Guard::Always, action: || Action::NewTab },
        Binding { key: egui::Key::CloseBracket, modifiers: cmd, guard: Guard::Always, action: || Action::NextTab },
        Binding { key: egui::Key::OpenBracket, modifiers: cmd, guard: Guard::Always, action: || Action::PrevTab },
        // App-level
        Binding { key: egui::Key::Q, modifiers: cmd, guard: Guard::Always, action: || Action::Quit },
        Binding { key: egui::Key::B, modifiers: cmd, guard: Guard::Always, action: || Action::ToggleSidebar },
        Binding { key: egui::Key::Enter, modifiers: cmd, guard: Guard::Always, action: || Action::ToggleZoom },
        Binding { key: egui::Key::Escape, modifiers: cmd, guard: Guard::Always, action: || Action::AscendDepth },
        Binding { key: egui::Key::Slash, modifiers: cmd, guard: Guard::Always, action: || Action::ToggleShortcuts },
        Binding { key: egui::Key::P, modifiers: cmd, guard: Guard::Always, action: || Action::ToggleCommandPalette },
        Binding { key: egui::Key::R, modifiers: cmd_shift, guard: Guard::Always, action: || Action::RenamePane },
        Binding { key: egui::Key::N, modifiers: cmd_shift, guard: Guard::Always, action: || Action::ToggleNotificationPalette },
        Binding { key: egui::Key::U, modifiers: cmd_shift, guard: Guard::Always, action: || Action::ToggleRunPalette },
        Binding { key: egui::Key::N, modifiers: cmd, guard: Guard::Always, action: || Action::NewContext },
        // Scroll
        Binding { key: egui::Key::ArrowUp, modifiers: cmd, guard: Guard::Always, action: || Action::ScrollUp },
        Binding { key: egui::Key::ArrowDown, modifiers: cmd, guard: Guard::Always, action: || Action::ScrollDown },
        // Font size
        Binding { key: egui::Key::Equals, modifiers: cmd, guard: Guard::Always, action: || Action::IncreasePaneFontSize },
        Binding { key: egui::Key::Minus, modifiers: cmd, guard: Guard::Always, action: || Action::DecreasePaneFontSize },
        // App surface focus toggle (Tab, no modifiers, only when app open)
        Binding { key: egui::Key::Tab, modifiers: none, guard: Guard::AppActive, action: || Action::ToggleAppFocus },
        // File/tree browsers
        Binding { key: egui::Key::E, modifiers: cmd_shift, guard: Guard::Always, action: || Action::OpenDepthTree },
        Binding { key: egui::Key::E, modifiers: cmd, guard: Guard::Always, action: || Action::OpenFileBrowser },
        // Other apps
        Binding { key: egui::Key::Num0, modifiers: cmd, guard: Guard::Always, action: || Action::OpenQuickNote },
        Binding { key: egui::Key::A, modifiers: cmd_shift, guard: Guard::Always, action: || Action::OpenAudioPlayer },
        Binding { key: egui::Key::Comma, modifiers: cmd, guard: Guard::Always, action: || Action::OpenConfig },
        Binding { key: egui::Key::S, modifiers: cmd_shift, guard: Guard::Always, action: || Action::OpenSecretsManager },
        // Agent mode (Ctrl+/)
        Binding { key: egui::Key::Slash, modifiers: ctrl, guard: Guard::Always, action: || Action::ToggleAgentMode },
        // Undo / Redo in focused app (only when app active)
        Binding { key: egui::Key::Z, modifiers: cmd_shift, guard: Guard::AppActive, action: || Action::AppRedo },
        Binding { key: egui::Key::Z, modifiers: cmd, guard: Guard::AppActive, action: || Action::AppUndo },
    ];

    // Sort more-specific bindings first (descending modifier count).
    // Stable sort preserves declaration order for ties.
    bindings.sort_by(|a, b| modifier_count(b.modifiers).cmp(&modifier_count(a.modifiers)));

    let mut actions = Vec::new();

    ctx.input_mut(|input| {
        for binding in &bindings {
            // Guard check
            let guard_ok = match binding.guard {
                Guard::Always => true,
                Guard::AppActive => app_active,
                Guard::AppInactive => !app_active,
            };
            if !guard_ok {
                continue;
            }

            // Specificity sort guarantees more-specific bindings (e.g.
            // Cmd+Shift+L) are checked before less-specific ones (Cmd+L).
            // consume_key requires *at least* the declared modifiers, so
            // Cmd+Shift+L won't match when only Cmd is held, and once it
            // consumes the event, Cmd+L won't see it.
            if input.consume_key(binding.modifiers, binding.key) {
                actions.push((binding.action)());
            }
        }

        // Switch context (Cmd+1 through Cmd+9).
        // Handled outside the table because the action carries a parameter.
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

        // Ctrl+Tab — ergonomic alias for ToggleAgentMode.
        let ctrl_only = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        if input.consume_key(ctrl_only, egui::Key::Tab) {
            actions.push(Action::ToggleAgentMode);
        }
    });

    actions
}
