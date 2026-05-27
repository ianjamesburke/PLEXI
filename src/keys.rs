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
// Cmd+I                       — context inspector
// Cmd+0                       — quick note
// Cmd+1–9                     — switch context (sidebar)
// Escape (app active)         — close app
// Tab (app active)            — navigate to linked terminal
//
// Apps should use Cmd+S, Cmd+Shift+<key>, Ctrl+<key>, or unmodified keys.
// Always guard with `!input.modifiers.command` before consuming Enter, H, J,
// K, L, Backspace, or other keys that Plexi uses with Cmd modifier.
//
// PLATFORM: the chords above are written with the macOS ⌘ key. egui maps
// `Modifiers::COMMAND` to ⌘ on macOS but to **Ctrl** everywhere else — so the
// mac scheme would turn every host shortcut into a bare Ctrl+<key> chord, the
// exact space terminals use for control codes (Ctrl+I = Tab, Ctrl+R =
// reverse-search, Ctrl+W = delete-word, …). On Windows/Linux the defaults
// therefore shift up one tier — `Cmd` → `Ctrl+Shift`, `Cmd+Shift` →
// `Ctrl+Alt` — leaving the bare-Ctrl namespace free for the focused terminal.
// See `cmd()` / `cmd_shift()` below.
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
    /// Toggle the context inspector modal. Bound to Cmd+I.
    ContextInspector,
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
    /// Open the scratchpad overlay. Bound to Cmd+Shift+Space.
    OpenScratchpad,
    /// Zoom into the sub-context tile that has focus. Bound to Cmd+Shift+Enter.
    ContextZoomIn,
    /// Zoom out of the current sub-context to the parent. Bound to Cmd+Escape.
    ContextZoomOut,
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
    pub context_inspector: (egui::Modifiers, egui::Key),
    pub open_scratchpad: (egui::Modifiers, egui::Key),
    pub context_zoom_in: (egui::Modifiers, egui::Key),
    pub context_zoom_out: (egui::Modifiers, egui::Key),
}

// ── Host shortcut modifier tiers ──────────────────────────────────────────
//
// macOS has a dedicated ⌘ key that namespaces every host shortcut away from
// the terminal's Ctrl-based control codes, so the tiers map straight onto Cmd:
//
//   cmd       → ⌘             cmd_ctrl → ⌘⌃
//   cmd_shift → ⌘⇧            cmd_alt  → ⌘⌥
//
// Windows/Linux have no ⌘, and egui collapses `Modifiers::COMMAND` to Ctrl, so
// the mac scheme would make every shortcut a bare Ctrl+<key> chord that steals
// terminal control codes. We shift each tier up one modifier — mirroring the
// Ctrl+Shift convention the terminal itself already uses for copy/paste — so
// bare Ctrl stays free for the focused terminal:
//
//   cmd       → Ctrl+Shift        cmd_ctrl → Ctrl+Alt
//   cmd_shift → Ctrl+Shift+Alt    cmd_alt  → Ctrl+Alt
//
// egui's `consume_key` matches modifiers as a *subset* (a Ctrl+Shift pattern
// also matches a Ctrl+Shift+Alt press — see `Modifiers::matches_logically`),
// so `poll_actions` must check the tiers most-specific-first: cmd_shift
// (Ctrl+Shift+Alt) before both cmd (Ctrl+Shift) and cmd_ctrl/cmd_alt
// (Ctrl+Alt). cmd_ctrl and cmd_alt collapse to the same Ctrl+Alt chord on
// non-mac but bind disjoint keys (swap = HJKL, force-reload = R), so they
// never collide.
#[cfg(target_os = "macos")]
fn cmd() -> egui::Modifiers { egui::Modifiers::COMMAND }
#[cfg(not(target_os = "macos"))]
fn cmd() -> egui::Modifiers {
    egui::Modifiers { ctrl: true, shift: true, ..egui::Modifiers::default() }
}

#[cfg(target_os = "macos")]
fn cmd_shift() -> egui::Modifiers {
    egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND }
}
#[cfg(not(target_os = "macos"))]
fn cmd_shift() -> egui::Modifiers {
    egui::Modifiers { ctrl: true, shift: true, alt: true, ..egui::Modifiers::default() }
}

