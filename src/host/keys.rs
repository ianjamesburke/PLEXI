use crate::app::registry::RegistrySource;
use crate::cli::UserCommandScope;
use crate::config::KeybindingsConfig;

// The command registry defines workspace shortcuts. The input router claims
// those chords before a focused view receives them. Local view input still
// owns text editing and the bare Escape fallback.
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
    SelectPane(Direction),
    CancelPaneSelection,
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
    OpenChangelog,
    ToggleZoom,
    ToggleCommandPalette,
    RenamePane,
    /// Increase the focused pane's text size.
    IncreasePaneFontSize,
    /// Decrease the focused pane's text size.
    DecreasePaneFontSize,
    /// Reset the focused pane's text size to default.
    ResetPaneFontSize,
    /// App-wide UI scale (browser-style zoom): terminal text, UI text, spacing.
    UiScaleUp,
    UiScaleDown,
    UiScaleReset,
    ScrollUp,
    ScrollDown,
    /// Dismiss the active plugin surface and return to full terminal.
    CloseView,
    /// Open the file browser plugin in the focused terminal.
    OpenFileBrowser,
    OpenAgents,
    NextBlockedAgent,
    FocusApproval,
    /// Open the quick note plugin (full pane, no terminal split).
    OpenQuickNote,
    /// Open config file in the text editor.
    OpenConfig,
    /// Open the secrets manager (read-only vault viewer).
    OpenSecretsManager,
    /// Open (or focus) the host Assistant pane. Bound to Cmd+Ctrl+A.
    OpenAssistant,
    /// Rename the active context. Bound to Cmd+Shift+R.
    RenameContext,
    /// Toggle the attention queue list (Cmd+I).
    ToggleAttentionQueue,
    /// Zoom in one level on the zoom ladder (Cmd+Enter).
    ZoomIn,
    /// Force-reload the focused view pane (#83). Bound to Cmd+Option+R.
    /// Cmd+R is RenamePane and Cmd+Shift+R is RenameContext, so the
    /// Option (Alt) modifier is the next free chord. No-op when the
    /// focused pane isn't a process-backed plugin.
    ForceReloadView,
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
    /// Pop plugin navigation, falling back to focus history. Unbound by default.
    NavBackView,
    /// Step forward through pane focus history. Unbound by default.
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
    /// Park/unpark the focused context. Bound to Cmd+Shift+U.
    ParkContext,
    /// Open the notes picker overlay from any pane type. Bound to Cmd+O.
    OpenNotesPicker,
    /// Close the active context with a confirm dialog. Bound to Cmd+Shift+W.
    CloseContext,
    OpenGlobalConfig,
    OpenWorkspaceConfig,
    StatePreset(&'static str),
    EditCommands,
    SwitchCommandsScope,
    OpenAttention,
    SetupCli,
    UiUpcycle(&'static str),
    CheckForUpdates,
    RestartToUpdate,
    CloseSelectedPanes,
    /// Open the installed app from the invoking workspace's registry view.
    LaunchApp(String, RegistrySource),
    /// Run a user-authored command from `commands.toml` (bound via `[keys]` `"<chord>" = "run:<name>"`).
    /// A discovered row also carries the scope of the definition it displayed.
    RunUserCommand(String, Option<UserCommandScope>),
}

const fn cmd() -> egui::Modifiers {
    egui::Modifiers::COMMAND
}
const fn cmd_shift() -> egui::Modifiers {
    egui::Modifiers {
        shift: true,
        ..egui::Modifiers::COMMAND
    }
}
const fn cmd_ctrl() -> egui::Modifiers {
    egui::Modifiers {
        ctrl: true,
        ..egui::Modifiers::COMMAND
    }
}
const fn cmd_alt() -> egui::Modifiers {
    egui::Modifiers {
        alt: true,
        ..egui::Modifiers::COMMAND
    }
}
const fn cmd_shift_alt() -> egui::Modifiers {
    egui::Modifiers {
        shift: true,
        alt: true,
        ..egui::Modifiers::COMMAND
    }
}
const fn cmd_ctrl_alt() -> egui::Modifiers {
    egui::Modifiers {
        ctrl: true,
        alt: true,
        ..egui::Modifiers::COMMAND
    }
}
const fn cmd_shift_ctrl() -> egui::Modifiers {
    egui::Modifiers {
        shift: true,
        ctrl: true,
        ..egui::Modifiers::COMMAND
    }
}
#[path = "command_registry.rs"]
mod command_registry;
pub(crate) use command_registry::{commands, discover_commands, DiscoveredCommand};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandTarget {
    Host,
    Context,
    Window,
    Pane,
    Selection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandArguments {
    None,
    Text(&'static str),
    Picker(&'static str),
    Choice(&'static str),
}

#[derive(Clone, Copy)]
pub(crate) enum CommandRequirement {
    Always,
    Workspace,
    Selection,
    Update,
    Assistant,
}

/// The host's declaration for a visible action, including the existing entry UI.
pub(crate) struct PaletteCommandEntry {
    pub id: &'static str,
    pub command: Action,
    pub name: &'static str,
    pub description: &'static str,
    pub search_text: &'static str,
    pub target: CommandTarget,
    pub arguments: CommandArguments,
    pub requirement: CommandRequirement,
    palette_rank: u8,
}

struct CommandBinding {
    id: &'static str,
    aliases: &'static [&'static str],
    command: Action,
    default: Option<Chord>,
    chord: fn(&KeyBindings) -> Option<Chord>,
    context: BindingContext,
    exact: bool,
    no_repeat: bool,
    visibility: CommandVisibility,
}

enum CommandVisibility {
    Show(PaletteCommandEntry),
    Except {
        label: &'static str,
        reason: &'static str,
    },
}

type Chord = (egui::Modifiers, egui::Key);

macro_rules! declare_bindings {
    ($($id:ident [$($alias:literal),*]: $mode:ident $default:tt => $action:expr, $context:ident, $exact:literal, $no_repeat:literal;
        $visibility:ident($($spec:tt)*);)*) => {
        /// Effective host bindings rebuilt on configuration reload.
        #[derive(Clone)]
        pub struct KeyBindings {
            $(pub $id: declare_bindings!(@type $mode),)*
            pub user_commands: Vec<(Chord, String)>,
        }

        impl Default for KeyBindings {
            fn default() -> Self {
                Self {
                    $($id: $default,)*
                    user_commands: Vec::new(),
                }
            }
        }

        static COMMAND_BINDINGS: &[CommandBinding] = &[
            $(CommandBinding {
                id: stringify!($id),
                aliases: &[$($alias),*],
                command: $action,
                default: declare_bindings!(@chord $mode $default),
                chord: |bindings| declare_bindings!(@chord $mode bindings.$id),
                context: BindingContext::$context,
                exact: $exact,
                no_repeat: $no_repeat,
                visibility: declare_bindings!(@visibility $id $action, $visibility($($spec)*)),
            },)*
        ];

        impl KeyBindings {
            fn apply_overrides(&mut self, overrides: &KeybindingsConfig) {
                $(if let Some(value) = overrides.$id.as_deref() {
                    match parse_chord(value) {
                        Some(chord) => self.$id = declare_bindings!(@chord override $mode chord),
                        None => log::warn!("keybindings: invalid chord for '{}'; keeping default", stringify!($id)),
                    }
                })*
            }
        }
    };
    (@type default) => { Chord };
    (@type optional) => { Option<Chord> };
    (@chord default $value:expr) => { Some($value) };
    (@chord optional $value:expr) => { $value };
    (@chord override default $value:expr) => { $value };
    (@chord override optional $value:expr) => { Some($value) };
    (@visibility $id:ident $action:expr, Show($name:literal, $description:literal, $search:literal, $target:ident, $kind:ident $(($arg:literal))? $(, $requirement:ident)? $(; $rank:literal)?)) => {
        CommandVisibility::Show(PaletteCommandEntry {
            id: stringify!($id),
            command: $action,
            name: $name,
            description: $description,
            search_text: $search,
            target: CommandTarget::$target,
            arguments: CommandArguments::$kind $(($arg))?,
            requirement: declare_bindings!(@require $($requirement)?),
            palette_rank: declare_bindings!(@rank $($rank)?),
        })
    };
    (@rank) => { u8::MAX };
    (@rank $rank:literal) => { $rank };
    (@require) => { CommandRequirement::Always };
    (@require $requirement:ident) => { CommandRequirement::$requirement };
    (@visibility $id:ident $action:expr, Except($label:literal, $reason:literal)) => {
        CommandVisibility::Except { label: $label, reason: $reason }
    };
}
commands!(declare_bindings);

pub(crate) fn palette_commands() -> impl Iterator<Item = &'static PaletteCommandEntry> {
    static IN_PALETTE_ORDER: std::sync::LazyLock<Vec<&'static PaletteCommandEntry>> =
        std::sync::LazyLock::new(|| {
            let mut commands: Vec<_> = COMMAND_BINDINGS
                .iter()
                .filter_map(|entry| match &entry.visibility {
                    CommandVisibility::Show(command) => Some(command),
                    CommandVisibility::Except { reason, .. } => {
                        debug_assert!(!reason.is_empty());
                        None
                    }
                })
                .collect();
            commands.sort_by_key(|entry| entry.palette_rank);
            commands
        });
    IN_PALETTE_ORDER.iter().copied()
}

impl Action {
    pub(crate) fn palette_entry(&self) -> Option<&'static PaletteCommandEntry> {
        palette_commands().find(|entry| entry.command == *self)
    }

    pub(crate) fn command_label(&self) -> Option<&'static str> {
        let entry = COMMAND_BINDINGS
            .iter()
            .find(|entry| entry.command == *self)?;
        Some(match &entry.visibility {
            CommandVisibility::Show(command) => command.name,
            CommandVisibility::Except { label, .. } => label,
        })
    }

    pub(crate) fn command_id(&self) -> std::borrow::Cow<'static, str> {
        if let Some(id) = self.dynamic_command_id() {
            return id.into();
        }
        COMMAND_BINDINGS
            .iter()
            .find(|entry| entry.command == *self)
            .map_or_else(|| self.non_registry_action().0, |entry| entry.id)
            .into()
    }

    pub(crate) fn dynamic_command_id(&self) -> Option<String> {
        match self {
            Self::LaunchApp(id, _) => Some(format!("view/{id}")),
            Self::RunUserCommand(name, _) => Some(format!("run/{name}")),
            _ => None,
        }
    }

    fn non_registry_action(&self) -> (&'static str, &'static str) {
        match self {
            Self::CloseView => ("close_view", "Bare Escape is a focused-view fallback after text input and modal dismissal."),
            _ => ("unregistered", ""),
        }
    }
}

