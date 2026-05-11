use crate::config::KeybindingsConfig;

// ─── Reserved Plexi shortcuts (apps must NOT consume these) ───────────────
//
// Cmd+D / Cmd+Shift+D       — split horizontal / vertical (terminal)
// Cmd+\                       — split focused pane to the right (mirror type)
// Cmd+Shift+\                 — split focused pane below (mirror type)
// Cmd+N                       — new window to the right on the current grid row
// Cmd+Shift+N                 — new context (sidebar item)
// Cmd+W                       — close pane
// Cmd+H/J/K/L                 — navigate panes (falls through to adjacent window at boundary)
// Cmd+Ctrl+H/J/K/L            — swap focused pane with neighbor in direction
// Cmd+Shift+M                 — toggle minimap overlay
// Cmd+T                       — new tab
// Cmd+Shift+L/H               — next/prev tab
// Cmd+Shift+K/J               — first/last tab
// Cmd+Q                       — quit
// Cmd+B                       — toggle sidebar
// Cmd+Enter                   — toggle zoom
// Cmd+/                       — toggle shortcuts overlay
// Cmd+P                       — command palette
// Cmd+R                       — rename pane
// Cmd+Shift+R                 — rename context
// Cmd+[                       — nav back / focus history back
// Cmd+]                       — focus history forward
// Cmd+Up / Cmd+Down           — scroll
// Cmd+= / Cmd+-               — font size
// Cmd+E                       — file browser
// Cmd+0                       — quick note
// Cmd+1–9                     — switch context (sidebar)
// Escape (app active)         — close app
// Tab (app active)            — navigate to linked terminal
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

pub enum Action {
    SplitHorizontal,
    SplitVertical,
    /// Split the focused pane to the right; new pane mirrors the focused pane's type.
    /// Bound to Cmd+\. If no pane is focused, creates a full-size terminal.
    SplitRight,
    /// Split the focused pane below; new pane mirrors the focused pane's type.
    /// Bound to Cmd+Shift+\. If no pane is focused, creates a full-size terminal.
    SplitDown,
    Navigate(Direction),
    ClosePane,
    NewTab,
    SwitchContext(usize),
    NextTab,
    PrevTab,
    FirstTab,
    LastTab,
    Quit,
    ToggleSidebar,
    ToggleShortcuts,
    ToggleZoom,
    ToggleCommandPalette,
    RenamePane,
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
    /// Open config file in the text editor.
    OpenConfig,
    /// Open the secrets manager (read-only vault viewer).
    OpenSecretsManager,
    /// Rename the active context. Bound to Cmd+Shift+R.
    RenameContext,
    /// Toggle the notification panel overlay (Cmd+Shift+A).
    ToggleNotificationModal,
    NotificationCycleNext,
    NotificationCyclePrev,
    /// Force-reload the focused app pane (#83). Bound to Cmd+Option+R.
    /// Cmd+R is RenamePane and Cmd+Shift+R is RenameContext, so the
    /// Option (Alt) modifier is the next free chord. No-op when the
    /// focused pane isn't a process-backed app.
    ForceReloadApp,
    /// Create a new window to the right of the active one on the same
    /// grid row. Bound to Cmd+N.
    NewPageRight,
    /// Create a new context (sidebar item) and immediately open the rename modal.
    /// Bound to Cmd+Shift+N.
    NewContext,
    /// Toggle the minimap overlay. Bound to Cmd+Shift+M.
    ToggleMinimap,
    /// Reload configuration from disk. Bound to Cmd+Shift+,.
    ReloadConfig,
    /// Cmd+[ when a nav-active app pane is focused: pop one nav level and
    /// emit `PlexiEvent::NavBack`. Falls through to cycling tabs backwards
    /// if no nav is active on the focused pane.
    NavBackApp,
    /// Step forward through pane focus history. Bound to Cmd+].
    FocusHistoryForward,
    /// Swap the focused pane with its neighbor in the given direction.
    /// Bound to Cmd+Ctrl+H/J/K/L.
    SwapPane(Direction),
}

