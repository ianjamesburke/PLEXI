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
// Cmd+Ctrl+H/J/K/L            — swap focused pane with neighbor in direction (focus follows)
// Cmd+Ctrl+Opt+H/J/K/L        — send focused pane in direction (focus stays)
// Cmd+Shift+M                 — toggle minimap overlay
// Cmd+T                       — new tab
// Cmd+Shift+L/H               — next/prev tab
// Cmd+Shift+J/K               — next/prev context
// Cmd+Shift+Ctrl+J/K           — move context down/up
// Cmd+Q                       — quit
// Cmd+B                       — toggle sidebar
// Cmd+Enter                   — zoom into sub-context (Portal focus) or toggle pane zoom
// Cmd+/                       — toggle shortcuts overlay
// Cmd+P                       — command palette
// Cmd+R                       — rename pane
// Cmd+Shift+R                 — rename context
// Cmd+[                       — nav back / focus history back
// Cmd+]                       — focus history forward
// Cmd+Shift+Up / Cmd+Shift+Down — scroll
// Cmd+= / Cmd+-               — font size
// Cmd+F                       — terminal search (handled inside egui_term, not host)
// Cmd+U                       — toggle pane hidden state
// Cmd+E                       — file browser
// Cmd+Shift+W                 — close context (with confirm dialog)
// Cmd+Shift+I                 — set context root from focused pane CWD
// Cmd+Shift+U                 — park/unpark context
// Cmd+Option+N                — extract focused pane into a new sub-context (portal)
// Cmd+Shift+Option+N          — new child context under current context (empty, auto-zoom)
// Cmd+0                       — quick note
// Cmd+1–9                     — switch context (sidebar)
// Escape (app active)         — close app
//
// Apps should use Cmd+S, Cmd+Shift+<key>, Ctrl+<key>, or unmodified keys.
// Always guard with `!input.modifiers.command` before consuming Enter, H, J,
// K, L, Backspace, or other keys that Plexi uses with Cmd modifier.
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    NextContext,
    PrevContext,
    MoveContextUp,
    MoveContextDown,
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
    /// Open the file browser app in the focused terminal.
    OpenFileBrowser,
    OpenAgents,
    NextBlockedAgent,
    FocusApproval,
    /// Open the quick note app (full pane, no terminal split).
    OpenQuickNote,
    /// Open config file in the text editor.
    OpenConfig,
    /// Open the secrets manager (read-only vault viewer).
    OpenSecretsManager,
    /// Open (or focus) the host Assistant pane. Bound to Cmd+Ctrl+A.
    OpenAssistant,
    /// Rename the active context. Bound to Cmd+Shift+R.
    RenameContext,
    /// Toggle the notification panel overlay (Cmd+Shift+A).
    ToggleNotificationModal,
    /// Force-reload the focused app pane (#83). Bound to Cmd+Option+R.
    /// Cmd+R is RenamePane and Cmd+Shift+R is RenameContext, so the
    /// Option (Alt) modifier is the next free chord. No-op when the
    /// focused pane isn't a process-backed app.
    ForceReloadApp,
    /// Create a new window to the right of the active one on the same
    /// grid row. Bound to Cmd+N.
    NewPageRight,
    /// Create a new context (sidebar item) with an automatic cwd-derived name.
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
    /// Send the focused pane in a direction without following focus.
    /// Focus stays at the vacated tile (on whatever pane fills that slot).
    /// Bound to Cmd+Ctrl+Opt+H/J/K/L.
    SendPane(Direction),
    /// Create a new scratch note in the inbox and open it in a text-editor pane.
    /// Bound to Cmd+Shift+Space.
    OpenScratchpad,
    /// Push the focused pane into a new sub-context. The pane becomes a portal
    /// and its content moves into the child. Bound to Cmd+Option+N.
    PushPaneToSubcontext,
    /// Create a new empty child context under the current context and auto-zoom
    /// into it. Bound to Cmd+Shift+Option+N.
    NewChildContext,
    /// Zoom out of the current sub-context to the parent. Bound to Cmd+Escape.
    ContextZoomOut,
    /// Set the active context root to the focused pane's CWD. Bound to Cmd+Shift+I.
    SetContextRootFromCwd,
    /// Toggle hidden state on the focused pane. Bound to Cmd+U.
    HidePane,
    /// Park/unpark the focused context. Bound to Cmd+Shift+U.
    ParkContext,
    /// Open the notes picker overlay from any pane type. Bound to Cmd+O.
    OpenNotesPicker,
    /// Close the active context with a confirm dialog. Bound to Cmd+Shift+W.
    CloseContext,
    /// Run a user-authored command from `commands.toml` (bound via `[keys]` `"<chord>" = "run:<name>"`).
    RunUserCommand(String),
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
    pub send_pane_left: (egui::Modifiers, egui::Key),
    pub send_pane_down: (egui::Modifiers, egui::Key),
    pub send_pane_up: (egui::Modifiers, egui::Key),
    pub send_pane_right: (egui::Modifiers, egui::Key),
    pub navigate_left: (egui::Modifiers, egui::Key),
    pub navigate_down: (egui::Modifiers, egui::Key),
    pub navigate_up: (egui::Modifiers, egui::Key),
    pub navigate_right: (egui::Modifiers, egui::Key),
    pub new_tab: (egui::Modifiers, egui::Key),
    pub next_tab: (egui::Modifiers, egui::Key),
    pub prev_tab: (egui::Modifiers, egui::Key),
    pub next_context: (egui::Modifiers, egui::Key),
    pub prev_context: (egui::Modifiers, egui::Key),
    pub move_context_up: (egui::Modifiers, egui::Key),
    pub move_context_down: (egui::Modifiers, egui::Key),
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
    pub next_blocked_agent: (egui::Modifiers, egui::Key),
    pub focus_approval: (egui::Modifiers, egui::Key),
    pub open_agents: (egui::Modifiers, egui::Key),
    pub open_file_browser: (egui::Modifiers, egui::Key),
    pub open_quick_note: (egui::Modifiers, egui::Key),
    pub open_config: (egui::Modifiers, egui::Key),
    pub reload_config: (egui::Modifiers, egui::Key),
    pub open_secrets_manager: (egui::Modifiers, egui::Key),
    pub open_assistant: (egui::Modifiers, egui::Key),
    pub force_reload_app: (egui::Modifiers, egui::Key),
    pub toggle_notification_modal: (egui::Modifiers, egui::Key),
    pub open_scratchpad: (egui::Modifiers, egui::Key),
    pub context_zoom_out: (egui::Modifiers, egui::Key),
    pub push_to_subcontext: (egui::Modifiers, egui::Key),
    pub new_child_context: (egui::Modifiers, egui::Key),
    pub set_context_root_from_cwd: (egui::Modifiers, egui::Key),
    pub hide_pane: (egui::Modifiers, egui::Key),
    pub park_context: (egui::Modifiers, egui::Key),
    pub open_notes_picker: (egui::Modifiers, egui::Key),
    pub close_context: (egui::Modifiers, egui::Key),
    /// User-command bindings configured via `[keys] "<chord>" = "run:<name>"`.
    pub user_commands: Vec<((egui::Modifiers, egui::Key), String)>,
}