/// Parse a key combo string like `"cmd+shift+d"` into `(Modifiers, Key)`.
/// Returns `None` and logs a warning if the string is malformed or the key is unknown.
pub(crate) fn parse_chord(s: &str) -> Option<(egui::Modifiers, egui::Key)> {
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

/// Build effective bindings from defaults and optional config overrides.
pub fn build_key_bindings(overrides: Option<&KeybindingsConfig>) -> KeyBindings {
    let mut bindings = KeyBindings::default();
    if let Some(config) = overrides {
        bindings.apply_overrides(config);
        for conflict in bindings.conflicts() {
            log::warn!("keybindings: {conflict}");
        }
    }
    bindings
}

fn reserved_host_shortcuts(bindings: &KeyBindings) -> std::collections::HashMap<u64, &'static str> {
    let mut reserved = std::collections::HashMap::new();
    for entry in COMMAND_BINDINGS {
        for (mods, key) in [entry.default, (entry.chord)(bindings)]
            .into_iter()
            .flatten()
        {
            reserved.insert(binding_id(mods, key), entry.id);
        }
    }
    reserved.insert(
        binding_id(egui::Modifiers::NONE, egui::Key::Escape),
        "close_view",
    );
    reserved
}

impl KeyBindings {
    pub(crate) fn chord_for_action(&self, action: &Action) -> Option<Chord> {
        if let Action::RunUserCommand(name, _) = action {
            return self
                .user_commands
                .iter()
                .find_map(|(chord, bound_name)| (bound_name == name).then_some(*chord));
        }
        COMMAND_BINDINGS
            .iter()
            .find(|entry| entry.command == *action)
            .and_then(|entry| (entry.chord)(self))
    }