/// Resolved keybindings — one `(Modifiers, Key)` pair per named action.
/// Built once at startup from defaults + config overrides via `build_key_bindings`.
#[derive(Clone)]
pub struct KeyBindings {
    pub quit: (egui::Modifiers, egui::Key),
    pub close_pane: (egui::Modifiers, egui::Key),
    pub toggle_command_palette: (egui::Modifiers, egui::Key),
    pub split_horizontal: (egui::Modifiers, egui::Key),
    pub split_vertical: (egui::Modifiers, egui::Key),
    pub split_right: (egui::Modifiers, egui::Key),
    pub split_down: (egui::Modifiers, egui::Key),
    pub swap_pane_left: (egui::Modifiers, egui::Key),
    pub swap_pane_down: (egui::Modifiers, egui::Key),
    pub swap_pane_up: (egui::Modifiers, egui::Key),
    pub swap_pane_right: (egui::Modifiers, egui::Key),
    pub navigate_left: (egui::Modifiers, egui::Key),
    pub navigate_down: (egui::Modifiers, egui::Key),
    pub navigate_up: (egui::Modifiers, egui::Key),
    pub navigate_right: (egui::Modifiers, egui::Key),
    pub new_tab: (egui::Modifiers, egui::Key),
    pub next_tab: (egui::Modifiers, egui::Key),
    pub prev_tab: (egui::Modifiers, egui::Key),
    pub first_tab: (egui::Modifiers, egui::Key),
    pub last_tab: (egui::Modifiers, egui::Key),
    pub nav_back: (egui::Modifiers, egui::Key),
    pub focus_history_forward: (egui::Modifiers, egui::Key),
    pub toggle_sidebar: (egui::Modifiers, egui::Key),
    pub toggle_zoom: (egui::Modifiers, egui::Key),
    pub toggle_shortcuts: (egui::Modifiers, egui::Key),
    pub rename_context: (egui::Modifiers, egui::Key),
    pub rename_pane: (egui::Modifiers, egui::Key),
    pub new_context: (egui::Modifiers, egui::Key),
    pub new_page_right: (egui::Modifiers, egui::Key),
    pub toggle_minimap: (egui::Modifiers, egui::Key),
    pub scroll_up: (egui::Modifiers, egui::Key),
    pub scroll_down: (egui::Modifiers, egui::Key),
    pub increase_font_size: (egui::Modifiers, egui::Key),
    pub decrease_font_size: (egui::Modifiers, egui::Key),
    pub open_file_browser: (egui::Modifiers, egui::Key),
    pub open_quick_note: (egui::Modifiers, egui::Key),
    pub open_config: (egui::Modifiers, egui::Key),
    pub reload_config: (egui::Modifiers, egui::Key),
    pub open_secrets_manager: (egui::Modifiers, egui::Key),
    pub force_reload_app: (egui::Modifiers, egui::Key),
    pub toggle_notification_modal: (egui::Modifiers, egui::Key),
}