fn cmd() -> egui::Modifiers {
    egui::Modifiers::COMMAND
}
fn cmd_shift() -> egui::Modifiers {
    egui::Modifiers {
        shift: true,
        ..egui::Modifiers::COMMAND
    }
}
fn cmd_ctrl() -> egui::Modifiers {
    egui::Modifiers {
        ctrl: true,
        ..egui::Modifiers::COMMAND
    }
}
fn cmd_alt() -> egui::Modifiers {
    egui::Modifiers {
        alt: true,
        ..egui::Modifiers::COMMAND
    }
}
fn cmd_shift_alt() -> egui::Modifiers {
    egui::Modifiers {
        shift: true,
        alt: true,
        ..egui::Modifiers::COMMAND
    }
}
fn cmd_ctrl_alt() -> egui::Modifiers {
    egui::Modifiers {
        ctrl: true,
        alt: true,
        ..egui::Modifiers::COMMAND
    }
}
fn cmd_shift_ctrl() -> egui::Modifiers {
    egui::Modifiers {
        shift: true,
        ctrl: true,
        ..egui::Modifiers::COMMAND
    }
}
impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            quit: (cmd(), egui::Key::Q),
            close_pane: (cmd(), egui::Key::W),
            toggle_command_palette: (cmd(), egui::Key::P),
            split_horizontal: (cmd(), egui::Key::D),
            split_vertical: (cmd_shift(), egui::Key::D),
            split_right: (cmd(), egui::Key::Backslash),
            split_down: (cmd_shift(), egui::Key::Backslash),
            swap_pane_left: (cmd_ctrl(), egui::Key::H),
            swap_pane_down: (cmd_ctrl(), egui::Key::J),
            swap_pane_up: (cmd_ctrl(), egui::Key::K),
            swap_pane_right: (cmd_ctrl(), egui::Key::L),
            send_pane_left: (cmd_ctrl_alt(), egui::Key::H),
            send_pane_down: (cmd_ctrl_alt(), egui::Key::J),
            send_pane_up: (cmd_ctrl_alt(), egui::Key::K),
            send_pane_right: (cmd_ctrl_alt(), egui::Key::L),
            navigate_left: (cmd(), egui::Key::H),
            navigate_down: (cmd(), egui::Key::J),
            navigate_up: (cmd(), egui::Key::K),
            navigate_right: (cmd(), egui::Key::L),
            new_tab: (cmd(), egui::Key::T),
            next_tab: (cmd_shift(), egui::Key::L),
            prev_tab: (cmd_shift(), egui::Key::H),
            next_context: (cmd_shift(), egui::Key::J),
            prev_context: (cmd_shift(), egui::Key::K),
            move_context_up: (cmd_shift_ctrl(), egui::Key::K),
            move_context_down: (cmd_shift_ctrl(), egui::Key::J),
            nav_back: (cmd(), egui::Key::OpenBracket),
            focus_history_forward: (cmd(), egui::Key::CloseBracket),
            toggle_sidebar: (cmd(), egui::Key::B),
            toggle_zoom: (cmd(), egui::Key::Enter),
            toggle_shortcuts: (cmd(), egui::Key::Slash),
            rename_context: (cmd_shift(), egui::Key::R),
            rename_pane: (cmd(), egui::Key::R),
            new_context: (cmd_shift(), egui::Key::N),
            new_page_right: (cmd(), egui::Key::N),
            toggle_minimap: (cmd_shift(), egui::Key::M),
            scroll_up: (cmd_shift(), egui::Key::ArrowUp),
            scroll_down: (cmd_shift(), egui::Key::ArrowDown),
            increase_font_size: (cmd(), egui::Key::Equals),
            decrease_font_size: (cmd(), egui::Key::Minus),
            next_blocked_agent: (cmd_alt(), egui::Key::B),
            focus_approval: (cmd_alt(), egui::Key::P),
            open_agents: (cmd_alt(), egui::Key::A),
            open_file_browser: (cmd(), egui::Key::E),
            open_quick_note: (cmd(), egui::Key::Num0),
            open_config: (cmd(), egui::Key::Comma),
            reload_config: (cmd_shift(), egui::Key::Comma),
            open_secrets_manager: (cmd_shift(), egui::Key::S),
            open_assistant: (cmd_ctrl(), egui::Key::A),
            force_reload_app: (cmd_alt(), egui::Key::R),
            toggle_notification_modal: (cmd_shift(), egui::Key::A),
            open_scratchpad: (cmd_shift(), egui::Key::Space),
            context_zoom_out: (cmd(), egui::Key::Escape),
            push_to_subcontext: (cmd_alt(), egui::Key::N),
            new_child_context: (cmd_shift_alt(), egui::Key::N),
            set_context_root_from_cwd: (cmd_shift(), egui::Key::I),
            hide_pane: (cmd(), egui::Key::U),
            park_context: (cmd_shift(), egui::Key::U),
            open_notes_picker: (cmd(), egui::Key::O),
            close_context: (cmd_shift(), egui::Key::W),
            user_commands: Vec::new(),
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
        "a" => egui::Key::A,
        "b" => egui::Key::B,
        "c" => egui::Key::C,
        "d" => egui::Key::D,
        "e" => egui::Key::E,
        "f" => egui::Key::F,
        "g" => egui::Key::G,
        "h" => egui::Key::H,
        "i" => egui::Key::I,
        "j" => egui::Key::J,
        "k" => egui::Key::K,
        "l" => egui::Key::L,
        "m" => egui::Key::M,
        "n" => egui::Key::N,
        "o" => egui::Key::O,
        "p" => egui::Key::P,
        "q" => egui::Key::Q,
        "r" => egui::Key::R,
        "s" => egui::Key::S,
        "t" => egui::Key::T,
        "u" => egui::Key::U,
        "v" => egui::Key::V,
        "w" => egui::Key::W,
        "x" => egui::Key::X,
        "y" => egui::Key::Y,
        "z" => egui::Key::Z,
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
                    log::warn!(
                        "keybindings: invalid combo '{}' for '{}' — keeping default",
                        s,
                        $name
                    );
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
    apply_override!(send_pane_left, "send_pane_left");
    apply_override!(send_pane_down, "send_pane_down");
    apply_override!(send_pane_up, "send_pane_up");
    apply_override!(send_pane_right, "send_pane_right");
    apply_override!(navigate_left, "navigate_left");
    apply_override!(navigate_down, "navigate_down");
    apply_override!(navigate_up, "navigate_up");
    apply_override!(navigate_right, "navigate_right");
    apply_override!(new_tab, "new_tab");
    apply_override!(next_tab, "next_tab");
    apply_override!(prev_tab, "prev_tab");
    apply_override!(next_context, "next_context");
    apply_override!(prev_context, "prev_context");
    apply_override!(move_context_up, "move_context_up");
    apply_override!(move_context_down, "move_context_down");
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
    apply_override!(next_blocked_agent, "next_blocked_agent");
    apply_override!(focus_approval, "focus_approval");
    apply_override!(open_agents, "open_agents");
    apply_override!(open_file_browser, "open_file_browser");
    apply_override!(open_quick_note, "open_quick_note");
    apply_override!(open_config, "open_config");
    apply_override!(reload_config, "reload_config");
    apply_override!(open_secrets_manager, "open_secrets_manager");
    apply_override!(open_assistant, "open_assistant");
    apply_override!(force_reload_app, "force_reload_app");
    apply_override!(toggle_notification_modal, "toggle_notification_modal");
    apply_override!(open_scratchpad, "open_scratchpad");
    apply_override!(push_to_subcontext, "push_to_subcontext");
    apply_override!(new_child_context, "new_child_context");
    apply_override!(set_context_root_from_cwd, "set_context_root_from_cwd");
    apply_override!(hide_pane, "hide_pane");
    apply_override!(park_context, "park_context");
    apply_override!(open_notes_picker, "open_notes_picker");
    apply_override!(close_context, "close_context");

    // Conflict detection
    let named: &[(&str, (egui::Modifiers, egui::Key))] = &[
        ("quit", bindings.quit),
        ("close_pane", bindings.close_pane),
        ("toggle_command_palette", bindings.toggle_command_palette),
        ("split_horizontal", bindings.split_horizontal),
        ("split_vertical", bindings.split_vertical),
        ("split_right", bindings.split_right),
        ("split_down", bindings.split_down),
        ("swap_pane_left", bindings.swap_pane_left),
        ("swap_pane_down", bindings.swap_pane_down),
        ("swap_pane_up", bindings.swap_pane_up),
        ("swap_pane_right", bindings.swap_pane_right),
        ("send_pane_left", bindings.send_pane_left),
        ("send_pane_down", bindings.send_pane_down),
        ("send_pane_up", bindings.send_pane_up),
        ("send_pane_right", bindings.send_pane_right),
        ("navigate_left", bindings.navigate_left),
        ("navigate_down", bindings.navigate_down),
        ("navigate_up", bindings.navigate_up),
        ("navigate_right", bindings.navigate_right),
        ("new_tab", bindings.new_tab),
        ("next_tab", bindings.next_tab),
        ("prev_tab", bindings.prev_tab),
        ("next_context", bindings.next_context),
        ("prev_context", bindings.prev_context),
        ("move_context_up", bindings.move_context_up),
        ("move_context_down", bindings.move_context_down),
        ("nav_back", bindings.nav_back),
        ("focus_history_forward", bindings.focus_history_forward),
        ("toggle_sidebar", bindings.toggle_sidebar),
        ("toggle_zoom", bindings.toggle_zoom),
        ("toggle_shortcuts", bindings.toggle_shortcuts),
        ("rename_context", bindings.rename_context),
        ("rename_pane", bindings.rename_pane),
        ("new_context", bindings.new_context),
        ("new_page_right", bindings.new_page_right),
        ("toggle_minimap", bindings.toggle_minimap),
        ("scroll_up", bindings.scroll_up),
        ("scroll_down", bindings.scroll_down),
        ("increase_font_size", bindings.increase_font_size),
        ("decrease_font_size", bindings.decrease_font_size),
        ("next_blocked_agent", bindings.next_blocked_agent),
        ("focus_approval", bindings.focus_approval),
        ("open_agents", bindings.open_agents),
        ("open_file_browser", bindings.open_file_browser),
        ("open_quick_note", bindings.open_quick_note),
        ("open_config", bindings.open_config),
        ("reload_config", bindings.reload_config),
        ("open_secrets_manager", bindings.open_secrets_manager),
        ("open_assistant", bindings.open_assistant),
        ("force_reload_app", bindings.force_reload_app),
        (
            "toggle_notification_modal",
            bindings.toggle_notification_modal,
        ),
        ("open_scratchpad", bindings.open_scratchpad),
        ("push_to_subcontext", bindings.push_to_subcontext),
        ("new_child_context", bindings.new_child_context),
        (
            "set_context_root_from_cwd",
            bindings.set_context_root_from_cwd,
        ),
        ("hide_pane", bindings.hide_pane),
        ("park_context", bindings.park_context),
        ("open_notes_picker", bindings.open_notes_picker),
        ("close_context", bindings.close_context),
    ];

    let mut seen: std::collections::HashMap<u64, &str> = std::collections::HashMap::new();
    for (name, (mods, key)) in named {
        let id = binding_id(*mods, *key);
        if let Some(other) = seen.insert(id, name) {
            log::error!(
                "keybindings: conflict — '{}' and '{}' share the same key combo",
                other,
                name
            );
        }
    }

    log::info!(
        "keybindings: {} override(s) applied from config",
        override_count
    );

    bindings
}