    pub(crate) fn chord_for_id(&self, id: &str) -> Option<Chord> {
        COMMAND_BINDINGS
            .iter()
            .find(|entry| entry.id == id || entry.aliases.contains(&id))
            .and_then(|entry| (entry.chord)(self))
    }

    pub(crate) fn conflicts(&self) -> Vec<String> {
        let mut seen = std::collections::HashMap::new();
        let mut conflicts = Vec::new();
        for entry in COMMAND_BINDINGS {
            if let Some((mods, key)) = (entry.chord)(self) {
                if let Some(first) = seen.get(&binding_id(mods, key)) {
                    conflicts.push(format!(
                        "'{}' and '{first}' share the same key combo",
                        entry.id
                    ));
                } else {
                    seen.insert(binding_id(mods, key), entry.id);
                }
            }
        }
        conflicts
    }
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
        let Some((mods, key)) = parse_chord(chord) else {
            errors.push(format!("invalid key chord `{chord}` in [keys]"));
            continue;
        };
        let Some(cmd_name) = target
            .strip_prefix("run:")
            .map(str::trim)
            .filter(|s| !s.is_empty())
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
            let Some((mods, key)) = parse_chord(chord) else {
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
    /// Active only when no overlay holds focus and no plugin has keyboard capture.
    Normal,
    /// Active only when a plugin surface is focused AND no overlay/capture is active.
    ViewActive,
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

// egui::Modifiers::COMMAND sets mac_cmd=false, but macOS runtime input also
// sets mac_cmd=true when Cmd is held. Treat those two fields as one logical
// modifier when matching exact chords; raw field equality would reject a real
// macOS Cmd key event.
// On non-macOS (Linux/Windows), egui sets both `ctrl` and `command` to true
// when physical Ctrl is held, while `cmd()` patterns set `command=true, ctrl=false`.
pub(crate) fn modifiers_match_exact_for_os(
    actual: &egui::Modifiers,
    pattern: &egui::Modifiers,
    is_macos: bool,
) -> bool {
    if is_macos {
        (actual.command || actual.mac_cmd) == (pattern.command || pattern.mac_cmd)
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
    let mut entries: Vec<_> = COMMAND_BINDINGS
        .iter()
        .filter_map(|entry| {
            let (modifiers, key) = (entry.chord)(b)?;
            Some(BindingEntry {
                modifiers,
                key,
                exact: entry.exact,
                no_repeat: entry.no_repeat,
                context: entry.context,
                action: entry.command.clone(),
            })
        })
        .collect();
    // Bare Escape is the focused-view fallback, after the view sees the key.
    entries.push(BindingEntry {
        modifiers: egui::Modifiers::NONE,
        key: egui::Key::Escape,
        exact: false,
        no_repeat: false,
        context: BindingContext::ViewActive,
        action: Action::CloseView,
    });

    for ((modifiers, key), cmd_name) in &b.user_commands {
        entries.push(BindingEntry {
            modifiers: *modifiers,
            key: *key,
            exact: true,
            no_repeat: true,
            context: BindingContext::Normal,
            action: Action::RunUserCommand(cmd_name.clone(), None),
        });
    }

    // Physical Equals and Plus companion bindings: if a chord is bound to Equals,
    // also accept Plus with the same modifiers (and vice versa) for layout / numpad parity.
    let mut companions = Vec::new();
    for entry in &entries {
        if matches!(
            entry.action,
            Action::IncreasePaneFontSize | Action::UiScaleUp
        ) {
            let alt_key = match entry.key {
                egui::Key::Equals => Some(egui::Key::Plus),
                egui::Key::Plus => Some(egui::Key::Equals),
                _ => None,
            };
            if let Some(key) = alt_key {
                companions.push(BindingEntry {
                    modifiers: entry.modifiers,
                    key,
                    exact: entry.exact,
                    no_repeat: entry.no_repeat,
                    context: entry.context,
                    action: entry.action.clone(),
                });
            }
        }
    }
    entries.extend(companions);

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
/// `view_active` — focused pane has an active plugin surface (affects Escape/Tab).
/// `keyboard_capture_active` — the focused pane's runtime reports `keyboard_capture()`.
///   When true, all host shortcuts are suppressed *except* Global-context bindings
///   (Quit, ClosePane, CommandPalette, QuickNote, ToggleNotificationModal).
/// `overlay_open` — an overlay owns keyboard focus. Same suppression as `keyboard_capture_active`.
///   Each overlay's `*_handle_key` method owns its own key contract and runs before this function.
/// `shortcuts_overlay_open` — the shortcuts overlay is open; suppresses CloseView (Escape)
///   so the overlay can own Escape for dismissal.
///
/// `input` is a [`crate::app::input_router::PlexiInput`] buffer already taken
/// out of `ctx` for this frame (stint 0240's ownership-transfer router).
/// The host calls this after the focused overlay owner in two passes: Global
/// and Normal bindings before focused-plugin key dispatch, then ViewActive
/// bindings afterward. Thus host chords are structurally removed before a
/// guest runtime can see them, while a plugin or focused text field still owns
/// bare Escape before CloseView.
/// The result retains each action's original input position. The dispatcher
/// captures its target after applying preceding actions in delivery order.
pub fn poll_actions(
    input: &mut crate::app::input_router::PlexiInput,
    table: &[BindingEntry],
    claimed_key_releases: &mut std::collections::HashSet<egui::Key>,
    view_active: bool,
    keyboard_capture_active: bool,
    overlay_open: bool,
    shortcuts_overlay_open: bool,
) -> Vec<(usize, Action)> {
    let mut actions = Vec::new();

    // Resolve the most specific binding for each press in delivery order.
    // The table's order breaks ties for a single event.
    input.retain_positioned_events(|position, event| {
        let egui::Event::Key {
            key,
            pressed,
            repeat,
            modifiers,
            ..
        } = event
        else {
            return true;
        };
        if !pressed {
            // A key-up can arrive with no modifiers after Command goes up.
            return !claimed_key_releases.remove(key);
        }

        let entry = table.iter().find(|entry| {
            if entry.key != *key || (entry.no_repeat && *repeat) {
                return false;
            }
            let active = match entry.context {
                BindingContext::Global => true,
                BindingContext::Normal => !keyboard_capture_active && !overlay_open,
                BindingContext::ViewActive => {
                    view_active
                        && !keyboard_capture_active
                        && !overlay_open
                        && (!shortcuts_overlay_open || !matches!(entry.action, Action::CloseView))
                }
            };
            active
                && if entry.exact {
                    modifiers_match_exact_for_os(
                        modifiers,
                        &entry.modifiers,
                        cfg!(target_os = "macos"),
                    )
                } else {
                    modifiers.matches_logically(entry.modifiers)
                }
        });
        let Some(entry) = entry else {
            return true;
        };
        claimed_key_releases.insert(*key);
        match key {
            egui::Key::Equals => {
                claimed_key_releases.insert(egui::Key::Plus);
            }
            egui::Key::Plus => {
                claimed_key_releases.insert(egui::Key::Equals);
            }
            _ => {}
        }
        if !matches!(
            entry.action,
            Action::IncreasePaneFontSize
                | Action::DecreasePaneFontSize
                | Action::ResetPaneFontSize
                | Action::UiScaleUp
                | Action::UiScaleDown
                | Action::UiScaleReset
        ) || !actions.iter().any(|(_, action)| action == &entry.action)
        {
            actions.push((position, entry.action.clone()));
        }
        false
    });

    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_command_is_bindable() {
        // Keep an explicit inventory so a new row requires review of its
        // stable ID. For each ID exercise config parsing and the event table.
        let expected = [
            "cancel_pane_selection",
            "check_for_updates",
            "close_context",
            "close_pane",
            "close_selected_panes",
            "context_zoom_out",
            "decrease_font_size",
            "edit_commands",
            "focus_approval",
            "focus_history_forward",
            "force_reload_view",
            "increase_font_size",
            "move_context_down",
            "move_context_up",
            "nav_back",
            "navigate_down",
            "navigate_left",
            "navigate_right",
            "navigate_up",
            "new_child_context",
            "new_context",
            "new_page_right",
            "new_tab",
            "next_blocked_agent",
            "next_context",
            "next_tab",
            "open_agents",
            "open_assistant",
            "open_attention",
            "open_changelog",
            "open_config",
            "open_file_browser",
            "open_global_config",
            "open_notes_picker",
            "open_quick_note",
            "open_scratchpad",
            "open_secrets_manager",
            "open_workspace_config",
            "park_context",
            "prev_context",
            "prev_tab",
            "push_to_subcontext",
            "quit",
            "reload_config",
            "rename_context",
            "rename_pane",
            "reset_font_size",
            "restart_to_update",
            "scroll_down",
            "scroll_up",
            "select_pane_down",
            "select_pane_left",
            "select_pane_right",
            "select_pane_up",
            "send_pane_down",
            "send_pane_left",
            "send_pane_right",
            "send_pane_up",
            "set_context_root_from_cwd",
            "setup_cli",
            "split_down",
            "split_horizontal",
            "split_right",
            "split_vertical",
            "state_preset_default",
            "state_preset_high_contrast",
            "state_preset_mono",
            "state_preset_okabe_ito",
            "swap_pane_down",
            "swap_pane_left",
            "swap_pane_right",
            "swap_pane_up",
            "switch_commands_scope",
            "switch_context_1",
            "switch_context_2",
            "switch_context_3",
            "switch_context_4",
            "switch_context_5",
            "switch_context_6",
            "switch_context_7",
            "switch_context_8",
            "switch_context_9",
            "toggle_attention_queue",
            "toggle_minimap",
            "toggle_shortcuts",
            "toggle_sidebar",
            "toggle_zoom",
            "ui_scale_down",
            "ui_scale_reset",
            "ui_scale_up",
            "ui_upcycle_cargo",
            "ui_upcycle_gh",
            "ui_upcycle_git",
            "zoom_in",
        ];
        let mut actual: Vec<_> = palette_commands().map(|entry| entry.id).collect();
        actual.sort_unstable();
        assert_eq!(actual, expected);

        for descriptor in palette_commands() {
            let config: crate::config::PlexiConfig =
                toml::from_str(&format!("[keybindings]\n{} = 'cmd+alt+c'", descriptor.id)).unwrap();
            let bindings = build_key_bindings(config.keybindings.as_ref());
            let chord = (cmd_alt(), egui::Key::C);
            assert_eq!(bindings.chord_for_action(&descriptor.command), Some(chord));
            assert!(
                build_binding_table(&bindings).iter().any(|entry| {
                    entry.action == descriptor.command && (entry.modifiers, entry.key) == chord
                }),
                "{} must resolve into keyboard dispatch",
                descriptor.id
            );
        }
    }

    fn uncovered_action_variants(source: &str) -> Vec<String> {
        let file = syn::parse_file(source).unwrap();
        let actions = file
            .items
            .into_iter()
            .find_map(|item| match item {
                syn::Item::Enum(item) if item.ident == "Action" => Some(item.variants),
                _ => None,
            })
            .unwrap();
        let exceptions = [Action::CloseView];
        let dynamic = [
            Action::LaunchApp("sample".into(), RegistrySource::Global),
            Action::RunUserCommand("sample".into(), Some(UserCommandScope::Global)),
        ];
        assert!(dynamic
            .iter()
            .all(|action| action.dynamic_command_id().is_some()));
        let mut covered = std::collections::HashSet::new();
        for action in COMMAND_BINDINGS
            .iter()
            .map(|entry| &entry.command)
            .chain(&exceptions)
            .chain(&dynamic)
        {
            if exceptions.contains(action) {
                let (id, reason) = action.non_registry_action();
                assert!(
                    !id.is_empty() && !reason.is_empty(),
                    "{action:?} needs a reason"
                );
            }
            let debug = format!("{action:?}");
            covered.insert(debug.split('(').next().unwrap().to_owned());
        }
        actions
            .into_iter()
            .map(|variant| variant.ident.to_string())
            .filter(|variant| !covered.contains(variant))
            .collect()
    }

    #[test]
    fn every_host_user_action_is_covered() {
        assert!(uncovered_action_variants(include_str!("keys.rs")).is_empty());
    }

    #[test]
    fn coverage_rejects_an_unregistered_host_action() {
        let source = include_str!("keys.rs").replacen(
            "pub enum Action {",
            "pub enum Action { DummyUnregisteredAction,",
            1,
        );
        assert_eq!(
            uncovered_action_variants(&source),
            ["DummyUnregisteredAction"]
        );
    }

    #[test]
    fn registry_resolves_legacy_reload_name_and_parent_context_remap() {
        let config: KeybindingsConfig = toml::from_str(
            "force_reload_plugin = 'cmd+alt+f'\ncontext_zoom_out = 'cmd+alt+escape'",
        )
        .unwrap();
        let bindings = build_key_bindings(Some(&config));
        assert_eq!(bindings.force_reload_view, (cmd_alt(), egui::Key::F));
        assert_eq!(bindings.context_zoom_out, (cmd_alt(), egui::Key::Escape));
        assert_eq!(
            bindings.chord_for_id("force_reload_plugin"),
            bindings.chord_for_id("force_reload_view")
        );
    }

    #[test]
    fn registry_reports_a_conflict_with_a_previously_unbound_command() {
        let config: KeybindingsConfig = toml::from_str("open_changelog = 'cmd+p'").unwrap();
        let bindings = build_key_bindings(Some(&config));
        assert_eq!(
            bindings.conflicts(),
            ["'open_changelog' and 'toggle_command_palette' share the same key combo"]
        );
    }

    #[test]
    fn changelog_is_bindable_without_a_default_chord() {
        let config: KeybindingsConfig = toml::from_str("open_changelog = 'cmd+alt+c'").unwrap();
        assert!(!build_binding_table(&KeyBindings::default())
            .iter()
            .any(|entry| entry.action == Action::OpenChangelog));
        assert!(build_binding_table(&build_key_bindings(Some(&config)))
            .iter()
            .any(|entry| entry.action == Action::OpenChangelog
                && entry.modifiers == cmd_alt()
                && entry.key == egui::Key::C));
    }

    #[test]
    fn ui_scale_defaults_are_unique() {
        let b = KeyBindings::default();
        assert_eq!(b.increase_font_size, (cmd(), egui::Key::Equals));
        assert_eq!(b.decrease_font_size, (cmd(), egui::Key::Minus));
        assert_eq!(b.reset_font_size, (cmd_alt(), egui::Key::Num0));
        assert_eq!(b.ui_scale_up, (cmd_shift(), egui::Key::Equals));
        assert_eq!(b.ui_scale_down, (cmd_shift(), egui::Key::Minus));
        assert_eq!(b.ui_scale_reset, (cmd_shift(), egui::Key::Num0));
        let table = build_binding_table(&b);
        for (chord, action) in [
            (b.increase_font_size, Action::IncreasePaneFontSize),
            (b.decrease_font_size, Action::DecreasePaneFontSize),
            (b.reset_font_size, Action::ResetPaneFontSize),
            (b.ui_scale_up, Action::UiScaleUp),
            (b.ui_scale_down, Action::UiScaleDown),
            (b.ui_scale_reset, Action::UiScaleReset),
        ] {
            let same: Vec<_> = table
                .iter()
                .filter(|e| e.modifiers == chord.0 && e.key == chord.1)
                .collect();
            assert_eq!(same.len(), 1, "{action:?} chord collides: {}", same.len());
            assert_eq!(same[0].action, action);
        }
        let mut seen = std::collections::HashSet::new();
        for (name, (m, k)) in [
            ("increase_font_size", b.increase_font_size),
            ("decrease_font_size", b.decrease_font_size),
            ("reset_font_size", b.reset_font_size),
            ("ui_scale_up", b.ui_scale_up),
            ("ui_scale_down", b.ui_scale_down),
            ("ui_scale_reset", b.ui_scale_reset),
            ("open_quick_note", b.open_quick_note),
        ] {
            assert!(seen.insert(binding_id(m, k)), "{name} shares a chord");
        }
    }

    #[test]
    fn ui_scale_reset_is_not_swallowed_by_quick_note() {
        let b = KeyBindings::default();
        let table = build_binding_table(&b);
        let reset = table
            .iter()
            .position(|e| e.action == Action::UiScaleReset)
            .unwrap();
        let quick = table
            .iter()
            .position(|e| e.action == Action::OpenQuickNote)
            .unwrap();
        assert!(table[reset].exact);
        assert!(
            reset < quick,
            "exact reset must sort before subset quick note"
        );
    }

    #[test]
    fn attention_keys_preserve_notifications_and_use_free_binding() {
        let b = KeyBindings::default();
        let table = build_binding_table(&b);
        assert_eq!(b.toggle_attention_queue, (cmd(), egui::Key::I));
        assert!(table
            .iter()
            .any(|e| (e.modifiers, e.key) == b.toggle_attention_queue
                && e.action == Action::ToggleAttentionQueue));
        assert!(!table
            .iter()
            .any(|e| (e.modifiers, e.key) == (cmd_shift(), egui::Key::A)));
        assert_eq!(b.focus_approval, (cmd_alt(), egui::Key::P));
        assert_eq!(
            table
                .iter()
                .filter(|e| (e.modifiers, e.key) == b.focus_approval)
                .count(),
            1
        );
        let approval: crate::config::KeybindingsConfig =
            toml::from_str("focus_approval = \"cmd+alt+g\"").unwrap();
        assert_eq!(
            build_key_bindings(Some(&approval)).focus_approval,
            (cmd_alt(), egui::Key::G)
        );
        assert_eq!(b.next_blocked_agent, (cmd_alt(), egui::Key::B));
        let overrides: crate::config::KeybindingsConfig =
            toml::from_str("next_blocked_agent = \"cmd+alt+g\"").unwrap();
        assert_eq!(
            build_key_bindings(Some(&overrides)).next_blocked_agent,
            (cmd_alt(), egui::Key::G)
        );
        assert_eq!(
            table
                .iter()
                .filter(|e| (e.modifiers, e.key) == b.next_blocked_agent)
                .count(),
            1
        );
        assert!(table.iter().any(|e| e.action == Action::NextBlockedAgent));
    }

    /// Run `poll_actions` for a single Tab keypress with an active plugin surface.
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
                /* view_active */ true,
                /* keyboard_capture */ false,
                /* overlay_open */ false,
                /* shortcuts_overlay_open */ false,
            )
            .into_iter()
            .map(|(_, action)| action)
            .collect();
        });
        actions
    }

