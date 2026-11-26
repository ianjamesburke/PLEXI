// Stable IDs are also the [keybindings] names. This table drives config,
// resolved chords, event dispatch, and command discovery. An explicit palette
// rank preserves the existing result order when a query does not break ties.
macro_rules! commands {
    ($emit:ident) => {
        $emit! {
        quit []: default (cmd(), egui::Key::Q) => Action::Quit, Global, false, false;
            Show("Quit Plexi", "Quit the application using the configured confirmation", "quit exit application", Host, None);
        close_pane []: default (cmd(), egui::Key::W) => Action::ClosePane, Global, false, true;
            Show("Close pane", "Close the focused pane (dismisses overlays and view back-stacks first)", "close pane tab", Window, None; 42);
        toggle_command_palette []: default (cmd(), egui::Key::P) => Action::ToggleCommandPalette, Global, true, false;
            Except("Command palette", "The palette reserves its own toggle for opening and dismissal.");
        open_quick_note []: default (cmd(), egui::Key::Num0) => Action::OpenQuickNote, Global, false, false;
            Show("Quick Note", "Capture a note in the current context", "quick note capture inbox memo", Context, Text("Note"); 25);
        open_agents []: default (cmd_alt(), egui::Key::A) => Action::OpenAgents, Global, true, true;
            Show("Open Agents pane", "Review tasks, workers and blocking reasons", "agents fleet team task workers blocked status", Host, None; 20);
        next_blocked_agent []: default (cmd_alt(), egui::Key::B) => Action::NextBlockedAgent, Global, true, true;
            Show("Focus next blocked agent", "Focus the next agent that needs attention", "next blocked agent focus needs attention", Host, None);
        focus_approval []: default (cmd_alt(), egui::Key::P) => Action::FocusApproval, Global, true, true;
            Show("Focus oldest approval", "Focus the permission banner for a keyboard decision", "attention approval permission keyboard allow deny", Host, None; 19);
        toggle_attention_queue []: default (cmd(), egui::Key::I) => Action::ToggleAttentionQueue, Global, true, true;
            Show("Review notifications", "Open or close the attention queue list", "review notifications notification attention queue alerts inbox pending", Host, None; 22);
        zoom_in []: default (cmd(), egui::Key::Enter) => Action::ZoomIn, Global, true, true;
            Show("Wrap selection or enter pane", "Wrap selected panes, enter a portal, or toggle pane zoom", "wrap selection group subcontext zoom enter", Window, None; 44);
        context_zoom_out []: default (cmd(), egui::Key::Escape) => Action::ContextZoomOut, Global, true, true;
            Show("Go to parent context", "Close the open overlay, else return to the parent context", "parent context zoom out back", Context, None; 37);
        close_context []: default (cmd_shift(), egui::Key::W) => Action::CloseContext, Global, true, true;
            Show("Close context", "Close the selected context (confirmation required)", "close delete context workspace", Context, None; 35);
        split_vertical []: default (cmd_shift(), egui::Key::D) => Action::SplitVertical, Normal, false, false;
            Show("Split down with terminal", "Open a new terminal below", "split down terminal pane", Window, None);
        split_horizontal []: default (cmd(), egui::Key::D) => Action::SplitHorizontal, Normal, true, false;
            Show("Split right with terminal", "Open a new terminal to the right", "split right terminal pane", Window, None);
        swap_pane_left []: default (cmd_ctrl(), egui::Key::H) => Action::SwapPane(Direction::Left), Normal, true, false;
            Show("Swap pane left", "Swap the focused pane with its neighbor left", "move swap pane left", Pane, None);
        swap_pane_down []: default (cmd_ctrl(), egui::Key::J) => Action::SwapPane(Direction::Down), Normal, true, false;
            Show("Swap pane down", "Swap the focused pane with its neighbor down", "move swap pane down", Pane, None);
        swap_pane_up []: default (cmd_ctrl(), egui::Key::K) => Action::SwapPane(Direction::Up), Normal, true, false;
            Show("Swap pane up", "Swap the focused pane with its neighbor up", "move swap pane up", Pane, None);
        swap_pane_right []: default (cmd_ctrl(), egui::Key::L) => Action::SwapPane(Direction::Right), Normal, true, false;
            Show("Swap pane right", "Swap the focused pane with its neighbor right", "move swap pane right", Pane, None);
        send_pane_left []: default (cmd_ctrl_alt(), egui::Key::H) => Action::SendPane(Direction::Left), Normal, false, false;
            Show("Send pane left", "Move the focused pane without following it left", "move send pane left", Pane, None);
        send_pane_down []: default (cmd_ctrl_alt(), egui::Key::J) => Action::SendPane(Direction::Down), Normal, false, false;
            Show("Send pane down", "Move the focused pane without following it down", "move send pane down", Pane, None);
        send_pane_up []: default (cmd_ctrl_alt(), egui::Key::K) => Action::SendPane(Direction::Up), Normal, false, false;
            Show("Send pane up", "Move the focused pane without following it up", "move send pane up", Pane, None);
        send_pane_right []: default (cmd_ctrl_alt(), egui::Key::L) => Action::SendPane(Direction::Right), Normal, false, false;
            Show("Send pane right", "Move the focused pane without following it right", "move send pane right", Pane, None);
        new_tab []: default (cmd(), egui::Key::T) => Action::NewTab, Normal, false, false;
            Show("New tab", "Create a new tab in the selected context", "new tab pane terminal", Window, None; 38);
        next_tab []: default (cmd(), egui::Key::CloseBracket) => Action::NextTab, Normal, true, false;
            Show("Next tab", "Focus the next tab", "next tab forward", Window, None; 45);
        prev_tab []: default (cmd(), egui::Key::OpenBracket) => Action::PrevTab, Normal, true, false;
            Show("Previous tab", "Focus the previous tab", "previous tab back", Window, None; 46);
        move_context_up []: default (cmd_shift_ctrl(), egui::Key::K) => Action::MoveContextUp, Normal, false, false;
            Show("Move context up", "Move the active context up in the sidebar", "move reorder context up", Context, None);
        move_context_down []: default (cmd_shift_ctrl(), egui::Key::J) => Action::MoveContextDown, Normal, false, false;
            Show("Move context down", "Move the active context down in the sidebar", "move reorder context down", Context, None);
        navigate_left []: default (cmd(), egui::Key::H) => Action::Navigate(Direction::Left), Normal, true, false;
            Show("Focus pane left", "Move focus left", "focus navigate move left", Pane, None);
        navigate_down []: default (cmd(), egui::Key::J) => Action::Navigate(Direction::Down), Normal, true, false;
            Show("Focus pane down", "Move focus down", "focus navigate move down", Pane, None);
        navigate_up []: default (cmd(), egui::Key::K) => Action::Navigate(Direction::Up), Normal, true, false;
            Show("Focus pane up", "Move focus up", "focus navigate move up", Pane, None);
        navigate_right []: default (cmd(), egui::Key::L) => Action::Navigate(Direction::Right), Normal, true, false;
            Show("Focus pane right", "Move focus right", "focus navigate move right", Pane, None);
        toggle_sidebar []: default (cmd(), egui::Key::B) => Action::ToggleSidebar, Normal, false, false;
            Show("Toggle sidebar", "Show or hide the context sidebar", "toggle show hide sidebar contexts", Host, None);
        toggle_zoom []: default (cmd_shift(), egui::Key::Enter) => Action::ToggleZoom, Normal, true, false;
            Show("Zoom pane", "Zoom or restore the selected pane (dives into an agent's children first)", "zoom restore pane fullscreen", Pane, None; 43);
        toggle_shortcuts []: default (cmd(), egui::Key::Slash) => Action::ToggleShortcuts, Normal, false, false;
            Show("Keyboard shortcuts", "Show keyboard shortcuts reference", "keyboard shortcuts keybindings help reference cheatsheet", Host, None; 15);
        rename_context []: default (cmd_shift(), egui::Key::R) => Action::RenameContext, Normal, false, false;
            Show("Rename context", "Rename the selected context", "rename context workspace", Context, Text("Context name"); 34);
        rename_pane []: default (cmd(), egui::Key::R) => Action::RenamePane, Normal, true, false;
            Show("Rename pane", "Rename the selected pane", "rename pane tab", Pane, Text("Pane name"); 40);
        split_down []: default (cmd_shift(), egui::Key::Backslash) => Action::SplitDown, Normal, false, false;
            Show("Split down", "Split down: a new terminal, or a new copy of the focused view", "split down below new terminal shell console pane hsplit horizontal", Window, None; 8);
        split_right []: default (cmd(), egui::Key::Backslash) => Action::SplitRight, Normal, true, false;
            Show("Split right", "Split right: a new terminal, or a new copy of the focused view", "split right new terminal shell console pane vsplit vertical", Window, None; 7);
        push_to_subcontext []: default (cmd_alt(), egui::Key::N) => Action::PushPaneToSubcontext, Normal, false, false;
            Show("Move pane to new subcontext", "Move the focused pane into a new child context", "move send push pane new child subcontext", Pane, None);
        new_child_context []: default (cmd_shift_alt(), egui::Key::N) => Action::NewChildContext, Normal, false, false;
            Show("New child context", "Create and enter an empty child context", "new create child subcontext", Context, None);
        new_context []: default (cmd_shift(), egui::Key::N) => Action::NewContext, Normal, false, false;
            Show("New context", "Create a new context", "new create open context workspace", Host, Picker("Context location"); 33);
        new_page_right []: default (cmd(), egui::Key::N) => Action::NewPageRight, Normal, true, false;
            Show("New window", "Create a new window in the current context", "new pane page window right", Window, None; 39);
        toggle_minimap []: default (cmd_shift(), egui::Key::M) => Action::ToggleMinimap, Normal, false, false;
            Show("Toggle minimap", "Show or hide the pane minimap", "toggle show hide minimap", Host, None);
        scroll_up []: default (cmd_shift(), egui::Key::ArrowUp) => Action::ScrollUp, Normal, false, false;
            Show("Scroll pane up", "Scroll the focused pane up", "scroll pane terminal up", Pane, None);
        scroll_down []: default (cmd_shift(), egui::Key::ArrowDown) => Action::ScrollDown, Normal, false, false;
            Show("Scroll pane down", "Scroll the focused pane down", "scroll pane terminal down", Pane, None);
        increase_font_size []: default (cmd(), egui::Key::Equals) => Action::IncreasePaneFontSize, Global, true, false;
            Show("Increase text size", "Make the focused pane's text larger", "text size font larger bigger increase pane terminal", Pane, None; 26);
        decrease_font_size []: default (cmd(), egui::Key::Minus) => Action::DecreasePaneFontSize, Global, true, false;
            Show("Decrease text size", "Make the focused pane's text smaller", "text size font smaller decrease pane terminal", Pane, None; 27);
        reset_font_size []: default (cmd_alt(), egui::Key::Num0) => Action::ResetPaneFontSize, Global, true, false;
            Show("Reset text size", "Restore the focused pane's text size to default", "text size font reset default pane terminal", Pane, None; 28);
        ui_scale_up []: default (cmd_shift(), egui::Key::Equals) => Action::UiScaleUp, Global, true, false;
            Show("Increase UI scale", "Make the whole interface larger", "ui scale zoom larger bigger increase interface", Host, None; 29);
        ui_scale_down []: default (cmd_shift(), egui::Key::Minus) => Action::UiScaleDown, Global, true, false;
            Show("Decrease UI scale", "Make the whole interface smaller", "ui scale zoom smaller decrease interface", Host, None; 30);
        ui_scale_reset []: default (cmd_shift(), egui::Key::Num0) => Action::UiScaleReset, Global, true, false;
            Show("Reset UI scale", "Restore the interface to 100%", "ui scale zoom reset default actual size", Host, None; 31);
        open_file_browser []: default (cmd(), egui::Key::E) => Action::OpenFileBrowser, Normal, false, false;
            Show("File Explorer", "Browse files and directories in the focused context", "file explorer file browser browse directory folder tree open files", Window, None; 18);
        open_config []: default (cmd(), egui::Key::Comma) => Action::OpenConfig, Normal, false, false;
            Show("Choose config scope", "Choose a global or workspace config to edit", "settings preferences choose config scope", Context, Picker("Config scope"));
        reload_config []: default (cmd_shift(), egui::Key::Comma) => Action::ReloadConfig, Normal, false, false;
            Show("Reload configuration", "Reload global and active-workspace configuration", "reload config configuration settings global workspace scoped refresh", Host, None; 16);
        open_secrets_manager []: default (cmd_shift(), egui::Key::S) => Action::OpenSecretsManager, Normal, false, false;
            Show("Open Secrets", "Manage scoped credentials, usage and grants", "open secrets credentials keys global workspace folder import env", Host, None; 21);
        open_assistant []: default (cmd_ctrl(), egui::Key::A) => Action::OpenAssistant, Normal, false, false;
            Show("Open Assistant", "Open or focus the Plexi Assistant", "open launch assistant chat", Host, None, Assistant);
        force_reload_view ["force_reload_plugin"]: default (cmd_alt(), egui::Key::R) => Action::ForceReloadView, Normal, false, false;
            Show("Force reload view", "Restart the focused process-backed view", "force reload restart view plugin", Pane, None);
        set_context_root_from_cwd []: default (cmd_shift(), egui::Key::I) => Action::SetContextRootFromCwd, Normal, false, false;
            Show("Set context root from pane directory", "Use the focused pane’s current directory as the context root", "set change context root cwd directory", Pane, None);
        park_context []: default (cmd_shift(), egui::Key::U) => Action::ParkContext, Normal, false, false;
            Show("Park or unpark context", "Park or restore the selected context", "park unpark restore context workspace", Context, None; 36);
        open_scratchpad []: default (cmd_shift(), egui::Key::Space) => Action::OpenScratchpad, Normal, false, false;
            Show("Scratch Pad", "Open a fresh scratch note editor", "scratch pad scratchpad note editor inbox memo", Context, None; 32);
        open_notes_picker []: default (cmd(), egui::Key::O) => Action::OpenNotesPicker, Normal, false, false;
            Show("Browse notes", "Open the notes picker for global and context notes", "browse notes note picker open find global context", Context, Picker("Note"); 17);
        next_context []: optional None => Action::NextContext, Normal, true, false;
            Show("Next context", "Focus the next context", "next context workspace forward", Host, None);
        prev_context []: optional None => Action::PrevContext, Normal, true, false;
            Show("Previous context", "Focus the previous context", "previous context workspace back", Host, None);
        nav_back []: optional None => Action::NavBackView, Normal, true, false;
            Show("Back", "Go back in pane navigation or focus history", "back previous history pane", Window, None; 47);
        focus_history_forward []: optional None => Action::FocusHistoryForward, Normal, true, false;
            Show("Forward in focus history", "Return to the next pane in focus history", "forward next focus history pane", Window, None);
        select_pane_left []: default (cmd_shift(), egui::Key::H) => Action::SelectPane(Direction::Left), Normal, true, false;
            Show("Extend pane selection left", "Extend the selection left", "select pane selection extend left", Pane, None);
        select_pane_down []: default (cmd_shift(), egui::Key::J) => Action::SelectPane(Direction::Down), Normal, true, false;
            Show("Extend pane selection down", "Extend the selection down", "select pane selection extend down", Pane, None);
        select_pane_up []: default (cmd_shift(), egui::Key::K) => Action::SelectPane(Direction::Up), Normal, true, false;
            Show("Extend pane selection up", "Extend the selection up", "select pane selection extend up", Pane, None);
        select_pane_right []: default (cmd_shift(), egui::Key::L) => Action::SelectPane(Direction::Right), Normal, true, false;
            Show("Extend pane selection right", "Extend the selection right", "select pane selection extend right", Pane, None);
        open_changelog []: optional None => Action::OpenChangelog, Normal, true, true;
            Show("Changelog", "Show what changed in this version", "changelog release notes what's new version", Host, None; 13);
        open_global_config []: optional None => Action::OpenGlobalConfig, Normal, true, true;
            Show("Open config: everywhere", "Edit the channel-wide Plexi config file", "open config: everywhere global settings preferences config.toml configuration channel", Host, None; 9);
        open_workspace_config []: optional None => Action::OpenWorkspaceConfig, Normal, true, true;
            Show("Open config: this repo", "Edit config for the active workspace scope", "open config: this repo workspace settings preferences config.toml configuration scoped project local", Context, None, Workspace; 10);
        state_preset_default []: optional None => Action::StatePreset("default"), Normal, true, true;
            Show("Settings: agent states — default", "Apply agent state colors; shapes always identify the state", "settings preferences theme state preset default colorblind contrast", Host, Choice("Agent state preset"); 0);
        state_preset_okabe_ito []: optional None => Action::StatePreset("okabe-ito"), Normal, true, true;
            Show("Settings: agent states — okabe-ito", "Apply agent state colors; shapes always identify the state", "settings preferences theme state preset okabe-ito colorblind contrast", Host, Choice("Agent state preset"); 1);
        state_preset_high_contrast []: optional None => Action::StatePreset("high-contrast"), Normal, true, true;
            Show("Settings: agent states — high-contrast", "Apply agent state colors; shapes always identify the state", "settings preferences theme state preset high-contrast colorblind contrast", Host, Choice("Agent state preset"); 2);
        state_preset_mono []: optional None => Action::StatePreset("mono"), Normal, true, true;
            Show("Settings: agent states — mono", "Apply agent state colors; shapes always identify the state", "settings preferences theme state preset mono colorblind contrast", Host, Choice("Agent state preset"); 3);
        edit_commands []: optional None => Action::EditCommands, Normal, true, true;
            Show("Edit commands", "Edit commands.toml for the active workspace or global profile", "edit commands commands.toml alias aliases hub scripts run workspace global", Context, None; 11);
        switch_commands_scope []: optional None => Action::SwitchCommandsScope, Normal, true, true;
            Show("Switch commands scope", "Switch commands editor between workspace and global scope", "switch commands scope toggle global workspace", Pane, None; 12);
        open_attention []: optional None => Action::OpenAttention, Normal, true, true;
            Show("Review issues needing attention", "Open the attention queue for approvals, questions and host notices", "needs attention review issues errors problems keychain integrity approvals recovery diagnostics", Host, None; 23);
        setup_cli []: optional None => Action::SetupCli, Normal, true, true;
            Show("Install 'plexi' CLI in PATH", "Install the ~/.local/bin launcher, completions, and PATH registration", "install plexi cli path completions setup terminal command", Host, None; 24);
        ui_upcycle_git []: optional None => Action::UiUpcycle("git"), Normal, true, true;
            Show("UI upcycle: git", "Open git's curated command list and forms", "ui upcycle git cli visualizer commands forms", Host, Choice("CLI descriptor"); 4);
        ui_upcycle_cargo []: optional None => Action::UiUpcycle("cargo"), Normal, true, true;
            Show("UI upcycle: cargo", "Open cargo's curated command list and forms", "ui upcycle cargo cli visualizer commands forms", Host, Choice("CLI descriptor"); 5);
        ui_upcycle_gh []: optional None => Action::UiUpcycle("gh"), Normal, true, true;
            Show("UI upcycle: gh", "Open gh's curated command list and forms", "ui upcycle gh cli visualizer commands forms", Host, Choice("CLI descriptor"); 6);
        check_for_updates []: optional None => Action::CheckForUpdates, Normal, true, true;
            Show("Check for updates", "Look for a newer Plexi build on this channel now", "check for updates update upgrade new version release download", Host, None; 14);
        restart_to_update []: optional None => Action::RestartToUpdate, Normal, true, true;
            Show("Restart to update", "Restart into the downloaded build after confirmation", "restart to update upgrade downloaded build", Host, None, Update);
        close_selected_panes []: optional None => Action::CloseSelectedPanes, Normal, true, true;
            Show("Close selected panes", "Close every selected pane, asking about unsaved work one pane at a time", "close selected panes selection all multiple", Selection, None, Selection; 41);
        cancel_pane_selection []: optional None => Action::CancelPaneSelection, Normal, true, true;
            Show("Clear pane selection", "Clear the active window’s pane selection", "clear cancel pane selection", Window, None);
        switch_context_1 []: default (cmd(), egui::Key::Num1) => Action::SwitchContext(0), Normal, true, false;
            Show("Switch to context 1", "Focus a sidebar context by its position", "switch context 1 jump workspace", Host, Choice("Context position"));
        switch_context_2 []: default (cmd(), egui::Key::Num2) => Action::SwitchContext(1), Normal, true, false;
            Show("Switch to context 2", "Focus a sidebar context by its position", "switch context 2 jump workspace", Host, Choice("Context position"));
        switch_context_3 []: default (cmd(), egui::Key::Num3) => Action::SwitchContext(2), Normal, true, false;
            Show("Switch to context 3", "Focus a sidebar context by its position", "switch context 3 jump workspace", Host, Choice("Context position"));
        switch_context_4 []: default (cmd(), egui::Key::Num4) => Action::SwitchContext(3), Normal, true, false;
            Show("Switch to context 4", "Focus a sidebar context by its position", "switch context 4 jump workspace", Host, Choice("Context position"));
        switch_context_5 []: default (cmd(), egui::Key::Num5) => Action::SwitchContext(4), Normal, true, false;
            Show("Switch to context 5", "Focus a sidebar context by its position", "switch context 5 jump workspace", Host, Choice("Context position"));
        switch_context_6 []: default (cmd(), egui::Key::Num6) => Action::SwitchContext(5), Normal, true, false;
            Show("Switch to context 6", "Focus a sidebar context by its position", "switch context 6 jump workspace", Host, Choice("Context position"));
        switch_context_7 []: default (cmd(), egui::Key::Num7) => Action::SwitchContext(6), Normal, true, false;
            Show("Switch to context 7", "Focus a sidebar context by its position", "switch context 7 jump workspace", Host, Choice("Context position"));
        switch_context_8 []: default (cmd(), egui::Key::Num8) => Action::SwitchContext(7), Normal, true, false;
            Show("Switch to context 8", "Focus a sidebar context by its position", "switch context 8 jump workspace", Host, Choice("Context position"));
        switch_context_9 []: default (cmd(), egui::Key::Num9) => Action::SwitchContext(8), Normal, true, false;
            Show("Switch to context 9", "Focus a sidebar context by its position", "switch context 9 jump workspace", Host, Choice("Context position"));
        }
    };
}
pub(crate) use commands;