fn reserved_host_shortcuts(
    bindings: &KeyBindings,
) -> std::collections::HashMap<u64, &'static str> {
    let defaults = KeyBindings::default();
    let mut reserved = std::collections::HashMap::new();
    for (source, b) in [("default", &defaults), ("configured", bindings)] {
        let _ = source;
        let entries: &[(&'static str, (egui::Modifiers, egui::Key))] = &[
            ("quit", b.quit),
            ("close_pane", b.close_pane),
            ("toggle_command_palette", b.toggle_command_palette),
            ("split_horizontal", b.split_horizontal),
            ("split_vertical", b.split_vertical),
            ("split_right", b.split_right),
            ("split_down", b.split_down),
            ("swap_pane_left", b.swap_pane_left),
            ("swap_pane_down", b.swap_pane_down),
            ("swap_pane_up", b.swap_pane_up),
            ("swap_pane_right", b.swap_pane_right),
            ("send_pane_left", b.send_pane_left),
            ("send_pane_down", b.send_pane_down),
            ("send_pane_up", b.send_pane_up),
            ("send_pane_right", b.send_pane_right),
            ("navigate_left", b.navigate_left),
            ("navigate_down", b.navigate_down),
            ("navigate_up", b.navigate_up),
            ("navigate_right", b.navigate_right),
            ("new_tab", b.new_tab),
            ("next_tab", b.next_tab),
            ("prev_tab", b.prev_tab),
            ("next_context", b.next_context),
            ("prev_context", b.prev_context),
            ("move_context_up", b.move_context_up),
            ("move_context_down", b.move_context_down),
            ("nav_back", b.nav_back),
            ("focus_history_forward", b.focus_history_forward),
            ("toggle_sidebar", b.toggle_sidebar),
            ("toggle_zoom", b.toggle_zoom),
            ("toggle_shortcuts", b.toggle_shortcuts),
            ("rename_context", b.rename_context),
            ("rename_pane", b.rename_pane),
            ("new_context", b.new_context),
            ("new_page_right", b.new_page_right),
            ("toggle_minimap", b.toggle_minimap),
            ("scroll_up", b.scroll_up),
            ("scroll_down", b.scroll_down),
            ("increase_font_size", b.increase_font_size),
            ("decrease_font_size", b.decrease_font_size),
            ("next_blocked_agent", b.next_blocked_agent),
            ("focus_approval", b.focus_approval),
            ("open_agents", b.open_agents),
            ("open_file_browser", b.open_file_browser),
            ("open_quick_note", b.open_quick_note),
            ("open_config", b.open_config),
            ("reload_config", b.reload_config),
            ("open_secrets_manager", b.open_secrets_manager),
            ("open_assistant", b.open_assistant),
            ("force_reload_app", b.force_reload_app),
            ("toggle_notification_modal", b.toggle_notification_modal),
            ("open_scratchpad", b.open_scratchpad),
            ("context_zoom_out", b.context_zoom_out),
            ("push_to_subcontext", b.push_to_subcontext),
            ("new_child_context", b.new_child_context),
            ("set_context_root_from_cwd", b.set_context_root_from_cwd),
            ("hide_pane", b.hide_pane),
            ("park_context", b.park_context),
            ("open_notes_picker", b.open_notes_picker),
            ("close_context", b.close_context),
        ];
        for (name, (mods, key)) in entries {
            reserved.insert(binding_id(*mods, *key), *name);
        }
    }
    for (key, label) in [
        (egui::Key::Num1, "switch_context_1"),
        (egui::Key::Num2, "switch_context_2"),
        (egui::Key::Num3, "switch_context_3"),
        (egui::Key::Num4, "switch_context_4"),
        (egui::Key::Num5, "switch_context_5"),
        (egui::Key::Num6, "switch_context_6"),
        (egui::Key::Num7, "switch_context_7"),
        (egui::Key::Num8, "switch_context_8"),
        (egui::Key::Num9, "switch_context_9"),
    ] {
        reserved.insert(binding_id(cmd(), key), label);
    }
    reserved.insert(binding_id(egui::Modifiers::NONE, egui::Key::Escape), "close_app");
    reserved
}