fn cmd() -> egui::Modifiers { egui::Modifiers::COMMAND }
fn cmd_shift() -> egui::Modifiers {
    egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND }
}
fn cmd_ctrl() -> egui::Modifiers {
    egui::Modifiers { ctrl: true, ..egui::Modifiers::COMMAND }
}
fn cmd_alt() -> egui::Modifiers {
    egui::Modifiers { alt: true, ..egui::Modifiers::COMMAND }
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            quit:                      (cmd(),       egui::Key::Q),
            close_pane:                (cmd(),       egui::Key::W),
            toggle_command_palette:    (cmd(),       egui::Key::P),
            split_horizontal:          (cmd(),       egui::Key::D),
            split_vertical:            (cmd_shift(), egui::Key::D),
            split_right:               (cmd(),       egui::Key::Backslash),
            split_down:                (cmd_shift(), egui::Key::Backslash),
            swap_pane_left:            (cmd_ctrl(),  egui::Key::H),
            swap_pane_down:            (cmd_ctrl(),  egui::Key::J),
            swap_pane_up:              (cmd_ctrl(),  egui::Key::K),
            swap_pane_right:           (cmd_ctrl(),  egui::Key::L),
            navigate_left:             (cmd(),       egui::Key::H),
            navigate_down:             (cmd(),       egui::Key::J),
            navigate_up:               (cmd(),       egui::Key::K),
            navigate_right:            (cmd(),       egui::Key::L),
            new_tab:                   (cmd(),       egui::Key::T),
            next_tab:                  (cmd_shift(), egui::Key::L),
            prev_tab:                  (cmd_shift(), egui::Key::H),
            first_tab:                 (cmd_shift(), egui::Key::K),
            last_tab:                  (cmd_shift(), egui::Key::J),
            nav_back:                  (cmd(),       egui::Key::OpenBracket),
            focus_history_forward:     (cmd(),       egui::Key::CloseBracket),
            toggle_sidebar:            (cmd(),       egui::Key::B),
            toggle_zoom:               (cmd(),       egui::Key::Enter),
            toggle_shortcuts:          (cmd(),       egui::Key::Slash),
            rename_context:            (cmd_shift(), egui::Key::R),
            rename_pane:               (cmd(),       egui::Key::R),
            new_context:               (cmd_shift(), egui::Key::N),
            new_page_right:            (cmd(),       egui::Key::N),
            toggle_minimap:            (cmd_shift(), egui::Key::M),
            scroll_up:                 (cmd(),       egui::Key::ArrowUp),
            scroll_down:               (cmd(),       egui::Key::ArrowDown),
            increase_font_size:        (cmd(),       egui::Key::Equals),
            decrease_font_size:        (cmd(),       egui::Key::Minus),
            open_file_browser:         (cmd(),       egui::Key::E),
            open_quick_note:           (cmd(),       egui::Key::Num0),
            open_config:               (cmd(),       egui::Key::Comma),
            reload_config:             (cmd_shift(), egui::Key::Comma),
            open_secrets_manager:      (cmd_shift(), egui::Key::S),
            force_reload_app:          (cmd_alt(),   egui::Key::R),
            toggle_notification_modal: (cmd_shift(), egui::Key::A),
        }
    }
}

/// Parse a key combo string like `"cmd+shift+d"` into `(Modifiers, Key)`.
/// Returns `None` and logs a warning if the string is malformed or the key is unknown.
fn parse_key_combo(s: &str) -> Option<(egui::Modifiers, egui::Key)> {
    let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
    if parts.is_empty() {
        return None;
    }
    let (modifier_parts, key_part) = parts.split_at(parts.len() - 1);
    let key_str = key_part[0].to_lowercase();

    let mut mods = egui::Modifiers::NONE;
    for m in modifier_parts {
        match m.to_lowercase().as_str() {
            "cmd" | "command" | "super" => mods.command = true,
            "shift" => mods.shift = true,
            "ctrl" | "control" => mods.ctrl = true,
            "alt" | "opt" | "option" => mods.alt = true,
            other => {
                log::warn!("keybindings: unknown modifier '{other}' in '{s}'");
                return None;
            }
        }
    }

    let key = match key_str.as_str() {
        "a" => egui::Key::A, "b" => egui::Key::B, "c" => egui::Key::C,
        "d" => egui::Key::D, "e" => egui::Key::E, "f" => egui::Key::F,
        "g" => egui::Key::G, "h" => egui::Key::H, "i" => egui::Key::I,
        "j" => egui::Key::J, "k" => egui::Key::K, "l" => egui::Key::L,
        "m" => egui::Key::M, "n" => egui::Key::N, "o" => egui::Key::O,
        "p" => egui::Key::P, "q" => egui::Key::Q, "r" => egui::Key::R,
        "s" => egui::Key::S, "t" => egui::Key::T, "u" => egui::Key::U,
        "v" => egui::Key::V, "w" => egui::Key::W, "x" => egui::Key::X,
        "y" => egui::Key::Y, "z" => egui::Key::Z,
        "0" | "num0" => egui::Key::Num0,
        "1" | "num1" => egui::Key::Num1,
        "2" | "num2" => egui::Key::Num2,
        "3" | "num3" => egui::Key::Num3,
        "4" | "num4" => egui::Key::Num4,
        "5" | "num5" => egui::Key::Num5,
        "6" | "num6" => egui::Key::Num6,
        "7" | "num7" => egui::Key::Num7,
        "8" | "num8" => egui::Key::Num8,
        "9" | "num9" => egui::Key::Num9,
        "enter" | "return" => egui::Key::Enter,
        "escape" | "esc" => egui::Key::Escape,
        "tab" => egui::Key::Tab,
        "space" => egui::Key::Space,
        "backspace" => egui::Key::Backspace,
        "delete" | "del" => egui::Key::Delete,
        "up" | "arrowup" => egui::Key::ArrowUp,
        "down" | "arrowdown" => egui::Key::ArrowDown,
        "left" | "arrowleft" => egui::Key::ArrowLeft,
        "right" | "arrowright" => egui::Key::ArrowRight,
        "[" | "open_bracket" | "openbracket" => egui::Key::OpenBracket,
        "]" | "close_bracket" | "closebracket" => egui::Key::CloseBracket,
        "\\" | "backslash" => egui::Key::Backslash,
        "/" | "slash" => egui::Key::Slash,
        "," | "comma" => egui::Key::Comma,
        "." | "period" => egui::Key::Period,
        "=" | "equals" | "plus" => egui::Key::Equals,
        "-" | "minus" => egui::Key::Minus,
        other => {
            log::warn!("keybindings: unknown key '{other}' in '{s}'");
            return None;
        }
    };

    Some((mods, key))
}