#[cfg(target_os = "macos")]
fn cmd_ctrl() -> egui::Modifiers {
    egui::Modifiers { ctrl: true, ..egui::Modifiers::COMMAND }
}
#[cfg(not(target_os = "macos"))]
fn cmd_ctrl() -> egui::Modifiers {
    egui::Modifiers { ctrl: true, alt: true, ..egui::Modifiers::default() }
}

#[cfg(target_os = "macos")]
fn cmd_alt() -> egui::Modifiers {
    egui::Modifiers { alt: true, ..egui::Modifiers::COMMAND }
}
#[cfg(not(target_os = "macos"))]
fn cmd_alt() -> egui::Modifiers {
    egui::Modifiers { ctrl: true, alt: true, ..egui::Modifiers::default() }
}

// The non-mac primary (`cmd` → Ctrl+Shift) and secondary (`cmd_shift` →
// Ctrl+Shift+Alt) tiers include Shift, which changes the *logical* key egui
// reports for punctuation/digits: e.g. Shift+`/` arrives as `Key::Questionmark`,
// not `Key::Slash`, so a `Slash` binding never fires (egui-winit sets the event
// key to `logical_key.or(physical_key)`). For keys egui exposes as a distinct
// shifted variant, bind to that variant so the chord the user types (Ctrl+Shift
// + the printed key) matches. Keys whose shifted symbol has NO egui variant
// (`,`→`<`, `-`→`_`, digits 2–9 and 0) fall back to their physical key and
// already match, so they pass through unchanged. macOS keeps the base key (its
// primary tier is bare ⌘, no Shift). Assumes a US/ANSI layout.
#[cfg(target_os = "macos")]
fn shift_variant(base: egui::Key, _shifted: egui::Key) -> egui::Key { base }
#[cfg(not(target_os = "macos"))]
fn shift_variant(_base: egui::Key, shifted: egui::Key) -> egui::Key { shifted }

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            quit:                      (cmd(),       egui::Key::Q),
            close_pane:                (cmd(),       egui::Key::W),
            toggle_command_palette:    (cmd(),       egui::Key::P),
            split_horizontal:          (cmd(),       egui::Key::D),
            split_vertical:            (cmd_shift(), egui::Key::D),
            split_right:               (cmd(),       shift_variant(egui::Key::Backslash, egui::Key::Pipe)),
            split_down:                (cmd_shift(), shift_variant(egui::Key::Backslash, egui::Key::Pipe)),
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
            nav_back:                  (cmd(),       shift_variant(egui::Key::OpenBracket, egui::Key::OpenCurlyBracket)),
            focus_history_forward:     (cmd(),       shift_variant(egui::Key::CloseBracket, egui::Key::CloseCurlyBracket)),
            toggle_sidebar:            (cmd(),       egui::Key::B),
            toggle_zoom:               (cmd(),       egui::Key::Enter),
            toggle_shortcuts:          (cmd(),       shift_variant(egui::Key::Slash, egui::Key::Questionmark)),
            rename_context:            (cmd_shift(), egui::Key::R),
            rename_pane:               (cmd(),       egui::Key::R),
            new_context:               (cmd_shift(), egui::Key::N),
            new_page_right:            (cmd(),       egui::Key::N),
            toggle_minimap:            (cmd_shift(), egui::Key::M),
            scroll_up:                 (cmd(),       egui::Key::ArrowUp),
            scroll_down:               (cmd(),       egui::Key::ArrowDown),
            increase_font_size:        (cmd(),       shift_variant(egui::Key::Equals, egui::Key::Plus)),
            decrease_font_size:        (cmd(),       egui::Key::Minus),
            open_file_browser:         (cmd(),       egui::Key::E),
            open_quick_note:           (cmd(),       egui::Key::Num0),
            open_config:               (cmd(),       egui::Key::Comma),
            reload_config:             (cmd_shift(), egui::Key::Comma),
            open_secrets_manager:      (cmd_shift(), egui::Key::S),
            force_reload_app:          (cmd_alt(),   egui::Key::R),
            toggle_notification_modal: (cmd_shift(), egui::Key::A),
            context_inspector:         (cmd(),       egui::Key::I),
            open_scratchpad:           (cmd_shift(), egui::Key::Space),
            context_zoom_in:           (cmd_shift(), egui::Key::Enter),
            context_zoom_out:          (cmd(),       egui::Key::Escape),
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
    apply_override!(context_inspector, "context_inspector");
    apply_override!(open_scratchpad, "open_scratchpad");

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
        ("context_inspector",         bindings.context_inspector),
        ("open_scratchpad",           bindings.open_scratchpad),
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
    notification_modal_active: bool,
    notification_shortcuts_blocked: bool,
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

        // Tab navigation — the most-specific modifier tier (Cmd+Shift on macOS,
        // Ctrl+Shift+Alt elsewhere). Checked first because egui's consume_key uses
        // subset modifier matching: a less-specific pattern (the Cmd / Ctrl+Shift
        // navigation chord, or the Cmd+Ctrl / Ctrl+Alt swap chord) also matches
        // this press, so the more-specific variant must be consumed first.
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

        // Pane swap (Cmd+Ctrl on macOS, Ctrl+Alt elsewhere). On non-macOS the
        // Ctrl+Alt chord is a subset of the tabs' Ctrl+Shift+Alt chord, so this
        // MUST come after the tab checks above — otherwise a tab chord would be
        // consumed here first. Still checked before plain navigation below.
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

        // Focus navigation — least-specific tier (Cmd / Ctrl+Shift), checked last.
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

        // Bare H/L cycle the notification queue when the modal is focused.
        // Restricted to the notification modal specifically (not other overlays).
        // Blocked when the active notification is Choice or Input — those kinds
        // need H/L as per-option shortcut keys or free text input.
        // modifiers.is_none() guard is required: consume_key(NONE, key) matches
        // regardless of modifiers (see GOTCHA at top of file), so Shift+H or
        // Cmd+H must not accidentally trigger cycling.
        if notification_modal_active && !notification_shortcuts_blocked {
            if input.modifiers.is_none() && input.consume_key(egui::Modifiers::NONE, egui::Key::H) {
                log::info!("notification cycle: prev (H)");
                actions.push(Action::NotificationCyclePrev);
            }
            if input.modifiers.is_none() && input.consume_key(egui::Modifiers::NONE, egui::Key::L) {
                log::info!("notification cycle: next (L)");
                actions.push(Action::NotificationCycleNext);
            }
        }

        if input.consume_key(bindings.toggle_sidebar.0, bindings.toggle_sidebar.1) {
            actions.push(Action::ToggleSidebar);
        }
        // context_zoom_in (Cmd+Shift+Enter) must be checked before toggle_zoom (Cmd+Enter)
        // because egui consume_key uses subset modifier matching.
        if input.consume_key(bindings.context_zoom_in.0, bindings.context_zoom_in.1) {
            actions.push(Action::ContextZoomIn);
        } else if input.consume_key(bindings.toggle_zoom.0, bindings.toggle_zoom.1) {
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
        // Reload config (Cmd+Shift+, / Ctrl+Alt+,) before open config (Cmd+, /
        // Ctrl+Shift+,) — more-specific modifier tier first, since the open-config
        // pattern also matches the reload press under subset matching.
        if input.consume_key(bindings.reload_config.0, bindings.reload_config.1) {
            actions.push(Action::ReloadConfig);
        } else if input.consume_key(bindings.open_config.0, bindings.open_config.1) {
            actions.push(Action::OpenConfig);
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
        if input.consume_key(bindings.context_inspector.0, bindings.context_inspector.1) {
            actions.push(Action::ContextInspector);
        }
        if input.consume_key(bindings.open_scratchpad.0, bindings.open_scratchpad.1) {
            actions.push(Action::OpenScratchpad);
        }
        if input.consume_key(bindings.context_zoom_out.0, bindings.context_zoom_out.1) {
            actions.push(Action::ContextZoomOut);
        }

        // Switch context (Cmd+1–9 on macOS, Ctrl+Shift+1–9 elsewhere). Not yet
        // configurable, but uses the same primary-modifier tier as the other
        // defaults via `cmd()` so it stays off the bare-Ctrl terminal namespace.
        let switch_context_mod = cmd();
        // Num1 → `!` under Shift on non-mac (has an egui variant); 2–9 fall back
        // to their physical key and match as-is. See `shift_variant`.
        let num_keys = [
            shift_variant(egui::Key::Num1, egui::Key::Exclamationmark),
            egui::Key::Num2, egui::Key::Num3,
            egui::Key::Num4, egui::Key::Num5, egui::Key::Num6,
            egui::Key::Num7, egui::Key::Num8, egui::Key::Num9,
        ];
        for (i, key) in num_keys.into_iter().enumerate() {
            if input.consume_key(switch_context_mod, key) {
                actions.push(Action::SwitchContext(i));
            }
        }
    });

    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a key-press event carrying the given modifier state.
    fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    /// Run `poll_actions` for one headless frame fed `events`, with no app or
    /// overlay active and notification shortcuts blocked (so bare H/L cycling
    /// stays off). Returns the actions the host would fire for those presses.
    fn poll(events: Vec<egui::Event>) -> Vec<Action> {
        let ctx = egui::Context::default();
        let raw = egui::RawInput { events, ..Default::default() };
        let mut out = Vec::new();
        let _ = ctx.run(raw, |ctx| {
            out = poll_actions(
                ctx,
                &KeyBindings::default(),
                /* app_active */ false,
                /* keyboard_capture_active */ false,
                /* overlay_open */ false,
                /* shortcuts_overlay_open */ false,
                /* notification_modal_active */ false,
                /* notification_shortcuts_blocked */ true,
            );
        });
        out
    }

    fn any(actions: &[Action], pred: impl Fn(&Action) -> bool) -> bool {
        actions.iter().any(pred)
    }

    // egui reports `command == ctrl` on Windows/Linux (no dedicated ⌘ key).
    #[cfg(not(target_os = "macos"))]
    fn win_mods(ctrl: bool, shift: bool, alt: bool) -> egui::Modifiers {
        egui::Modifiers { alt, ctrl, shift, mac_cmd: false, command: ctrl }
    }

    /// The whole reason for the non-mac scheme: bare Ctrl+<key> must reach the
    /// focused terminal (neovim, readline, …) instead of being eaten by a host
    /// shortcut. Ctrl+I in particular is the terminal's Tab.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn bare_ctrl_falls_through_to_terminal() {
        let bare_ctrl = win_mods(true, false, false);
        for key in [
            egui::Key::I, egui::Key::R, egui::Key::W, egui::Key::P, egui::Key::D,
            egui::Key::T, egui::Key::H, egui::Key::J, egui::Key::K, egui::Key::L,
            egui::Key::B, egui::Key::N, egui::Key::E, egui::Key::A, egui::Key::S,
            egui::Key::M, egui::Key::Q, egui::Key::OpenBracket, egui::Key::CloseBracket,
            egui::Key::Backslash, egui::Key::Slash, egui::Key::Num1, egui::Key::Num9,
        ] {
            let actions = poll(vec![press(key, bare_ctrl)]);
            assert!(
                actions.is_empty(),
                "bare Ctrl+{key:?} should fall through to the terminal, got {} host action(s)",
                actions.len()
            );
        }
    }

    /// Primary host shortcuts move to Ctrl+Shift on non-mac.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn ctrl_shift_triggers_primary_host_shortcuts() {
        let m = win_mods(true, true, false); // Ctrl+Shift
        assert!(any(&poll(vec![press(egui::Key::I, m)]), |a| matches!(a, Action::ContextInspector)));
        assert!(any(&poll(vec![press(egui::Key::W, m)]), |a| matches!(a, Action::ClosePane)));
        assert!(any(&poll(vec![press(egui::Key::P, m)]), |a| matches!(a, Action::ToggleCommandPalette)));
        assert!(any(&poll(vec![press(egui::Key::R, m)]), |a| matches!(a, Action::RenamePane)));
        assert!(any(&poll(vec![press(egui::Key::H, m)]), |a| matches!(a, Action::Navigate(Direction::Left))));
    }

    /// swap_pane (Ctrl+Alt) is a modifier subset of the tab chord
    /// (Ctrl+Shift+Alt), so ordering must resolve the tab chord to a tab action,
    /// not a pane swap — and vice versa.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn tab_chord_not_swallowed_by_swap_pane() {
        let tab = win_mods(true, true, true); // Ctrl+Shift+Alt+H → prev_tab
        let acts = poll(vec![press(egui::Key::H, tab)]);
        assert!(any(&acts, |a| matches!(a, Action::PrevTab)), "Ctrl+Shift+Alt+H should be PrevTab");
        assert!(!any(&acts, |a| matches!(a, Action::SwapPane(_))), "tab chord must not trigger SwapPane");

        let swap = win_mods(true, false, true); // Ctrl+Alt+H → swap_pane_left
        let acts = poll(vec![press(egui::Key::H, swap)]);
        assert!(any(&acts, |a| matches!(a, Action::SwapPane(Direction::Left))), "Ctrl+Alt+H should be SwapPane(Left)");
        assert!(!any(&acts, |a| matches!(a, Action::PrevTab | Action::Navigate(_))), "swap chord must not trigger tab/nav");
    }

    /// open_config (Ctrl+Shift+,) is a subset of reload_config (Ctrl+Shift+Alt+,),
    /// so reload must be checked first.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn reload_config_not_swallowed_by_open_config() {
        let reload = win_mods(true, true, true); // Ctrl+Shift+Alt+,
        let acts = poll(vec![press(egui::Key::Comma, reload)]);
        assert!(any(&acts, |a| matches!(a, Action::ReloadConfig)));
        assert!(!any(&acts, |a| matches!(a, Action::OpenConfig)));

        let open = win_mods(true, true, false); // Ctrl+Shift+,
        let acts = poll(vec![press(egui::Key::Comma, open)]);
        assert!(any(&acts, |a| matches!(a, Action::OpenConfig)));
        assert!(!any(&acts, |a| matches!(a, Action::ReloadConfig)));
    }

    /// Shift mutates punctuation/digit logical keys on non-mac (egui reports the
    /// shifted variant). Bindings must match the variant the keypress produces,
    /// e.g. Ctrl+Shift+`/` arrives as `Questionmark`, Ctrl+Shift+`\` as `Pipe`.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn ctrl_shift_punctuation_matches_shifted_keys() {
        let m = win_mods(true, true, false); // Ctrl+Shift
        assert!(any(&poll(vec![press(egui::Key::Questionmark, m)]), |a| matches!(a, Action::ToggleShortcuts)), "Ctrl+Shift+/ (?)");
        assert!(any(&poll(vec![press(egui::Key::Pipe, m)]), |a| matches!(a, Action::SplitRight)), "Ctrl+Shift+\\ (|)");
        assert!(any(&poll(vec![press(egui::Key::OpenCurlyBracket, m)]), |a| matches!(a, Action::NavBackApp)), "Ctrl+Shift+open-bracket");
        assert!(any(&poll(vec![press(egui::Key::CloseCurlyBracket, m)]), |a| matches!(a, Action::FocusHistoryForward)), "Ctrl+Shift+close-bracket");
        assert!(any(&poll(vec![press(egui::Key::Plus, m)]), |a| matches!(a, Action::IncreasePaneFontSize)), "Ctrl+Shift+= (+)");
        assert!(any(&poll(vec![press(egui::Key::Exclamationmark, m)]), |a| matches!(a, Action::SwitchContext(0))), "Ctrl+Shift+1 (!)");

        // split_down is the Ctrl+Shift+Alt tier on the same shifted key (|).
        let acts = poll(vec![press(egui::Key::Pipe, win_mods(true, true, true))]);
        assert!(any(&acts, |a| matches!(a, Action::SplitDown)), "Ctrl+Shift+Alt+\\ (|) → SplitDown");
        assert!(!any(&acts, |a| matches!(a, Action::SplitRight)), "must not also fire SplitRight");

        // Keys with no shifted egui variant still match their base key (physical
        // fallback): Ctrl+Shift+, stays Comma, Ctrl+Shift+2 stays Num2.
        assert!(any(&poll(vec![press(egui::Key::Comma, m)]), |a| matches!(a, Action::OpenConfig)), "Ctrl+Shift+, → OpenConfig");
        assert!(any(&poll(vec![press(egui::Key::Num2, m)]), |a| matches!(a, Action::SwitchContext(1))), "Ctrl+Shift+2 → context 2");
    }

    // egui reports `command == mac_cmd` for ⌘ on macOS; Ctrl is independent.
    #[cfg(target_os = "macos")]
    fn mac_mods(cmd: bool, ctrl: bool, shift: bool, alt: bool) -> egui::Modifiers {
        egui::Modifiers { alt, ctrl, shift, mac_cmd: cmd, command: cmd }
    }

    /// macOS keeps the Cmd-based scheme: Cmd+I is the host shortcut, and bare
    /// Ctrl+I still falls through to the terminal.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_cmd_and_leaves_ctrl_for_terminal() {
        let cmd_i = poll(vec![press(egui::Key::I, mac_mods(true, false, false, false))]);
        assert!(any(&cmd_i, |a| matches!(a, Action::ContextInspector)));

        let ctrl_i = poll(vec![press(egui::Key::I, mac_mods(false, true, false, false))]);
        assert!(ctrl_i.is_empty(), "bare Ctrl+I must fall through to the terminal on macOS too");
    }
}