/// Validate `[keys]` entries (`"<chord>" = "run:<command>"`) against reserved
/// Plexi shortcuts and syntax rules. Returns human-readable error strings for
/// `plexi config check`.
pub fn validate_user_keys(
    keys: &std::collections::HashMap<String, String>,
    bindings: &KeyBindings,
) -> Vec<String> {
    let reserved = reserved_host_shortcuts(bindings);
    let mut errors = Vec::new();
    let mut sorted_entries: Vec<(&String, &String)> = keys.iter().collect();
    sorted_entries.sort_by(|a, b| a.0.cmp(b.0));

    let mut seen_user: std::collections::HashMap<u64, &str> = std::collections::HashMap::new();
    for (chord, target) in sorted_entries {
        let Some((mods, key)) = parse_key_combo(chord) else {
            errors.push(format!("invalid key chord `{chord}` in [keys]"));
            continue;
        };
        let Some(cmd_name) = target.strip_prefix("run:").map(str::trim).filter(|s| !s.is_empty())
        else {
            errors.push(format!(
                "invalid [keys] target `{target}` for `{chord}` (expected `run:<command>`)"
            ));
            continue;
        };
        let _ = cmd_name;
        let id = binding_id(mods, key);
        if let Some(reserved_name) = reserved.get(&id) {
            errors.push(format!(
                "[keys] `{chord}` (\"{target}\") conflicts with reserved shortcut `{reserved_name}`"
            ));
            continue;
        }
        if let Some(prev_chord) = seen_user.insert(id, chord.as_str()) {
            errors.push(format!(
                "[keys] `{chord}` duplicates key chord `{prev_chord}`"
            ));
        }
    }
    errors
}