    #[test]
    fn tab_passes_through_to_active_view() {
        let actions = poll_tab();
        assert!(
            actions.is_empty(),
            "plain Tab should not be consumed by host plugin focus; plugins own it"
        );
    }

    #[test]
    fn font_and_ui_scale_chords_claim_press_repeat_release_and_modal_edges() {
        let table = build_binding_table(&KeyBindings::default());
        let ctx = egui::Context::default();
        let mut claimed_key_releases = std::collections::HashSet::new();
        let poll =
            |events: Vec<egui::Event>,
             overlay_open: bool,
             claimed_key_releases: &mut std::collections::HashSet<egui::Key>| {
                let mut raw = egui::RawInput::default();
                raw.events = events;
                let mut actions = Vec::new();
                let mut remaining_events = 0;
                let _ = ctx.run_ui(raw, |ui| {
                    let mut input = crate::app::input_router::PlexiInput::take_from(ui.ctx());
                    actions = poll_actions(
                        &mut input,
                        &table,
                        claimed_key_releases,
                        /* view_active */ true,
                        /* keyboard_capture */ false,
                        overlay_open,
                        /* shortcuts_overlay_open */ false,
                    )
                    .into_iter()
                    .map(|(_, action)| action)
                    .collect::<Vec<_>>();
                    remaining_events = input.events().len();
                });
                (actions, remaining_events)
            };
        let key_event = |key, pressed, repeat, modifiers| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat,
            modifiers,
        };