use super::{palette_commands, Action, PaletteCommandEntry};
use crate::app::registry::{InstalledPlugin, PluginRegistry, RegistrySource};
use crate::cli::{ResolvedUserCommand, UserCommandScope};
use std::{borrow::Cow, path::Path};

/// Search metadata paired with the action that the shared dispatcher runs.
pub(crate) struct DiscoveredCommand {
    pub action: Action,
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    pub search_text: Cow<'static, str>,
}

impl From<&'static PaletteCommandEntry> for DiscoveredCommand {
    fn from(entry: &'static PaletteCommandEntry) -> Self {
        Self {
            action: entry.command.clone(),
            name: entry.name.into(),
            description: entry.description.into(),
            search_text: entry.search_text.into(),
        }
    }
}

impl DiscoveredCommand {
    fn launch_app(app: &InstalledPlugin, workspace: Option<&Path>) -> Option<Self> {
        if app.source != RegistrySource::Global && app.workspace_root.as_deref() != workspace {
            return None;
        }
        let manifest = &app.manifest;
        Some(Self {
            action: Action::LaunchApp(manifest.id.clone(), app.source),
            name: manifest.name.clone().into(),
            description: manifest.description.clone().into(),
            search_text: format!("{} {} {}", manifest.name, manifest.id, manifest.description)
                .to_lowercase()
                .into(),
        })
    }