impl KeyBindings {
    /// Apply validated `[keys]` user-command bindings to this `KeyBindings`.
    /// Entries that fail validation or collide with reserved shortcuts are logged
    /// and skipped.
    pub fn apply_user_keys(
        &mut self,
        user_keys: Option<&std::collections::HashMap<String, String>>,
    ) {
        self.user_commands.clear();
        let Some(keys) = user_keys else {
            return;
        };
        let reserved = reserved_host_shortcuts(self);
        let mut sorted_entries: Vec<(&String, &String)> = keys.iter().collect();
        sorted_entries.sort_by(|a, b| a.0.cmp(b.0));
        let mut seen_user: std::collections::HashSet<u64> = std::collections::HashSet::new();

        for (chord, target) in sorted_entries {
            let Some((mods, key)) = parse_key_combo(chord) else {
                log::warn!("keys: ignoring invalid chord '{chord}'");
                continue;
            };
            let Some(cmd_name) = target
                .strip_prefix("run:")
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                log::warn!("keys: ignoring invalid target '{target}' for '{chord}' (expected 'run:<name>')");
                continue;
            };
            let id = binding_id(mods, key);
            if let Some(reserved_name) = reserved.get(&id) {
                log::error!(
                    "keys: '{chord}' ('{target}') conflicts with reserved shortcut '{reserved_name}' — ignoring"
                );
                continue;
            }
            if !seen_user.insert(id) {
                log::error!("keys: duplicate user chord '{chord}' — ignoring");
                continue;
            }
            self.user_commands.push(((mods, key), cmd_name.to_string()));
        }
        if !self.user_commands.is_empty() {
            log::info!(
                "keys: registered {} user command binding(s)",
                self.user_commands.len()
            );
        }
    }
}

pub fn build_key_bindings_with_user_keys(
    overrides: Option<&KeybindingsConfig>,
    user_keys: Option<&std::collections::HashMap<String, String>>,
) -> KeyBindings {
    let mut bindings = build_key_bindings(overrides);
    bindings.apply_user_keys(user_keys);
    bindings
}

/// Context under which a binding is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingContext {
    /// Always active — even when overlays/keyboard capture is on.
    /// Use for Quit, ClosePane, CommandPalette, QuickNote, ToggleNotificationModal.
    Global,
    /// Active only when no overlay holds focus and no app has keyboard capture.
    Normal,
    /// Active only when an app surface is focused AND no overlay/capture is active.
    AppActive,
}

/// A single entry in the declarative binding table.
#[derive(Clone)]
pub struct BindingEntry {
    pub modifiers: egui::Modifiers,
    pub key: egui::Key,
    /// When `true`, `input.modifiers` must equal `modifiers` exactly (not just be a subset).
    /// Use this when a less-specific binding (e.g. Cmd+D) could be triggered by a
    /// more-specific chord (e.g. Cmd+Shift+D) because egui's `consume_key` uses subset matching.
    pub exact: bool,
    /// When `true`, key-repeat events (key held down) are ignored for this binding.
    /// Use for destructive one-shot actions like ClosePane and CloseContext so that
    /// holding the key doesn't fire the action multiple times per second.
    pub no_repeat: bool,
    pub context: BindingContext,
    pub action: Action,
}

fn modifier_count(m: &egui::Modifiers) -> u32 {
    m.command as u32 + m.shift as u32 + m.ctrl as u32 + m.alt as u32
}

// egui::Modifiers::COMMAND sets mac_cmd=false, but macOS runtime input always
// sets mac_cmd=true when Cmd is held. Raw == comparison fails on macOS.
// On non-macOS (Linux/Windows), egui sets both `ctrl` and `command` to true
// when physical Ctrl is held, while `cmd()` patterns set `command=true, ctrl=false`.
fn modifiers_match_exact(actual: &egui::Modifiers, pattern: &egui::Modifiers) -> bool {
    modifiers_match_exact_for_os(actual, pattern, cfg!(target_os = "macos"))
}

pub(crate) fn modifiers_match_exact_for_os(
    actual: &egui::Modifiers,
    pattern: &egui::Modifiers,
    is_macos: bool,
) -> bool {
    if is_macos {
        actual.command == pattern.command
            && actual.ctrl == pattern.ctrl
            && actual.shift == pattern.shift
            && actual.alt == pattern.alt
    } else {
        (actual.command || actual.ctrl) == (pattern.command || pattern.ctrl)
            && actual.shift == pattern.shift
            && actual.alt == pattern.alt
    }
}