        let (actions, remaining) = poll(
            vec![key_event(egui::Key::Equals, true, false, cmd())],
            false,
            &mut claimed_key_releases,
        );
        assert_eq!(actions, vec![Action::IncreasePaneFontSize]);
        assert_eq!(
            remaining, 0,
            "font-size press must not reach the focused pane"
        );

        let (actions, remaining) = poll(
            vec![key_event(
                egui::Key::Equals,
                false,
                false,
                egui::Modifiers::NONE,
            )],
            false,
            &mut claimed_key_releases,
        );
        assert!(
            actions.is_empty(),
            "key release must not trigger a second action"
        );
        assert_eq!(
            remaining, 0,
            "font-size release must not leak to the focused pane"
        );

        let (actions, remaining) = poll(
            vec![key_event(egui::Key::Equals, true, true, cmd())],
            false,
            &mut claimed_key_releases,
        );
        assert_eq!(actions, vec![Action::IncreasePaneFontSize]);
        assert_eq!(
            remaining, 0,
            "font-size repeat must be claimed exactly once"
        );

        let (actions, remaining) = poll(
            vec![key_event(egui::Key::Plus, true, false, cmd_shift())],
            true,
            &mut claimed_key_releases,
        );
        assert_eq!(actions, vec![Action::UiScaleUp]);
        assert_eq!(
            remaining, 0,
            "modal ui-scale chord must not leak to the focused pane"
        );