/// Opaque repr for conflict detection: modifier bits + key discriminant.
fn binding_id(mods: egui::Modifiers, key: egui::Key) -> u64 {
    let mod_bits = (mods.command as u64)
        | ((mods.shift as u64) << 1)
        | ((mods.ctrl as u64) << 2)
        | ((mods.alt as u64) << 3);
    (mod_bits << 32) | (key as u64)
}

/// Build the effective `KeyBindings` from defaults + optional config overrides.
/// Logs `warn` for unparseable overrides (preserves default) and `error` for conflicts.
pub fn build_key_bindings(overrides: Option<&KeybindingsConfig>) -> KeyBindings {
    let mut bindings = KeyBindings::default();
    let Some(cfg) = overrides else {
        return bindings;
    };

    let mut override_count: usize = 0;
    macro_rules! apply_override {
        ($field:ident, $name:expr) => {
            if let Some(ref s) = cfg.$field {
                if let Some(combo) = parse_key_combo(s) {
                    bindings.$field = combo;
                    override_count += 1;
                } else {
                    log::warn!("keybindings: invalid combo '{}' for '{}' — keeping default", s, $name);
                }
            }
        };
    }

    apply_override!(quit, "quit");
    apply_override!(close_pane, "close_pane");
    apply_override!(toggle_command_palette, "toggle_command_palette");
    apply_override!(split_horizontal, "split_horizontal");
    apply_override!(split_vertical, "split_vertical");
    apply_override!(split_right, "split_right");
    apply_override!(split_down, "split_down");
    apply_override!(swap_pane_left, "swap_pane_left");
    apply_override!(swap_pane_down, "swap_pane_down");
    apply_override!(swap_pane_up, "swap_pane_up");
    apply_override!(swap_pane_right, "swap_pane_right");
    apply_override!(navigate_left, "navigate_left");
    apply_override!(navigate_down, "navigate_down");
    apply_override!(navigate_up, "navigate_up");
    apply_override!(navigate_right, "navigate_right");
    apply_override!(new_tab, "new_tab");
    apply_override!(next_tab, "next_tab");
    apply_override!(prev_tab, "prev_tab");
    apply_override!(first_tab, "first_tab");
    apply_override!(last_tab, "last_tab");
    apply_override!(nav_back, "nav_back");
    apply_override!(focus_history_forward, "focus_history_forward");
    apply_override!(toggle_sidebar, "toggle_sidebar");
    apply_override!(toggle_zoom, "toggle_zoom");
    apply_override!(toggle_shortcuts, "toggle_shortcuts");
    apply_override!(rename_context, "rename_context");
    apply_override!(rename_pane, "rename_pane");
    apply_override!(new_context, "new_context");
    apply_override!(new_page_right, "new_page_right");
    apply_override!(toggle_minimap, "toggle_minimap");
    apply_override!(scroll_up, "scroll_up");
    apply_override!(scroll_down, "scroll_down");
    apply_override!(increase_font_size, "increase_font_size");
    apply_override!(decrease_font_size, "decrease_font_size");
    apply_override!(open_file_browser, "open_file_browser");
    apply_override!(open_quick_note, "open_quick_note");
    apply_override!(open_config, "open_config");
    apply_override!(reload_config, "reload_config");
    apply_override!(open_secrets_manager, "open_secrets_manager");
    apply_override!(force_reload_app, "force_reload_app");
    apply_override!(toggle_notification_modal, "toggle_notification_modal");

    // Conflict detection
    let named: &[(&str, (egui::Modifiers, egui::Key))] = &[
        ("quit",                      bindings.quit),
        ("close_pane",                bindings.close_pane),
        ("toggle_command_palette",    bindings.toggle_command_palette),
        ("split_horizontal",          bindings.split_horizontal),
        ("split_vertical",            bindings.split_vertical),
        ("split_right",               bindings.split_right),
        ("split_down",                bindings.split_down),
        ("swap_pane_left",            bindings.swap_pane_left),
        ("swap_pane_down",            bindings.swap_pane_down),
        ("swap_pane_up",              bindings.swap_pane_up),
        ("swap_pane_right",           bindings.swap_pane_right),
        ("navigate_left",             bindings.navigate_left),
        ("navigate_down",             bindings.navigate_down),
        ("navigate_up",               bindings.navigate_up),
        ("navigate_right",            bindings.navigate_right),
        ("new_tab",                   bindings.new_tab),
        ("next_tab",                  bindings.next_tab),
        ("prev_tab",                  bindings.prev_tab),
        ("first_tab",                 bindings.first_tab),
        ("last_tab",                  bindings.last_tab),
        ("nav_back",                  bindings.nav_back),
        ("focus_history_forward",     bindings.focus_history_forward),
        ("toggle_sidebar",            bindings.toggle_sidebar),
        ("toggle_zoom",               bindings.toggle_zoom),
        ("toggle_shortcuts",          bindings.toggle_shortcuts),
        ("rename_context",            bindings.rename_context),
        ("rename_pane",               bindings.rename_pane),
        ("new_context",               bindings.new_context),
        ("new_page_right",            bindings.new_page_right),
        ("toggle_minimap",            bindings.toggle_minimap),
        ("scroll_up",                 bindings.scroll_up),
        ("scroll_down",               bindings.scroll_down),
        ("increase_font_size",        bindings.increase_font_size),
        ("decrease_font_size",        bindings.decrease_font_size),
        ("open_file_browser",         bindings.open_file_browser),
        ("open_quick_note",           bindings.open_quick_note),
        ("open_config",               bindings.open_config),
        ("reload_config",             bindings.reload_config),
        ("open_secrets_manager",      bindings.open_secrets_manager),
        ("force_reload_app",          bindings.force_reload_app),
        ("toggle_notification_modal", bindings.toggle_notification_modal),
    ];

    let mut seen: std::collections::HashMap<u64, &str> = std::collections::HashMap::new();
    for (name, (mods, key)) in named {
        let id = binding_id(*mods, *key);
        if let Some(other) = seen.insert(id, name) {
            log::error!("keybindings: conflict — '{}' and '{}' share the same key combo", other, name);
        }
    }

    log::info!("keybindings: {} override(s) applied from config", override_count);

    bindings
}