/// Build a sorted binding table from resolved `KeyBindings`.
///
/// Sorting guarantees: exact matches before subset matches; within each group,
/// higher modifier count before lower. This means more-specific bindings always
/// win without requiring careful manual ordering in the if-chain.
pub fn build_binding_table(b: &KeyBindings) -> Vec<BindingEntry> {
    let mut entries: Vec<BindingEntry> = vec![
        // ── Global bindings (always active) ──────────────────────────────────
        BindingEntry {
            modifiers: b.quit.0,
            key: b.quit.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Global,
            action: Action::Quit,
        },
        BindingEntry {
            modifiers: b.close_pane.0,
            key: b.close_pane.1,
            exact: false,
            no_repeat: true,
            context: BindingContext::Global,
            action: Action::ClosePane,
        },
        BindingEntry {
            modifiers: b.toggle_command_palette.0,
            key: b.toggle_command_palette.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Global,
            action: Action::ToggleCommandPalette,
        },
        BindingEntry {
            modifiers: b.open_quick_note.0,
            key: b.open_quick_note.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Global,
            action: Action::OpenQuickNote,
        },
        BindingEntry {
            modifiers: b.open_agents.0,
            key: b.open_agents.1,
            exact: true,
            no_repeat: true,
            context: BindingContext::Global,
            action: Action::OpenAgents,
        },
        BindingEntry {
            modifiers: b.next_blocked_agent.0,
            key: b.next_blocked_agent.1,
            exact: true,
            no_repeat: true,
            context: BindingContext::Global,
            action: Action::NextBlockedAgent,
        },
        BindingEntry {
            modifiers: b.focus_approval.0,
            key: b.focus_approval.1,
            exact: true,
            no_repeat: true,
            context: BindingContext::Global,
            action: Action::FocusApproval,
        },
        // toggle_notification_modal is Cmd+Shift+A; exact=true so plain Cmd+A (select-all)
        // is not consumed by the subset match.
        BindingEntry {
            modifiers: b.toggle_notification_modal.0,
            key: b.toggle_notification_modal.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Global,
            action: Action::ToggleNotificationModal,
        },
        BindingEntry {
            modifiers: b.close_context.0,
            key: b.close_context.1,
            exact: true,
            no_repeat: true,
            context: BindingContext::Global,
            action: Action::CloseContext,
        },
        // ── Normal bindings (suppressed when overlay/capture active) ─────────
        BindingEntry {
            modifiers: b.split_vertical.0,
            key: b.split_vertical.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SplitVertical,
        },
        BindingEntry {
            modifiers: b.split_horizontal.0,
            key: b.split_horizontal.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SplitHorizontal,
        },
        BindingEntry {
            modifiers: b.swap_pane_left.0,
            key: b.swap_pane_left.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SwapPane(Direction::Left),
        },
        BindingEntry {
            modifiers: b.swap_pane_down.0,
            key: b.swap_pane_down.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SwapPane(Direction::Down),
        },
        BindingEntry {
            modifiers: b.swap_pane_up.0,
            key: b.swap_pane_up.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SwapPane(Direction::Up),
        },
        BindingEntry {
            modifiers: b.swap_pane_right.0,
            key: b.swap_pane_right.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SwapPane(Direction::Right),
        },
        BindingEntry {
            modifiers: b.send_pane_left.0,
            key: b.send_pane_left.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SendPane(Direction::Left),
        },
        BindingEntry {
            modifiers: b.send_pane_down.0,
            key: b.send_pane_down.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SendPane(Direction::Down),
        },
        BindingEntry {
            modifiers: b.send_pane_up.0,
            key: b.send_pane_up.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SendPane(Direction::Up),
        },
        BindingEntry {
            modifiers: b.send_pane_right.0,
            key: b.send_pane_right.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SendPane(Direction::Right),
        },
        BindingEntry {
            modifiers: b.new_tab.0,
            key: b.new_tab.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::NewTab,
        },
        BindingEntry {
            modifiers: b.next_tab.0,
            key: b.next_tab.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::NextTab,
        },
        BindingEntry {
            modifiers: b.prev_tab.0,
            key: b.prev_tab.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::PrevTab,
        },
        BindingEntry {
            modifiers: b.next_context.0,
            key: b.next_context.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::NextContext,
        },
        BindingEntry {
            modifiers: b.prev_context.0,
            key: b.prev_context.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::PrevContext,
        },
        BindingEntry {
            modifiers: b.move_context_up.0,
            key: b.move_context_up.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::MoveContextUp,
        },
        BindingEntry {
            modifiers: b.move_context_down.0,
            key: b.move_context_down.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::MoveContextDown,
        },
        BindingEntry {
            modifiers: b.navigate_left.0,
            key: b.navigate_left.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::Navigate(Direction::Left),
        },
        BindingEntry {
            modifiers: b.navigate_down.0,
            key: b.navigate_down.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::Navigate(Direction::Down),
        },
        BindingEntry {
            modifiers: b.navigate_up.0,
            key: b.navigate_up.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::Navigate(Direction::Up),
        },
        BindingEntry {
            modifiers: b.navigate_right.0,
            key: b.navigate_right.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::Navigate(Direction::Right),
        },
        BindingEntry {
            modifiers: b.nav_back.0,
            key: b.nav_back.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::NavBackApp,
        },
        BindingEntry {
            modifiers: b.focus_history_forward.0,
            key: b.focus_history_forward.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::FocusHistoryForward,
        },
        BindingEntry {
            modifiers: b.toggle_sidebar.0,
            key: b.toggle_sidebar.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ToggleSidebar,
        },
        BindingEntry {
            modifiers: b.toggle_zoom.0,
            key: b.toggle_zoom.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ToggleZoom,
        },
        BindingEntry {
            modifiers: b.toggle_shortcuts.0,
            key: b.toggle_shortcuts.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ToggleShortcuts,
        },
        BindingEntry {
            modifiers: b.rename_context.0,
            key: b.rename_context.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::RenameContext,
        },
        BindingEntry {
            modifiers: b.rename_pane.0,
            key: b.rename_pane.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::RenamePane,
        },
        BindingEntry {
            modifiers: b.split_down.0,
            key: b.split_down.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SplitDown,
        },
        BindingEntry {
            modifiers: b.split_right.0,
            key: b.split_right.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SplitRight,
        },
        BindingEntry {
            modifiers: b.push_to_subcontext.0,
            key: b.push_to_subcontext.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::PushPaneToSubcontext,
        },
        BindingEntry {
            modifiers: b.new_child_context.0,
            key: b.new_child_context.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::NewChildContext,
        },
        BindingEntry {
            modifiers: b.new_context.0,
            key: b.new_context.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::NewContext,
        },
        BindingEntry {
            modifiers: b.new_page_right.0,
            key: b.new_page_right.1,
            exact: true,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::NewPageRight,
        },
        BindingEntry {
            modifiers: b.toggle_minimap.0,
            key: b.toggle_minimap.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ToggleMinimap,
        },
        BindingEntry {
            modifiers: b.scroll_up.0,
            key: b.scroll_up.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ScrollUp,
        },
        BindingEntry {
            modifiers: b.scroll_down.0,
            key: b.scroll_down.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ScrollDown,
        },
        BindingEntry {
            modifiers: b.increase_font_size.0,
            key: b.increase_font_size.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::IncreasePaneFontSize,
        },
        BindingEntry {
            modifiers: b.decrease_font_size.0,
            key: b.decrease_font_size.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::DecreasePaneFontSize,
        },
        BindingEntry {
            modifiers: b.open_file_browser.0,
            key: b.open_file_browser.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::OpenFileBrowser,
        },
        BindingEntry {
            modifiers: b.open_config.0,
            key: b.open_config.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::OpenConfig,
        },
        BindingEntry {
            modifiers: b.reload_config.0,
            key: b.reload_config.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ReloadConfig,
        },
        BindingEntry {
            modifiers: b.open_secrets_manager.0,
            key: b.open_secrets_manager.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::OpenSecretsManager,
        },
        BindingEntry {
            modifiers: b.open_assistant.0,
            key: b.open_assistant.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::OpenAssistant,
        },
        BindingEntry {
            modifiers: b.force_reload_app.0,
            key: b.force_reload_app.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ForceReloadApp,
        },
        BindingEntry {
            modifiers: b.set_context_root_from_cwd.0,
            key: b.set_context_root_from_cwd.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::SetContextRootFromCwd,
        },
        BindingEntry {
            modifiers: b.hide_pane.0,
            key: b.hide_pane.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::HidePane,
        },
        BindingEntry {
            modifiers: b.park_context.0,
            key: b.park_context.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ParkContext,
        },
        BindingEntry {
            modifiers: b.open_scratchpad.0,
            key: b.open_scratchpad.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::OpenScratchpad,
        },
        // Notes picker opens from any pane type — terminal, app, text-editor.
        BindingEntry {
            modifiers: b.open_notes_picker.0,
            key: b.open_notes_picker.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::OpenNotesPicker,
        },
        // context_zoom_out is Cmd+Escape — not exact because plain Escape is AppActive only,
        // so there is no subset conflict on this key.
        BindingEntry {
            modifiers: b.context_zoom_out.0,
            key: b.context_zoom_out.1,
            exact: false,
            no_repeat: false,
            context: BindingContext::Normal,
            action: Action::ContextZoomOut,
        },
        // ── AppActive bindings (only when app surface focused) ───────────────
        // CloseApp (Escape) — also suppressed when shortcuts overlay is open.
        BindingEntry {
            modifiers: egui::Modifiers::NONE,
            key: egui::Key::Escape,
            exact: false,
            no_repeat: false,
            context: BindingContext::AppActive,
            action: Action::CloseApp,
        },
    ];

    for ((modifiers, key), cmd_name) in &b.user_commands {
        entries.push(BindingEntry {
            modifiers: *modifiers,
            key: *key,
            exact: true,
            no_repeat: true,
            context: BindingContext::Normal,
            action: Action::RunUserCommand(cmd_name.clone()),
        });
    }

    // Sort: exact before non-exact; within each group, higher modifier count first.
    // This eliminates ordering bugs — the table author never needs to worry about
    // more-specific vs. less-specific placement.
    entries.sort_by(|a, b| {
        // exact=true sorts before exact=false
        b.exact
            .cmp(&a.exact)
            .then_with(|| modifier_count(&b.modifiers).cmp(&modifier_count(&a.modifiers)))
    });

    entries
}