        let (actions, remaining) = poll(
            vec![key_event(
                egui::Key::Plus,
                false,
                false,
                egui::Modifiers::NONE,
            )],
            true,
            &mut claimed_key_releases,
        );
        assert!(
            actions.is_empty(),
            "modal key release must not trigger a second action"
        );
        assert_eq!(
            remaining, 0,
            "modal ui-scale release must not leak to the focused pane"
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
        assert!(modifiers_match_exact_for_os(
            &linux_ctrl_alt,
            &cmd_alt(),
            false
        ));
        assert!(!modifiers_match_exact_for_os(
            &linux_ctrl,
            &cmd_alt(),
            false
        ));
        assert!(!modifiers_match_exact_for_os(
            &linux_ctrl_alt,
            &cmd(),
            false
        ));

        // macOS: `cmd` and `ctrl` are distinct physical modifiers. winit's
        // runtime snapshot carries both command and mac_cmd for Cmd, while a
        // synthetic input may carry only mac_cmd.
        let mac_cmd_only = egui::Modifiers {
            mac_cmd: true,
            ctrl: false,
            ..egui::Modifiers::NONE
        };
        let mac_cmd_ctrl = egui::Modifiers {
            mac_cmd: true,
            ctrl: true,
            ..egui::Modifiers::NONE
        };
        assert!(modifiers_match_exact_for_os(&mac_cmd_only, &cmd(), true));
        assert!(!modifiers_match_exact_for_os(&mac_cmd_ctrl, &cmd(), true));
        assert!(modifiers_match_exact_for_os(
            &mac_cmd_ctrl,
            &cmd_ctrl(),
            true
        ));
        assert!(!modifiers_match_exact_for_os(
            &mac_cmd_only,
            &cmd_ctrl(),
            true
        ));

        let mac_cmd_with_command = egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..egui::Modifiers::NONE
        };
        assert!(modifiers_match_exact_for_os(
            &mac_cmd_with_command,
            &cmd(),
            true
        ));
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
                    /* view_active */ false,
                    /* keyboard_capture */ false,
                    /* overlay_open */ false,
                    /* shortcuts_overlay_open */ false,
                )
                .into_iter()
                .map(|(_, action)| action)
                .collect();
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