    fn saved_command(command: &ResolvedUserCommand) -> Self {
        let description = match (&command.description, &command.run) {
            (Some(description), _) => description.clone(),
            (None, Some(run)) => run.clone(),
            (None, None) if !command.steps.is_empty() => format!("{} steps", command.steps.len()),
            (None, None) => "global script".to_string(),
        };
        let aliases = match command.scope {
            UserCommandScope::Workspace => "workspace ws",
            UserCommandScope::Global => "global script",
        };
        Self {
            action: Action::RunUserCommand(command.name.clone(), Some(command.scope)),
            name: command.name.clone().into(),
            search_text: format!("{} {description} run {aliases}", command.name)
                .to_lowercase()
                .into(),
            description: description.into(),
        }
    }
}

/// A manifest command bound to the open view that will run its handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginCommandTarget {
    pub(crate) command: String,
    pub(crate) handler: String,
    pub(crate) plugin: String,
    pub(crate) pane_id: u64,
}

/// The manifest commands `plugin` offers through its open view `pane_id`.
/// The row shows the command ID with the owner plugin's name.
pub(crate) fn plugin_view_commands(
    plugin: &InstalledPlugin,
    pane_id: u64,
) -> impl Iterator<Item = DiscoveredCommand> + '_ {
    let owner = &plugin.manifest;
    plugin
        .contributes
        .commands
        .iter()
        .map(move |command| DiscoveredCommand {
            action: Action::RunPluginCommand(PluginCommandTarget {
                command: command.id.clone(),
                handler: command.handler.clone(),
                plugin: owner.id.clone(),
                pane_id,
            }),
            name: command.id.clone().into(),
            description: owner.name.clone().into(),
            search_text: format!("{} {} {}", command.id, owner.name, owner.id)
                .to_lowercase()
                .into(),
        })
}

pub(crate) fn discover_commands<'a>(
    apps: &'a PluginRegistry,
    workspace: Option<&'a Path>,
    saved: &'a [ResolvedUserCommand],
) -> impl Iterator<Item = DiscoveredCommand> + 'a {
    // Keep the original declaration order within each palette section.
    apps.list()
        .into_iter()
        .filter_map(move |app| DiscoveredCommand::launch_app(app, workspace))
        .chain(palette_commands().map(DiscoveredCommand::from))
        .chain(saved.iter().map(DiscoveredCommand::saved_command))
}