/// Poll global keyboard actions using the pre-built binding table.
///
/// `app_active` — focused pane has an active app surface (affects Escape/Tab).
/// `keyboard_capture_active` — focused app declared `keyboard_capture = true` in its manifest.
///   When true, all host shortcuts are suppressed *except* Global-context bindings
///   (Quit, ClosePane, CommandPalette, QuickNote, ToggleNotificationModal).
/// `overlay_open` — an overlay owns keyboard focus. Same suppression as `keyboard_capture_active`.
///   Each overlay's `*_handle_key` method owns its own key contract and runs before this function.
/// `shortcuts_overlay_open` — the shortcuts overlay is open; suppresses CloseApp (Escape)
///   so the overlay can own Escape for dismissal.
///
/// `input` is a [`crate::app::input_router::PlexiInput`] buffer already taken
/// out of `ctx` for this frame (stint 0240's ownership-transfer router).
/// The host calls this after the focused overlay owner in two passes: Global
/// and Normal bindings before focused-app key dispatch, then AppActive
/// bindings afterward. Thus host chords are structurally removed before a
/// guest runtime can see them, while an app or focused text field still owns
/// bare Escape before CloseApp.
pub fn poll_actions(
    input: &mut crate::app::input_router::PlexiInput,
    table: &[BindingEntry],
    claimed_key_releases: &mut std::collections::HashSet<egui::Key>,
    app_active: bool,
    keyboard_capture_active: bool,
    overlay_open: bool,
    shortcuts_overlay_open: bool,
) -> Vec<Action> {
    let mut actions = Vec::new();

    // A host chord owns both edges even if Command is released first and the
    // matching key-up therefore arrives with no modifiers in a later frame.
    for key in claimed_key_releases.iter().copied().collect::<Vec<_>>() {
        if input.consume_key_release(key) {
            claimed_key_releases.remove(&key);
        }
    }

    for entry in table {
        match entry.context {
            BindingContext::Global => {
                // Always checked — no suppression.
            }
            BindingContext::Normal => {
                if keyboard_capture_active || overlay_open {
                    continue;
                }
            }
            BindingContext::AppActive => {
                if !app_active || keyboard_capture_active || overlay_open {
                    continue;
                }
                // Escape is suppressed when the shortcuts overlay is open so the overlay
                // can own it for dismissal.
                if matches!(entry.action, Action::CloseApp) && shortcuts_overlay_open {
                    continue;
                }
            }
        }

        if entry.exact && !modifiers_match_exact(&input.modifiers(), &entry.modifiers) {
            continue;
        }

        let triggered = if entry.no_repeat {
            input.consume_key_no_repeat(entry.modifiers, entry.key)
        } else {
            input.consume_key(entry.modifiers, entry.key)
        };
        if triggered {
            claimed_key_releases.insert(entry.key);
            actions.push(entry.action.clone());
        }
    }

    // Switch context (Cmd+1 through Cmd+9) — hardcoded loop; not individual table entries.
    if !(keyboard_capture_active || overlay_open) {
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
                claimed_key_releases.insert(key);
                actions.push(Action::SwitchContext(i));
            }
        }
    }

    // Also claim a release delivered in the same raw-input batch as its press.
    for key in claimed_key_releases.iter().copied().collect::<Vec<_>>() {
        if input.consume_key_release(key) {
            claimed_key_releases.remove(&key);
        }
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn attention_keys_preserve_notifications_and_use_free_binding() {
        let b = KeyBindings::default();
        let table = build_binding_table(&b);
        assert_eq!(b.toggle_notification_modal, (cmd_shift(), egui::Key::A));
        assert!(table.iter().any(|e| (e.modifiers, e.key) == b.toggle_notification_modal && e.action == Action::ToggleNotificationModal));
        assert_eq!(b.focus_approval, (cmd_alt(), egui::Key::P));
        assert_eq!(table.iter().filter(|e| (e.modifiers, e.key) == b.focus_approval).count(), 1);
        let approval: crate::config::KeybindingsConfig = toml::from_str("focus_approval = \"cmd+alt+g\"").unwrap();
        assert_eq!(build_key_bindings(Some(&approval)).focus_approval, (cmd_alt(), egui::Key::G));
        assert_eq!(b.next_blocked_agent, (cmd_alt(), egui::Key::B));
        let overrides: crate::config::KeybindingsConfig = toml::from_str(
            "next_blocked_agent = \"cmd+alt+g\"").unwrap();
        assert_eq!(build_key_bindings(Some(&overrides)).next_blocked_agent, (cmd_alt(), egui::Key::G));
        assert_eq!(table.iter().filter(|e| (e.modifiers, e.key) == b.next_blocked_agent).count(), 1);
        assert!(table.iter().any(|e| e.action == Action::NextBlockedAgent));
    }

    /// Run `poll_actions` for a single Tab keypress with an active app surface.
    fn poll_tab() -> Vec<Action> {
        let table = build_binding_table(&KeyBindings::default());
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let mut actions = Vec::new();
        let mut claimed_key_releases = std::collections::HashSet::new();
        let _ = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx();
            let mut input = crate::app::input_router::PlexiInput::take_from(ctx);
            actions = poll_actions(
                &mut input,
                &table,
                &mut claimed_key_releases,
                /* app_active */ true,
                /* keyboard_capture */ false,
                /* overlay_open */ false, /* shortcuts_overlay_open */ false,
            );
        });
        actions
    }

    #[test]
    fn tab_passes_through_to_active_app() {
        let actions = poll_tab();
        assert!(
            actions.is_empty(),
            "plain Tab should not be consumed by host app focus; apps own it"
        );
    }

    #[test]
    fn modifiers_match_exact_treats_command_and_ctrl_as_one_on_non_mac_and_distinct_on_mac() {
        // Linux/Windows runtime modifiers when physical Ctrl is pressed:
        // egui sets both `ctrl: true` and `command: true`.
        let linux_ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let linux_ctrl_alt = egui::Modifiers {
            ctrl: true,
            command: true,
            alt: true,
            ..egui::Modifiers::NONE
        };
        // Non-macOS: `cmd()` (`command=true, ctrl=false`) and `cmd_alt()` match Linux Ctrl chords.
        assert!(modifiers_match_exact_for_os(&linux_ctrl, &cmd(), false));
        assert!(modifiers_match_exact_for_os(&linux_ctrl_alt, &cmd_alt(), false));
        assert!(!modifiers_match_exact_for_os(&linux_ctrl, &cmd_alt(), false));
        assert!(!modifiers_match_exact_for_os(&linux_ctrl_alt, &cmd(), false));

        // macOS: `cmd` and `ctrl` are distinct physical modifiers.
        let mac_cmd_only = egui::Modifiers {
            command: true,
            mac_cmd: true,
            ctrl: false,
            ..egui::Modifiers::NONE
        };
        let mac_cmd_ctrl = egui::Modifiers {
            command: true,
            mac_cmd: true,
            ctrl: true,
            ..egui::Modifiers::NONE
        };
        assert!(modifiers_match_exact_for_os(&mac_cmd_only, &cmd(), true));
        assert!(!modifiers_match_exact_for_os(&mac_cmd_ctrl, &cmd(), true));
        assert!(modifiers_match_exact_for_os(&mac_cmd_ctrl, &cmd_ctrl(), true));
        assert!(!modifiers_match_exact_for_os(&mac_cmd_only, &cmd_ctrl(), true));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn linux_ctrl_chords_fire_focus_approval_and_command_palette_without_leaking() {
        let table = build_binding_table(&KeyBindings::default());
        let ctx = egui::Context::default();

        for (mods, key, expected) in [
            (
                egui::Modifiers {
                    ctrl: true,
                    command: true,
                    alt: true,
                    ..egui::Modifiers::NONE
                },
                egui::Key::P,
                Action::FocusApproval,
            ),
            (
                egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..egui::Modifiers::NONE
                },
                egui::Key::P,
                Action::ToggleCommandPalette,
            ),
        ] {
            let mut raw = egui::RawInput {
                modifiers: mods,
                ..Default::default()
            };
            raw.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            });
            let mut actions = Vec::new();
            let mut remaining_events = 0;
            let mut claimed_key_releases = std::collections::HashSet::new();
            let _ = ctx.run_ui(raw, |ui| {
                let ctx = ui.ctx();
                let mut input = crate::app::input_router::PlexiInput::take_from(ctx);
                actions = poll_actions(
                    &mut input,
                    &table,
                    &mut claimed_key_releases,
                    /* app_active */ false,
                    /* keyboard_capture */ false,
                    /* overlay_open */ false,
                    /* shortcuts_overlay_open */ false,
                );
                remaining_events = input.events().len();
            });
            assert_eq!(actions, vec![expected]);
            assert_eq!(
                remaining_events, 0,
                "chord must be consumed from PlexiInput so it does not leak to the terminal"
            );
        }
    }
}