/// Poll global keyboard actions.
///
/// `app_active` — focused pane has an active app surface (affects Escape/Tab).
/// `keyboard_capture_active` — focused app declared `keyboard_capture = true` in its manifest.
///   When true, all host shortcuts are suppressed *except* Cmd+Q (quit), Cmd+W (close pane),
///   and Cmd+P (command palette) — structural safety operations that must always work.
pub fn poll_actions(
    ctx: &egui::Context,
    bindings: &KeyBindings,
    app_active: bool,
    keyboard_capture_active: bool,
    overlay_open: bool,
    shortcuts_overlay_open: bool,
) -> Vec<Action> {
    let mut actions = Vec::new();

    ctx.input_mut(|input| {
        if input.consume_key(bindings.quit.0, bindings.quit.1) {
            actions.push(Action::Quit);
        }
        if input.consume_key(bindings.close_pane.0, bindings.close_pane.1) {
            actions.push(Action::ClosePane);
        }
        if input.consume_key(bindings.toggle_command_palette.0, bindings.toggle_command_palette.1) {
            actions.push(Action::ToggleCommandPalette);
        }

        // All remaining shortcuts are suppressed when an app has declared keyboard capture.
        if keyboard_capture_active {
            return;
        }

        // Check split_vertical (Cmd+Shift+D) before split_horizontal (Cmd+D) — more specific first.
        if input.consume_key(bindings.split_vertical.0, bindings.split_vertical.1) {
            actions.push(Action::SplitVertical);
        } else if input.consume_key(bindings.split_horizontal.0, bindings.split_horizontal.1) {
            actions.push(Action::SplitHorizontal);
        }

        // Pane swap — check before plain pane navigation.
        if input.consume_key(bindings.swap_pane_left.0, bindings.swap_pane_left.1) {
            actions.push(Action::SwapPane(Direction::Left));
        }
        if input.consume_key(bindings.swap_pane_down.0, bindings.swap_pane_down.1) {
            actions.push(Action::SwapPane(Direction::Down));
        }
        if input.consume_key(bindings.swap_pane_up.0, bindings.swap_pane_up.1) {
            actions.push(Action::SwapPane(Direction::Up));
        }
        if input.consume_key(bindings.swap_pane_right.0, bindings.swap_pane_right.1) {
            actions.push(Action::SwapPane(Direction::Right));
        }

        // Tab navigation — checked before plain Cmd+H/J/K/L pane navigation because
        // egui's consume_key uses subset modifier matching: Cmd+H matches Cmd+Shift+H,
        // so more-specific (Cmd+Shift) variants must be consumed first.
        if input.consume_key(bindings.new_tab.0, bindings.new_tab.1) {
            actions.push(Action::NewTab);
        }
        // When the notification modal is open, next/prev tab cycle the queue instead.
        if input.consume_key(bindings.next_tab.0, bindings.next_tab.1) {
            actions.push(if overlay_open {
                Action::NotificationCycleNext
            } else {
                Action::NextTab
            });
        }
        if input.consume_key(bindings.prev_tab.0, bindings.prev_tab.1) {
            actions.push(if overlay_open {
                Action::NotificationCyclePrev
            } else {
                Action::PrevTab
            });
        }
        if input.consume_key(bindings.first_tab.0, bindings.first_tab.1) {
            actions.push(Action::FirstTab);
        }
        if input.consume_key(bindings.last_tab.0, bindings.last_tab.1) {
            actions.push(Action::LastTab);
        }

        // Focus navigation — checked after Cmd+Shift tab variants above.
        if input.consume_key(bindings.navigate_left.0, bindings.navigate_left.1) {
            actions.push(Action::Navigate(Direction::Left));
        }
        if input.consume_key(bindings.navigate_down.0, bindings.navigate_down.1) {
            actions.push(Action::Navigate(Direction::Down));
        }
        if input.consume_key(bindings.navigate_up.0, bindings.navigate_up.1) {
            actions.push(Action::Navigate(Direction::Up));
        }
        if input.consume_key(bindings.navigate_right.0, bindings.navigate_right.1) {
            actions.push(Action::Navigate(Direction::Right));
        }

        if input.consume_key(bindings.nav_back.0, bindings.nav_back.1) {
            actions.push(if overlay_open {
                Action::NotificationCyclePrev
            } else {
                Action::NavBackApp
            });
        }
        if input.consume_key(bindings.focus_history_forward.0, bindings.focus_history_forward.1) {
            actions.push(if overlay_open {
                Action::NotificationCycleNext
            } else {
                Action::FocusHistoryForward
            });
        }

        if input.consume_key(bindings.toggle_sidebar.0, bindings.toggle_sidebar.1) {
            actions.push(Action::ToggleSidebar);
        }
        if input.consume_key(bindings.toggle_zoom.0, bindings.toggle_zoom.1) {
            actions.push(Action::ToggleZoom);
        }
        if input.consume_key(bindings.toggle_shortcuts.0, bindings.toggle_shortcuts.1) {
            actions.push(Action::ToggleShortcuts);
        }

        // Rename context before rename pane — check shifted variant first.
        if input.consume_key(bindings.rename_context.0, bindings.rename_context.1) {
            actions.push(Action::RenameContext);
        } else if !input.modifiers.alt && input.consume_key(bindings.rename_pane.0, bindings.rename_pane.1) {
            actions.push(Action::RenamePane);
        }

        // Split down before split right — check shifted variant first.
        if input.consume_key(bindings.split_down.0, bindings.split_down.1) {
            actions.push(Action::SplitDown);
        } else if input.consume_key(bindings.split_right.0, bindings.split_right.1) {
            actions.push(Action::SplitRight);
        }

        // New context before new page right — check shifted variant first.
        if input.consume_key(bindings.new_context.0, bindings.new_context.1) {
            actions.push(Action::NewContext);
        } else if input.consume_key(bindings.new_page_right.0, bindings.new_page_right.1) {
            actions.push(Action::NewPageRight);
        }

        if input.consume_key(bindings.toggle_minimap.0, bindings.toggle_minimap.1) {
            actions.push(Action::ToggleMinimap);
        }

        if input.consume_key(bindings.scroll_up.0, bindings.scroll_up.1) {
            actions.push(Action::ScrollUp);
        }
        if input.consume_key(bindings.scroll_down.0, bindings.scroll_down.1) {
            actions.push(Action::ScrollDown);
        }

        if input.consume_key(bindings.increase_font_size.0, bindings.increase_font_size.1) {
            actions.push(Action::IncreasePaneFontSize);
        }
        if input.consume_key(bindings.decrease_font_size.0, bindings.decrease_font_size.1) {
            actions.push(Action::DecreasePaneFontSize);
        }

        // App surface: Escape closes app, Tab toggles terminal split.
        // Only intercepted when an app is active so Escape/Tab work normally in plain terminals.
        // Escape is suppressed when the shortcuts overlay is open — the overlay owns it.
        if app_active {
            if !shortcuts_overlay_open
                && input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
            {
                actions.push(Action::CloseApp);
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                actions.push(Action::ToggleAppFocus);
            }
        }

        if input.consume_key(bindings.open_file_browser.0, bindings.open_file_browser.1) {
            actions.push(Action::OpenFileBrowser);
        }
        if input.consume_key(bindings.open_quick_note.0, bindings.open_quick_note.1) {
            actions.push(Action::OpenQuickNote);
        }
        if input.consume_key(bindings.open_config.0, bindings.open_config.1) {
            actions.push(Action::OpenConfig);
        }
        if input.consume_key(bindings.reload_config.0, bindings.reload_config.1) {
            actions.push(Action::ReloadConfig);
        }
        if input.consume_key(bindings.open_secrets_manager.0, bindings.open_secrets_manager.1) {
            actions.push(Action::OpenSecretsManager);
        }
        // Force-reload focused app. The rename_pane branch above guards with
        // `!input.modifiers.alt` so Cmd+Alt+R still reaches this branch.
        if input.consume_key(bindings.force_reload_app.0, bindings.force_reload_app.1) {
            actions.push(Action::ForceReloadApp);
        }
        if input.consume_key(bindings.toggle_notification_modal.0, bindings.toggle_notification_modal.1) {
            actions.push(Action::ToggleNotificationModal);
        }

        // Switch context (Cmd+1 through Cmd+9) — these remain hardcoded for now.
        let num_keys = [
            egui::Key::Num1, egui::Key::Num2, egui::Key::Num3,
            egui::Key::Num4, egui::Key::Num5, egui::Key::Num6,
            egui::Key::Num7, egui::Key::Num8, egui::Key::Num9,
        ];
        for (i, key) in num_keys.into_iter().enumerate() {
            if input.consume_key(egui::Modifiers::COMMAND, key) {
                actions.push(Action::SwitchContext(i));
            }
        }
    });

    actions
}
