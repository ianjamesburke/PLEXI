use super::user_command_run::{spawn_shell_step, HostStepOutcome, UserCommandRun};
use crate::app::PlexiApp;
use crate::host::model::{HostAction, HostEffect};
use crate::host::picker::{
    files::{FileHit, FileIndex},
    preview::PreviewLoader,
    usage::PaletteUsage,
    PickerCommand, PickerEffect, PickerId, PickerRow, PreviewStatus, RowAction, RowKey,
    COMMANDS_SOURCE, FILES_SOURCE, PALETTE_STOPS,
};
use crate::ui::{
    hints::{HintBar, HintGroup},
    list::ListRowPips,
    overlay::ModalShell,
    picker::{is_field_input, picker_key},
    style,
};

enum PaletteEntry {
    Command {
        command: PaletteCommand,
        name: &'static str,
        description: &'static str,
        search_text: &'static str,
    },
    Context {
        target: PaletteFocusTarget,
        name: String,
        workspace_name: String,
        metadata_chips: Vec<&'static str>,
        pane_pips: Option<ListRowPips>,
        search_text: String,
    },
    /// A context persists without a live window. It remains discoverable, but
    /// cannot be restored until something recreates a window for it.
    UnavailableContext { name: String, search_text: String },
    View {
        id: String,
        name: String,
        description: String,
        running_in_background: bool,
        is_workspace_local: bool,
        search_text: String,
    },
    /// Host-native builtin plugin (no registry manifest), launched by id.
    Builtin {
        id: &'static str,
        name: &'static str,
        description: &'static str,
        search_text: String,
    },
    Note {
        path: std::path::PathBuf,
        title: String,
        preview: String,
        search_text: String,
    },
    /// A downloaded build waiting for a restart. Pinned as the first row;
    /// Enter restarts into it (`PaletteCommand::RestartToUpdate`).
    UpdateReady {
        version: String,
        search_text: String,
    },
    /// A user-authored command from `commands.toml` or a global script,
    /// executed via `plexi run <name>` in a new terminal pane.
    UserCommand {
        name: String,
        /// Secondary text: description, run snippet, or "global script".
        secondary: String,
        scope: crate::cli::UserCommandScope,
        search_text: String,
    },
}

/// Why a parked context row refuses Enter.
const PARKED_UNAVAILABLE: &str = "Parked · no live window to restore";

/// The Commands stop's row identity: the stable-id half of a picker row key,
/// and the only thing activation resolves from. Display order is never an
/// identity: panes close and agents change state while the palette is open.
#[derive(Clone, Debug, PartialEq, Eq)]
enum CommandRowId {
    Command(PaletteCommand),
    Focus(PaletteFocusTarget),
    Unavailable(String),
    View(String),
    Builtin(&'static str),
    Note(std::path::PathBuf),
    UserCommand {
        name: String,
        scope: crate::cli::UserCommandScope,
    },
    UpdateReady(String),
}

impl CommandRowId {
    fn stable_id(&self) -> String {
        match self {
            CommandRowId::Command(command) => {
                let name = PALETTE_COMMANDS
                    .iter()
                    .find(|entry| entry.command == *command)
                    .map_or("", |entry| entry.name);
                format!("cmd/{name}")
            }
            CommandRowId::Focus(target) => format!(
                "focus/{}/{}/{}",
                target.context_id,
                target.window_id,
                target
                    .pane_id
                    .map_or_else(|| "-".to_string(), |id| id.to_string())
            ),
            CommandRowId::Unavailable(name) => format!("parked/{name}"),
            CommandRowId::View(id) => format!("view/{id}"),
            CommandRowId::Builtin(id) => format!("builtin/{id}"),
            CommandRowId::Note(path) => format!("note/{}", native_path_key(path)),
            CommandRowId::UserCommand { name, scope } => {
                format!("run/{}/{name}", user_command_scope_id(*scope))
            }
            CommandRowId::UpdateReady(version) => format!("update/{version}"),
        }
    }

    fn key(&self) -> RowKey {
        RowKey::new(COMMANDS_SOURCE, &self.stable_id())
    }

    /// The row's one action: what Enter and a click run.
    fn action(&self) -> RowAction {
        match self {
            CommandRowId::Command(_)
            | CommandRowId::UserCommand { .. }
            | CommandRowId::UpdateReady(_) => RowAction {
                id: "run",
                label: "run",
            },
            CommandRowId::Focus(_) | CommandRowId::Unavailable(_) => RowAction {
                id: "focus",
                label: "focus",
            },
            CommandRowId::View(_) | CommandRowId::Builtin(_) | CommandRowId::Note(_) => RowAction {
                id: "open",
                label: "open",
            },
        }
    }
}

/// A collision-free spelling of a native path for a row key: the tagged hex
/// of its Unix bytes or Windows wide units. Display text is lossy for
/// non-UTF-8 names, so it is never an identity; labels and search keep it.
fn native_path_key(path: &std::path::Path) -> String {
    use std::fmt::Write;
    let mut key = String::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        key.push_str("u:");
        for byte in path.as_os_str().as_bytes() {
            let _ = write!(key, "{byte:02x}");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        key.push_str("w:");
        for unit in path.as_os_str().encode_wide() {
            let _ = write!(key, "{unit:04x}");
        }
    }
    key
}

/// What a palette row runs: a Commands row, or a file of the Files stop.
#[derive(Clone, Debug, PartialEq)]
enum PaletteTarget {
    Command(CommandRowId),
    File(std::path::PathBuf),
}

const OPEN_FILE: RowAction = RowAction {
    id: "open",
    label: "open",
};

impl PaletteTarget {
    fn action(&self) -> RowAction {
        match self {
            PaletteTarget::Command(id) => id.action(),
            PaletteTarget::File(_) => OPEN_FILE,
        }
    }
}

fn command_targets(
    targets: std::collections::HashMap<RowKey, CommandRowId>,
) -> std::collections::HashMap<RowKey, PaletteTarget> {
    targets
        .into_iter()
        .map(|(key, id)| (key, PaletteTarget::Command(id)))
        .collect()
}

/// One answer of the open stop's source: its rows' pane pips and the owned
/// target behind every row key, installed together with the rows of
/// `generation`. Activation takes its target from here before close cleanup
/// drops the answer.
pub(crate) struct PaletteAnswer {
    generation: u64,
    pips: std::collections::HashMap<RowKey, ListRowPips>,
    targets: std::collections::HashMap<RowKey, PaletteTarget>,
}

/// The palette's background sources: the Files index, the preview loader
/// and the usage history. Started on first use and kept across sessions, so
/// the next open answers from the cached index while a fresh walk runs.
#[derive(Default)]
pub(crate) struct PaletteSources {
    files: Option<FileIndex>,
    /// The context root this session's Files stop lists, fixed at its first
    /// Files query.
    root: Option<std::path::PathBuf>,
    /// The generation last asked of the Files source.
    asked: Option<u64>,
    preview: Option<PreviewLoader>,
    /// The uploaded image of the selected row's preview.
    texture: Option<(RowKey, egui::TextureHandle)>,
    usage: PaletteUsage,
}

/// A Files answer's hit as a picker row.
fn file_row(hit: FileHit) -> PickerRow {
    let parent = hit
        .rel_path
        .strip_suffix(hit.file_name.as_str())
        .unwrap_or("")
        .trim_end_matches('/')
        .to_owned();
    PickerRow {
        key: hit.key,
        title: hit.file_name,
        secondary: parent,
        chips: Vec::new(),
        group: "",
        group_rank: 0,
        score: hit.score,
        pinned: false,
        unavailable: None,
        actions: vec![OPEN_FILE],
        usage: hit.usage,
        modified: hit.modified,
        preview: Some(hit.preview),
    }
}

fn user_command_scope_id(scope: crate::cli::UserCommandScope) -> &'static str {
    match scope {
        crate::cli::UserCommandScope::Workspace => "workspace",
        crate::cli::UserCommandScope::Global => "global",
    }
}

impl PaletteEntry {
    fn row_id(&self) -> CommandRowId {
        match self {
            PaletteEntry::Command { command, .. } => CommandRowId::Command(command.clone()),
            PaletteEntry::Context { target, .. } => CommandRowId::Focus(*target),
            PaletteEntry::UnavailableContext { name, .. } => {
                CommandRowId::Unavailable(name.clone())
            }
            PaletteEntry::View { id, .. } => CommandRowId::View(id.clone()),
            PaletteEntry::Builtin { id, .. } => CommandRowId::Builtin(id),
            PaletteEntry::Note { path, .. } => CommandRowId::Note(path.clone()),
            PaletteEntry::UserCommand { name, scope, .. } => CommandRowId::UserCommand {
                name: name.clone(),
                scope: *scope,
            },
            PaletteEntry::UpdateReady { version, .. } => CommandRowId::UpdateReady(version.clone()),
        }
    }

    /// Section header and group order.
    fn group(&self) -> (&'static str, u32) {
        match self {
            PaletteEntry::UpdateReady { .. } => ("", 0),
            PaletteEntry::Context { .. } | PaletteEntry::UnavailableContext { .. } => {
                ("CONTEXTS", 0)
            }
            PaletteEntry::Note { .. } => ("NOTES", 2),
            PaletteEntry::View { .. } | PaletteEntry::Builtin { .. } => ("Plugins", 3),
            PaletteEntry::Command { .. } => ("COMMANDS", 4),
            PaletteEntry::UserCommand { .. } => ("RUN", 5),
        }
    }

    fn primary_label(&self) -> &str {
        match self {
            PaletteEntry::Command { name, .. } | PaletteEntry::Builtin { name, .. } => name,
            PaletteEntry::Context { name, .. }
            | PaletteEntry::UnavailableContext { name, .. }
            | PaletteEntry::View { name, .. }
            | PaletteEntry::UserCommand { name, .. } => name.as_str(),
            PaletteEntry::Note { title, .. } => title.as_str(),
            PaletteEntry::UpdateReady { .. } => "Restart to update",
        }
    }

    fn search_text_str(&self) -> &str {
        match self {
            PaletteEntry::Command { search_text, .. } => search_text,
            PaletteEntry::Context { search_text, .. }
            | PaletteEntry::UnavailableContext { search_text, .. }
            | PaletteEntry::View { search_text, .. }
            | PaletteEntry::Builtin { search_text, .. }
            | PaletteEntry::Note { search_text, .. }
            | PaletteEntry::UpdateReady { search_text, .. }
            | PaletteEntry::UserCommand { search_text, .. } => search_text.as_str(),
        }
    }

    /// Match score for a non-empty normalized query, higher is better.
    fn score(&self, query: &str) -> Option<i64> {
        if query.is_empty() {
            return None;
        }
        let candidate = crate::search::CandidateInput::new(self.primary_label())
            .with_secondary(self.search_text_str());
        crate::search::score_candidate(query, &candidate, std::time::SystemTime::UNIX_EPOCH)
            .map(|m| m.total)
    }

    fn matches_query(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        self.search_text_str().contains(query)
            || crate::search::score_candidate(
                query,
                &crate::search::CandidateInput::new(self.primary_label()),
                std::time::SystemTime::UNIX_EPOCH,
            )
            .is_some()
    }

    /// The picker row for this entry, plus its pane-pip decoration.
    fn into_row(self, query: &str, shortcut: Option<String>) -> (PickerRow, Option<ListRowPips>) {
        let id = self.row_id();
        let (group, group_rank) = self.group();
        let score = self.score(query);
        let mut unavailable = None;
        let mut pips = None;
        let (title, secondary, chips): (String, String, Vec<&'static str>) = match self {
            PaletteEntry::Command {
                name, description, ..
            } => {
                let secondary = match shortcut {
                    Some(shortcut) => format!("{description} · {shortcut}"),
                    None => description.to_string(),
                };
                (name.to_string(), secondary, vec!["cmd"])
            }
            PaletteEntry::Context {
                name,
                workspace_name,
                metadata_chips,
                pane_pips,
                ..
            } => {
                pips = pane_pips;
                (name, workspace_name, metadata_chips)
            }
            PaletteEntry::UnavailableContext { name, .. } => {
                unavailable = Some(PARKED_UNAVAILABLE);
                (name, String::new(), vec!["ctx", "parked", "unavailable"])
            }
            PaletteEntry::View {
                name,
                description,
                running_in_background,
                is_workspace_local,
                ..
            } => (
                name,
                description,
                plugin_metadata_chips(running_in_background, is_workspace_local).to_vec(),
            ),
            PaletteEntry::Builtin {
                name, description, ..
            } => (
                name.to_string(),
                description.to_string(),
                vec!["view", "Bundled"],
            ),
            PaletteEntry::Note { title, preview, .. } => (title, preview, vec!["text"]),
            PaletteEntry::UpdateReady { version, .. } => (
                format!("Update ready: v{version}"),
                "Restart to update · asks before closing running panes".to_string(),
                vec!["update"],
            ),
            PaletteEntry::UserCommand {
                name,
                secondary,
                scope,
                ..
            } => (name, secondary, vec!["run", user_command_scope_id(scope)]),
        };
        let row = PickerRow {
            key: id.key(),
            title,
            secondary,
            chips,
            group,
            group_rank,
            score,
            pinned: matches!(id, CommandRowId::UpdateReady(_)),
            unavailable,
            actions: vec![id.action()],
            usage: None,
            modified: None,
            preview: None,
        };
        (row, pips)
    }
}

/// Stable navigation identity carried from collection through execution. Window
/// ordering is display policy, never an address: a row must survive a sort or
/// another window closing while the palette is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PaletteFocusTarget {
    context_id: u64,
    window_id: u64,
    pane_id: Option<u64>,
}

/// Palette commands either dispatch the canonical host key action or perform
/// the palette-only scoped config handoffs.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PaletteCommand {
    Host(crate::host::keys::Action),
    OpenGlobalConfig,
    OpenWorkspaceConfig,
    StatePreset(&'static str),
    /// Open the effective `commands.toml` (workspace if in a workspace, else
    /// global) in a `text-editor` split, creating it from a template if absent.
    EditCommands,
    /// Switch the focused `commands.toml` editor between workspace and global scope.
    SwitchCommandsScope,
    /// Open the Attention Queue list (`plugin::attention_queue`).
    OpenAttention,
    /// Install the CLI launcher, completions, and PATH registration for external terminals.
    SetupCli,
    UiUpcycle(&'static str),
    /// Check for a newer build now, skipping the update cache.
    CheckForUpdates,
    /// Restart into the downloaded build. Listed only while one is ready.
    RestartToUpdate,
    /// Close every pane in the active window's selection. Listed only while
    /// a selection exists; shares the close-pane shortcut.
    CloseSelectedPanes,
}

struct PaletteCommandEntry {
    command: PaletteCommand,
    name: &'static str,
    description: &'static str,
    search_text: &'static str,
}

const PALETTE_COMMANDS: &[PaletteCommandEntry] = &[
    PaletteCommandEntry {
        command: PaletteCommand::StatePreset("default"),
        name: "Settings: agent states — default",
        description: "Apply agent state colors; shapes always identify the state",
        search_text: "settings preferences theme state preset default colorblind contrast",
    },
    PaletteCommandEntry {
        command: PaletteCommand::StatePreset("okabe-ito"),
        name: "Settings: agent states — okabe-ito",
        description: "Apply agent state colors; shapes always identify the state",
        search_text: "settings preferences theme state preset okabe-ito colorblind contrast",
    },
    PaletteCommandEntry {
        command: PaletteCommand::StatePreset("high-contrast"),
        name: "Settings: agent states — high-contrast",
        description: "Apply agent state colors; shapes always identify the state",
        search_text: "settings preferences theme state preset high-contrast colorblind contrast",
    },
    PaletteCommandEntry {
        command: PaletteCommand::StatePreset("mono"),
        name: "Settings: agent states — mono",
        description: "Apply agent state colors; shapes always identify the state",
        search_text: "settings preferences theme state preset mono colorblind contrast",
    },
    PaletteCommandEntry {
        command: PaletteCommand::UiUpcycle("git"),
        name: "UI upcycle: git",
        description: "Open git's curated command list and forms",
        search_text: "ui upcycle git cli visualizer commands forms",
    },
    PaletteCommandEntry {
        command: PaletteCommand::UiUpcycle("cargo"),
        name: "UI upcycle: cargo",
        description: "Open cargo's curated command list and forms",
        search_text: "ui upcycle cargo cli visualizer commands forms",
    },
    PaletteCommandEntry {
        command: PaletteCommand::UiUpcycle("gh"),
        name: "UI upcycle: gh",
        description: "Open gh's curated command list and forms",
        search_text: "ui upcycle gh cli visualizer commands forms",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::SplitRight),
        name: "Split right",
        description: "Split right: a new terminal, or a new copy of the focused view",
        search_text: "split right new terminal shell console pane vsplit vertical",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::SplitDown),
        name: "Split down",
        description: "Split down: a new terminal, or a new copy of the focused view",
        search_text: "split down below new terminal shell console pane hsplit horizontal",
    },
    PaletteCommandEntry {
        command: PaletteCommand::OpenGlobalConfig,
        name: "Open config: everywhere",
        description: "Edit the channel-wide Plexi config file",
        search_text: "open config: everywhere global settings preferences config.toml configuration channel",
    },
    PaletteCommandEntry {
        command: PaletteCommand::OpenWorkspaceConfig,
        name: "Open config: this repo",
        description: "Edit config for the active workspace scope",
        search_text: "open config: this repo workspace settings preferences config.toml configuration scoped project local",
    },
    PaletteCommandEntry {
        command: PaletteCommand::EditCommands,
        name: "Edit commands",
        description: "Edit commands.toml for the active workspace or global profile",
        search_text: "edit commands commands.toml alias aliases hub scripts run workspace global",
    },
    PaletteCommandEntry {
        command: PaletteCommand::SwitchCommandsScope,
        name: "Switch commands scope",
        description: "Switch commands editor between workspace and global scope",
        search_text: "switch commands scope toggle global workspace",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenChangelog),
        name: "Changelog",
        description: "Show what changed in this version",
        search_text: "changelog release notes what's new version",
    },
    PaletteCommandEntry {
        command: PaletteCommand::CheckForUpdates,
        name: "Check for updates",
        description: "Look for a newer Plexi build on this channel now",
        search_text: "check for updates update upgrade new version release download",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::ToggleShortcuts),
        name: "Keyboard shortcuts",
        description: "Show keyboard shortcuts reference",
        search_text: "keyboard shortcuts keybindings help reference cheatsheet",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::ReloadConfig),
        name: "Reload configuration",
        description: "Reload global and active-workspace configuration",
        search_text: "reload config configuration settings global workspace scoped refresh",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenNotesPicker),
        name: "Browse notes",
        description: "Open the notes picker for global and context notes",
        search_text: "browse notes note picker open find global context",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenFileBrowser),
        name: "File Explorer",
        description: "Browse files and directories in the focused context",
        search_text: "file explorer file browser browse directory folder tree open files",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::FocusApproval),
        name: "Focus oldest approval",
        description: "Focus the permission banner for a keyboard decision",
        search_text: "attention approval permission keyboard allow deny",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenAgents),
        name: "Open Agents pane",
        description: "Review tasks, workers and blocking reasons",
        search_text: "agents fleet team task workers blocked status",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenSecretsManager),
        name: "Open Secrets",
        description: "Manage scoped credentials, usage and grants",
        search_text: "open secrets credentials keys global workspace folder import env",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::ToggleAttentionQueue),
        name: "Review notifications",
        description: "Open or close the attention queue list",
        search_text: "review notifications notification attention queue alerts inbox pending",
    },
    PaletteCommandEntry {
        command: PaletteCommand::OpenAttention,
        name: "Review issues needing attention",
        description: "Open the attention queue for approvals, questions and host notices",
        search_text: "needs attention review issues errors problems keychain integrity approvals recovery diagnostics",
    },
    PaletteCommandEntry {
        command: PaletteCommand::SetupCli,
        name: "Install 'plexi' CLI in PATH",
        description: "Install the ~/.local/bin launcher, completions, and PATH registration",
        search_text: "install plexi cli path completions setup terminal command",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenQuickNote),
        name: "Quick Note",
        description: "Capture a note in the current context",
        search_text: "quick note capture inbox memo",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::IncreasePaneFontSize),
        name: "Increase text size",
        description: "Make the focused pane's text larger",
        search_text: "text size font larger bigger increase pane terminal",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::DecreasePaneFontSize),
        name: "Decrease text size",
        description: "Make the focused pane's text smaller",
        search_text: "text size font smaller decrease pane terminal",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::ResetPaneFontSize),
        name: "Reset text size",
        description: "Restore the focused pane's text size to default",
        search_text: "text size font reset default pane terminal",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::UiScaleUp),
        name: "Increase UI scale",
        description: "Make the whole interface larger",
        search_text: "ui scale zoom larger bigger increase interface",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::UiScaleDown),
        name: "Decrease UI scale",
        description: "Make the whole interface smaller",
        search_text: "ui scale zoom smaller decrease interface",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::UiScaleReset),
        name: "Reset UI scale",
        description: "Restore the interface to 100%",
        search_text: "ui scale zoom reset default actual size",
    },
    PaletteCommandEntry {
        command: PaletteCommand::Host(crate::host::keys::Action::OpenScratchpad),
        name: "Scratch Pad",
        description: "Open a fresh scratch note editor",
        search_text: "scratch pad scratchpad note editor inbox memo",
    },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::NewContext), name: "New context", description: "Create a new context", search_text: "new create open context workspace" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::RenameContext), name: "Rename context", description: "Rename the selected context", search_text: "rename context workspace" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::CloseContext), name: "Close context", description: "Close the selected context (confirmation required)", search_text: "close delete context workspace" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::ParkContext), name: "Park or unpark context", description: "Park or restore the selected context", search_text: "park unpark restore context workspace" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::ContextZoomOut), name: "Go to parent context", description: "Close the open overlay, else return to the parent context", search_text: "parent context zoom out back" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::NewTab), name: "New tab", description: "Create a new tab in the selected context", search_text: "new tab pane terminal" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::NewPageRight), name: "New window", description: "Create a new window in the current context", search_text: "new pane page window right" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::RenamePane), name: "Rename pane", description: "Rename the selected pane", search_text: "rename pane tab" },
    PaletteCommandEntry { command: PaletteCommand::CloseSelectedPanes, name: "Close selected panes", description: "Close every selected pane, asking about unsaved work one pane at a time", search_text: "close selected panes selection all multiple" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::ClosePane), name: "Close pane", description: "Close the focused pane (dismisses overlays and view back-stacks first)", search_text: "close pane tab" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::ToggleZoom), name: "Zoom pane", description: "Zoom or restore the selected pane (dives into an agent's children first)", search_text: "zoom restore pane fullscreen" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::ZoomIn), name: "Wrap selection or enter pane", description: "Wrap selected panes, enter a portal, or toggle pane zoom", search_text: "wrap selection group subcontext zoom enter" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::NextTab), name: "Next tab", description: "Focus the next tab", search_text: "next tab forward" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::PrevTab), name: "Previous tab", description: "Focus the previous tab", search_text: "previous tab back" },
    PaletteCommandEntry { command: PaletteCommand::Host(crate::host::keys::Action::NavBackView), name: "Back", description: "Go back in pane navigation or focus history", search_text: "back previous history pane" },
];

/// The pinned "Update ready" row, when a build is ready and the query matches.
fn update_ready_entry(ready: Option<&str>, query: &str) -> Option<PaletteEntry> {
    let version = ready?;
    let entry = PaletteEntry::UpdateReady {
        version: version.to_string(),
        search_text: searchable_text(&[
            "update ready restart to update upgrade new version",
            version,
        ]),
    };
    entry.matches_query(query).then_some(entry)
}

fn searchable_text(parts: &[&str]) -> String {
    parts.join(" ").to_lowercase()
}

fn normalized_palette_query(query: &str) -> String {
    query.trim().to_lowercase()
}

fn shortcut_label((modifiers, key): (egui::Modifiers, egui::Key)) -> String {
    let mut parts = Vec::new();
    if modifiers.command {
        parts.push(if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl"
        });
    }
    if modifiers.ctrl && !modifiers.command {
        parts.push(if cfg!(target_os = "macos") {
            "⌃"
        } else {
            "Ctrl"
        });
    }
    if modifiers.alt {
        parts.push(if cfg!(target_os = "macos") {
            "⌥"
        } else {
            "Alt"
        });
    }
    if modifiers.shift {
        parts.push(if cfg!(target_os = "macos") {
            "⇧"
        } else {
            "Shift"
        });
    }
    let separator = if cfg!(target_os = "macos") { "" } else { "+" };
    let prefix = parts.join(separator);
    let key_separator = (!prefix.is_empty() && !cfg!(target_os = "macos")).then_some("+");
    format!("{prefix}{}{}", key_separator.unwrap_or(""), key.name())
}

fn inactive_pane_matches_query(pane_name: &str, query: &str) -> bool {
    searchable_text(&[pane_name]).contains(query)
}

/// POSIX single-quote a shell argument: wrap in `'…'` and escape embedded
/// single quotes as `'\''`. Command names are usually bare identifiers, but
/// global script filenames may contain spaces.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Saved command execution is serviced by logic, preserving step order and
/// failure handling while keeping admission I/O out of UI dispatch.
#[derive(Clone)]
pub(crate) struct PendingUserCommand {
    pub(super) name: String,
    palette_scope: Option<crate::cli::UserCommandScope>,
    palette_root: Option<std::path::PathBuf>,
    context_root: std::path::PathBuf,
    source_window: u64,
    source_context: u64,
    source_tile: Option<egui_tiles::TileId>,
    caller: Option<u64>,
}

#[cfg(test)]
fn palette_command_matches(query: &str) -> Vec<PaletteCommand> {
    PALETTE_COMMANDS
        .iter()
        .filter(|entry| query.is_empty() || entry.search_text.contains(query))
        .map(|entry| entry.command.clone())
        .collect()
}

struct BuiltinPaletteView {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    gate: crate::release::ReleaseFeature,
}

/// Builtin host plugins surfaced in the palette alongside registry plugins.
const BUILTIN_VIEWS: &[BuiltinPaletteView] = &[BuiltinPaletteView {
    id: "assistant",
    name: "Assistant",
    description: "Host-native AI chat for this workspace",
    gate: crate::release::ReleaseFeature::Assistant,
}];

pub(crate) fn plugin_metadata_chips(
    running_in_background: bool,
    is_workspace_local: bool,
) -> &'static [&'static str] {
    match (running_in_background, is_workspace_local) {
        (false, false) => &["view"],
        (true, false) => &["view", "bg"],
        (false, true) => &["view", "ws"],
        (true, true) => &["view", "bg", "ws"],
    }
}

fn palette_pips_for_context(
    windows: &[crate::host::context::Window],
    context_active_window: &std::collections::HashMap<u64, u64>,
    ctx_id: u64,
    fallback_active_window_id: Option<u64>,
) -> Option<ListRowPips> {
    let mut ctx_windows: Vec<usize> = windows
        .iter()
        .enumerate()
        .filter(|(_, w)| w.context_id == ctx_id)
        .map(|(idx, _)| idx)
        .collect();
    ctx_windows.sort_by_key(|&idx| {
        let w = &windows[idx];
        (w.grid_y, w.grid_x)
    });

    let mut pane_ids = Vec::new();
    for &win_idx in &ctx_windows {
        let win = &windows[win_idx];
        if let Some(root) = win.tree.root() {
            pane_ids.extend(crate::spatial::tiling::collect_pane_ids_spatial(
                &win.tree.tiles,
                root,
            ));
        }
    }
    if pane_ids.is_empty() {
        return None;
    }

    let active_window_id = context_active_window
        .get(&ctx_id)
        .copied()
        .or_else(|| {
            fallback_active_window_id.filter(|win_id| {
                windows
                    .iter()
                    .any(|w| w.window_id == *win_id && w.context_id == ctx_id)
            })
        })
        .or_else(|| ctx_windows.first().map(|idx| windows[*idx].window_id));

    let focused_pane_id = active_window_id
        .and_then(|active_win_id| windows.iter().find(|w| w.window_id == active_win_id))
        .and_then(|win| {
            win.focused_pane
                .and_then(|tile_id| match win.tree.tiles.get(tile_id) {
                    Some(egui_tiles::Tile::Pane(pid)) => Some(*pid),
                    _ => None,
                })
        });
    let focused_idx = focused_pane_id.and_then(|pid| pane_ids.iter().position(|&p| p == pid));
    let activities = pane_ids
        .iter()
        .map(|pane_id| {
            windows
                .iter()
                .filter(|w| w.context_id == ctx_id)
                .find_map(|w| w.panes.get(pane_id))
                .and_then(|pane| pane.effective_activity())
                .cloned()
        })
        .collect();

    Some(ListRowPips {
        count: pane_ids.len(),
        focused_idx,
        activities,
        static_health_pips: pane_ids
            .iter()
            .enumerate()
            .filter_map(|(idx, pane_id)| {
                windows
                    .iter()
                    .filter(|w| w.context_id == ctx_id)
                    .find_map(|w| w.panes.get(pane_id))
                    .is_some_and(|pane| pane.static_health_pip())
                    .then_some(idx)
            })
            .collect(),
    })
}

/// One palette row per pane — the unpacked view shown for the active context
/// (and for query-pierced matches in collapsed contexts). Spatial order across
/// the context's windows, matching the pip strip on the collapsed `ctx` row.
struct PaneRow {
    window_id: u64,
    pane_id: u64,
    name: String,
    chip: &'static str,
    pips: ListRowPips,
    recency_rank: usize,
    spatial_idx: usize,
}

fn pane_recency_rank(
    active_window_id: u64,
    active_focused_tile: Option<egui_tiles::TileId>,
    focus_history: &[(u64, egui_tiles::TileId)],
    window_id: u64,
    tile_id: egui_tiles::TileId,
) -> usize {
    if window_id == active_window_id && active_focused_tile == Some(tile_id) {
        return 0;
    }

    focus_history
        .iter()
        .rev()
        .position(|(history_window_id, history_tile_id)| {
            *history_window_id == window_id && *history_tile_id == tile_id
        })
        .map(|idx| idx + 1)
        .unwrap_or(usize::MAX)
}

fn palette_pane_rows_for_context(
    windows: &[crate::host::context::Window],
    context_active_window: &std::collections::HashMap<u64, u64>,
    context_names: &[(u64, String)],
    ctx_id: u64,
    host_active_window_id: u64,
    active_focused_tile: Option<egui_tiles::TileId>,
    focus_history: &[(u64, egui_tiles::TileId)],
) -> Vec<PaneRow> {
    let mut ctx_windows: Vec<usize> = windows
        .iter()
        .enumerate()
        .filter(|(_, w)| w.context_id == ctx_id)
        .map(|(idx, _)| idx)
        .collect();
    ctx_windows.sort_by_key(|&idx| {
        let w = &windows[idx];
        (w.grid_y, w.grid_x)
    });

    let context_focused_window_id = context_active_window
        .get(&ctx_id)
        .copied()
        .filter(|id| {
            windows
                .iter()
                .any(|w| w.window_id == *id && w.context_id == ctx_id)
        })
        .or_else(|| ctx_windows.first().map(|idx| windows[*idx].window_id));
    let focused_pane_id = context_focused_window_id
        .and_then(|active_win_id| windows.iter().find(|w| w.window_id == active_win_id))
        .and_then(|win| {
            win.focused_pane
                .and_then(|tile_id| match win.tree.tiles.get(tile_id) {
                    Some(egui_tiles::Tile::Pane(pid)) => Some(*pid),
                    _ => None,
                })
        });

    let mut rows = Vec::new();
    let mut spatial_idx = 0;
    for &win_idx in &ctx_windows {
        let win = &windows[win_idx];
        let Some(root) = win.tree.root() else {
            continue;
        };
        for pane_id in crate::spatial::tiling::collect_pane_ids_spatial(&win.tree.tiles, root) {
            let Some(pane) = win.panes.get(&pane_id) else {
                continue;
            };
            let Some(tile_id) = win.tree.tiles.find_pane(&pane_id) else {
                continue;
            };
            let (name, chip) = pane_row_identity(pane, context_names);
            rows.push(PaneRow {
                window_id: win.window_id,
                pane_id,
                name,
                chip,
                pips: ListRowPips {
                    count: 1,
                    focused_idx: (Some(pane_id) == focused_pane_id).then_some(0),
                    activities: vec![pane.effective_activity().cloned()],
                    static_health_pips: if pane.static_health_pip() {
                        vec![0]
                    } else {
                        Vec::new()
                    },
                },
                recency_rank: pane_recency_rank(
                    host_active_window_id,
                    active_focused_tile,
                    focus_history,
                    win.window_id,
                    tile_id,
                ),
                spatial_idx,
            });
            spatial_idx += 1;
        }
    }
    rows.sort_by_key(|row| (row.recency_rank, row.spatial_idx));
    rows
}

/// Display name + type chip for a palette pane row.
fn pane_row_identity(
    pane: &crate::host::pane::Pane,
    context_names: &[(u64, String)],
) -> (String, &'static str) {
    match pane {
        crate::host::pane::Pane::Terminal(t) => (
            t.name
                .clone()
                .or_else(|| t.pty_title.clone())
                .unwrap_or_else(|| "terminal".to_string()),
            "term",
        ),
        crate::host::pane::Pane::View(a) => {
            let chip = if a.manifest_id == "text-editor" {
                "text"
            } else {
                "view"
            };
            let name = if a.name.is_empty() {
                a.runtime.display_name()
            } else {
                a.name.clone()
            };
            (name, chip)
        }
        crate::host::pane::Pane::Portal(p) => (
            context_names
                .iter()
                .find(|(id, _)| *id == p.target_context_id)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "sub-context".to_string()),
            "ctx",
        ),
    }
}

/// Key chord shown next to a palette command, if the host binds one.
fn palette_host_binding(
    bindings: &crate::host::keys::KeyBindings,
    command: &PaletteCommand,
) -> Option<(egui::Modifiers, egui::Key)> {
    use crate::host::keys::Action;
    let binding = match command {
        PaletteCommand::Host(Action::NextBlockedAgent) => bindings.next_blocked_agent,
        PaletteCommand::Host(Action::FocusApproval) => bindings.focus_approval,
        PaletteCommand::Host(Action::OpenAgents) => bindings.open_agents,
        PaletteCommand::Host(Action::OpenSecretsManager) => bindings.open_secrets_manager,
        // Cmd+, opens the chooser rather than this global-only action.
        PaletteCommand::Host(Action::OpenConfig) => return None,
        PaletteCommand::Host(Action::ReloadConfig) => bindings.reload_config,
        PaletteCommand::Host(Action::OpenNotesPicker) => bindings.open_notes_picker,
        PaletteCommand::Host(Action::OpenFileBrowser) => bindings.open_file_browser,
        PaletteCommand::Host(Action::ToggleAttentionQueue) => bindings.toggle_attention_queue,
        PaletteCommand::Host(Action::ZoomIn) => bindings.zoom_in,
        PaletteCommand::Host(Action::SplitRight) => bindings.split_right,
        PaletteCommand::Host(Action::SplitDown) => bindings.split_down,
        PaletteCommand::Host(Action::OpenQuickNote) => bindings.open_quick_note,
        PaletteCommand::Host(Action::OpenScratchpad) => bindings.open_scratchpad,
        PaletteCommand::Host(Action::NewContext) => bindings.new_context,
        PaletteCommand::Host(Action::RenameContext) => bindings.rename_context,
        PaletteCommand::Host(Action::CloseContext) => bindings.close_context,
        PaletteCommand::Host(Action::ParkContext) => bindings.park_context,
        PaletteCommand::Host(Action::ContextZoomOut) => bindings.context_zoom_out,
        PaletteCommand::Host(Action::NewTab) => bindings.new_tab,
        PaletteCommand::Host(Action::NextTab) => bindings.next_tab,
        PaletteCommand::Host(Action::PrevTab) => bindings.prev_tab,
        PaletteCommand::Host(Action::NewPageRight) => bindings.new_page_right,
        PaletteCommand::Host(Action::RenamePane) => bindings.rename_pane,
        PaletteCommand::Host(Action::ClosePane) | PaletteCommand::CloseSelectedPanes => {
            bindings.close_pane
        }
        PaletteCommand::Host(Action::ToggleZoom) => bindings.toggle_zoom,
        PaletteCommand::Host(Action::NavBackView) => return bindings.nav_back,
        PaletteCommand::Host(Action::ToggleShortcuts) => bindings.toggle_shortcuts,
        PaletteCommand::Host(Action::IncreasePaneFontSize) => bindings.increase_font_size,
        PaletteCommand::Host(Action::DecreasePaneFontSize) => bindings.decrease_font_size,
        PaletteCommand::Host(Action::ResetPaneFontSize) => bindings.reset_font_size,
        PaletteCommand::Host(Action::UiScaleUp) => bindings.ui_scale_up,
        PaletteCommand::Host(Action::UiScaleDown) => bindings.ui_scale_down,
        PaletteCommand::Host(Action::UiScaleReset) => bindings.ui_scale_reset,
        PaletteCommand::OpenGlobalConfig
        | PaletteCommand::OpenWorkspaceConfig
        | PaletteCommand::EditCommands
        | PaletteCommand::SwitchCommandsScope
        | PaletteCommand::OpenAttention
        | PaletteCommand::SetupCli
        | PaletteCommand::StatePreset(_)
        | PaletteCommand::UiUpcycle(_)
        | PaletteCommand::CheckForUpdates
        | PaletteCommand::RestartToUpdate => return None,
        PaletteCommand::Host(other) => {
            log_missing_palette_shortcut_once(other);
            return None;
        }
    };
    Some(binding)
}

/// Bindings with no palette label are logged once per action per process.
/// Repeating the line on every Cmd+P open (the palette paints each command
/// on every frame) drowned the host log.
fn log_missing_palette_shortcut_once(action: &crate::host::keys::Action) {
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let key = format!("{action:?}");
    let mut seen = SEEN.lock().unwrap_or_else(|err| err.into_inner());
    if seen.iter().any(|seen| seen == &key) {
        return;
    }
    seen.push(key);
    log::debug!("palette: no shortcut label for host binding {action:?}");
}

impl PlexiApp {
    fn palette_shortcut(&self, command: &PaletteCommand) -> Option<String> {
        palette_host_binding(&self.key_bindings, command).map(shortcut_label)
    }

    /// Commands that only apply in some states are listed only then.
    fn palette_command_listed(&self, command: &PaletteCommand) -> bool {
        match command {
            PaletteCommand::OpenWorkspaceConfig => self.palette_config_workspace_root.is_some(),
            // A selection of one is just the focused pane: "Close pane" covers it.
            PaletteCommand::CloseSelectedPanes => self
                .host
                .pane_selection(self.windows[self.active_window].window_id)
                .is_some_and(|selection| selection.panes.len() >= 2),
            _ => true,
        }
    }

    /// The Commands stop's source: today's rows for `raw_query`, and the
    /// pane-pip decoration of each row that carries one. Files and agents
    /// never appear here; they have their own stops.
    fn palette_command_rows(
        &mut self,
        raw_query: &str,
    ) -> (
        Vec<PickerRow>,
        std::collections::HashMap<RowKey, ListRowPips>,
        std::collections::HashMap<RowKey, CommandRowId>,
    ) {
        #[cfg(test)]
        palette_test_log::source_called();
        let query = normalized_palette_query(raw_query);

        // ── Window entries (active context first, then by visit recency) ──
        // Two-tier sort: panes whose window belongs to the active context float
        // above panes in other contexts. Within each tier, recency wins.
        // Mirrors macOS Cmd+Tab — current plugin's windows first.
        let active_win_id = self.windows[self.active_window].window_id;
        let active_ctx_id = self.windows[self.active_window].context_id;
        let active_focused_tile = self.windows[self.active_window].focused_pane;

        let rank_of = |win_id: u64| -> (usize, usize) {
            let in_active_ctx = self
                .windows
                .iter()
                .find(|w| w.window_id == win_id)
                .map(|w| w.context_id == active_ctx_id)
                .unwrap_or(false);
            let tier = if in_active_ctx { 0 } else { 1 };
            let recency = if win_id == active_win_id {
                0
            } else {
                self.context_visit_history
                    .iter()
                    .position(|&id| id == win_id)
                    .map(|p| p + 1)
                    .unwrap_or(usize::MAX)
            };
            (tier, recency)
        };

        let mut entries: Vec<PaletteEntry> = Vec::new();

        let context_names: Vec<(u64, String)> = self
            .router
            .iter()
            .map(|c| (c.context_id, c.name.to_string()))
            .collect();

        let ctx_entries: Vec<PaletteEntry> = {
            // Context-scoped unpacking: the ACTIVE context expands to one row
            // per pane (each carrying a single status pip), while every OTHER
            // context collapses to one `ctx` row carrying its full pip strip.
            // A non-empty query pierces the collapse — matching pane names
            // surface from inactive contexts so the palette stays a global
            // jump tool.
            // Every context stays addressable here, including parked contexts
            // and metadata that outlived its last live window. A parked
            // context is always collapsed, even in the all-parked case where
            // it happens to be the active router context.
            struct CtxInfo {
                ctx_id: u64,
                name: String,
                parked: bool,
                resolved: Option<(usize, u64)>,
            }
            let mut contexts: Vec<CtxInfo> = Vec::new();
            for ctx_meta in self.router.iter() {
                let resolved =
                    self.context_active_window
                        .get(&ctx_meta.context_id)
                        .copied()
                        .and_then(|id| {
                            self.windows.iter().enumerate().find(|(_, w)| {
                                w.window_id == id && w.context_id == ctx_meta.context_id
                            })
                        })
                        .or_else(|| {
                            self.windows
                                .iter()
                                .enumerate()
                                .find(|(_, w)| w.context_id == ctx_meta.context_id)
                        });
                contexts.push(CtxInfo {
                    ctx_id: ctx_meta.context_id,
                    name: ctx_meta.name.to_string(),
                    parked: ctx_meta.parked,
                    resolved: resolved.map(|(win_idx, win)| (win_idx, win.window_id)),
                });
            }
            // rank_of already tiers active-context windows first, then recency.
            contexts.sort_by_key(|c| {
                c.resolved
                    .map(|(_, window_id)| rank_of(window_id))
                    .unwrap_or((2, usize::MAX))
            });

            let mut ctx_entries: Vec<PaletteEntry> = Vec::new();
            for c in &contexts {
                let Some((_win_idx, window_id)) = c.resolved else {
                    let search_text = searchable_text(&[c.name.as_str(), "parked unavailable"]);
                    if query.is_empty() || search_text.contains(&query) {
                        ctx_entries.push(PaletteEntry::UnavailableContext {
                            name: c.name.clone(),
                            search_text,
                        });
                    }
                    continue;
                };
                if c.ctx_id == active_ctx_id && !c.parked {
                    // Active context — unpack every pane, one pip each.
                    for row in palette_pane_rows_for_context(
                        &self.windows,
                        &self.context_active_window,
                        &context_names,
                        c.ctx_id,
                        active_win_id,
                        active_focused_tile,
                        &self.pane_focus_history,
                    ) {
                        let search_text = searchable_text(&[row.name.as_str(), c.name.as_str()]);
                        if query.is_empty() || search_text.contains(&query) {
                            ctx_entries.push(PaletteEntry::Context {
                                target: PaletteFocusTarget {
                                    context_id: c.ctx_id,
                                    window_id: row.window_id,
                                    pane_id: Some(row.pane_id),
                                },
                                name: row.name,
                                workspace_name: c.name.clone(),
                                metadata_chips: vec![row.chip],
                                pane_pips: Some(row.pips),
                                search_text,
                            });
                        }
                    }
                } else {
                    // Inactive and parked contexts — one collapsed row, full
                    // pip strip. Search still pierces the collapse below.
                    let search_text = searchable_text(&[c.name.as_str()]);
                    if query.is_empty() || search_text.contains(&query) {
                        ctx_entries.push(PaletteEntry::Context {
                            target: PaletteFocusTarget {
                                context_id: c.ctx_id,
                                window_id,
                                pane_id: None,
                            },
                            name: c.name.clone(),
                            workspace_name: if c.parked {
                                "Parked".to_string()
                            } else {
                                String::new()
                            },
                            metadata_chips: if c.parked {
                                vec!["ctx", "parked"]
                            } else {
                                vec!["ctx"]
                            },
                            pane_pips: palette_pips_for_context(
                                &self.windows,
                                &self.context_active_window,
                                c.ctx_id,
                                Some(active_win_id),
                            ),
                            search_text,
                        });
                    }
                    if !query.is_empty() {
                        // Search pierces the collapse — match on pane name only,
                        // so typing a context name keeps the single ctx row.
                        for row in palette_pane_rows_for_context(
                            &self.windows,
                            &self.context_active_window,
                            &context_names,
                            c.ctx_id,
                            active_win_id,
                            active_focused_tile,
                            &self.pane_focus_history,
                        ) {
                            // A context-name match keeps this context collapsed.
                            // Only the pane's own title pierces the collapse.
                            let search_text = searchable_text(&[row.name.as_str()]);
                            if inactive_pane_matches_query(&row.name, &query) {
                                ctx_entries.push(PaletteEntry::Context {
                                    target: PaletteFocusTarget {
                                        context_id: c.ctx_id,
                                        window_id: row.window_id,
                                        pane_id: Some(row.pane_id),
                                    },
                                    name: row.name,
                                    workspace_name: if c.parked {
                                        format!("Parked · {}", c.name)
                                    } else {
                                        c.name.clone()
                                    },
                                    metadata_chips: if c.parked {
                                        vec![row.chip, "parked"]
                                    } else {
                                        vec![row.chip]
                                    },
                                    pane_pips: Some(row.pips),
                                    search_text,
                                });
                            }
                        }
                    }
                }
            }

            ctx_entries
        };
        entries.extend(ctx_entries);

        // ── Note entries ────────────────────────────────────────────────────
        for note in self
            .palette_notes
            .iter()
            .filter(|_| !self.config.core_only.unwrap_or(false))
        {
            let search_text = searchable_text(&[note.title.as_str(), note.search_text.as_str()]);
            let matches = query.is_empty() || search_text.contains(&query);
            if matches {
                entries.push(PaletteEntry::Note {
                    path: note.path.clone(),
                    title: note.title.clone(),
                    preview: note.preview.clone(),
                    search_text,
                });
            }
        }

        // ── Workspace-aware plugin entries ────────────────────────────────────
        // Use the workspace root cached at palette-open time (not re-resolved
        // per frame) to avoid filesystem traversal in the egui draw loop.
        //
        // The palette is a viewer surface for the focused window, not a
        // resource owner (stint 0724 Phase B) — it resolves its view by the
        // focused pane's own resolved workspace root via
        // `RegistryViews::view_for_root`, a synchronous per-root cache. No
        // staleness comparison is needed the way the old single shared
        // `PluginRegistry` required: a root already seen is a cache hit, and a
        // never-seen root loads on this first access.
        let focused_workspace_root = self.palette_workspace_root.clone();
        let home = dirs::home_dir();
        let rescan_cwd = focused_workspace_root
            .as_deref()
            .or(home.as_deref())
            .unwrap_or(std::path::Path::new("/"));

        let plugin_entries: Vec<(String, String, String, bool, String)> = self
            .registries
            .view_for_root(rescan_cwd)
            .list()
            .into_iter()
            .filter_map(|app| {
                // Local plugins are visible only when the focused pane is in their workspace.
                // Use the same explicit predicate here as in the badge below — no wildcard.
                let workspace_visible = match app.source {
                    crate::app::registry::RegistrySource::Global => true,
                    crate::app::registry::RegistrySource::LocalPlugin
                    | crate::app::registry::RegistrySource::LocalAgent => {
                        app.workspace_root.as_ref() == focused_workspace_root.as_ref()
                    }
                };
                if !workspace_visible {
                    return None;
                }
                let search_text = searchable_text(&[
                    app.manifest.name.as_str(),
                    app.manifest.id.as_str(),
                    app.manifest.description.as_str(),
                ]);
                if !query.is_empty() && !search_text.contains(&query) {
                    return None;
                }
                let is_local = matches!(
                    app.source,
                    crate::app::registry::RegistrySource::LocalPlugin
                        | crate::app::registry::RegistrySource::LocalAgent
                );
                Some((
                    app.manifest.id.clone(),
                    app.manifest.name.clone(),
                    app.manifest.description.clone(),
                    is_local,
                    search_text,
                ))
            })
            .collect();

        for app in BUILTIN_VIEWS {
            if self.config.core_only.unwrap_or(false) {
                continue;
            }
            if !crate::release::feature_enabled(app.gate) {
                continue;
            }
            let search_text = searchable_text(&[app.name, app.id, app.description]);
            if query.is_empty() || search_text.contains(&query) {
                entries.push(PaletteEntry::Builtin {
                    id: app.id,
                    name: app.name,
                    description: app.description,
                    search_text,
                });
            }
        }

        for (id, name, description, is_workspace_local, search_text) in plugin_entries {
            let running_in_background = self.background_apps.contains_key(&id);
            entries.push(PaletteEntry::View {
                id,
                name,
                description,
                running_in_background,
                is_workspace_local,
                search_text,
            });
        }

        entries.extend(
            PALETTE_COMMANDS
                .iter()
                .filter(|entry| self.palette_command_listed(&entry.command))
                .map(|entry| PaletteEntry::Command {
                    command: entry.command.clone(),
                    name: if entry.command == PaletteCommand::OpenWorkspaceConfig
                        && !self.palette_workspace_config_exists
                    {
                        "Open config: this repo (not created yet)"
                    } else {
                        entry.name
                    },
                    description: entry.description,
                    search_text: entry.search_text,
                })
                .filter(|entry| entry.matches_query(&query)),
        );

        // ── User commands (commands.toml + global scripts) ──────────────────
        // Resolved at palette-open time into self.palette_commands; mirrors
        // `plexi run` precedence. Executed via `plexi run <name>` on select.
        for cmd in &self.palette_commands {
            let scope_word = match cmd.scope {
                crate::cli::UserCommandScope::Workspace => "workspace ws",
                crate::cli::UserCommandScope::Global => "global script",
            };
            let secondary = match (&cmd.description, &cmd.run) {
                (Some(desc), _) => desc.clone(),
                (None, Some(run)) => run.clone(),
                (None, None) if !cmd.steps.is_empty() => format!("{} steps", cmd.steps.len()),
                (None, None) => "global script".to_string(),
            };
            let search_text =
                searchable_text(&[cmd.name.as_str(), secondary.as_str(), "run", scope_word]);
            if query.is_empty() || search_text.contains(&query) {
                entries.push(PaletteEntry::UserCommand {
                    name: cmd.name.clone(),
                    secondary,
                    scope: cmd.scope,
                    search_text,
                });
            }
        }

        if let Some(entry) = update_ready_entry(self.update_status.ready(), &query) {
            entries.push(entry);
        }
        let mut rows = Vec::with_capacity(entries.len());
        let mut pips = std::collections::HashMap::new();
        let mut targets = std::collections::HashMap::new();
        for entry in entries {
            let shortcut = match &entry {
                PaletteEntry::Command { command, .. } => self.palette_shortcut(command),
                _ => None,
            };
            let target = entry.row_id();
            let (row, row_pips) = entry.into_row(&query, shortcut);
            if let Some(row_pips) = row_pips {
                pips.insert(row.key.clone(), row_pips);
            }
            targets.insert(row.key.clone(), target);
            rows.push(row);
        }
        (rows, pips, targets)
    }

    pub(crate) fn command_palette_open(&self) -> bool {
        self.host
            .picker(PickerId::Palette)
            .is_some_and(|picker| picker.is_open())
    }

    /// Open the palette on its first stop with `query` typed. Callers resolve
    /// the open-time caches (`palette_workspace_root`, notes, user commands)
    /// first.
    pub(crate) fn open_command_palette(&mut self, query: &str) {
        // Usage history loads in the background on the first open.
        self.palette_sources
            .usage
            .start(crate::host::picker::usage::usage_path());
        self.palette_input.reset(query);
        self.apply_palette_command(PickerCommand::Open {
            stop: PALETTE_STOPS[0].id,
            query: query.to_owned(),
        });
        self.palette_input.opening = self
            .focus_stack
            .opening(crate::app::FocusKind::CommandPalette)
            .map(|opening| opening.id);
    }

    pub(crate) fn close_command_palette(&mut self) {
        self.apply_palette_command(PickerCommand::Close);
    }

    /// Run one picker command through the model and service its effects in
    /// order. `Query` is answered from `App::logic` (`refresh_command_palette`),
    /// so no source I/O runs on the draw path. `Activate` comes before
    /// `Closed`; the row runs after the close, with focus synchronized once.
    pub(crate) fn apply_palette_command(&mut self, command: PickerCommand) {
        let effects = self
            .host
            .apply(HostAction::Picker(PickerId::Palette, command));
        if effects.is_empty() {
            return;
        }
        let mut activation = None;
        for HostEffect::Picker(id, effect) in effects {
            match effect {
                PickerEffect::Query {
                    source,
                    text,
                    generation,
                } => log::debug!(
                    "palette: {id:?} query generation {generation} ({} chars) waits for source '{source}' in logic",
                    text.chars().count()
                ),
                PickerEffect::Activate { stamp, row, action } => {
                    self.palette_sources.usage.record(row.as_str());
                    // Owned before `Closed` drops the answer it came from.
                    let target = self
                        .palette_answer
                        .as_ref()
                        .filter(|answer| answer.generation == stamp.generation)
                        .and_then(|answer| answer.targets.get(&row))
                        .cloned();
                    match target {
                        Some(target) if target.action().id == action => {
                            activation = Some(target);
                        }
                        Some(_) => log::warn!("palette: row has no action '{action}'"),
                        None => log::warn!("palette: no target for an activated row"),
                    }
                }
                PickerEffect::LoadPreview { row, kind } => self.load_palette_preview(row, kind),
                PickerEffect::Closed => {
                    self.palette_input.reset("");
                    self.palette_answer = None;
                    let sources = &mut self.palette_sources;
                    sources.root = None;
                    sources.asked = None;
                    sources.texture = None;
                }
            }
        }
        self.sync_command_palette_focus();
        if let Some(target) = activation {
            self.run_palette_target(target);
        }
    }

    /// Ask the preview worker for `row`'s preview. Its path comes from the
    /// answer the row belongs to.
    fn load_palette_preview(&mut self, row: RowKey, kind: crate::host::picker::PreviewRef) {
        let path = self
            .palette_answer
            .as_ref()
            .and_then(|answer| answer.targets.get(&row))
            .and_then(|target| match target {
                PaletteTarget::File(path) => Some(path.clone()),
                PaletteTarget::Command(_) => None,
            });
        let Some(path) = path else {
            log::warn!("palette: a preview was asked for a row with no file");
            return;
        };
        if self.palette_sources.preview.is_none() {
            match PreviewLoader::spawn(std::sync::Arc::clone(&self.ui_wake)) {
                Ok(loader) => self.palette_sources.preview = Some(loader),
                Err(error) => {
                    log::warn!("palette: {error}");
                    self.apply_palette_command(PickerCommand::Preview {
                        row,
                        body: crate::host::picker::PreviewBody::Note(format!(
                            "Could not read: {error}"
                        )),
                    });
                    return;
                }
            }
        }
        if let Some(loader) = self.palette_sources.preview.as_mut() {
            loader.load(row, path, kind);
        }
        // Nothing else wakes the host when a read never answers.
        self.ctx
            .request_repaint_after(crate::host::picker::preview::PREVIEW_TIMEOUT);
    }

    /// Install finished previews, uploading an image's texture here, in
    /// `App::logic`. A preview for a row no longer loading is the reducer's
    /// to drop; its pixels are never uploaded.
    fn drain_palette_previews(&mut self) {
        let Some(loader) = self.palette_sources.preview.as_mut() else {
            return;
        };
        for done in loader.poll(std::time::Instant::now()) {
            let wanted = self.host.picker(PickerId::Palette).is_some_and(|picker| {
                picker.selected() == Some(&done.row)
                    && matches!(picker.preview(), Some(PreviewStatus::Loading))
            });
            if wanted {
                if let Some(pixels) = &done.pixels {
                    let size = [pixels.width() as usize, pixels.height() as usize];
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw());
                    let texture = self.ctx.load_texture(
                        "palette-preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.palette_sources.texture = Some((done.row.clone(), texture));
                }
            }
            self.apply_palette_command(PickerCommand::Preview {
                row: done.row,
                body: done.body,
            });
        }
    }

    /// Answer the Files stop: ask the index worker once per generation and
    /// install whatever it answered since. The first Files query of a
    /// session fixes the context root and refreshes its index.
    fn answer_palette_files(&mut self, generation: u64, query: &str) {
        let sources = &mut self.palette_sources;
        if sources.files.is_none() {
            match FileIndex::spawn(
                std::sync::Arc::new(crate::host::picker::files::walk_root),
                std::sync::Arc::clone(&self.ui_wake),
            ) {
                Ok(index) => sources.files = Some(index),
                Err(error) => {
                    log::warn!("palette: {error}");
                    return;
                }
            }
        }
        let Some(index) = sources.files.as_ref() else {
            return;
        };
        let root = match &sources.root {
            Some(root) => root.clone(),
            None => {
                let root = self.router.active().root.clone();
                log::info!("palette: Files stop lists the active context root");
                index.refresh(root.clone());
                sources.root = Some(root.clone());
                root
            }
        };
        if sources.asked != Some(generation) {
            index.query(
                root,
                query.to_owned(),
                generation,
                sources.usage.book().clone(),
            );
            sources.asked = Some(generation);
        }
        for answer in index.answers() {
            if answer.generation != generation {
                continue;
            }
            let mut targets = std::collections::HashMap::new();
            let rows: Vec<PickerRow> = answer
                .hits
                .into_iter()
                .map(|hit| {
                    targets.insert(hit.key.clone(), PaletteTarget::File(hit.path.clone()));
                    file_row(hit)
                })
                .collect();
            self.palette_answer = Some(PaletteAnswer {
                generation,
                pips: std::collections::HashMap::new(),
                targets,
            });
            self.apply_palette_command(PickerCommand::Rows {
                generation,
                rows,
                done: answer.done,
            });
        }
    }

    /// Answer the open stop's current query: a new query's first answer, and
    /// every later pass so live rows (pane state, context pips, update ready)
    /// stay current. Serviced from `App::logic`, never from drawing.
    pub(crate) fn refresh_command_palette(&mut self) {
        self.palette_sources.usage.poll();
        self.drain_palette_previews();
        let Some((source, query, generation)) = self
            .host
            .picker(PickerId::Palette)
            .filter(|picker| picker.is_open())
            .map(|picker| {
                (
                    picker.active_stop().source,
                    picker.query().to_owned(),
                    picker.generation(),
                )
            })
        else {
            return;
        };
        // A deferred Enter completes only while the palette still owns input.
        if !self.palette_owns_input() {
            self.apply_palette_command(PickerCommand::Yield);
        }
        #[cfg(test)]
        if let Some(scripted) = palette_test_log::scripted_answer(generation, &query) {
            self.answer_scripted(generation, scripted);
            return;
        }
        let rows = match source {
            COMMANDS_SOURCE => {
                let (rows, pips, targets) = self.palette_command_rows(&query);
                // Installed with its rows: an Enter this answer completes
                // resolves against exactly this table.
                self.palette_answer = Some(PaletteAnswer {
                    generation,
                    pips,
                    targets: command_targets(targets),
                });
                rows
            }
            FILES_SOURCE => {
                self.answer_palette_files(generation, &query);
                return;
            }
            other => {
                log::warn!("palette: no source '{other}'");
                Vec::new()
            }
        };
        self.apply_palette_command(PickerCommand::Rows {
            generation,
            rows,
            done: true,
        });
    }

    /// A scripted test answer in place of the Commands source: rows named
    /// by view id, each targeting `CommandRowId::View`, installed exactly as
    /// the source's answer is. A stale generation's answer is rejected with
    /// its targets.
    #[cfg(test)]
    fn answer_scripted(&mut self, current: u64, scripted: Option<(u64, Vec<String>, bool)>) {
        let Some((generation, ids, done)) = scripted else {
            return;
        };
        let targets: std::collections::HashMap<RowKey, CommandRowId> = ids
            .iter()
            .map(|id| {
                let target = CommandRowId::View(id.clone());
                (target.key(), target)
            })
            .collect();
        let rows = ids
            .iter()
            .map(|id| PickerRow {
                key: CommandRowId::View(id.clone()).key(),
                title: id.clone(),
                secondary: String::new(),
                chips: Vec::new(),
                group: "",
                group_rank: 0,
                score: None,
                pinned: false,
                unavailable: None,
                actions: vec![CommandRowId::View(id.clone()).action()],
                usage: None,
                modified: None,
                preview: None,
            })
            .collect();
        if generation == current {
            self.palette_answer = Some(PaletteAnswer {
                generation,
                pips: std::collections::HashMap::new(),
                targets: command_targets(targets),
            });
        }
        self.apply_palette_command(PickerCommand::Rows {
            generation,
            rows,
            done,
        });
    }

    /// Whether this session's palette is the surface that owns input: its
    /// opening is the focus stack's top, not merely open beneath a surface
    /// it handed focus to.
    pub(crate) fn palette_owns_input(&self) -> bool {
        self.command_palette_open()
            && self.focus_stack.top_opening().is_some_and(|top| {
                top.kind == crate::app::FocusKind::CommandPalette
                    && Some(top.id) == self.palette_input.opening
            })
    }

    /// Note, in `App::logic`, that this pass's input belongs to the open
    /// session, before a deferred activation can close it and hand the
    /// pass to whatever takes focus next.
    pub(crate) fn note_palette_input_owner(&mut self, ctx: &egui::Context) {
        self.palette_input.owned_pass = self.palette_owns_input().then(|| ctx.cumulative_pass_nr());
    }

    /// The palette's ordered input adapter, run once at the start of
    /// `App::ui`, before close-key rewriting, Quick Note preemption or any
    /// owner takes input (stint 1391). egui has already folded the delivery,
    /// so it saw both edges of every key and drives the results' scroll area
    /// natively; this reads egui's ordered `events` and gives the palette its
    /// share in arrival order:
    /// - while the palette owns input, host chords and picker keys resolve at
    ///   their own position (the close chord, Quick Note, the binding table
    ///   with overlay rules, then the picker's keys). The field is egui's own
    ///   text edit and reads its edits when it draws, so a command that
    ///   follows an edit (or a pointer activation) waits until the field and
    ///   rows have drawn this pass, then acts in order
    ///   (`finish_palette_draw`). An edit that follows such a command waits
    ///   for the next pass's field, ahead of that pass's own input;
    /// - a Cmd+P from a pane ends the old owner's share at its position. The
    ///   input before it finishes through the normal host and pane dispatch
    ///   of this pass; the session opens after that (`finish_palette_opening`)
    ///   and what followed the chord is its own;
    /// - once a session closes, the rest of its delivery is still its own, as
    ///   is the input of a pass whose logic closed it, and a press or phased
    ///   scroll it began stays its own until released or ended.
    ///
    /// What the palette takes leaves `events`, so no later reader (the
    /// binding table, a pane, a terminal) acts on it. Key releases stay, so
    /// the binding table retires the release claims of the chords it saw.
    /// A press the palette owns is recorded on the focus stack, which is what
    /// pane and tab readers of egui's folded pointer state check.
    pub(crate) fn route_palette_input(&mut self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        let logic_owned = self.palette_input.owned_pass.take() == Some(pass);
        self.palette_input.fed.clear();
        let stale = std::mem::take(&mut self.palette_input.after_draw);
        if !stale.is_empty() {
            log::info!(
                "palette: {} input event(s) waiting for a draw that never came were dropped",
                stale.len()
            );
        }
        self.palette_input.activation_at = None;
        self.palette_input.hover_at = None;
        let waiting = std::mem::take(&mut self.palette_input.waiting);
        if !waiting.is_empty() {
            if self.palette_owns_input() {
                log::debug!(
                    "palette: {} waiting field edit(s) lead this pass",
                    waiting.len()
                );
                ctx.input_mut(|i| {
                    let fresh = std::mem::replace(&mut i.events, waiting);
                    i.events.extend(fresh);
                });
            } else {
                log::info!(
                    "palette: {} waiting field edit(s) expired with their session",
                    waiting.len()
                );
            }
        }
        // A release lost to another window: egui holds no button and this
        // delivery releases none, so every gesture is over.
        if ctx.input(|i| {
            !i.pointer.any_down()
                && !i
                    .events
                    .iter()
                    .any(|e| matches!(e, egui::Event::PointerButton { pressed: false, .. }))
        }) {
            self.palette_input.held.clear();
            self.palette_input.pane_held.clear();
        }
        let owns = self.palette_owns_input();
        let continuing = !self.palette_input.held.is_empty() || self.palette_input.scrolling;
        if !owns && !logic_owned && !continuing {
            let opening = self.overlay_surface().is_none()
                && ctx.input(|i| i.events.iter().any(|e| self.palette_toggle_chord(e)));
            if !opening {
                let events = ctx.input(|i| i.events.clone());
                for event in &events {
                    self.note_pane_pointer(event);
                }
                return;
            }
        }
        let events = ctx.input_mut(|i| std::mem::take(&mut i.events));
        let mut kept = Vec::with_capacity(events.len());
        let mut closed_session = logic_owned;
        let mut dropped = 0usize;
        // Whether the field has focus to edit with. egui's own begin-of-pass
        // handling (an Escape in this delivery) may have dropped focus the
        // field held last pass; the reconciler gives it back.
        let field_focused = ctx.memory(|m| {
            m.has_focus(palette_field_id()) || m.had_focus_last_frame(palette_field_id())
        });
        let mut order = DrawOrder::default();
        for (index, event) in events.into_iter().enumerate() {
            let release = matches!(event, egui::Event::Key { pressed: false, .. });
            // A gesture a pane took keeps its moves and release, whoever
            // owns input now.
            if self.pane_gesture_continues(&event) {
                self.note_pane_pointer(&event);
                kept.push(event);
                continue;
            }
            if self.palette_owns_input() {
                closed_session = true;
                self.note_palette_pointer(&event);
                if self.take_owned_palette_event(ctx, index, &event, field_focused, &mut order) {
                    continue;
                }
                if release || !is_palette_input(&event) || is_field_input(&event) {
                    kept.push(event);
                }
                continue;
            }
            if self.palette_input.opener.is_some() {
                if release || !is_palette_input(&event) {
                    kept.push(event);
                } else if is_field_input(&event) {
                    self.palette_input.opening_tail.push(event);
                } else {
                    self.note_palette_pointer(&event);
                }
                continue;
            }
            if closed_session || self.palette_pointer_continues(&event) {
                if release || !is_palette_input(&event) {
                    kept.push(event);
                } else {
                    self.note_palette_pointer(&event);
                    dropped += 1;
                }
                continue;
            }
            if self.overlay_surface().is_none() && self.palette_toggle_chord(&event) {
                log::info!("palette: opening chord at input {index} of its delivery");
                // Resolved here, so the chord's release claim is in place for
                // a key-up later in this delivery; it acts after the old
                // owner has finished what came before it.
                self.palette_input.opener =
                    Some(self.palette_host_actions(&event).unwrap_or_default());
                continue;
            }
            self.note_pane_pointer(&event);
            kept.push(event);
        }
        if dropped > 0 {
            log::info!("palette: {dropped} input event(s) after its session closed were its own");
        }
        if !ctx.input(|i| i.pointer.any_down()) {
            self.palette_input.held.clear();
            self.palette_input.pane_held.clear();
        }
        ctx.input_mut(|i| {
            let late = std::mem::replace(&mut i.events, kept);
            i.events.extend(late);
        });
    }

    /// One event while the palette owns input. Returns whether a host chord
    /// or the picker took it (a release nobody claimed stays for egui's
    /// other readers).
    fn route_owned_palette_event(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        event: &egui::Event,
    ) -> bool {
        let close = self.key_bindings.close_pane;
        let quick_note = self.key_bindings.open_quick_note;
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        {
            if *key == close.1 && modifiers.matches_logically(close.0) {
                self.claimed_host_key_releases.insert(close.1);
                self.close_command_palette();
                return true;
            }
            if *key == quick_note.1
                && crate::host::keys::modifiers_match_exact_for_os(
                    modifiers,
                    &quick_note.0,
                    cfg!(target_os = "macos"),
                )
                && self.is_quick_note_preemptable()
            {
                self.dismiss_preemptable_modal();
                self.open_quick_note_modal();
                self.sync_command_palette_focus();
                log::info!("quick_note: opened by preempting the palette at input {index}");
                return true;
            }
        }
        if let Some(actions) = self.palette_host_actions(event) {
            for action in actions {
                log::debug!("palette: input {index} -> host {action:?}");
                self.dispatch_host_action(action, ctx);
            }
            // A chord that handed focus to another surface (the Attention
            // Queue, an approval) ends the session, and any Enter it froze.
            if self.command_palette_open() && !self.palette_owns_input() {
                log::info!(
                    "palette: handed off to {:?}; closing",
                    self.overlay_surface()
                );
                self.close_command_palette();
            }
            return true;
        }
        match picker_key(event, self.palette_input.composing) {
            Some(command) => {
                log::debug!("palette: input {index} -> {command:?}");
                self.apply_palette_command(command);
                true
            }
            None => false,
        }
    }

    /// Whether the palette acts on this key press itself rather than leaving
    /// it to the field: its close chord, Quick Note, a host chord, or a
    /// picker key. Nothing is claimed by asking.
    fn palette_command_key(&self, event: &egui::Event) -> bool {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            return false;
        };
        let close = self.key_bindings.close_pane;
        let quick_note = self.key_bindings.open_quick_note;
        if *key == close.1 && modifiers.matches_logically(close.0) {
            return true;
        }
        if *key == quick_note.1
            && crate::host::keys::modifiers_match_exact_for_os(
                modifiers,
                &quick_note.0,
                cfg!(target_os = "macos"),
            )
            && self.is_quick_note_preemptable()
        {
            return true;
        }
        if picker_key(event, self.palette_input.composing).is_some() {
            return true;
        }
        let mut single =
            crate::app::input_router::PlexiInput::synthetic(vec![event.clone()], *modifiers);
        let mut releases = self.claimed_host_key_releases.clone();
        crate::host::keys::poll_actions(
            &mut single,
            &self.binding_table,
            &mut releases,
            false,
            false,
            true,
            self.show_shortcuts,
        );
        single.is_empty()
    }

    /// After this pass's host and pane dispatch: apply a Cmd+P found in the
    /// delivery. The input that followed it waits for the new field.
    pub(crate) fn finish_palette_opening(&mut self, ctx: &egui::Context) {
        let Some(actions) = self.palette_input.opener.take() else {
            return;
        };
        let tail = std::mem::take(&mut self.palette_input.opening_tail);
        for action in actions {
            self.dispatch_host_action(action, ctx);
        }
        if self.palette_owns_input() {
            log::info!(
                "palette: opened; {} input event(s) after its chord wait for its field",
                tail.len()
            );
            self.palette_input.waiting = tail;
            ctx.request_repaint();
        } else if !tail.is_empty() {
            log::info!(
                "palette: the opening chord opened nothing; {} input event(s) after it dropped",
                tail.len()
            );
        }
    }

    /// One event while the palette owns input, in order. Returns whether the
    /// palette took it; what it leaves stays in `events` for egui (field
    /// edits, unclaimed key releases, accessibility requests).
    fn take_owned_palette_event(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        event: &egui::Event,
        field_focused: bool,
        order: &mut DrawOrder,
    ) -> bool {
        match event {
            egui::Event::PointerButton {
                button: egui::PointerButton::Primary,
                pressed: false,
                ..
            }
            | egui::Event::AccessKitActionRequest(_) => {
                // The draw decides whether this activated a row or the scrim;
                // commands after it wait for that.
                self.palette_input.activation_at.get_or_insert(index);
                order.deferring = true;
                // egui reads accessibility requests from `events`.
                return !matches!(event, egui::Event::AccessKitActionRequest(_));
            }
            egui::Event::PointerMoved(_) | egui::Event::MouseMoved(_) => {
                // A hover lands when the rows draw; a command after it waits.
                self.palette_input.hover_at = Some(index);
                order.moved = true;
                return true;
            }
            egui::Event::Key { pressed: false, .. } => {
                return self.palette_host_actions(event).is_some();
            }
            _ => {}
        }
        if self.palette_command_key(event) {
            if matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::ArrowUp | egui::Key::ArrowDown,
                    ..
                }
            ) {
                // The arrow moves the picker's selection; egui must not also
                // walk focus out of the field with it.
                ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
            }
            if order.deferring || order.field_dirty || order.moved {
                order.deferring = true;
                self.palette_input.after_draw.push((index, event.clone()));
            } else {
                self.route_owned_palette_event(ctx, index, event);
            }
            return true;
        }
        if is_field_input(event) {
            if order.deferring || !field_focused {
                order.deferring = true;
                self.palette_input.after_draw.push((index, event.clone()));
                return true;
            }
            self.note_palette_composition(event);
            order.field_dirty = true;
            self.palette_input.fed.push(event.clone());
            return false;
        }
        is_palette_input(event)
    }

    /// Follow the field's IME composition: preedit text means composing,
    /// a commit or the IME going away ends it.
    fn note_palette_composition(&mut self, event: &egui::Event) {
        if let egui::Event::Ime(ime) = event {
            self.palette_input.composing = match ime {
                egui::ImeEvent::Preedit(text) => !text.is_empty(),
                egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled => false,
                egui::ImeEvent::Enabled => self.palette_input.composing,
            };
        }
    }

    /// After the field and rows drew this pass: publish the field's
    /// committed text, then act on what followed it in the delivery, in
    /// arrival order: commands, a row or scrim activation, a hover. An edit
    /// behind one of them waits for the next pass's field; a pointer action
    /// behind such an edit bound rows that edit replaces, so it is dropped.
    fn finish_palette_draw(
        &mut self,
        ctx: &egui::Context,
        rows: crate::ui::picker::RowsResponse,
        dismissed: bool,
    ) {
        // The field has read its edits: no later reader may act on them.
        // Input before an opening Cmd+P stays the pane's.
        let fed = std::mem::take(&mut self.palette_input.fed);
        ctx.input_mut(|i| {
            for event in fed.iter().rev() {
                if let Some(at) = i.events.iter().rposition(|e| e == event) {
                    i.events.remove(at);
                }
            }
        });
        let after = std::mem::take(&mut self.palette_input.after_draw);
        let activation_at = self.palette_input.activation_at.take();
        let hover_at = self.palette_input.hover_at.take();
        if !self.palette_owns_input() {
            return;
        }
        let query = self
            .host
            .picker(PickerId::Palette)
            .map(|picker| picker.query().to_owned());
        if !self.palette_input.composing && query.as_deref() != Some(&self.palette_input.draft) {
            let draft = self.palette_input.draft.clone();
            log::debug!("palette: field committed {} char(s)", draft.chars().count());
            self.apply_palette_command(PickerCommand::QueryEdited(draft));
        }
        enum Step {
            Input(egui::Event),
            Picker(PickerCommand),
        }
        let mut steps: Vec<(usize, Step)> = after
            .into_iter()
            .map(|(index, event)| (index, Step::Input(event)))
            .collect();
        if let Some(at) = activation_at {
            if let Some(hit) = rows.clicked {
                steps.push((at, Step::Picker(PickerCommand::Click(hit))));
            } else if dismissed {
                steps.push((at, Step::Picker(PickerCommand::Close)));
            }
        }
        if let (Some(at), Some(hit)) = (hover_at, rows.hovered) {
            steps.push((at, Step::Picker(PickerCommand::Hover(hit))));
        }
        steps.sort_by_key(|(index, _)| *index);
        let mut steps = steps.into_iter();
        let mut dropped = 0usize;
        while let Some((index, step)) = steps.next() {
            if !self.palette_owns_input() {
                dropped += 1 + steps.len();
                break;
            }
            match step {
                Step::Picker(command) => {
                    log::debug!("palette: input {index} -> {command:?} after the draw");
                    self.apply_palette_command(command);
                }
                Step::Input(event) if self.palette_command_key(&event) => {
                    self.route_owned_palette_event(ctx, index, &event);
                }
                Step::Input(event) => {
                    self.palette_input.waiting.push(event);
                    for (_, rest) in steps.by_ref() {
                        match rest {
                            Step::Input(event) => self.palette_input.waiting.push(event),
                            Step::Picker(_) => dropped += 1,
                        }
                    }
                    log::debug!(
                        "palette: {} input event(s) wait for the next field pass",
                        self.palette_input.waiting.len()
                    );
                    ctx.request_repaint();
                }
            }
        }
        if dropped > 0 {
            log::info!("palette: {dropped} input step(s) after the session's end were dropped");
        }
    }

    /// Whether the binding table reads `event` as the palette's own chord,
    /// by its matching rules (exact chords and their order).
    fn palette_toggle_chord(&self, event: &egui::Event) -> bool {
        let egui::Event::Key {
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            return false;
        };
        let mut single =
            crate::app::input_router::PlexiInput::synthetic(vec![event.clone()], *modifiers);
        let mut releases = self.claimed_host_key_releases.clone();
        crate::host::keys::poll_actions(
            &mut single,
            &self.binding_table,
            &mut releases,
            false,
            false,
            true,
            self.show_shortcuts,
        )
        .contains(&crate::host::keys::Action::ToggleCommandPalette)
    }

    /// Whether `event` continues a gesture pressed before the palette owned
    /// input: its moves and release stay with the pane that took the press.
    fn pane_gesture_continues(&self, event: &egui::Event) -> bool {
        match event {
            egui::Event::PointerButton {
                button,
                pressed: false,
                ..
            } => self.palette_input.pane_held.contains(button),
            egui::Event::PointerMoved(_) | egui::Event::MouseMoved(_) => {
                !self.palette_input.pane_held.is_empty()
            }
            _ => false,
        }
    }

    fn note_pane_pointer(&mut self, event: &egui::Event) {
        if let egui::Event::PointerButton {
            button, pressed, ..
        } = event
        {
            let held = &mut self.palette_input.pane_held;
            if *pressed {
                if !held.contains(button) {
                    held.push(*button);
                }
            } else {
                held.retain(|b| b != button);
            }
        }
    }

    /// Whether `event` continues a press or phased scroll a session began.
    fn palette_pointer_continues(&self, event: &egui::Event) -> bool {
        match event {
            egui::Event::PointerButton { .. }
            | egui::Event::PointerMoved(_)
            | egui::Event::MouseMoved(_) => !self.palette_input.held.is_empty(),
            egui::Event::MouseWheel { .. } => self.palette_input.scrolling,
            _ => false,
        }
    }

    /// Track the pointer gestures a session owns; its presses are recorded
    /// on the focus stack.
    fn note_palette_pointer(&mut self, event: &egui::Event) {
        match event {
            egui::Event::PointerButton {
                button, pressed, ..
            } => {
                let held = &mut self.palette_input.held;
                if *pressed {
                    if !held.contains(button) {
                        held.push(*button);
                    }
                    self.focus_stack
                        .own_press(crate::app::FocusKind::CommandPalette);
                } else {
                    held.retain(|b| b != button);
                }
            }
            egui::Event::MouseWheel { phase, .. } => match phase {
                egui::TouchPhase::Start => self.palette_input.scrolling = true,
                egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                    self.palette_input.scrolling = false;
                }
                egui::TouchPhase::Move => {}
            },
            _ => {}
        }
    }

    /// The host actions a key event triggers while the palette owns input,
    /// or `None` when no binding claims it. Releases of claimed host chords
    /// are claimed too.
    fn palette_host_actions(
        &mut self,
        event: &egui::Event,
    ) -> Option<Vec<crate::host::keys::Action>> {
        let egui::Event::Key { modifiers, .. } = event else {
            return None;
        };
        let mut single =
            crate::app::input_router::PlexiInput::synthetic(vec![event.clone()], *modifiers);
        let actions = crate::host::keys::poll_actions(
            &mut single,
            &self.binding_table,
            &mut self.claimed_host_key_releases,
            false,
            false,
            true,
            self.show_shortcuts,
        );
        single.is_empty().then_some(actions)
    }

    pub(crate) fn draw_command_palette(&mut self, ctx: &egui::Context) {
        let colors = self.colors;
        let Some(picker) = self
            .host
            .picker(PickerId::Palette)
            .filter(|picker| picker.is_open())
        else {
            return;
        };
        // 0.8: the palette is a launcher, not a workspace — filling nearly
        // the whole screen height read as massive.
        let max_list_h = ((ctx.content_rect().height() - 80.0 - 120.0) * 0.8).max(200.0);
        let enter_label = picker
            .selected()
            .and_then(|key| picker.rows().iter().find(|row| row.key == *key))
            .and_then(|row| row.actions.first())
            .map_or("open", |action| action.label);
        let stops = picker.stops();
        let next_stop = (stops.len() > 1).then(|| {
            let active = stops
                .iter()
                .position(|stop| stop.id == picker.active_stop().id)
                .unwrap_or(0);
            stops[(active + 1) % stops.len()].label.to_lowercase()
        });
        let pips = |key: &RowKey| {
            self.palette_answer
                .as_ref()
                .and_then(|answer| answer.pips.get(key))
                .cloned()
        };
        // A stop with a preview widens to hold it; below the compact modal
        // width the preview half is dropped, as the notes picker does.
        let stop = picker.active_stop();
        let width = if stop.preview {
            style::MODAL_WIDTH_PALETTE_PREVIEW
                .min((ctx.content_rect().width() - style::SPACE_XL * 2.0).max(0.0))
        } else {
            style::MODAL_WIDTH_PALETTE
        };
        let show_preview = stop.preview && width >= style::MODAL_WIDTH_MD;
        let image = self
            .palette_sources
            .texture
            .as_ref()
            .filter(|(key, _)| picker.selected() == Some(key))
            .map(|(_, texture)| texture);
        let draft = &mut self.palette_input.draft;
        let mut rows = crate::ui::picker::RowsResponse::default();
        #[cfg(test)]
        let mut modal_rect = egui::Rect::NOTHING;
        // Escape is a picker key routed in order; the shell reports only the
        // scrim's click-away.
        let dismissed = ModalShell::centered("command_palette")
            .own_scroll()
            .width(width)
            .show(ctx, &colors, |ui| {
                crate::ui::picker::field(
                    ui,
                    picker,
                    draft,
                    palette_field_id(),
                    crate::ui::focus::SurfaceKey::Overlay(
                        crate::app::input_owner::OverlaySurface::Layer(
                            crate::app::FocusKind::CommandPalette,
                        ),
                    ),
                    &colors,
                );
                ui.add_space(style::SPACE_SM);
                if show_preview {
                    let list_width = ((ui.available_width() - style::SPACE_MD)
                        * style::NOTES_PICKER_LIST_FRACTION)
                        .floor();
                    let preview_width = ui.available_width() - list_width - style::SPACE_MD;
                    let halves = egui::UiBuilder::new()
                        .id_salt("palette_halves")
                        .layout(egui::Layout::left_to_right(egui::Align::Min));
                    ui.scope_builder(halves, |ui| {
                        ui.spacing_mut().item_spacing.x = style::SPACE_MD;
                        let column = |salt: &str| {
                            egui::UiBuilder::new()
                                .id_salt(salt)
                                .layout(egui::Layout::top_down(egui::Align::Min))
                        };
                        ui.scope_builder(column("palette_results_half"), |ui| {
                            ui.set_width(list_width);
                            rows =
                                crate::ui::picker::results(ui, picker, max_list_h, &pips, &colors);
                        });
                        ui.scope_builder(column("palette_preview_half"), |ui| {
                            ui.set_width(preview_width);
                            crate::ui::picker::preview(ui, picker, image, max_list_h, &colors);
                        });
                    });
                } else {
                    rows = crate::ui::picker::results(ui, picker, max_list_h, &pips, &colors);
                }
                ui.add_space(style::SPACE_SM);
                let mut hints = vec![
                    HintGroup::alternatives(&[&["\u{2318}", "j"], &["\u{2318}", "k"]], "navigate"),
                    HintGroup::new(&["\u{21b5}"], enter_label),
                ];
                if let Some(next) = next_stop.as_deref() {
                    hints.push(HintGroup::new(&["tab"], next));
                }
                hints.push(HintGroup::new(&["esc"], "dismiss"));
                HintBar::new(&hints).show(ui, &colors);
                #[cfg(test)]
                {
                    modal_rect = ui.min_rect().expand2(egui::vec2(
                        f32::from(style::MODAL_PADDING_H),
                        f32::from(style::MODAL_PADDING_V),
                    ));
                }
            })
            .dismissed;
        // A discarded sizing pass was never seen.
        #[cfg(test)]
        if !ctx.will_discard() {
            self.palette_input.drawn = Some((rows.rects.clone(), modal_rect));
        }
        self.finish_palette_draw(ctx, rows, dismissed);
    }

    /// Run an activated row: a Commands row, or open a file through the
    /// host's one open resolver, anchored on the pane the palette serves.
    fn run_palette_target(&mut self, target: PaletteTarget) {
        match target {
            PaletteTarget::Command(id) => self.run_palette_selection(id),
            PaletteTarget::File(path) => {
                #[cfg(test)]
                if let Some(key) = crate::host::picker::files::file_key(&path) {
                    palette_test_log::activated(key);
                }
                log::info!("palette: opening a file result");
                let anchor = self.focused_pane_id().unwrap_or_default();
                self.open_file_in_view(anchor, &path);
            }
        }
    }

    /// Execute a Commands-stop row.
    fn run_palette_selection(&mut self, id: CommandRowId) {
        #[cfg(test)]
        palette_test_log::activated(id.key());
        // Bounded: the result kind only, never the query or row text.
        let kind = match &id {
            CommandRowId::Command(_) => "command",
            CommandRowId::Focus(_) => "focus",
            CommandRowId::Unavailable(_) => "unavailable",
            CommandRowId::View(_) => "view",
            CommandRowId::Builtin(_) => "builtin",
            CommandRowId::Note(_) => "note",
            CommandRowId::UserCommand { .. } => "user-command",
            CommandRowId::UpdateReady(_) => "update-ready",
        };
        log::info!("palette: running {kind} result");
        match id {
            CommandRowId::Command(command) => self.run_palette_command(command),
            CommandRowId::UpdateReady(_) => {
                self.run_palette_command(PaletteCommand::RestartToUpdate)
            }
            CommandRowId::UserCommand { name, scope } => self.run_user_command(&name, scope),
            CommandRowId::Focus(target) => self.focus_palette_target(target),
            CommandRowId::Unavailable(name) => {
                log::warn!("palette: parked context '{name}' has no live window to restore");
            }
            CommandRowId::View(id) => self.launch_view_by_id(&id),
            CommandRowId::Builtin(id) => self.launch_builtin_by_id(id),
            CommandRowId::Note(path) => {
                if let Some(pane_id) = self.find_open_note_pane(&path) {
                    log::info!("palette: note already open in pane {pane_id}, navigating");
                    self.pane_navigate(pane_id);
                } else if let Err(error) = self.open_text_editor_at_path(path) {
                    log::error!("palette: opening a note in a new pane failed: {error}");
                }
            }
        }
    }

    /// Launch a host-native builtin palette entry by its id.
    pub(crate) fn launch_builtin_by_id(&mut self, id: &str) {
        if self.plugin_blocked(id) {
            return;
        }
        log::info!("palette: launching builtin plugin '{id}'");
        match id {
            "assistant" => {
                if crate::release::feature_enabled(crate::release::ReleaseFeature::Assistant) {
                    self.open_assistant_pane();
                } else {
                    log::info!(
                        "assistant: palette launch blocked; the assistant experiment is off"
                    );
                }
            }
            other => log::warn!("palette: unknown builtin plugin id '{other}'"),
        }
    }

    /// Execute the subset of keyboard bindings intentionally surfaced by the
    /// palette. The descriptor is the host binding `Action`, so labels/search
    /// metadata cannot drift onto a private palette-only command enum.
    fn run_palette_command(&mut self, command: PaletteCommand) {
        if matches!(command, PaletteCommand::UiUpcycle(_)) && self.plugin_blocked("cli-renderer") {
            return;
        }
        match command {
            PaletteCommand::UiUpcycle(binary) => match crate::cli::registry::lookup(binary, None) {
                Ok(descriptor) => {
                    let root = self
                        .effective_commands_workspace_root()
                        .unwrap_or_else(|| self.router.active().root.clone());
                    log::info!("palette: UI upcycle opening curated CLI '{binary}'");
                    self.open_builtin_view_pane(
                        Box::new(
                            crate::render::cli_renderer_view::CliRendererView::from_descriptor(
                                descriptor,
                            ),
                        ),
                        crate::app::permissions::PluginPermissions::builtin(),
                        root,
                        None,
                        None,
                        None,
                    );
                }
                Err(error) => {
                    log::error!("palette: UI upcycle '{binary}' descriptor failed: {error}")
                }
            },
            PaletteCommand::Host(action) => self.run_palette_host_binding(action),
            PaletteCommand::OpenAttention => {
                self.sync_attention_queue();
                self.attention_queue.open = true;
                self.sync_attention_queue_layer();
                self.attention_queue.selected_idx = 0;
                log::info!(
                    "attention_queue: opened source=palette-needs-attention items={}",
                    self.attention_queue.items.len()
                );
            }
            PaletteCommand::StatePreset(preset) => {
                let result =
                    crate::config::edit::edit_config_file(&crate::config::config_path(), |doc| {
                        crate::config::edit::set_value(
                            doc,
                            &["theme", "state_preset"],
                            preset.into(),
                        )
                    });
                match result {
                    Ok(()) => {
                        log::info!("settings: state preset={preset}");
                        self.reload_config_after_global_save("theme.state_preset");
                    }
                    Err(error) => log::error!("settings: state preset failed: {error}"),
                }
            }
            PaletteCommand::OpenGlobalConfig => {
                let path = crate::config::ensure_config_file();
                log::info!("palette: opening global config at {}", path.display());
                self.open_config_in_editor(path);
            }
            PaletteCommand::OpenWorkspaceConfig => {
                let Some(root) = self
                    .palette_config_workspace_root
                    .clone()
                    .or_else(|| self.active_config_workspace_root())
                else {
                    log::warn!("palette: workspace config requested outside a workspace");
                    return;
                };
                log::info!("palette: opening workspace config at {}", root.display());
                let path = crate::config::ensure_workspace_config_file(&root);
                self.open_config_in_editor(path);
            }
            PaletteCommand::EditCommands => {
                let ws_root = self.effective_commands_workspace_root();
                let path = crate::cli::ensure_commands_file(ws_root.as_deref());
                log::info!(
                    "palette: opening commands.toml at {} (workspace_root={:?})",
                    path.display(),
                    ws_root
                );
                self.open_config_in_editor_with_cwd(path, ws_root);
            }
            PaletteCommand::SwitchCommandsScope => {
                self.switch_focused_commands_editor_scope();
            }
            PaletteCommand::CheckForUpdates => {
                log::info!("palette: check for updates");
                self.start_update_check(crate::host::update_status::CheckMode::Manual);
                if !self.show_changelog {
                    self.toggle_changelog();
                }
            }
            PaletteCommand::RestartToUpdate => self.offer_update_restart("palette"),
            // No selection means nothing to close: never fall back to the
            // focused pane, which may be the pane under the pointer.
            PaletteCommand::CloseSelectedPanes => {
                if !self.start_close_selected_panes("palette_close_selected") {
                    log::info!("palette: close selected panes found no selection");
                    self.attention_queue.transient_hint =
                        Some(("No panes are selected.".into(), std::time::Instant::now()));
                }
            }
            PaletteCommand::SetupCli => match crate::cli::setup::setup_external_shell(true) {
                Ok(report) => {
                    log::info!(
                        "palette: SetupCli installed {} at {} (rc_files={:?})",
                        report.cli_name,
                        report.shim_path.display(),
                        report.modified_rc_files
                    );
                }
                Err(error) => log::error!("palette: SetupCli failed: {error}"),
            },
        }
    }

    pub(crate) fn active_config_workspace_root(&self) -> Option<std::path::PathBuf> {
        // A workspace root is the context's own root directory; the channel dir
        // is created on first open. A root whose repo config would be the global
        // file (HOME) is not a repo.
        let root = &self.router.active().root;
        (root.is_dir()
            && crate::config::workspace_config_path(root) != crate::config::config_path())
        .then(|| root.clone())
    }

    pub(crate) fn effective_commands_workspace_root(&self) -> Option<std::path::PathBuf> {
        self.palette_workspace_root.clone().or_else(|| {
            let win = &self.windows[self.active_window];
            let focused_tile = win
                .focused_pane
                .or_else(|| win.tree.root.and_then(|r| win.find_first_pane_in(r)));
            focused_tile
                .and_then(|tile_id| win.get_focused_pane_cwd(tile_id))
                .and_then(|cwd| crate::cli::resolve_commands_workspace_root(&cwd))
                .or_else(|| crate::cli::resolve_commands_workspace_root(&self.router.active().root))
        })
    }

    fn run_palette_host_binding(&mut self, action: crate::host::keys::Action) {
        log::info!("palette: executing host binding {action:?}");
        let ctx = self.ctx.clone();
        self.dispatch_host_action(action, &ctx);
    }

    /// Execute a user-authored command. Stepped commands (`steps = [...]`) or
    /// commands with `on_load` execute their steps directly through the host's
    /// step runner; single-line `run` commands run in the focused terminal pane
    /// when it is an idle shell (unless `split = true`), or open a new split
    /// pane when focused on a busy shell, agent pane, or view pane.
    fn run_user_command(&mut self, name: &str, scope: crate::cli::UserCommandScope) {
        self.queue_user_command(name, Some(scope));
    }

    pub(crate) fn queue_user_command(
        &mut self,
        name: &str,
        palette_scope: Option<crate::cli::UserCommandScope>,
    ) {
        let win = &self.windows[self.active_window];
        let source_tile = win
            .focused_pane
            .or_else(|| win.tree.root.and_then(|root| win.find_first_pane_in(root)));
        log::info!("user_command: queued name={name} scope={palette_scope:?}");
        self.pending_user_commands.push(PendingUserCommand {
            name: name.into(),
            palette_scope,
            palette_root: self.palette_workspace_root.clone(),
            context_root: self.router.active().root.clone(),
            source_window: win.window_id,
            source_context: win.context_id,
            source_tile,
            caller: self.focused_pane_id(),
        });
        self.ctx.request_repaint();
    }

    pub(crate) fn drain_pending_user_commands(&mut self) {
        for command in std::mem::take(&mut self.pending_user_commands) {
            if self.user_command_window(&command).is_none() {
                self.notify_command_refused(
                    &command.name,
                    "The invoking pane, window, or context is no longer available",
                );
                continue;
            }
            // Resolve paths and probe the captured caller's cwd only in logic.
            let workspace_root = command
                .palette_root
                .clone()
                .or_else(|| {
                    self.windows
                        .iter()
                        .find(|win| win.window_id == command.source_window)
                        .and_then(|win| win.get_focused_pane_cwd(command.source_tile?))
                        .and_then(|cwd| crate::cli::resolve_commands_workspace_root(&cwd))
                })
                .or_else(|| crate::cli::resolve_commands_workspace_root(&command.context_root))
                .or_else(crate::config::adopted_workspace_root)
                .or_else(|| Some(command.context_root.clone()));
            let started = self.run_user_command_at(&command, workspace_root);
            if !started && command.palette_scope.is_some() {
                let initial_cmd = format!("plexi run {}", shell_single_quote(&command.name));
                self.dispatch_single_line_user_command(
                    &command,
                    &initial_cmd,
                    true,
                    command.palette_root.clone(),
                );
                self.mark_workspace_dirty();
            }
        }
    }

    fn user_command_window(&self, command: &PendingUserCommand) -> Option<usize> {
        self.router
            .position(|context| context.context_id == command.source_context)?;
        self.windows.iter().position(|window| {
            window.window_id == command.source_window
                && window.context_id == command.source_context
                && command
                    .caller
                    .is_none_or(|pane| window.panes.contains_key(&pane))
        })
    }

    pub(super) fn user_command_focused_pane(&self, command: &PendingUserCommand) -> Option<u64> {
        let window = &self.windows[self.user_command_window(command)?];
        let tile = window
            .focused_pane
            .and_then(|tile| window.find_first_pane_in(tile))?;
        match window.tree.tiles.get(tile) {
            Some(egui_tiles::Tile::Pane(pane)) => Some(*pane),
            _ => None,
        }
    }

    /// Start the saved command `command` names. Returns `false` when no such
    /// command resolves or its invoking window is gone.
    fn run_user_command_at(
        &mut self,
        command: &PendingUserCommand,
        ws_root: Option<std::path::PathBuf>,
    ) -> bool {
        let name = command.name.as_str();
        let invoker_pane_id = command.caller;
        let Some(cmd) = crate::cli::resolve_user_commands(ws_root.as_deref())
            .into_iter()
            .find(|c| c.name == name)
            .or_else(|| {
                self.palette_commands
                    .iter()
                    .find(|c| c.name == name)
                    .cloned()
            })
        else {
            return false;
        };
        // One check for every saved-command form, shared with `plexi run`:
        // a retired short form is refused before anything runs.
        let refused = cmd
            .steps
            .iter()
            .chain(cmd.run.iter())
            .find_map(|step| crate::cli::retired_step_error(step));
        if let Some(reason) = refused {
            log::warn!("palette: refused retired command '{name}'");
            self.notify_command_refused(name, &reason);
            return true;
        }
        let Some(window) = self.user_command_window(command) else {
            return false;
        };
        self.windows[window].clear_zoom();
        let steps: Vec<String> = if !cmd.steps.is_empty() {
            cmd.steps.clone()
        } else if let Some(ref run_line) = cmd.run {
            if matches!(
                crate::cli::parse_step(run_line),
                crate::cli::ParsedStep::PlexiCli(_)
            ) {
                vec![run_line.clone()]
            } else {
                let shell_cmd = if cmd.secrets.is_empty() {
                    crate::cli::substitute_step_args(run_line, &[])
                } else {
                    format!("plexi run {}", shell_single_quote(name))
                };
                let focused = self.dispatch_single_line_user_command(
                    command,
                    &shell_cmd,
                    cmd.split,
                    ws_root.clone(),
                );
                self.mark_workspace_dirty();
                self.start_on_load_run(command, ws_root, cmd.on_load, focused);
                return true;
            }
        } else {
            let shell_cmd = format!("plexi run {}", shell_single_quote(name));
            let focused = self.dispatch_single_line_user_command(
                command,
                &shell_cmd,
                cmd.split,
                ws_root.clone(),
            );
            self.mark_workspace_dirty();
            self.start_on_load_run(command, ws_root, cmd.on_load, focused);
            return true;
        };

        log::info!(
            "user_command: running '{}' ({} step(s)) in {:?} (invoker_pane={:?})",
            name,
            steps.len(),
            ws_root,
            invoker_pane_id
        );
        self.start_user_command_run(UserCommandRun::new(
            command.clone(),
            ws_root,
            steps,
            invoker_pane_id,
            cmd.on_load,
        ));
        true
    }

    /// Run a single-line command's `on_load` steps against the pane it ran in.
    fn start_on_load_run(
        &mut self,
        command: &PendingUserCommand,
        ws_root: Option<std::path::PathBuf>,
        on_load: Vec<String>,
        pane: Option<u64>,
    ) {
        if on_load.is_empty() {
            return;
        }
        self.start_user_command_run(UserCommandRun::new(
            command.clone(),
            ws_root,
            on_load,
            pane,
            Vec::new(),
        ));
    }

    fn dispatch_single_line_user_command(
        &mut self,
        command: &PendingUserCommand,
        shell_cmd: &str,
        force_split: bool,
        ws_root: Option<std::path::PathBuf>,
    ) -> Option<u64> {
        let name = &command.name;
        if !force_split {
            if let Some(pane_id) = command
                .caller
                .filter(|pane| self.idle_shell_pane_id(*pane).is_some())
            {
                log::info!(
                    "user_command: running '{}' in focused idle shell pane {} command_chars={}",
                    name,
                    pane_id,
                    shell_cmd.chars().count()
                );
                self.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostUi, crate::protocol::HostRequest::SendToPane {
                    generated_operation_id: false,
                    pane_id,
                    text: shell_cmd.to_string(),
                    submit: true,
                    paste: false,
                    response_file: None,
                    operation_id: None,
                });
                return Some(pane_id);
            }
        }
        log::info!(
            "user_command: splitting for '{}' (split={}, command_chars={})",
            name,
            force_split,
            shell_cmd.chars().count()
        );
        let win_idx = self.user_command_window(command)?;
        let pane = command.caller?;
        let tile = self.windows[win_idx].tree.tiles.find_pane(&pane)?;
        let target_tile = self.windows[win_idx]
            .find_ancestor_tabs(tile)
            .map_or(tile, |(tabs, _)| tabs);
        let context = self.pane_context_env_for_window(win_idx);
        // A user command runs the workspace's command, not a shell for its
        // folder: no folder secrets.
        let launch = self.terminal_launch_with(
            crate::broker::pane_authority::GrantSource::DesktopInput,
            context,
            ws_root,
            Some(crate::pane_ops::construct::LaunchCommand {
                command: shell_cmd.to_string(),
                close_on_exit: false,
            }),
            false,
        );
        self.spawn_terminal_pane_at(
            crate::pane_ops::create::SplitPlacement {
                win_idx,
                target_tile,
                vertical: true,
                new_pane_first: false,
                keep_focus: false,
            },
            launch,
        )
        .map_err(|error| log::warn!("user_command: terminal not created: {error}"))
        .ok()
    }

    fn idle_shell_pane_id(&self, pane_id: u64) -> Option<u64> {
        if self.submit_in_flight(pane_id) {
            return None;
        }
        let pane = self
            .windows
            .iter()
            .find_map(|win| win.panes.get(&pane_id))?;
        if pane.agent().is_some() {
            return None;
        }
        match pane {
            crate::host::pane::Pane::Terminal(t) if t.is_idle_shell() => Some(pane_id),
            _ => None,
        }
    }

    pub(crate) fn focused_pane_id(&self) -> Option<u64> {
        let win = self.windows.get(self.active_window)?;
        let pane_tile = win
            .focused_pane
            .and_then(|t| win.find_first_pane_in(t))
            .or_else(|| win.tree.root.and_then(|r| win.find_first_pane_in(r)))?;
        match win.tree.tiles.get(pane_tile) {
            Some(egui_tiles::Tile::Pane(pane_id)) => Some(*pane_id),
            _ => None,
        }
    }

    pub(super) fn execute_host_step(
        &mut self,
        command: &PendingUserCommand,
        rendered: &str,
        parsed: &crate::cli::ParsedStep,
        ws_root: Option<&std::path::Path>,
        default_pane_id: Option<u64>,
    ) -> HostStepOutcome {
        if self.user_command_window(command).is_none() {
            self.notify_command_refused(
                &command.name,
                "The invoking pane, window, or context is no longer available",
            );
            return HostStepOutcome::Exited(1);
        }
        match parsed {
            crate::cli::ParsedStep::PlexiCli(argv) => {
                self.execute_host_cli_step(command, argv, ws_root, default_pane_id)
            }
            crate::cli::ParsedStep::Shell(sh) => {
                log::info!(
                    "user_command: shell step off logic name={} command_chars={}",
                    command.name,
                    rendered.chars().count()
                );
                spawn_shell_step(
                    sh,
                    ws_root,
                    default_pane_id,
                    std::sync::Arc::clone(&self.ui_wake),
                )
            }
        }
    }

    fn resolve_host_pane_target(&self, target: &crate::cli::PaneTarget) -> Option<u64> {
        let q = target.as_str().trim();
        if let Ok(id) = q.parse::<u64>() {
            return Some(id);
        }
        let candidates: Vec<(u64, String)> = self
            .windows
            .iter()
            .flat_map(|w| w.panes.iter())
            .map(|(&id, pane)| {
                let title = match pane {
                    crate::host::pane::Pane::Terminal(t) => {
                        t.name.clone().unwrap_or_else(|| "terminal".to_string())
                    }
                    crate::host::pane::Pane::View(a) => a.name.clone(),
                    crate::host::pane::Pane::Portal(_) => String::new(),
                };
                (id, title)
            })
            .collect();
        crate::cli::resolve_target_from_candidates("pane", q, &candidates).ok()
    }

    fn execute_host_cli_step(
        &mut self,
        command: &PendingUserCommand,
        argv: &[String],
        ws_root: Option<&std::path::Path>,
        default_pane_id: Option<u64>,
    ) -> HostStepOutcome {
        use crate::cli::args::{Cli, Commands, PaneCmd, ViewCmd};
        use clap::Parser;

        let cli = match Cli::try_parse_from(argv) {
            Ok(c) => c,
            Err(e) => {
                log::warn!(
                    "user_command: invalid CLI step argc={} error_kind={:?}",
                    argv.len(),
                    e.kind()
                );
                return HostStepOutcome::Exited(2);
            }
        };

        static HOST_STEP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let next_resp_path = || {
            let seq = HOST_STEP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::env::temp_dir().join(format!(
                "plexi-host-step-{}-{}.json",
                std::process::id(),
                seq
            ))
        };

        let layout_from_dir = |down: bool,
                               left: bool,
                               up: bool,
                               right: bool,
                               tab: bool,
                               window: bool,
                               overlay: bool|
         -> Option<String> {
            if down {
                Some("split_v".to_string())
            } else if left {
                Some("split_left".to_string())
            } else if up {
                Some("split_above".to_string())
            } else if right {
                Some("split_h".to_string())
            } else if tab {
                Some("tab".to_string())
            } else if window {
                Some("new_window".to_string())
            } else if overlay {
                Some("overlay".to_string())
            } else {
                None
            }
        };

        match cli.command {
            Some(Commands::Pane { cmd }) => match cmd {
                PaneCmd::New {
                    cmd,
                    name,
                    down,
                    left,
                    up,
                    right,
                    tab,
                    window,
                    overlay,
                    from,
                    ephemeral,
                    no_focus,
                    cwd,
                    worktree,
                    result_schema,
                    on_load,
                    extra_args,
                    agent,
                    boot_timeout,
                } => {
                    let from_id = match from {
                        Some(ref t) => match self.resolve_host_pane_target(t) {
                            Some(id) => Some(id),
                            None => return HostStepOutcome::Exited(1),
                        },
                        None => None,
                    };
                    let layout = Some(
                        layout_from_dir(down, left, up, right, tab, window, overlay)
                            .unwrap_or_else(|| "split_h".to_string()),
                    );
                    let plugin_id =
                        (cmd.as_deref() == Some("agents") && agent.is_none()).then_some("agents");
                    let is_plugin = plugin_id.is_some();
                    let type_id = if let Some(a) = plugin_id {
                        a.to_string()
                    } else {
                        "terminal".to_string()
                    };
                    let args: Vec<String> = if is_plugin {
                        extra_args
                    } else if let Some(c) = cmd {
                        let joined = std::iter::once(c.as_str())
                            .chain(extra_args.iter().map(String::as_str))
                            .collect::<Vec<_>>()
                            .join(" ");
                        vec![joined]
                    } else {
                        Vec::new()
                    };
                    let parsed_result_schema = match result_schema.as_deref() {
                        Some(spec) => {
                            let base = cwd
                                .as_deref()
                                .map(std::path::PathBuf::from)
                                .or_else(|| ws_root.map(std::path::Path::to_path_buf));
                            match crate::host::pane_result::parse_schema_spec(spec, base.as_deref())
                            {
                                Ok(s) => Some(s),
                                Err(_) => return HostStepOutcome::Exited(1),
                            }
                        }
                        None => None,
                    };
                    let resp_path = next_resp_path();
                    let resp_str = resp_path.to_string_lossy().into_owned();
                    let req = crate::protocol::HostRequest::SpawnPane {
                        type_id,
                        layout,
                        args,
                        from_pane_id: from_id.or(default_pane_id),
                        request_id: None,
                        response_file: Some(resp_str),
                        ephemeral,
                        cwd: cwd.or_else(|| ws_root.map(|p| p.to_string_lossy().into_owned())),
                        no_focus,
                        path: None,
                        workspace_root: ws_root.map(|p| p.to_string_lossy().into_owned()),
                        target_context: default_pane_id.is_none().then_some(command.source_context),
                        context_name: None,
                        name,
                        agent_cmd: agent,
                        boot_timeout_secs: boot_timeout,
                        worktree,
                        result_schema: parsed_result_schema,
                    };
                    self.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostUi, req);
                    let mut created_pane_id = self.user_command_focused_pane(command);
                    if let Ok(content) = std::fs::read_to_string(&resp_path) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            if val.get("error").is_some() {
                                return HostStepOutcome::Exited(1);
                            }
                            if let Some(pid) = val.get("pane_id").and_then(|v| v.as_u64()) {
                                created_pane_id = Some(pid);
                            }
                        }
                    }
                    if !on_load.is_empty() {
                        return HostStepOutcome::Nested {
                            steps: on_load,
                            pane: created_pane_id,
                        };
                    }
                    HostStepOutcome::Exited(0)
                }
                PaneCmd::Name { first, second } => {
                    let (pane_id, name) = match second {
                        Some(title) => {
                            let target = crate::cli::PaneTarget::from(first);
                            match self.resolve_host_pane_target(&target) {
                                Some(id) => (Some(id), title),
                                None => return HostStepOutcome::Exited(1),
                            }
                        }
                        None => (None, first),
                    };
                    let Some(pid) = pane_id.or(default_pane_id) else {
                        return HostStepOutcome::Exited(1);
                    };
                    let exists = self.windows.iter().any(|w| w.panes.contains_key(&pid));
                    if !exists {
                        return HostStepOutcome::Exited(1);
                    }
                    self.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostUi, crate::protocol::HostRequest::SetPaneTitle {
                        pane_id: pid,
                        name,
                    });
                    HostStepOutcome::Exited(0)
                }
                PaneCmd::Focus {
                    pane_id, forward, ..
                } => {
                    let Some(target) = pane_id else {
                        if forward {
                            self.step_focus_history_forward();
                        } else {
                            self.step_focus_history_back();
                        }
                        return HostStepOutcome::Exited(0);
                    };
                    let Some(pane_id) = self.resolve_host_pane_target(&target) else {
                        return HostStepOutcome::Exited(1);
                    };
                    let exists = self.windows.iter().any(|w| w.panes.contains_key(&pane_id));
                    if !exists {
                        return HostStepOutcome::Exited(1);
                    }
                    self.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostUi, crate::protocol::HostRequest::FocusPane {
                        pane_id,
                    });
                    HostStepOutcome::Exited(0)
                }
                PaneCmd::Close { pane_id } => {
                    let resolved_id = match pane_id {
                        Some(ref t) => match self.resolve_host_pane_target(t) {
                            Some(id) => Some(id),
                            None => return HostStepOutcome::Exited(1),
                        },
                        None => None,
                    };
                    let Some(pid) = resolved_id.or(default_pane_id) else {
                        return HostStepOutcome::Exited(1);
                    };
                    let resp_path = next_resp_path();
                    let resp_str = resp_path.to_string_lossy().into_owned();
                    self.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostUi, crate::protocol::HostRequest::ClosePane {
                        pane_id: pid,
                        response_file: Some(resp_str),
                    });
                    if let Ok(content) = std::fs::read_to_string(&resp_path) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            if val.get("error").is_some() {
                                return HostStepOutcome::Exited(1);
                            }
                        }
                    }
                    HostStepOutcome::Exited(0)
                }
                PaneCmd::Send {
                    pane_id,
                    text,
                    submit,
                    paste,
                    operation_id,
                } => {
                    let Some(pane_id) = self.resolve_host_pane_target(&pane_id) else {
                        return HostStepOutcome::Exited(1);
                    };
                    let resp_path = next_resp_path();
                    let resp_str = resp_path.to_string_lossy().into_owned();
                    self.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostUi, crate::protocol::HostRequest::SendToPane {
                        generated_operation_id: false,
                        pane_id,
                        text,
                        submit,
                        paste,
                        response_file: Some(resp_str),
                        operation_id,
                    });
                    if let Ok(content) = std::fs::read_to_string(&resp_path) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            if val.get("error").is_some() {
                                return HostStepOutcome::Exited(1);
                            }
                        }
                    }
                    HostStepOutcome::Exited(0)
                }
                PaneCmd::Key { pane_id, key } => {
                    let Some(pane_id) = self.resolve_host_pane_target(&pane_id) else {
                        return HostStepOutcome::Exited(1);
                    };
                    let resp_path = next_resp_path();
                    let resp_str = resp_path.to_string_lossy().into_owned();
                    self.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostUi, crate::protocol::HostRequest::KeyPane {
                        pane_id,
                        key,
                        response_file: Some(resp_str),
                    });
                    if let Ok(content) = std::fs::read_to_string(&resp_path) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                            if val.get("error").is_some() {
                                return HostStepOutcome::Exited(1);
                            }
                        }
                    }
                    HostStepOutcome::Exited(0)
                }
                _ => HostStepOutcome::Exited(1),
            },
            Some(Commands::Agent {
                cmd:
                    crate::cli::args::AgentCmd::Start {
                        name: agent_name,
                        message,
                        prompt,
                        cwd,
                    },
            }) => {
                let admitted =
                    match crate::app::agent_open::admit_agent_mesh_route("palette agent start") {
                        Ok(admitted) => admitted,
                        Err(error) => {
                            self.notify_command_refused("palette agent start", &error);
                            return HostStepOutcome::Exited(1);
                        }
                    };
                let eff_cwd = cwd
                    .as_deref()
                    .map(std::path::PathBuf::from)
                    .or_else(|| ws_root.map(std::path::Path::to_path_buf))
                    .unwrap_or_else(|| command.context_root.clone());
                let task_str = message.or(prompt);
                let Ok(payload) = crate::cli::agent::resolve_agent_start_in_stopping_at(
                    &agent_name,
                    &crate::config::config_dir(),
                    &eff_cwd,
                    task_str.as_deref(),
                    &[],
                ) else {
                    return HostStepOutcome::Exited(1);
                };
                let Ok(req) = serde_json::from_value::<crate::protocol::HostRequest>(payload)
                else {
                    return HostStepOutcome::Exited(1);
                };
                let reply = self.agent_spawn_reply_admitted(&crate::broker::caller::RequestOrigin::HostUi, default_pane_id, &req, admitted);
                if let Some(error) = reply.get("error").and_then(serde_json::Value::as_str) {
                    self.notify_command_refused("agent launch", error);
                    return HostStepOutcome::Exited(1);
                }
                HostStepOutcome::Exited(0)
            }
            Some(Commands::Agent {
                cmd: crate::cli::args::AgentCmd::Open { name, cwd },
            }) => {
                let eff_cwd = cwd
                    .as_deref()
                    .map(std::path::PathBuf::from)
                    .or_else(|| ws_root.map(std::path::Path::to_path_buf))
                    .unwrap_or_else(|| command.context_root.clone());
                // The entire saved command now runs in logic, so admission
                // and resolution can finish before the next step executes.
                match self.open_named_agent(
                    &name,
                    &eff_cwd,
                    default_pane_id,
                    &crate::broker::caller::RequestOrigin::HostUi,
                ) {
                    Ok(_) => HostStepOutcome::Exited(0),
                    Err(error) => {
                        self.notify_command_refused(&format!("agent open {name}"), &error);
                        HostStepOutcome::Exited(1)
                    }
                }
            }
            Some(Commands::Agent {
                cmd:
                    crate::cli::args::AgentCmd::Spawn {
                        name,
                        task,
                        subcontext,
                        ephemeral,
                        from,
                        cwd,
                    },
            }) => {
                let admitted =
                    match crate::app::agent_open::admit_agent_mesh_route("palette agent spawn") {
                        Ok(admitted) => admitted,
                        Err(error) => {
                            self.notify_command_refused("palette agent spawn", &error);
                            return HostStepOutcome::Exited(1);
                        }
                    };
                let eff_cwd = cwd
                    .as_deref()
                    .map(std::path::PathBuf::from)
                    .or_else(|| ws_root.map(std::path::Path::to_path_buf))
                    .unwrap_or_else(|| command.context_root.clone());
                let from_id = match from {
                    Some(ref target) => match self.resolve_host_pane_target(target) {
                        Some(id) => Some(id),
                        None => return HostStepOutcome::Exited(1),
                    },
                    None => default_pane_id,
                };
                let Ok(payload) = crate::cli::agent::resolve_agent_spawn_in_stopping_at(
                    &name,
                    &crate::config::config_dir(),
                    &eff_cwd,
                    (!task.is_empty()).then_some(task.as_str()),
                    subcontext.as_deref(),
                    ephemeral.then_some(true),
                    &[],
                ) else {
                    return HostStepOutcome::Exited(1);
                };
                let Ok(req) = serde_json::from_value::<crate::protocol::HostRequest>(payload)
                else {
                    return HostStepOutcome::Exited(1);
                };
                let reply = self.agent_spawn_reply_admitted(&crate::broker::caller::RequestOrigin::HostUi, from_id, &req, admitted);
                if let Some(error) = reply.get("error").and_then(serde_json::Value::as_str) {
                    self.notify_command_refused("agent launch", error);
                    return HostStepOutcome::Exited(1);
                }
                HostStepOutcome::Exited(0)
            }
            Some(Commands::View {
                cmd:
                    ViewCmd::Open {
                        type_id,
                        mcp,
                        cli: cli_flag,
                        extra_args,
                        down,
                        left,
                        up,
                        right,
                        tab,
                        window,
                        split,
                        from,
                    },
            }) => {
                let from_id = match from {
                    Some(ref t) => match self.resolve_host_pane_target(t) {
                        Some(id) => Some(id),
                        None => return HostStepOutcome::Exited(1),
                    },
                    None => None,
                };
                let layout = match split.as_deref() {
                    Some("down") => Some("split_v".to_string()),
                    Some("right") => Some("split_h".to_string()),
                    Some("left") => Some("split_left".to_string()),
                    Some("up") => Some("split_above".to_string()),
                    Some("tab") => Some("tab".to_string()),
                    Some("window") => Some("new_window".to_string()),
                    Some(_) => return HostStepOutcome::Exited(1),
                    None => layout_from_dir(down, left, up, right, tab, window, false),
                };
                let (target_type_id, target_args) = if let Some(tid) = type_id {
                    (tid, extra_args)
                } else if !mcp.is_empty() {
                    ("mcp-renderer".to_string(), mcp)
                } else if let Some(bin) = cli_flag {
                    ("cli-renderer".to_string(), vec![bin])
                } else {
                    return HostStepOutcome::Exited(1);
                };
                let resp_path = next_resp_path();
                let resp_str = resp_path.to_string_lossy().into_owned();
                let req = crate::protocol::HostRequest::SpawnPane {
                    type_id: target_type_id,
                    layout,
                    args: target_args,
                    from_pane_id: from_id.or(default_pane_id),
                    request_id: None,
                    response_file: Some(resp_str),
                    ephemeral: false,
                    cwd: ws_root.map(|p| p.to_string_lossy().into_owned()),
                    no_focus: false,
                    path: None,
                    workspace_root: ws_root.map(|p| p.to_string_lossy().into_owned()),
                    target_context: default_pane_id.is_none().then_some(command.source_context),
                    context_name: None,
                    name: None,
                    agent_cmd: None,
                    boot_timeout_secs: None,
                    worktree: None,
                    result_schema: None,
                };
                self.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostUi, req);
                if let Ok(content) = std::fs::read_to_string(&resp_path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        if val.get("error").is_some() {
                            return HostStepOutcome::Exited(1);
                        }
                    }
                }
                HostStepOutcome::Exited(0)
            }
            _ => HostStepOutcome::Exited(1),
        }
    }

    /// Resolve a stable palette target against current host state, then focus
    /// it. Collection never leaks a mutable window index into execution.
    /// Sanctioned cross-context focus path (also used by the `[launch] on_launch`
    /// resolver, #0336) — it switches the sidebar context via `switch_workspace`
    /// rather than mutating `active_window` mid-spawn.
    fn focus_palette_target(&mut self, target: PaletteFocusTarget) {
        let Some(ctx_idx) = self.windows.iter().position(|window| {
            window.window_id == target.window_id && window.context_id == target.context_id
        }) else {
            log::warn!(
                "palette: target context={} window={} disappeared before focus",
                target.context_id,
                target.window_id
            );
            return;
        };
        log::info!(
            "palette: focus target context={} window={} pane={:?}",
            target.context_id,
            target.window_id,
            target.pane_id
        );
        if let Some(ctx_idx_sidebar) = self.router.position(|c| c.context_id == target.context_id) {
            if self.router.get(ctx_idx_sidebar).parked {
                // Parking keeps windows alive. Reuse the established unpark
                // transition before targeting this row's specific window/pane
                // so agent and ordinary-pane results restore consistently.
                self.unpark_context(ctx_idx_sidebar);
            } else if ctx_idx_sidebar != self.router.active_idx() {
                // switch_workspace → pick_active_context_from_workspace sets
                // active_window based on context_active_window. We override it
                // immediately below.
                self.switch_workspace(ctx_idx_sidebar);
            }
        }
        self.active_window = ctx_idx;
        self.windows[ctx_idx].zoomed_pane = None;
        let ctx_id = self.router.active().context_id;
        self.context_active_window.insert(ctx_id, target.window_id);
        self.record_context_visit(target.window_id);

        // Focus the specific pane if requested — find its TileId in the tree.
        if let Some(pid) = target.pane_id {
            let win = &mut self.windows[ctx_idx];
            if let Some(tile_id) = win.tree.tiles.find_pane(&pid) {
                // navigate_to, not a bare assignment: a pane in an inactive
                // tab must become the visible tab (stint 1230).
                win.navigate_to(tile_id);
            }
        }
    }

    /// Compatibility adapter for non-palette callers that still resolve a
    /// window by index. Palette rows must use `focus_palette_target` instead.
    pub(crate) fn jump_to_context(
        &mut self,
        window_index: usize,
        window_id: u64,
        pane_id: Option<u64>,
    ) {
        let Some(context_id) = self
            .windows
            .get(window_index)
            .map(|window| window.context_id)
        else {
            log::warn!("palette: window index {window_index} disappeared before context focus");
            return;
        };
        self.focus_palette_target(PaletteFocusTarget {
            context_id,
            window_id,
            pane_id,
        });
    }
}

#[cfg(test)]
impl PlexiApp {
    pub(crate) fn palette_selected_key(&self) -> Option<String> {
        self.host
            .picker(PickerId::Palette)
            .and_then(|picker| picker.selected())
            .map(|key| key.as_str().to_owned())
    }

    pub(crate) fn palette_selected_index(&self) -> Option<usize> {
        let picker = self.host.picker(PickerId::Palette)?;
        let key = picker.selected()?;
        picker.rows().iter().position(|row| row.key == *key)
    }

    /// Replace the field's text as typing would, for tests that set a
    /// query directly.
    #[cfg(test)]
    pub(crate) fn set_palette_query(&mut self, query: &str) {
        self.palette_input.draft = query.to_owned();
        self.apply_palette_command(PickerCommand::QueryEdited(query.to_owned()));
    }
}

/// The palette's query field.
fn palette_field_id() -> egui::Id {
    egui::Id::new("palette_search")
}

/// Where this delivery's commands go relative to the draw.
#[derive(Default)]
struct DrawOrder {
    /// The field took an edit this pass: a later command waits for it.
    field_dirty: bool,
    /// A command, an activation or an edit already waits for the draw, so
    /// everything after it waits too.
    deferring: bool,
    /// The pointer moved: a command after it waits for the hover.
    moved: bool,
}

/// Person input a palette session owns: keys, text, clipboard, IME and
/// pointer edges, moves and scrolling. Window, screenshot and accessibility
/// events stay where they are.
fn is_palette_input(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Ime(_)
            | egui::Event::PointerMoved(_)
            | egui::Event::MouseMoved(_)
            | egui::Event::PointerButton { .. }
            | egui::Event::PointerGone
            | egui::Event::MouseWheel { .. }
            | egui::Event::Touch { .. }
    )
}

/// Test-only observation of the palette's production path: which rows ran,
/// in order, and how often the Commands source answered. Per test thread.
#[cfg(test)]
pub(crate) mod palette_test_log {
    use crate::host::picker::RowKey;
    use std::cell::{Cell, RefCell};

    thread_local! {
        static ACTIVATED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static SOURCE_CALLS: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn activated(key: RowKey) {
        ACTIVATED.with(|log| log.borrow_mut().push(key.as_str().to_owned()));
    }

    pub(super) fn source_called() {
        SOURCE_CALLS.with(|calls| calls.set(calls.get() + 1));
    }

    /// The row keys run since the last take, in activation order.
    pub(crate) fn take_activations() -> Vec<String> {
        ACTIVATED.with(|log| std::mem::take(&mut *log.borrow_mut()))
    }

    /// Commands-source answers since the last take.
    pub(crate) fn take_source_calls() -> usize {
        SOURCE_CALLS.with(|calls| calls.replace(0))
    }

    /// A script answering the palette's query in `App::logic` instead of the
    /// Commands source: given the current generation and query, the
    /// generation answered, the row view ids and `done`; `None` answers
    /// nothing this pass.
    pub(crate) type Script = Box<dyn FnMut(u64, &str) -> Option<(u64, Vec<String>, bool)>>;

    thread_local! {
        static SCRIPT: RefCell<Option<Script>> = const { RefCell::new(None) };
    }

    pub(crate) fn set_script(script: Option<Script>) {
        SCRIPT.with(|slot| *slot.borrow_mut() = script);
    }

    /// `None` when unscripted; otherwise this pass's scripted answer.
    pub(super) fn scripted_answer(
        generation: u64,
        query: &str,
    ) -> Option<Option<(u64, Vec<String>, bool)>> {
        SCRIPT.with(|slot| {
            slot.borrow_mut()
                .as_mut()
                .map(|script| script(generation, query))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::pane::{Pane, PortalPane};
    use crate::host::picker::PickerState;
    use crate::ui::list::ListRowPips;

    #[test]
    fn queued_command_without_focus_captures_first_pane() {
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_test_pane();
        let win = &mut h.app.windows[0];
        let tile = win.tree.tiles.find_pane(&pane).unwrap();
        win.tree.root = Some(win.tree.tiles.insert_tab_tile(vec![tile]));
        win.focused_pane = None;
        h.app.queue_user_command("test", None);
        assert_eq!(h.app.pending_user_commands[0].source_tile, Some(tile));
    }

    #[test]
    fn queued_command_keeps_origin_after_active_context_changes() {
        let mut h = crate::testing::HostHarness::new();
        let origin = h.add_test_pane();
        h.focus_pane(origin);
        let source_tile = h.app.windows[0].focused_pane;
        h.app.windows[0].zoomed_pane = source_tile;
        h.app.palette_commands = vec![crate::cli::ResolvedUserCommand {
            name: "test".into(),
            scope: crate::cli::UserCommandScope::Workspace,
            description: None,
            run: Some("echo queued-origin".into()),
            steps: Vec::new(),
            on_load: vec!["pane name QueuedOrigin".into()],
            secrets: Vec::new(),
            split: true,
        }];
        h.app.queue_user_command("test", None);
        let other_root = tempfile::tempdir().unwrap();
        let mut context = h.app.router.active().clone();
        context.context_id += 100;
        context.root = other_root.path().into();
        h.app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: context.root.clone(),
            tree: egui_tiles::Tree::empty("queued-other-window"),
            panes: Default::default(),
            focused_pane: None,
            zoomed_pane: source_tile,
            grid_x: 1,
            grid_y: 0,
            window_id: h.app.next_window_id,
            context_id: context.context_id,
            tree_history: Default::default(),
        });
        h.app.next_window_id += 1;
        h.app.router.push(context);
        h.app.router.set_active(1);
        h.app.active_window = 1;
        h.app.drain_pending_user_commands();
        assert_eq!(h.app.windows[0].zoomed_pane, None);
        assert_eq!(h.app.windows[1].zoomed_pane, source_tile);
        assert!(h.app.windows[1].panes.is_empty());
        assert!(h.app.windows[0].panes.values().any(|pane| {
            pane.as_terminal()
                .is_some_and(|pane| pane.name.as_deref() == Some("QueuedOrigin"))
        }));
        assert_eq!(h.app.active_window, 1);
        assert_eq!(h.app.router.active_idx(), 1);
    }

    fn stepped_command(h: &mut crate::testing::HostHarness, steps: Vec<String>) {
        h.app.palette_commands = vec![crate::cli::ResolvedUserCommand {
            name: "slow".into(),
            scope: crate::cli::UserCommandScope::Workspace,
            description: None,
            run: None,
            steps,
            on_load: Vec::new(),
            secrets: Vec::new(),
            split: false,
        }];
        h.app.queue_user_command("slow", None);
    }

    /// Run frames until `done` holds, failing after `limit`.
    fn frames_until(
        h: &mut crate::testing::HostHarness,
        limit: std::time::Duration,
        mut done: impl FnMut(&crate::testing::HostHarness) -> bool,
    ) {
        let start = std::time::Instant::now();
        while !done(h) {
            assert!(
                start.elapsed() < limit,
                "condition not met within {limit:?}"
            );
            h.run_frames(1);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Stint 1625: a slow shell step must not hold App::logic. Frames keep
    /// running while it waits, and the next step starts only after it exits.
    #[test]
    fn slow_shell_step_does_not_block_logic() {
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_test_pane();
        h.focus_pane(pane);
        let dir = tempfile::tempdir().unwrap();
        let release = dir.path().join("release");
        let second = dir.path().join("second");
        // Waits for the test to release it; gives up (exit 1) after ~5s so a
        // blocking host fails the timing assertion instead of hanging.
        let wait = format!(
            "i=0; while [ ! -e '{r}' ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i+1)); done; [ -e '{r}' ]",
            r = release.display()
        );
        stepped_command(
            &mut h,
            vec![wait, format!("printf ok > '{}'", second.display())],
        );
        let start = std::time::Instant::now();
        h.run_frames(1);
        assert!(
            start.elapsed() < std::time::Duration::from_millis(1500),
            "logic frame blocked on the shell step for {:?}",
            start.elapsed()
        );
        h.run_frames(5);
        assert!(!second.exists(), "step 2 started before step 1 exited");
        std::fs::write(&release, "").unwrap();
        frames_until(&mut h, std::time::Duration::from_secs(5), |_| {
            second.exists()
        });
    }

    /// Off-thread shell steps keep the saved-command contract: steps run in
    /// order, a nested `pane new --on-load` finishes before the next step, and
    /// the first failure stops the command.
    #[test]
    fn host_steps_keep_order_and_stop_at_failure() {
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_test_pane();
        h.focus_pane(pane);
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let append = |tag: &str| format!("printf {tag} >> '{}'", log.display());
        stepped_command(
            &mut h,
            vec![
                format!("sleep 0.3; {}", append("a")),
                format!(
                    "pane new --right --on-load \"sleep 0.2; {}\" --on-load \"{}\"",
                    append("b"),
                    append("c")
                ),
                append("d"),
                "exit 3".into(),
                append("e"),
            ],
        );
        h.run_frames(1);
        assert_eq!(h.app.user_command_runs.len(), 1, "run parks on step 1");
        frames_until(&mut h, std::time::Duration::from_secs(10), |h| {
            h.app.user_command_runs.is_empty()
        });
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "abcd");
    }

    #[test]
    fn queued_command_closed_source_is_refused_before_steps() {
        for close_context in [false, true] {
            let mut h = crate::testing::HostHarness::new();
            let pane = h.add_test_pane();
            h.focus_pane(pane);
            h.app.notifications_enabled = true;
            let marker = h.workspace_root().join("should-not-run");
            h.app.palette_commands = vec![crate::cli::ResolvedUserCommand {
                name: "test".into(),
                scope: crate::cli::UserCommandScope::Workspace,
                description: None,
                run: None,
                steps: vec![format!("printf ran > '{}'", marker.display())],
                on_load: Vec::new(),
                secrets: Vec::new(),
                split: false,
            }];
            h.app.queue_user_command("test", None);
            if close_context {
                h.app.router.get_mut(0).context_id += 100;
            } else {
                h.app.windows[0].take_pane(pane);
            }
            h.app.drain_pending_user_commands();
            assert!(!marker.exists(), "closed source must not execute steps");
            assert!(h
                .app
                .pending_notifications
                .iter()
                .any(|n| n.title == "Command 'test' not run"));
        }
    }

    #[test]
    fn agent_mesh_gate_palette_admission_runs_in_logic() {
        for route in ["open", "start", "spawn"] {
            for from_key in [false, true] {
                let mut h = crate::testing::HostHarness::new();
                let _enabled = crate::release::set_test_experiments(&[
                    crate::release::ReleaseFeature::AgentMesh,
                ]);
                h.app.notifications_enabled = true;
                h.app.palette_commands = vec![crate::cli::ResolvedUserCommand {
                    name: "gate-test".into(),
                    scope: crate::cli::UserCommandScope::Workspace,
                    description: None,
                    run: None,
                    steps: vec![
                        format!(
                            "agent {route} missing{}",
                            if route == "spawn" { " --task test" } else { "" }
                        ),
                        "pane new --right".into(),
                    ],
                    on_load: Vec::new(),
                    secrets: Vec::new(),
                    split: false,
                }];
                let before = h.pane_count();
                let logs = crate::testing::test_log::capture();
                if from_key {
                    let ctx = h.app.ctx.clone();
                    h.app.dispatch_host_action(
                        crate::host::keys::Action::RunUserCommand("gate-test".into()),
                        &ctx,
                    );
                } else {
                    h.app.run_palette_selection(CommandRowId::UserCommand {
                        name: "gate-test".into(),
                        scope: crate::cli::UserCommandScope::Workspace,
                    });
                }
                assert!(
                    !logs
                        .lines()
                        .iter()
                        .any(|line| line.starts_with("release_gate:")),
                    "UI must not read admission config: {:?}",
                    logs.lines()
                );
                assert!(h.app.pending_notifications.is_empty());
                // Admission observes the config at logic servicing, not selection.
                let _disabled = crate::release::set_test_experiments(&[]);
                h.hidden_frame();
                assert!(h
                    .app
                    .pending_notifications
                    .iter()
                    .any(|n| n.body.contains("experiments enable agent_mesh")));
                assert_eq!(h.pane_count(), before, "a denied step stops the command");
                assert_eq!(
                    logs.lines()
                        .iter()
                        .filter(|line| line.starts_with("release_gate:"))
                        .count(),
                    1
                );
                assert!(
                    !logs
                        .records()
                        .iter()
                        .any(|(level, line)| *level == log::Level::Warn
                            && line.contains("agent_mesh"))
                );
            }
        }
    }

    #[test]
    fn plugin_metadata_chips_add_workspace_scope_without_secondary_text() {
        assert_eq!(plugin_metadata_chips(false, false), &["view"]);
        assert_eq!(plugin_metadata_chips(false, true), &["view", "ws"]);
        assert_eq!(plugin_metadata_chips(true, true), &["view", "bg", "ws"]);
    }

    #[test]
    fn palette_command_aliases_match_starter_synonyms() {
        assert_eq!(
            palette_command_matches("shell"),
            vec![
                PaletteCommand::Host(crate::host::keys::Action::SplitRight),
                PaletteCommand::Host(crate::host::keys::Action::SplitDown),
            ]
        );
        assert_eq!(
            palette_command_matches("console"),
            vec![
                PaletteCommand::Host(crate::host::keys::Action::SplitRight),
                PaletteCommand::Host(crate::host::keys::Action::SplitDown),
            ]
        );
        assert_eq!(
            palette_command_matches("hsplit"),
            vec![PaletteCommand::Host(crate::host::keys::Action::SplitDown)]
        );
        assert_eq!(
            palette_command_matches("config"),
            vec![
                PaletteCommand::OpenGlobalConfig,
                PaletteCommand::OpenWorkspaceConfig,
                PaletteCommand::Host(crate::host::keys::Action::ReloadConfig),
            ]
        );
        assert_eq!(
            palette_command_matches("notification"),
            vec![PaletteCommand::Host(
                crate::host::keys::Action::ToggleAttentionQueue
            )]
        );
        assert_eq!(
            palette_command_matches("picker"),
            vec![PaletteCommand::Host(
                crate::host::keys::Action::OpenNotesPicker
            )]
        );
        assert_eq!(
            palette_command_matches("scratchpad"),
            vec![PaletteCommand::Host(
                crate::host::keys::Action::OpenScratchpad
            )]
        );
        assert_eq!(
            palette_command_matches("scratch pad"),
            vec![PaletteCommand::Host(
                crate::host::keys::Action::OpenScratchpad
            )]
        );
    }

    /// The native file surface is reachable from Cmd+P like every other host
    /// action: the registry must carry a File Explorer entry whose aliases
    /// cover the words a user types, and its shortcut label must follow the
    /// configured binding (never a hardcoded chord).
    #[test]
    fn palette_registers_file_explorer_with_search_aliases_and_configured_shortcut() {
        use crate::host::keys::{Action, KeyBindings};
        let command = PaletteCommand::Host(Action::OpenFileBrowser);

        for query in [
            "file explorer",
            "file browser",
            "file",
            "browse",
            "directory",
            "tree",
            "open",
        ] {
            assert!(
                palette_command_matches(query).contains(&command),
                "palette search '{query}' must surface File Explorer"
            );
        }

        let entry = PALETTE_COMMANDS
            .iter()
            .find(|entry| entry.command == command)
            .expect("File Explorer must be registered in PALETTE_COMMANDS");
        assert_eq!(entry.name, "File Explorer");

        let mut bindings = KeyBindings::default();
        assert_eq!(
            palette_host_binding(&bindings, &command),
            Some(bindings.open_file_browser),
            "File Explorer must show its configured shortcut"
        );

        // A user override renders the override, not the default chord.
        bindings.open_file_browser = (
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::F,
        );
        let overridden = palette_host_binding(&bindings, &command)
            .expect("overridden binding must still render a label");
        assert_eq!(overridden, bindings.open_file_browser);
        assert_eq!(
            shortcut_label(overridden),
            shortcut_label(bindings.open_file_browser)
        );
    }

    #[test]
    fn palette_check_for_updates_is_always_listed_and_starts_a_check() {
        for query in ["check for updates", "update", "upgrade"] {
            assert!(
                palette_command_matches(query).contains(&PaletteCommand::CheckForUpdates),
                "missing {query}"
            );
        }
        assert!(
            !palette_command_matches("").contains(&PaletteCommand::RestartToUpdate),
            "Restart to update is listed only through the ready row"
        );
        let mut h = crate::testing::HostHarness::new();
        crate::app::update_check::test_launcher::take();
        h.app.run_palette_command(PaletteCommand::CheckForUpdates);
        let launched = crate::app::update_check::test_launcher::take();
        assert_eq!(launched.len(), 1);
        assert_eq!(launched[0].0, crate::host::update_status::CheckMode::Manual);
        assert_eq!(
            h.app.update_status.line(),
            crate::host::update_status::UpdateLine::Checking
        );
        assert!(
            h.app.show_changelog,
            "the result shows in the changelog view"
        );
        // Running it again while the check is in flight launches nothing
        // and keeps the changelog open.
        h.app.run_palette_command(PaletteCommand::CheckForUpdates);
        assert!(crate::app::update_check::test_launcher::take().is_empty());
        assert!(h.app.show_changelog);
    }

    #[test]
    fn palette_update_ready_row_only_when_ready_and_restarts() {
        assert!(update_ready_entry(None, "").is_none());
        let entry = update_ready_entry(Some("0.5.1"), "").expect("ready row on empty query");
        assert_eq!(entry.row_id(), CommandRowId::UpdateReady("0.5.1".into()));
        for query in ["restart", "update", "0.5.1"] {
            assert!(
                update_ready_entry(Some("0.5.1"), query).is_some(),
                "missing {query}"
            );
        }
        assert!(update_ready_entry(Some("0.5.1"), "zzqx").is_none());
        let (row, _) = entry.into_row("", None);
        assert!(row.pinned, "the ready row leads the list");
        assert_eq!(row.actions[0].id, "run");

        let mut h = crate::testing::HostHarness::new();
        h.app.run_palette_command(PaletteCommand::RestartToUpdate);
        assert!(!h.app.show_changelog, "nothing ready, nothing offered");
        assert!(!h.app.update_confirm_prompt);
        assert!(!h.app.update_quit_pending);
    }

    fn harness_with_ready_update() -> crate::testing::HostHarness {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.update_status.begin_check();
        h.app
            .update_status
            .apply(crate::host::update_status::UpdateEvent::Ready {
                version: "0.5.1".into(),
            });
        h.run_frames(1);
        h
    }

    fn open_palette_and_press_enter(h: &mut crate::testing::HostHarness) {
        h.app.open_command_palette("");
        h.run_frames(1);
        h.press_key(egui::Key::Enter, egui::Modifiers::NONE);
    }

    /// Cmd+P then Enter on the ready row only asks; a second Enter restarts.
    /// The harness's restart scheduler only records, so the frames run the
    /// real quit path without touching an installed channel.
    #[test]
    fn palette_update_ready_enter_asks_then_second_enter_restarts() {
        crate::app::quit::test_restart_scheduler::take();
        let mut h = harness_with_ready_update();
        open_palette_and_press_enter(&mut h);
        assert!(!h.app.command_palette_open());
        assert!(h.app.show_changelog, "the restart confirm opens");
        assert!(h.app.update_confirm_prompt);
        assert!(
            !h.app.update_quit_pending,
            "the first Enter must never restart"
        );
        assert_eq!(
            h.app.focus_stack.top().as_ref(),
            Some(&crate::app::FocusKind::Changelog)
        );

        // The second Enter confirms; the logic pass it requests takes the
        // restart. A frame after the window hides would reach `exit_host`,
        // so the test stops after that logic-only pass.
        h.press_key(egui::Key::Enter, egui::Modifiers::NONE);
        h.hidden_frame();
        assert_eq!(crate::app::quit::test_restart_scheduler::take(), 1);
        assert!(!h.app.update_confirm_prompt);
        assert!(!h.app.update_quit_pending);
        assert_eq!(
            h.app.quit_coordinator.reason,
            Some(crate::app::quit::ExitReason::UpdateRestart)
        );
    }

    #[test]
    fn palette_update_ready_confirm_esc_cancels() {
        let mut h = harness_with_ready_update();
        open_palette_and_press_enter(&mut h);
        assert!(h.app.update_confirm_prompt);
        h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!h.app.update_confirm_prompt);
        assert!(!h.app.show_changelog);
        h.run_frames(2);
        assert!(!h.app.update_quit_pending);
        assert!(!h.app.quit_coordinator.is_active());
        assert_eq!(h.app.update_status.ready(), Some("0.5.1"));
    }

    #[test]
    fn palette_changelog_search_and_open_are_idempotent() {
        use crate::host::keys::Action;
        let command = PaletteCommand::Host(Action::OpenChangelog);
        for query in ["changelog", "release notes", "what's new", "version"] {
            assert!(
                palette_command_matches(query).contains(&command),
                "missing {query}"
            );
        }
        let mut h = crate::testing::HostHarness::new();
        h.app.run_palette_command(command.clone());
        assert!(h.app.show_changelog);
        assert_eq!(
            h.app.focus_stack.top().as_ref(),
            Some(&crate::app::FocusKind::Changelog)
        );
        h.app.run_palette_command(command);
        assert!(h.app.show_changelog);
        assert_eq!(
            h.app.focus_stack.kinds().collect::<Vec<_>>(),
            [crate::app::FocusKind::Changelog]
        );
        h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!h.app.show_changelog);
        assert!(h.app.focus_stack.is_empty());
    }

    #[test]
    fn palette_searchable_text_is_cached_lowercase() {
        assert_eq!(
            searchable_text(&["Open Config", "Settings", "CONFIG.toml"]),
            "open config settings config.toml"
        );
        for entry in PALETTE_COMMANDS {
            assert_eq!(
                entry.search_text,
                entry.search_text.to_lowercase(),
                "palette command search text must be pre-lowercased"
            );
        }
    }

    #[test]
    fn palette_shortcut_label_uses_platform_command_name_and_configured_key() {
        let configured = (
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::F,
        );
        let expected = if cfg!(target_os = "macos") {
            "⌘⇧F"
        } else {
            "Ctrl+Shift+F"
        };
        assert_eq!(shortcut_label(configured), expected);
    }

    /// Every host command the palette lists that has a key binding must show
    /// that binding. The dogfood set (NewContext, ToggleZoom, NavBackView, …)
    /// used to miss this match and warn once per row per Cmd+P open.
    #[test]
    fn palette_lists_a_shortcut_for_every_bound_host_command() {
        use crate::host::keys::{Action, KeyBindings};
        let bindings = KeyBindings::default();
        for entry in PALETTE_COMMANDS {
            match &entry.command {
                PaletteCommand::OpenAttention => {
                    assert!(
                        palette_host_binding(&bindings, &entry.command).is_none(),
                        "needs attention is a palette-only entry with no host chord"
                    );
                }
                PaletteCommand::CloseSelectedPanes => {
                    assert_eq!(
                        palette_host_binding(&bindings, &entry.command),
                        Some(bindings.close_pane),
                        "close selected shares the close-pane chord, no second binding"
                    );
                }
                PaletteCommand::OpenGlobalConfig
                | PaletteCommand::OpenWorkspaceConfig
                | PaletteCommand::EditCommands
                | PaletteCommand::SwitchCommandsScope
                | PaletteCommand::SetupCli
                | PaletteCommand::StatePreset(_)
                | PaletteCommand::UiUpcycle(_)
                | PaletteCommand::CheckForUpdates
                | PaletteCommand::RestartToUpdate => {
                    assert!(
                        palette_host_binding(&bindings, &entry.command).is_none(),
                        "workspace config / edit commands are palette-only handoffs with no host chord"
                    );
                }
                PaletteCommand::Host(
                    Action::OpenConfig
                    | Action::OpenChangelog
                    | Action::NavBackView
                    | Action::FocusHistoryForward
                    | Action::NextContext
                    | Action::PrevContext,
                ) => {
                    assert!(palette_host_binding(&bindings, &entry.command).is_none());
                }
                PaletteCommand::Host(action) => {
                    let label = palette_host_binding(&bindings, &entry.command).map(shortcut_label);
                    assert!(
                        label.is_some(),
                        "palette command {action:?} ({}) must show its shortcut, got None",
                        entry.name
                    );
                }
            }
        }
        // Actions the palette does not list stay quiet: one debug line, not a
        // warn on every paint. Calling twice must not grow the seen-set twice
        // in a way that changes the result — both calls return None.
        let unlisted = PaletteCommand::Host(Action::CloseView);
        assert!(
            !PALETTE_COMMANDS.iter().any(|e| e.command == unlisted),
            "CloseView must stay unlisted for this coverage"
        );
        assert!(palette_host_binding(&bindings, &unlisted).is_none());
        assert!(palette_host_binding(&bindings, &unlisted).is_none());
    }

    #[test]
    fn empty_palette_puts_commands_after_apps() {
        let entries = vec![
            PaletteEntry::Command {
                command: PaletteCommand::Host(crate::host::keys::Action::OpenConfig),
                name: "Open config",
                description: "Edit config",
                search_text: "open config",
            },
            PaletteEntry::View {
                id: "balls".to_string(),
                name: "Balls".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "balls demo".to_string(),
            },
            PaletteEntry::Context {
                target: PaletteFocusTarget {
                    context_id: 1,
                    window_id: 1,
                    pane_id: None,
                },
                name: "Workspace".to_string(),
                workspace_name: String::new(),
                metadata_chips: vec!["ctx"],
                pane_pips: None,
                search_text: "workspace".to_string(),
            },
        ];

        let entries = ranked(entries, "");

        assert_eq!(row_kind(&entries[0]), "focus");
        assert_eq!(row_kind(&entries[1]), "view");
        assert_eq!(row_kind(&entries[2]), "cmd");
    }

    #[test]
    fn parked_and_windowless_context_entries_remain_searchable() {
        let parked = PaletteEntry::Context {
            target: PaletteFocusTarget {
                context_id: 7,
                window_id: 70,
                pane_id: None,
            },
            name: "parked release work".to_string(),
            workspace_name: "Parked".to_string(),
            metadata_chips: vec!["ctx", "parked"],
            pane_pips: None,
            search_text: "parked release work".to_string(),
        };
        let unavailable = PaletteEntry::UnavailableContext {
            name: "parked without window".to_string(),
            search_text: "parked without window unavailable".to_string(),
        };

        assert!(parked.matches_query("release"));
        assert!(unavailable.matches_query("window"));
        assert_eq!(parked.group().1, 0);
        assert_eq!(unavailable.group().1, 0);
    }

    /// The results painter prints CONTEXTS when the row group changes to it,
    /// so every Context row must land in one contiguous leading block on an
    /// empty query, and no CONTEXTS group exists without a Context row.
    #[test]
    fn contexts_header_shows_once_for_a_leading_context_block_and_never_without_one() {
        let with_contexts = vec![
            PaletteEntry::View {
                id: "balls".to_string(),
                name: "Balls".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "balls demo".to_string(),
            },
            PaletteEntry::Context {
                target: PaletteFocusTarget {
                    context_id: 1,
                    window_id: 1,
                    pane_id: None,
                },
                name: "Workspace A".to_string(),
                workspace_name: String::new(),
                metadata_chips: vec!["ctx"],
                pane_pips: None,
                search_text: "workspace a".to_string(),
            },
            PaletteEntry::Context {
                target: PaletteFocusTarget {
                    context_id: 2,
                    window_id: 2,
                    pane_id: None,
                },
                name: "Workspace B".to_string(),
                workspace_name: String::new(),
                metadata_chips: vec!["ctx"],
                pane_pips: None,
                search_text: "workspace b".to_string(),
            },
        ];
        let with_contexts = ranked(with_contexts, "");

        // Both Context rows land in one leading block under one header.
        assert_eq!(row_kind(&with_contexts[0]), "focus");
        assert_eq!(row_kind(&with_contexts[1]), "focus");
        assert_eq!(row_kind(&with_contexts[2]), "view");

        let without_contexts = vec![PaletteEntry::View {
            id: "balls".to_string(),
            name: "Balls".to_string(),
            description: "Demo".to_string(),
            running_in_background: false,
            is_workspace_local: false,
            search_text: "balls demo".to_string(),
        }];
        assert!(
            !ranked(without_contexts, "")
                .iter()
                .any(|row| row.group == "CONTEXTS"),
            "no Context row means CONTEXTS never renders"
        );
    }

    /// Rank `entries` through the picker reducer exactly as the Commands
    /// stop does, and return the rows in display order.
    fn ranked(entries: Vec<PaletteEntry>, query: &str) -> Vec<PickerRow> {
        ranked_state(entries, query).rows().to_vec()
    }

    fn ranked_state(entries: Vec<PaletteEntry>, query: &str) -> PickerState {
        let mut state = PickerState::new(PALETTE_STOPS);
        state.apply(PickerCommand::Open {
            stop: PALETTE_STOPS[0].id,
            query: query.to_string(),
        });
        answer(&mut state, entries, query);
        state
    }

    fn answer(state: &mut PickerState, entries: Vec<PaletteEntry>, query: &str) {
        let normalized = normalized_palette_query(query);
        let rows = entries
            .into_iter()
            .map(|entry| entry.into_row(&normalized, None).0)
            .collect();
        let generation = state.generation();
        state.apply(PickerCommand::Rows {
            generation,
            rows,
            done: true,
        });
    }

    /// The row's kind, the stable id up to its first \`/\`.
    fn row_kind(row: &PickerRow) -> &str {
        row.key.stable_id().split('/').next().unwrap_or_default()
    }

    /// Click a listed row of the open palette, the pointer's path to
    /// activation.
    fn click_row(h: &mut crate::testing::HostHarness, id: CommandRowId) {
        // Rows are answered on the next logic pass.
        h.run_frames(1);
        let key = id.key();
        assert!(
            h.app
                .host
                .picker(PickerId::Palette)
                .is_some_and(|picker| picker.rows().iter().any(|row| row.key == key)),
            "{id:?} must be listed in the open palette"
        );
        let stamp = h
            .app
            .host
            .picker(PickerId::Palette)
            .expect("palette picker")
            .rows_stamp();
        h.app
            .apply_palette_command(PickerCommand::Click(crate::host::picker::Hit {
                stamp,
                key,
            }));
    }

    /// The results painter prints a section header each time the group
    /// changes, so a group split into two runs would attach its later rows
    /// to another group's header. Returns the group of each run.
    fn palette_group_runs(rows: &[PickerRow]) -> Vec<u32> {
        let mut runs: Vec<u32> = Vec::new();
        for row in rows {
            if runs.last() != Some(&row.group_rank) {
                runs.push(row.group_rank);
            }
        }
        runs
    }

    fn assert_each_group_is_one_run(rows: &[PickerRow]) {
        let runs = palette_group_runs(rows);
        let mut seen = runs.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            runs.len(),
            "every palette group must render as one contiguous run under its header, got runs {runs:?}"
        );
    }

    fn todo_view_entry() -> PaletteEntry {
        // The shipped Todo manifest: its description mentions a file, which
        // is incidental to a "file" query.
        let description = "Context-scoped todo list, backed by a markdown checklist file a human or agent can edit directly.";
        PaletteEntry::View {
            id: "todo".to_string(),
            name: "Todo".to_string(),
            description: description.to_string(),
            running_in_background: false,
            is_workspace_local: false,
            search_text: searchable_text(&["Todo", "todo", description]),
        }
    }

    fn registered_command_entry(command: PaletteCommand) -> PaletteEntry {
        let entry = PALETTE_COMMANDS
            .iter()
            .find(|entry| entry.command == command)
            .expect("palette command must be registered in PALETTE_COMMANDS");
        PaletteEntry::Command {
            command: entry.command.clone(),
            name: entry.name,
            description: entry.description,
            search_text: entry.search_text,
        }
    }

    fn file_explorer_command() -> PaletteCommand {
        PaletteCommand::Host(crate::host::keys::Action::OpenFileBrowser)
    }

    #[test]
    fn typed_query_selects_direct_title_match_above_incidental_description_match_across_groups() {
        for query in ["file", "file explorer"] {
            let state = ranked_state(
                vec![
                    todo_view_entry(),
                    registered_command_entry(file_explorer_command()),
                ],
                query,
            );
            let rows = state.rows();
            assert_eq!(
                rows[0].key,
                CommandRowId::Command(file_explorer_command()).key(),
                "query {query:?} must rank File Explorer's title match first"
            );
            // A fresh query selects the first displayed row, and Enter acts on it.
            assert_eq!(state.selected(), Some(&rows[0].key));
            assert_each_group_is_one_run(rows);
        }

        // Description-only discovery survives: Todo still matches "file",
        // just below the direct title match.
        let todo = todo_view_entry();
        assert!(todo.matches_query("file"));
        let rows = ranked(
            vec![todo, registered_command_entry(file_explorer_command())],
            "file",
        );
        assert_eq!(rows[1].key, CommandRowId::View("todo".into()).key());
    }

    #[test]
    fn typed_palette_query_keeps_groups_contiguous_for_their_headers() {
        let view = |id: &str, name: &str, search_text: &str| PaletteEntry::View {
            id: id.to_string(),
            name: name.to_string(),
            description: "Demo".to_string(),
            running_in_background: false,
            is_workspace_local: false,
            search_text: search_text.to_string(),
        };
        let entries = vec![
            // Weak app: "open" only in its description text.
            view("viewer", "Viewer", "viewer demo open anything"),
            PaletteEntry::Command {
                command: PaletteCommand::Host(crate::host::keys::Action::OpenConfig),
                name: "Open config",
                description: "Edit config",
                search_text: "open config settings preferences",
            },
            // Second app: mid-word title substring, weaker than the
            // command's title prefix but stronger than any description hit.
            view("reopen", "Reopen", "reopen demo"),
            PaletteEntry::Command {
                command: PaletteCommand::Host(crate::host::keys::Action::OpenQuickNote),
                name: "Quick Note",
                description: "Capture a note",
                search_text: "quick note capture open inbox",
            },
        ];

        let entries = ranked(entries, "open");

        // The group holding the strongest match leads; each group stays one
        // run, so every header sits above its own rows.
        assert_each_group_is_one_run(&entries);
        assert_eq!(palette_group_runs(&entries), vec![4, 3]);
        assert_eq!(entries[0].title, "Open config");
        assert_eq!(entries[1].title, "Quick Note");
        assert_eq!(entries[2].title, "Reopen");
        assert_eq!(entries[3].title, "Viewer");
    }

    #[test]
    fn typed_query_with_equal_relevance_keeps_default_group_order() {
        let entries = vec![
            PaletteEntry::Command {
                command: PaletteCommand::Host(crate::host::keys::Action::OpenChangelog),
                name: "Changelog",
                description: "Show what changed",
                search_text: "changelog",
            },
            PaletteEntry::View {
                id: "changelog".to_string(),
                name: "Changelog".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "changelog".to_string(),
            },
        ];

        let entries = ranked(entries, "changelog");

        assert_eq!(row_kind(&entries[0]), "view");
        assert_eq!(row_kind(&entries[1]), "cmd");
    }

    #[test]
    fn palette_selection_follows_its_target_when_relevance_reorders_groups() {
        let mut state = ranked_state(
            vec![
                todo_view_entry(),
                registered_command_entry(file_explorer_command()),
            ],
            "file",
        );
        // The user moves to the incidental Todo row.
        state.apply(PickerCommand::Move(crate::host::picker::Step::Down));
        // A rebuild with the same query, collected in a different order.
        answer(
            &mut state,
            vec![
                registered_command_entry(file_explorer_command()),
                todo_view_entry(),
            ],
            "file",
        );
        let selected = state.selected().expect("selection kept");
        assert_eq!(*selected, CommandRowId::View("todo".to_string()).key());
    }

    #[test]
    fn empty_palette_puts_user_commands_below_host_commands() {
        let entries = vec![
            PaletteEntry::UserCommand {
                name: "build".to_string(),
                secondary: "cargo build".to_string(),
                scope: crate::cli::UserCommandScope::Workspace,
                search_text: "build cargo build run workspace ws".to_string(),
            },
            PaletteEntry::Command {
                command: PaletteCommand::Host(crate::host::keys::Action::OpenConfig),
                name: "Open config",
                description: "Edit config",
                search_text: "open config",
            },
            PaletteEntry::View {
                id: "balls".to_string(),
                name: "Balls".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "balls demo".to_string(),
            },
        ];

        let entries = ranked(entries, "");

        assert_eq!(row_kind(&entries[0]), "view");
        assert_eq!(row_kind(&entries[1]), "cmd");
        assert_eq!(row_kind(&entries[2]), "run");
    }

    #[test]
    fn typed_query_surfaces_user_command_by_name() {
        let entries = vec![
            PaletteEntry::View {
                // Only a substring match for "test" (inside "fastest").
                id: "fastest".to_string(),
                name: "Fastest".to_string(),
                description: "Demo".to_string(),
                running_in_background: false,
                is_workspace_local: false,
                search_text: "fastest demo loader".to_string(),
            },
            PaletteEntry::UserCommand {
                name: "test".to_string(),
                secondary: "cargo test".to_string(),
                scope: crate::cli::UserCommandScope::Workspace,
                search_text: "test cargo test run workspace ws".to_string(),
            },
        ];

        // The exact-name user command outranks the app's mid-word substring,
        // so its group leads; each group stays one contiguous run.
        let entries = ranked(entries, "test");
        assert_eq!(row_kind(&entries[0]), "run");
        assert_eq!(row_kind(&entries[1]), "view");
    }

    #[test]
    fn inactive_context_name_query_stays_collapsed_but_pane_name_pierces() {
        assert!(
            !inactive_pane_matches_query("build log", "squad-alpha"),
            "a context-name match must leave its inactive context collapsed"
        );
        assert!(inactive_pane_matches_query("build log", "build log"));
    }

    #[test]
    fn palette_query_trims_outer_whitespace() {
        assert_eq!(normalized_palette_query("  Blocked  "), "blocked");
    }

    #[test]
    fn palette_exposes_review_notifications() {
        assert_eq!(
            palette_command_matches("review notifications"),
            vec![PaletteCommand::Host(
                crate::host::keys::Action::ToggleAttentionQueue
            )]
        );
    }

    #[test]
    fn palette_exposes_open_secrets() {
        assert_eq!(
            palette_command_matches("open secrets"),
            vec![PaletteCommand::Host(
                crate::host::keys::Action::OpenSecretsManager
            )]
        );
        assert_eq!(
            palette_host_binding(
                &crate::host::keys::KeyBindings::default(),
                &PaletteCommand::Host(crate::host::keys::Action::OpenSecretsManager)
            ),
            Some(crate::host::keys::KeyBindings::default().open_secrets_manager)
        );
    }

    /// Every row kind has its own key, and the Commands answer maps each key
    /// to the owned target activation runs.
    #[test]
    fn command_row_keys_are_distinct_and_answer_their_targets() {
        let ids = [
            CommandRowId::Command(file_explorer_command()),
            CommandRowId::Command(PaletteCommand::StatePreset("okabe-ito")),
            CommandRowId::Focus(PaletteFocusTarget {
                context_id: 7,
                window_id: 70,
                pane_id: Some(42),
            }),
            CommandRowId::Focus(PaletteFocusTarget {
                context_id: 7,
                window_id: 70,
                pane_id: None,
            }),
            CommandRowId::Unavailable("parked/work".into()),
            CommandRowId::View("todo".into()),
            CommandRowId::Builtin("assistant"),
            CommandRowId::Note(std::path::PathBuf::from("/tmp/notes/a:b.md")),
            CommandRowId::UserCommand {
                name: "deploy/all".into(),
                scope: crate::cli::UserCommandScope::Global,
            },
            CommandRowId::UpdateReady("0.5.1".into()),
        ];
        let keys: std::collections::HashSet<_> = ids.iter().map(CommandRowId::key).collect();
        assert_eq!(keys.len(), ids.len());

        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        let note = std::path::PathBuf::from("/tmp/notes/a:b.md");
        h.app.palette_notes = vec![note_entry(note.clone())];
        h.app.open_command_palette("");
        h.run_frames(1);
        let answer = h.app.palette_answer.as_ref().expect("an installed answer");
        let rows = h
            .app
            .host
            .picker(PickerId::Palette)
            .expect("palette")
            .rows();
        assert!(rows.iter().all(|row| answer.targets.contains_key(&row.key)));
        assert_eq!(
            answer.targets.get(&CommandRowId::Note(note.clone()).key()),
            Some(&PaletteTarget::Command(CommandRowId::Note(note)))
        );
    }

    /// Sources run in \`App::logic\`: an edit only starts a new generation,
    /// and a logic-only pass (a hidden window skips \`ui\`) answers it.
    #[test]
    fn palette_query_is_answered_in_logic_not_on_the_draw_path() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.open_command_palette("");
        h.run_frames(1);
        h.app.set_palette_query("keyboard");
        let picker = |h: &crate::testing::HostHarness| {
            let picker = h.app.host.picker(PickerId::Palette).expect("palette");
            (
                picker.is_loading(),
                picker.rows().first().map(|row| row.title.clone()),
            )
        };
        assert!(picker(&h).0, "the edit ran no source on the draw path");
        h.hidden_frame();
        assert_eq!(
            picker(&h),
            (false, Some("Keyboard shortcuts".to_string())),
            "the logic pass answered the edited query"
        );
    }

    fn note_entry(path: std::path::PathBuf) -> crate::notes::NotePickerEntry {
        crate::notes::NotePickerEntry {
            title: path.display().to_string(),
            path,
            preview: String::new(),
            search_text: String::new(),
            tier_label: None,
            modified: None,
        }
    }

    /// Round 1 and 2: a note key is the native path, never its display.
    /// `n\xff.md` and the literal `n\u{FFFD}.md` display the same and are two
    /// keys. Restoring a `display()`-based key fails this.
    #[cfg(unix)]
    #[test]
    fn note_keys_never_collide_on_display_lossy_paths() {
        use std::os::unix::ffi::OsStrExt;
        let raw = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/notes/n\xff.md"));
        let lossy = std::path::PathBuf::from("/tmp/notes/n\u{FFFD}.md");
        assert_eq!(raw.display().to_string(), lossy.display().to_string());
        assert_ne!(
            CommandRowId::Note(raw).key(),
            CommandRowId::Note(lossy).key()
        );
    }

    #[test]
    fn palette_command_names_are_unique_row_ids() {
        let mut names: Vec<_> = PALETTE_COMMANDS.iter().map(|entry| entry.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PALETTE_COMMANDS.len());
    }

    #[test]
    fn shell_single_quote_escapes_embedded_quotes() {
        assert_eq!(shell_single_quote("test"), "'test'");
        assert_eq!(shell_single_quote("my command"), "'my command'");
        assert_eq!(shell_single_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn palette_pips_include_the_focused_pane() {
        let win = test_window(1, 1, 0, 0, &[10, 20, 30], 1);
        let context_active_window = std::collections::HashMap::from([(1, 1)]);
        let windows = vec![win];
        let pips = palette_pips_for_context(&windows, &context_active_window, 1, None);

        assert_eq!(
            pips,
            Some(ListRowPips {
                count: 3,
                focused_idx: Some(1),
                activities: vec![None, None, None],
                static_health_pips: Vec::new(),
            })
        );
    }

    #[test]
    fn palette_pips_cover_every_window_in_the_context() {
        let first = test_window(1, 1, 0, 0, &[10, 20, 30], 0);
        let second = test_window(1, 2, 1, 0, &[40, 50, 60], 2);
        let context_active_window = std::collections::HashMap::from([(1, 2)]);
        let windows = vec![first, second];
        let pips = palette_pips_for_context(&windows, &context_active_window, 1, None);

        assert_eq!(
            pips,
            Some(ListRowPips {
                count: 6,
                focused_idx: Some(5),
                activities: vec![None, None, None, None, None, None],
                static_health_pips: Vec::new(),
            })
        );
    }

    fn test_window(
        context_id: u64,
        window_id: u64,
        grid_x: u32,
        grid_y: u32,
        panes_spec: &[u64],
        focused_idx: usize,
    ) -> crate::host::context::Window {
        let mut panes = std::collections::HashMap::new();
        let mut tiles = egui_tiles::Tiles::default();
        let mut tile_ids = Vec::new();
        for &pane_id in panes_spec {
            panes.insert(pane_id, test_pane(pane_id));
            tile_ids.push(tiles.insert_pane(pane_id));
        }
        let focused_pane = tile_ids.get(focused_idx).copied();
        let root = match tile_ids.as_slice() {
            [only] => *only,
            _ => tiles.insert_horizontal_tile(tile_ids),
        };
        let tree = egui_tiles::Tree::new("test", root, tiles);
        crate::host::context::Window {
            name: "test".to_string(),
            path: std::path::PathBuf::from("/tmp"),
            tree,
            panes: panes.into(),
            focused_pane,
            zoomed_pane: None,
            grid_x,
            grid_y,
            window_id,
            context_id,
            tree_history: Default::default(),
        }
    }

    /// An view pane carrying hook-reported agent state.
    fn test_agent_pane(
        id: u64,
        pane_name: &str,
        agent: &str,
        state: crate::protocol::AgentState,
        detail: Option<&str>,
    ) -> Pane {
        use crate::app::permissions::PluginPermissions;
        use crate::host::pane::{ViewPane, ViewRuntime};

        Pane::View(Box::new(ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id,
            runtime: ViewRuntime::Builtin(Box::new(crate::file_browser::FileBrowserView::new(
                std::env::temp_dir(),
            ))),
            workspace_root: std::env::temp_dir(),
            permissions: PluginPermissions::builtin(),
            manifest_id: "terminal".to_string(),
            name: pane_name.to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: Some(crate::protocol::PaneAgentState {
                pane_id: id,
                state,
                agent: agent.to_string(),
                detail: detail.map(str::to_string),
                session_id: None,
            }),
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        }))
    }

    /// A window built from explicit panes, so a test can mix agent-bearing and
    /// plain panes in one tree.
    fn window_of(
        context_id: u64,
        window_id: u64,
        panes_spec: Vec<(u64, Pane)>,
        focused_idx: usize,
    ) -> crate::host::context::Window {
        let mut panes = std::collections::HashMap::new();
        let mut tiles = egui_tiles::Tiles::default();
        let mut tile_ids = Vec::new();
        for (pane_id, pane) in panes_spec {
            panes.insert(pane_id, pane);
            tile_ids.push(tiles.insert_pane(pane_id));
        }
        let focused_pane = tile_ids.get(focused_idx).copied();
        let root = match tile_ids.as_slice() {
            [only] => *only,
            _ => tiles.insert_horizontal_tile(tile_ids),
        };
        crate::host::context::Window {
            name: "test".to_string(),
            path: std::path::PathBuf::from("/tmp"),
            tree: egui_tiles::Tree::new("test", root, tiles),
            panes: panes.into(),
            focused_pane,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id,
            context_id,
            tree_history: Default::default(),
        }
    }

    #[test]
    fn agent_jump_switches_window_context_and_focused_pane_together() {
        use crate::protocol::AgentState;
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let target = window_of(
            7,
            70,
            vec![(
                10,
                test_agent_pane(10, "claude", "claude-code", AgentState::Working, None),
            )],
            0,
        );
        h.app.windows.push(target);
        h.app.router.push(crate::host::context::Context {
            name: "squad-alpha".to_string().into(),
            root: std::env::temp_dir(),
            description: None,
            context_id: 7,
            parent_id: None,
            depth: 0,
            parked: true,
        });

        h.app.focus_palette_target(PaletteFocusTarget {
            context_id: 7,
            window_id: 70,
            pane_id: Some(10),
        });

        assert_eq!(h.app.active_window, 1);
        assert_eq!(h.app.windows[h.app.active_window].context_id, 7);
        assert_eq!(h.app.router.active().context_id, 7);
        let focused = h.app.windows[1]
            .focused_pane
            .and_then(|tile| h.app.windows[1].tree.tiles.get(tile));
        assert!(matches!(focused, Some(egui_tiles::Tile::Pane(10))));
        assert_eq!(h.app.context_active_window.get(&7), Some(&70));
        assert!(
            !h.app.router.get(1).parked,
            "agent selection restores parked work"
        );
    }

    #[test]
    fn palette_jump_unparks_and_focuses_a_parked_context_pane() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let target = test_window(7, 70, 1, 0, &[10, 11], 0);
        h.app.windows.push(target);
        h.app.router.push(crate::host::context::Context {
            name: "parked-alpha".to_string().into(),
            root: std::env::temp_dir(),
            description: None,
            context_id: 7,
            parent_id: None,
            depth: 0,
            parked: true,
        });

        h.app.focus_palette_target(PaletteFocusTarget {
            context_id: 7,
            window_id: 70,
            pane_id: Some(11),
        });

        assert!(
            !h.app.router.get(1).parked,
            "selection restores the context"
        );
        assert_eq!(h.app.router.active().context_id, 7);
        assert_eq!(h.app.active_window, 1);
        let focused = h.app.windows[1]
            .focused_pane
            .and_then(|tile| h.app.windows[1].tree.tiles.get(tile));
        assert!(matches!(focused, Some(egui_tiles::Tile::Pane(11))));
    }

    /// Stint 1230: focusing a palette row for a pane in an inactive tab
    /// activates that tab, so the tab bar shows the focused pane.
    #[test]
    fn palette_focus_target_activates_the_target_tab() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let mut target = test_window(7, 70, 1, 0, &[10, 11], 0);
        let old_root = target.tree.root.expect("test window root");
        target.tree.tiles.remove(old_root);
        let ten = target.tree.tiles.find_pane(&10).expect("pane 10 tile");
        let eleven = target.tree.tiles.find_pane(&11).expect("pane 11 tile");
        let tabs = target.tree.tiles.insert_tab_tile(vec![ten, eleven]);
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(container))) =
            target.tree.tiles.get_mut(tabs)
        {
            container.set_active(ten);
        }
        target.tree.root = Some(tabs);
        target.focused_pane = Some(ten);
        h.app.windows.push(target);
        h.app.router.push(crate::host::context::Context {
            name: "tabbed".to_string().into(),
            root: crate::testing::scratch_context_root("palette-tabbed"),
            description: None,
            context_id: 7,
            parent_id: None,
            depth: 0,
            parked: false,
        });

        h.app.focus_palette_target(PaletteFocusTarget {
            context_id: 7,
            window_id: 70,
            pane_id: Some(11),
        });

        let window = &h.app.windows[1];
        assert_eq!(window.focused_pane, Some(eleven));
        let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(container))) =
            window.tree.tiles.get(tabs)
        else {
            panic!("tab group survives the focus change");
        };
        assert_eq!(
            container.active,
            Some(eleven),
            "the tab bar must show the pane the palette focused"
        );
    }

    #[test]
    fn every_registered_palette_host_command_reaches_shared_dispatcher() {
        let mut h = crate::testing::HostHarness::new();
        let expected: Vec<_> = PALETTE_COMMANDS
            .iter()
            .filter_map(|entry| {
                if let PaletteCommand::Host(action) = &entry.command {
                    Some(action.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(!expected.is_empty());
        crate::app::HOST_ACTION_PROBE.with(|probe| *probe.borrow_mut() = Some(Vec::new()));
        for action in &expected {
            h.app
                .run_palette_command(PaletteCommand::Host(action.clone()));
        }
        let actual = crate::app::HOST_ACTION_PROBE.with(|probe| probe.borrow_mut().take().unwrap());
        assert_eq!(actual, expected);
    }

    fn scale_shortcut(h: &mut crate::testing::HostHarness, key: egui::Key) {
        h.press_key(key, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    }

    fn palette_action(h: &mut crate::testing::HostHarness, action: crate::host::keys::Action) {
        h.app.run_palette_command(PaletteCommand::Host(action));
    }

    #[test]
    fn palette_reset_ui_scale_matches_shortcut() {
        use crate::host::keys::Action;
        let mut h = crate::testing::HostHarness::new();
        scale_shortcut(&mut h, egui::Key::Equals);
        assert!(h.app.ui_scale > 1.0);
        palette_action(&mut h, Action::UiScaleReset);
        assert_eq!(h.app.ui_scale, 1.0);
        palette_action(&mut h, Action::UiScaleUp);
        assert!(h.app.ui_scale > 1.0);
        scale_shortcut(&mut h, egui::Key::Num0);
        assert_eq!(h.app.ui_scale, 1.0);
        palette_action(&mut h, Action::UiScaleUp);
        palette_action(&mut h, Action::UiScaleReset);
        assert_eq!(h.app.ui_scale, 1.0);
    }

    #[test]
    fn palette_increase_ui_scale_matches_shortcut() {
        use crate::host::keys::Action;
        let mut h = crate::testing::HostHarness::new();
        scale_shortcut(&mut h, egui::Key::Equals);
        let expected = h.app.ui_scale;
        scale_shortcut(&mut h, egui::Key::Num0);
        palette_action(&mut h, Action::UiScaleUp);
        assert_eq!(h.app.ui_scale, expected);
    }

    #[test]
    fn palette_decrease_ui_scale_matches_shortcut() {
        use crate::host::keys::Action;
        let mut h = crate::testing::HostHarness::new();
        scale_shortcut(&mut h, egui::Key::Minus);
        let expected = h.app.ui_scale;
        scale_shortcut(&mut h, egui::Key::Num0);
        palette_action(&mut h, Action::UiScaleDown);
        assert_eq!(h.app.ui_scale, expected);
    }

    #[test]
    fn palette_text_size_commands_match_shortcuts() {
        use crate::host::keys::Action;
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_test_pane();
        h.focus_pane(pane).run_frames(2);
        fn font(h: &crate::testing::HostHarness, pane: u64) -> f32 {
            let crate::host::pane::Pane::View(app) = &h.app.windows[0].panes[&pane] else {
                panic!("expected view pane")
            };
            let crate::host::pane::ViewRuntime::Builtin(b) = &app.runtime else {
                panic!("expected builtin")
            };
            b.font_size().unwrap()
        }
        let initial = font(&h, pane);
        h.press_key(egui::Key::Equals, egui::Modifiers::COMMAND);
        let increased = font(&h, pane);
        assert_eq!(increased, initial + 1.0);
        palette_action(&mut h, Action::ResetPaneFontSize);
        assert_eq!(font(&h, pane), initial);
        palette_action(&mut h, Action::IncreasePaneFontSize);
        assert_eq!(font(&h, pane), increased);
        h.press_key(egui::Key::Minus, egui::Modifiers::COMMAND);
        let decreased = font(&h, pane);
        palette_action(&mut h, Action::DecreasePaneFontSize);
        assert_eq!(font(&h, pane), decreased - 1.0);
        h.press_key(
            egui::Key::Num0,
            egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        );
        assert_eq!(font(&h, pane), initial);
    }

    /// Open the palette, type `query`, and press Enter through the real
    /// frame path so the open-palette selection code runs end to end.
    fn run_open_palette(h: &mut crate::testing::HostHarness, query: &str) {
        h.app.open_command_palette(query);
        h.run_frames(3);
        assert!(
            h.app.command_palette_open(),
            "palette must stay open for {query:?}"
        );
        h.press_key(egui::Key::Enter, egui::Modifiers::NONE);
    }

    #[test]
    fn open_palette_reset_ui_scale_row_restores_scale() {
        let mut h = crate::testing::HostHarness::new();
        scale_shortcut(&mut h, egui::Key::Equals);
        scale_shortcut(&mut h, egui::Key::Equals);
        assert!(h.app.ui_scale > 1.0);
        h.app.ui_scale_persist_at = None;
        run_open_palette(&mut h, "reset ui scale");
        assert_eq!(h.app.ui_scale, 1.0);
        assert!(!h.app.command_palette_open());
        assert!(
            h.app.ui_scale_persist_at.is_some(),
            "palette scale change must schedule persistence like the shortcut"
        );
    }

    #[test]
    fn open_palette_reset_text_size_row_restores_focused_pane_font() {
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_test_pane();
        h.focus_pane(pane).run_frames(2);
        let font = |h: &crate::testing::HostHarness| {
            let crate::host::pane::Pane::View(app) = &h.app.windows[0].panes[&pane] else {
                panic!("expected view pane")
            };
            let crate::host::pane::ViewRuntime::Builtin(b) = &app.runtime else {
                panic!("expected builtin")
            };
            b.font_size().unwrap()
        };
        let initial = font(&h);
        h.press_key(egui::Key::Equals, egui::Modifiers::COMMAND);
        h.press_key(egui::Key::Equals, egui::Modifiers::COMMAND);
        assert_eq!(font(&h), initial + 2.0);
        run_open_palette(&mut h, "reset text size");
        assert_eq!(font(&h), initial);
        assert!(!h.app.command_palette_open());
    }

    #[test]
    fn palette_ui_scale_up_clamps_at_maximum_and_schedules_persist() {
        use crate::host::keys::Action;
        let mut h = crate::testing::HostHarness::new();
        for _ in 0..20 {
            palette_action(&mut h, Action::UiScaleUp);
        }
        assert_eq!(h.app.ui_scale, crate::app::ui_scale::UI_SCALE_MAX);
        assert!(h.app.ui_scale_persist_at.is_some());
        palette_action(&mut h, Action::UiScaleUp);
        assert_eq!(h.app.ui_scale, 2.0);
    }

    #[test]
    fn palette_split_on_terminal_pane_adds_a_terminal() {
        use crate::host::keys::Action;
        for action in [Action::SplitRight, Action::SplitDown] {
            let mut h = crate::testing::HostHarness::new();
            h.add_focused_terminal();
            let before = h.pane_count();
            palette_action(&mut h, action.clone());
            assert_eq!(h.pane_count(), before + 1, "{action:?}");
        }
    }

    #[test]
    fn palette_split_on_view_pane_mirrors_the_view_not_a_terminal() {
        use crate::host::keys::Action;
        for action in [Action::SplitRight, Action::SplitDown] {
            let mut h = crate::testing::HostHarness::new();
            let pane = h.add_test_pane();
            h.focus_pane(pane).run_frames(2);
            palette_action(&mut h, action.clone());
            let terminals = h.app.windows[0]
                .panes
                .values()
                .filter(|p| matches!(p, crate::host::pane::Pane::Terminal(_)))
                .count();
            assert_eq!(
                terminals, 0,
                "{action:?} on a view pane must mirror the view (shortcut behavior), never open a terminal"
            );
        }
    }

    #[test]
    fn palette_review_notifications_toggles_like_the_shortcut() {
        use crate::host::keys::Action;
        let mut h = crate::testing::HostHarness::new();
        assert!(!h.app.attention_queue.open);
        palette_action(&mut h, Action::ToggleAttentionQueue);
        assert!(h.app.attention_queue.open);
        palette_action(&mut h, Action::ToggleAttentionQueue);
        assert!(!h.app.attention_queue.open);
        h.press_key(egui::Key::I, egui::Modifiers::COMMAND);
        assert!(h.app.attention_queue.open);
        palette_action(&mut h, Action::ToggleAttentionQueue);
        assert!(!h.app.attention_queue.open);
    }

    /// Stint 1458: "Close selected panes" is listed only while a selection
    /// exists and closes every selected pane.
    #[test]
    fn palette_close_selected_panes_is_listed_with_a_selection_and_closes_it() {
        let mut h = crate::testing::HostHarness::new();
        let a = h.add_test_pane();
        let b = h.add_test_pane();
        let c = h.add_test_pane();
        let context = h.app.router.active().context_id;
        h.app
            .set_context_columns(context, vec![vec![a], vec![b], vec![c]]);
        h.focus_pane(a).run_frames(2);
        let listed = |h: &mut crate::testing::HostHarness| {
            h.app
                .palette_command_listed(&PaletteCommand::CloseSelectedPanes)
        };
        assert!(!listed(&mut h), "hidden without a selection");
        h.app
            .run_palette_command(PaletteCommand::CloseSelectedPanes);
        h.run_frames(2);
        assert_eq!(
            h.app.windows[h.app.active_window].panes.len(),
            3,
            "with no selection the row closes nothing, not the focused pane"
        );
        let window = h.app.windows[h.app.active_window].window_id;
        h.app.host.extend_pane_selection(window, a, a);
        assert!(
            !listed(&mut h),
            "a selection of one is the focused pane, not listed"
        );
        h.app.host.clear_pane_selection(window);
        h.app.host.extend_pane_selection(window, a, b);
        h.focus_pane(b);
        assert!(listed(&mut h), "listed with a selection");
        h.app
            .run_palette_command(PaletteCommand::CloseSelectedPanes);
        h.run_frames(2);
        let open = &h.app.windows[h.app.active_window].panes;
        assert!(!open.contains_key(&a) && !open.contains_key(&b));
        assert!(open.contains_key(&c));
    }

    #[test]
    fn palette_close_pane_runs_the_shortcut_ladder_first_step() {
        use crate::host::keys::Action;
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_test_pane();
        h.focus_pane(pane).run_frames(2);
        let before = h.pane_count();
        h.app.attention_queue.open = true;
        h.app.sync_attention_queue_layer();
        palette_action(&mut h, Action::ClosePane);
        assert!(
            !h.app.attention_queue.open,
            "first ladder step closes the open attention queue"
        );
        assert_eq!(h.pane_count(), before, "the pane survives the first step");
        h.app.show_shortcuts = true;
        palette_action(&mut h, Action::ClosePane);
        assert!(!h.app.show_shortcuts);
        assert_eq!(h.pane_count(), before);
    }

    #[test]
    fn palette_go_to_parent_context_closes_open_surface_first() {
        use crate::host::keys::Action;
        let mut h = crate::testing::HostHarness::new();
        h.app.attention_queue.open = true;
        h.app.sync_attention_queue_layer();
        palette_action(&mut h, Action::ContextZoomOut);
        assert!(!h.app.attention_queue.open);
    }

    #[test]
    fn palette_close_context_refuses_the_only_context() {
        use crate::host::keys::Action;
        let mut h = crate::testing::HostHarness::new();
        let contexts = h.app.router.len();
        assert_eq!(contexts, 1);
        palette_action(&mut h, Action::CloseContext);
        assert_eq!(h.app.router.len(), contexts);
        assert!(h.app.pending_context_close.is_none());
    }

    #[test]
    fn palette_jump_restores_the_last_parked_context() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let idx = h.app.router.active_idx();
        let window_id = h.app.windows[h.app.active_window].window_id;
        h.app.router.get_mut(idx).parked = true;

        let context_id = h.app.router.active().context_id;
        h.app.focus_palette_target(PaletteFocusTarget {
            context_id,
            window_id,
            pane_id: None,
        });

        assert!(!h.app.router.get(idx).parked);
        assert_eq!(h.app.router.active_idx(), idx);
        assert_eq!(h.app.active_window, 0);
    }

    #[test]
    fn palette_jump_restores_the_selected_context_when_all_are_parked() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let first_idx = h.app.router.active_idx();
        h.app.router.get_mut(first_idx).parked = true;
        h.app.windows.push(test_window(7, 70, 1, 0, &[10], 0));
        h.app.router.push(crate::host::context::Context {
            name: "second-parked".to_string().into(),
            root: std::env::temp_dir(),
            description: None,
            context_id: 7,
            parent_id: None,
            depth: 0,
            parked: true,
        });

        h.app.focus_palette_target(PaletteFocusTarget {
            context_id: 7,
            window_id: 70,
            pane_id: Some(10),
        });

        assert!(h.app.router.get(0).parked, "other parked work stays parked");
        assert!(!h.app.router.get(1).parked, "selected work is restored");
        assert_eq!(h.app.router.active().context_id, 7);
        assert_eq!(h.app.active_window, 1);
    }

    #[test]
    fn stale_agent_jump_leaves_focus_unchanged() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let active_window = h.app.active_window;
        let active_context = h.app.router.active().context_id;

        h.app.focus_palette_target(PaletteFocusTarget {
            context_id: 999,
            window_id: 999,
            pane_id: Some(123),
        });

        assert_eq!(h.app.active_window, active_window);
        assert_eq!(h.app.router.active().context_id, active_context);
    }

    fn test_pane(id: u64) -> Pane {
        Pane::Portal(Box::new(PortalPane {
            pane_id: id,
            target_context_id: id + 1000,
            context_state: None,
        }))
    }

    #[test]
    fn pane_rows_unpack_each_pane_with_a_single_pip() {
        let win = test_window(1, 1, 0, 0, &[10, 20, 30], 1);
        let context_active_window = std::collections::HashMap::from([(1, 1)]);
        let windows = vec![win];
        let names = vec![(1010, "child-a".to_string())];

        let rows = palette_pane_rows_for_context(
            &windows,
            &context_active_window,
            &names,
            1,
            999,
            None,
            &[],
        );

        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert_eq!(row.pips.count, 1);
            assert_eq!(row.chip, "ctx"); // test panes are portals
        }
        // Portal target 1010 resolves to its context name; unknown targets fall back.
        assert_eq!(rows[0].name, "child-a");
        assert_eq!(rows[1].name, "sub-context");
        // Focused pane (idx 1) carries the focus pip.
        assert_eq!(rows[0].pips.focused_idx, None);
        assert_eq!(rows[1].pips.focused_idx, Some(0));
    }

    #[test]
    fn pane_rows_cover_every_window_in_the_context() {
        // Regression: panes must unpack from ALL windows of a context, not
        // just the first one.
        let first = test_window(1, 1, 0, 0, &[10], 0);
        let second = test_window(1, 2, 1, 0, &[40, 50], 1);
        let context_active_window = std::collections::HashMap::from([(1, 2)]);
        let windows = vec![first, second];

        let rows =
            palette_pane_rows_for_context(&windows, &context_active_window, &[], 1, 999, None, &[]);

        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter().map(|r| r.window_id).collect::<Vec<_>>(),
            vec![1, 2, 2]
        );
        assert_eq!(
            rows.iter().map(|r| r.pane_id).collect::<Vec<_>>(),
            vec![10, 40, 50]
        );
        // Focus follows the context's active window (window 2, pane 50).
        assert_eq!(rows[2].pips.focused_idx, Some(0));
        assert_eq!(rows[0].pips.focused_idx, None);
    }

    #[test]
    fn pane_rows_sort_current_focus_then_reverse_history_before_spatial_fallback() {
        let win = test_window(1, 1, 0, 0, &[10, 20, 30], 0);
        let current_tile = win.focused_pane.expect("focused tile");
        let older_tile = win.tree.tiles.find_pane(&20).expect("older tile");
        let newer_tile = win.tree.tiles.find_pane(&30).expect("newer tile");
        let context_active_window = std::collections::HashMap::from([(1, 1)]);
        let windows = vec![win];

        let rows = palette_pane_rows_for_context(
            &windows,
            &context_active_window,
            &[],
            1,
            1,
            Some(current_tile),
            &[(1, older_tile), (1, newer_tile)],
        );

        assert_eq!(
            rows.iter().map(|r| r.pane_id).collect::<Vec<_>>(),
            vec![10, 30, 20]
        );
    }

    #[test]
    fn pane_row_identity_chips_apps_and_text_editors() {
        use crate::app::permissions::PluginPermissions;
        use crate::host::pane::{ViewPane, ViewRuntime};

        let make_app = |manifest_id: &str, name: &str| {
            Pane::View(Box::new(ViewPane {
                incarnation: Default::default(),
                pip_status: None,
                id: 99,
                runtime: ViewRuntime::Builtin(Box::new(crate::file_browser::FileBrowserView::new(
                    std::env::temp_dir(),
                ))),
                workspace_root: std::env::temp_dir(),
                permissions: PluginPermissions::builtin(),
                manifest_id: manifest_id.to_string(),
                name: name.to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            }))
        };

        let (name, chip) = pane_row_identity(&make_app("notes", "My Notes"), &[]);
        assert_eq!((name.as_str(), chip), ("My Notes", "view"));

        let (name, chip) = pane_row_identity(&make_app("text-editor", "todo.md"), &[]);
        assert_eq!((name.as_str(), chip), ("todo.md", "text"));

        // Empty pane name falls back to the runtime display name.
        let (name, _) = pane_row_identity(&make_app("notes", ""), &[]);
        assert!(!name.is_empty());

        // User-renamed view pane uses the assigned name in palette search.
        let (name, chip) = pane_row_identity(&make_app("assistant", "chad"), &[]);
        assert_eq!((name.as_str(), chip), ("chad", "view"));
        let search = searchable_text(&[name.as_str(), "work"]);
        assert!(
            search.contains("chad"),
            "renamed pane title must appear in palette search_text; got: {search:?}"
        );
    }

    #[test]
    fn palette_global_state_preset_save_reports_only_its_workspace_override() {
        let mut h = crate::testing::HostHarness::new();
        let root = tempfile::tempdir().unwrap();
        h.app.set_context_root(root.path().to_path_buf(), None);
        std::fs::write(
            crate::config::config_path(),
            "[theme]\nstate_preset = \"mono\"\n",
        )
        .unwrap();
        let path = crate::config::workspace_config_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[theme]\nstate_preset = \"mono\"\n").unwrap();
        h.app
            .run_palette_command(PaletteCommand::StatePreset("mono"));
        let notice = &h
            .app
            .config_notice
            .as_ref()
            .expect("workspace key shadows global save")
            .0;
        assert!(
            notice.contains("Saved globally")
                && notice.contains("theme.state_preset")
                && notice.contains("mono"),
            "{notice}"
        );
        std::fs::write(path, "[theme]\npreset = \"plexi-night\"\n").unwrap();
        h.app
            .run_palette_command(PaletteCommand::StatePreset("default"));
        assert!(h.app.config_notice.is_none());
    }

    #[test]
    fn palette_reload_config_workspace_theme_shadows_global_and_reports_it() {
        let mut h = crate::testing::HostHarness::new();
        let root = tempfile::tempdir().unwrap();
        h.app.set_context_root(root.path().to_path_buf(), None);
        std::fs::write(
            crate::config::config_path(),
            "[theme]\npreset = \"plexi-dusk\"\n",
        )
        .unwrap();
        let path = crate::config::workspace_config_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "[theme]\npreset = \"plexi-night\"\n").unwrap();
        h.app.run_palette_command(PaletteCommand::Host(
            crate::host::keys::Action::ReloadConfig,
        ));
        let notice = &h
            .app
            .config_notice
            .as_ref()
            .expect("reload must report shadow")
            .0;
        assert!(
            notice.contains("theme.preset") && notice.contains("plexi-night"),
            "{notice}"
        );
        // An omitted global preset still has an effective default.
        std::fs::write(crate::config::config_path(), "").unwrap();
        h.app.config_notice = None;
        h.app.run_palette_command(PaletteCommand::Host(
            crate::host::keys::Action::ReloadConfig,
        ));
        assert!(
            h.app
                .config_notice
                .as_ref()
                .is_some_and(|(text, _)| text.contains("theme.preset")),
            "implicit global default must be compared"
        );
    }

    #[test]
    fn reload_reports_workspace_font_override_of_implicit_global_default() {
        let mut h = crate::testing::HostHarness::new();
        let root = tempfile::tempdir().unwrap();
        h.app.set_context_root(root.path().to_path_buf(), None);
        std::fs::write(crate::config::config_path(), "").unwrap();
        let path = crate::config::workspace_config_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "font_size = 19.0\n").unwrap();
        h.app.reload_config_for_workspace(Some(root.path()));
        assert!(h
            .app
            .config_notice
            .as_ref()
            .is_some_and(|(text, _)| text.contains("font_size") && text.contains("19")));
        std::fs::write(path, "font_size = 14.0\n").unwrap();
        h.app.reload_config_for_workspace(Some(root.path()));
        assert!(
            h.app.config_notice.is_none(),
            "equal effective values should not produce a shadow notice"
        );
    }

    #[test]
    fn repo_config_row_opens_the_active_context_config_not_nested_pane_repo() {
        let mut h = crate::testing::HostHarness::new();
        let outer = tempfile::tempdir().unwrap();
        h.app.set_context_root(outer.path().to_path_buf(), None);
        let nested = outer.path().join("nested");
        std::fs::create_dir_all(nested.join(crate::config::workspace_channel_dir())).unwrap();
        let pane = h.add_test_pane();
        h.app.windows[0]
            .panes
            .get_mut(&pane)
            .unwrap()
            .as_view_mut()
            .unwrap()
            .workspace_root = nested.clone();
        h.focus_pane(pane);
        h.press_key(egui::Key::Comma, egui::Modifiers::COMMAND);
        let active_path = crate::config::workspace_config_path(outer.path());
        let nested_path = crate::config::workspace_config_path(&nested);
        assert!(
            !active_path.exists() && !nested_path.exists(),
            "viewing the chooser must not create a file"
        );
        h.app
            .run_palette_command(PaletteCommand::OpenWorkspaceConfig);
        assert!(
            active_path.exists(),
            "the row must open the file used by active-context reload"
        );
        assert!(
            !nested_path.exists(),
            "a nested pane repo must not receive a config the host will not load"
        );
    }

    #[test]
    fn repo_config_row_creates_template_in_plain_repo_without_channel_dir() {
        let mut h = crate::testing::HostHarness::new();
        let root = tempfile::tempdir().unwrap();
        h.app.set_context_root(root.path().to_path_buf(), None);
        let channel_dir = root.path().join(crate::config::workspace_channel_dir());
        let _ = std::fs::remove_dir_all(&channel_dir);
        assert!(!channel_dir.exists());
        assert_eq!(
            h.app.active_config_workspace_root().as_deref(),
            Some(root.path())
        );
        let before = crate::config::PlexiConfig::load_with_workspace(Some(root.path()));
        h.app
            .run_palette_command(PaletteCommand::OpenWorkspaceConfig);
        let path = crate::config::workspace_config_path(root.path());
        assert!(path.exists(), "choosing the row must create the template");
        let after = crate::config::PlexiConfig::load_with_workspace(Some(root.path()));
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
    }

    #[test]
    fn repo_config_row_hidden_when_workspace_path_is_the_global_config() {
        let mut h = crate::testing::HostHarness::new();
        let home = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(
            home.path().join(crate::config::workspace_channel_dir()),
        );
        h.app.set_context_root(home.path().to_path_buf(), None);
        assert_eq!(
            crate::config::workspace_config_path(home.path()),
            crate::config::config_path()
        );
        assert!(h.app.active_config_workspace_root().is_none());
        let global_before = std::fs::read(crate::config::config_path()).ok();
        h.app
            .run_palette_command(PaletteCommand::OpenWorkspaceConfig);
        assert_eq!(
            std::fs::read(crate::config::config_path()).ok(),
            global_before,
            "must not create or rewrite the global file as a repo config"
        );
    }

    #[test]
    fn palette_workspace_config_open_preserves_global_effective_theme() {
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_test_pane();
        h.focus_pane(pane);
        let root = h.app.windows[0]
            .panes
            .get(&pane)
            .unwrap()
            .as_view()
            .unwrap()
            .workspace_root
            .clone();
        std::fs::create_dir_all(root.join(crate::config::workspace_channel_dir())).unwrap();
        std::fs::write(
            crate::config::config_path(),
            "[theme]\npreset = \"plexi-night\"\n",
        )
        .unwrap();
        let before = crate::config::PlexiConfig::load_with_workspace(Some(&root));
        h.app
            .run_palette_command(PaletteCommand::OpenWorkspaceConfig);
        let path = crate::config::workspace_config_path(&root);
        let first = std::fs::read_to_string(&path).unwrap();
        let after = crate::config::PlexiConfig::load_with_workspace(Some(&root));
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        h.app
            .run_palette_command(PaletteCommand::OpenWorkspaceConfig);
        assert_eq!(first, std::fs::read_to_string(path).unwrap());
    }

    #[test]
    fn palette_ui_upcycle_launches_curated_cli_descriptors() {
        let mut h = crate::testing::HostHarness::new();
        for binary in ["git", "cargo", "gh"] {
            let commands = palette_command_matches(&format!("ui upcycle {binary}"));
            assert_eq!(commands.len(), 1, "UI upcycle entry for {binary}");
            h.app.run_palette_command(commands[0].clone());
            assert!(
                h.app
                    .windows
                    .iter()
                    .flat_map(|w| w.panes.values())
                    .any(|pane| { pane.as_view().is_some_and(|app| app.name == binary) }),
                "{binary} renderer opened"
            );
        }
    }

    #[test]
    fn palette_exposes_edit_commands() {
        assert_eq!(
            palette_command_matches("edit commands"),
            vec![PaletteCommand::EditCommands]
        );
        assert_eq!(
            palette_command_matches("commands.toml"),
            vec![PaletteCommand::EditCommands]
        );
    }

    #[test]
    fn palette_edit_commands_creates_template_opens_split_and_hot_reloads_on_save() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let first_pane = h.add_test_pane();
        h.focus_pane(first_pane);
        let ws_root = h.app.windows[0]
            .panes
            .get(&first_pane)
            .and_then(|p| p.as_view())
            .expect("test view pane")
            .workspace_root
            .clone();
        let channel_dir = ws_root.join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&channel_dir).expect("create workspace channel dir");
        let commands_path = channel_dir.join("commands.toml");
        assert!(
            !commands_path.exists(),
            "commands.toml starts absent before Edit commands"
        );

        // Open the palette in this workspace via Cmd+P: no user commands yet.
        h.press_key(egui::Key::P, egui::Modifiers::COMMAND);
        assert!(h.app.command_palette_open());
        assert_eq!(
            h.app.palette_workspace_root.as_deref(),
            Some(ws_root.as_path())
        );
        assert!(
            !h.app.palette_commands.iter().any(|c| c.name == "hello"),
            "hello must not exist before editing commands.toml"
        );

        // Select "Edit commands": creates commands.toml from template and opens a split editor.
        click_row(&mut h, CommandRowId::Command(PaletteCommand::EditCommands));
        assert!(!h.app.command_palette_open());
        assert!(
            commands_path.exists(),
            "Edit commands must create commands.toml from template when absent"
        );
        let template_disk =
            std::fs::read_to_string(&commands_path).expect("read created commands.toml");
        assert_eq!(template_disk, crate::cli::COMMANDS_TEMPLATE);
        assert_eq!(
            h.app.windows[0].panes.len(),
            2,
            "Edit commands must open text-editor in a split pane"
        );

        let editor_pane_id = h
            .app
            .find_open_text_editor_pane_any_window(&commands_path)
            .expect("commands.toml open in text-editor pane");

        // Add `hello = "echo hi"` in the open editor and save with Cmd+S.
        h.text_editor_mut(editor_pane_id)
            .test_set_unsaved("[commands]\nhello = \"echo hi\"\n");
        h.press_key(egui::Key::S, egui::Modifiers::COMMAND);

        let saved_disk = std::fs::read_to_string(&commands_path).expect("read saved commands.toml");
        assert!(
            saved_disk.contains("hello = \"echo hi\""),
            "Cmd+S must persist commands.toml edit to disk"
        );

        // Reopen the palette via Cmd+P: `hello` hot-reloads into `palette_commands`.
        h.press_key(egui::Key::P, egui::Modifiers::COMMAND);
        assert!(h.app.command_palette_open());
        let hello = h
            .app
            .palette_commands
            .iter()
            .find(|c| c.name == "hello")
            .expect("reopened palette must hot-reload hello from saved commands.toml");
        assert_eq!(hello.run.as_deref(), Some("echo hi"));
        assert_eq!(hello.scope, crate::cli::UserCommandScope::Workspace);

        // `plexi run` and `plexi run --json` list it with scope.
        let human =
            crate::cli::format_list_commands_for_root(Some(&ws_root), false).expect("human list");
        assert!(human.contains("hello") && human.contains("[workspace]"));
        let json =
            crate::cli::format_list_commands_for_root(Some(&ws_root), true).expect("json list");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let arr = parsed.as_array().expect("json array");
        assert!(arr.iter().any(|item| {
            item["name"] == "hello" && item["scope"] == "workspace" && item["run"] == "echo hi"
        }));
    }

    #[test]
    fn palette_edit_commands_falls_back_to_global_when_not_in_workspace() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        h.add_test_pane();
        let global_commands = h.profile_dir().join("commands.toml");
        assert!(!global_commands.exists());

        h.app
            .run_palette_selection(CommandRowId::Command(PaletteCommand::EditCommands));

        assert!(
            global_commands.exists(),
            "Edit commands outside a workspace must create global commands.toml from template"
        );
        let disk = std::fs::read_to_string(&global_commands).expect("read global commands.toml");
        assert_eq!(disk, crate::cli::COMMANDS_TEMPLATE);
        assert!(
            h.app
                .find_open_text_editor_pane_any_window(&global_commands)
                .is_some(),
            "global commands.toml must be opened in a text-editor split"
        );
    }

    #[test]
    fn two_step_play_command_opens_two_panes_via_cli_palette_and_bound_key_and_shows_in_pane_list()
    {
        use crate::testing::HostHarness;
        use std::io::BufRead;

        let mut h = HostHarness::new();
        let invoker = h.add_test_pane();
        let initial_count = h.pane_count();
        assert_eq!(initial_count, 1);

        let ws_commands = h
            .workspace_root()
            .join(crate::config::workspace_channel_dir())
            .join("commands.toml");
        std::fs::create_dir_all(ws_commands.parent().unwrap()).unwrap();
        std::fs::write(
            &ws_commands,
            r#"
[commands.play]
description = "Open mixing terminal + editor"
steps = [
  "pane new --right --name mixer -- echo mix",
  "view open text-editor --split down",
]
"#,
        )
        .unwrap();

        let ws_root = h.workspace_root().to_path_buf();
        h.app.palette_workspace_root = Some(ws_root.clone());

        // 1. Trigger `plexi run play` via CLI over a real Unix socket serviced by the harness.
        let sock_dir = tempfile::tempdir().unwrap();
        let sock_path = sock_dir.path().join("notify.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let _sg = crate::cli::test_env::socket_env_guard();
        crate::cli::set_command_socket_override(sock_path.clone());

        let ws_for_cli = ws_root.clone();
        let cli_handle = std::thread::spawn(move || {
            crate::cli::run_command_for_root_from_pane(
                Some(&ws_for_cli),
                &ws_for_cli,
                "play",
                &[],
                Some(invoker),
            )
        });

        while !cli_handle.is_finished() {
            if let Ok((stream, _)) = listener.accept() {
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() && !line.trim().is_empty() {
                    if let Ok(req) =
                        serde_json::from_str::<crate::protocol::HostRequest>(line.trim())
                    {
                        // Classify the peer the way the socket entry does: the
                        // CLI runs on a thread of this test process.
                        let origin = h.app.socket_origin_for_pids(&[std::process::id()]);
                        h.app.handle_pane_ipc_request(origin, req);
                    }
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let report = cli_handle.join().unwrap();
        assert_eq!(report.exit_code, 0, "plexi run play must exit 0");
        assert_eq!(
            h.pane_count(),
            initial_count + 2,
            "two-step command must open two panes in one call via plexi run play"
        );

        // Verify `plexi pane list --json` (`ListPanes`) shows both new panes.
        let list_file = sock_dir.path().join("pane-list.json");
        h.app
            .handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostSystem, crate::protocol::HostRequest::ListPanes {
                response_file: list_file.to_string_lossy().into_owned(),
                context_id: None,
            });
        let list_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&list_file).unwrap()).unwrap();
        let panes_arr = list_json.as_array().expect("pane list JSON array");
        assert_eq!(panes_arr.len(), 3, "pane list --json must show all 3 panes");
        assert!(
            panes_arr
                .iter()
                .any(|p| p["type"] == "terminal" && p["title"] == "mixer"),
            "pane list --json must include the mixer terminal pane: {panes_arr:?}"
        );
        assert!(
            panes_arr.iter().any(|p| p["type"] == "view"
                && p["title"].as_str().is_some_and(|t| t.starts_with("note-"))),
            "pane list --json must include the text-editor view pane: {panes_arr:?}"
        );

        // 2. Trigger `play` from the Command Palette and verify it also opens two panes.
        let before_palette = h.pane_count();
        h.app.palette_commands = crate::cli::resolve_user_commands(Some(&ws_root));
        h.app.run_palette_selection(CommandRowId::UserCommand {
            name: "play".to_string(),
            scope: crate::cli::UserCommandScope::Workspace,
        });
        h.hidden_frame();
        assert_eq!(
            h.pane_count(),
            before_palette + 2,
            "selecting play in the command palette must open two panes"
        );

        // 3. Bind `[keys] "cmd+shift+p" = "run:play"` in config.toml and press Cmd+Shift+P.
        let cfg_path = h.profile_dir().join("config.toml");
        std::fs::write(
            &cfg_path,
            r#"
config_version = 1

[keys]
"cmd+shift+p" = "run:play"
"#,
        )
        .unwrap();
        h.app.reload_config();
        let before_key = h.pane_count();
        h.press_key(
            egui::Key::P,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        );
        h.hidden_frame();
        assert_eq!(
            h.pane_count(),
            before_key + 2,
            "pressing bound key cmd+shift+p (run:play) must trigger the two-step command"
        );
    }

    #[test]
    fn pane_new_on_load_reuses_step_runner() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        h.add_test_pane();
        let ws_commands = h
            .workspace_root()
            .join(crate::config::workspace_channel_dir())
            .join("commands.toml");
        std::fs::create_dir_all(ws_commands.parent().unwrap()).unwrap();
        std::fs::write(
            &ws_commands,
            r#"
[commands.boot]
steps = [
  "pane new --right --on-load 'pane name ReadyPane'",
]
"#,
        )
        .unwrap();

        let ws_root = h.workspace_root().to_path_buf();
        h.app.palette_workspace_root = Some(ws_root);
        h.app.queue_user_command("boot", None);
        h.hidden_frame();

        let list_dir = tempfile::tempdir().unwrap();
        let list_file = list_dir.path().join("pane-list-onload.json");
        h.app
            .handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostSystem, crate::protocol::HostRequest::ListPanes {
                response_file: list_file.to_string_lossy().into_owned(),
                context_id: None,
            });
        let list_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&list_file).unwrap()).unwrap();
        let panes_arr = list_json.as_array().unwrap();
        assert!(
            panes_arr.iter().any(|p| p["title"] == "ReadyPane"),
            "pane new --on-load must run through the step runner and rename the new pane: {panes_arr:?}"
        );
    }

    #[test]
    fn edit_commands_shows_scope_and_path_switches_scope_in_place_and_saves_reloads() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let first_pane = h.add_test_pane();
        h.focus_pane(first_pane);
        let ws_root = h.workspace_root().to_path_buf();
        let ws_channel_dir = ws_root.join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&ws_channel_dir).unwrap();
        let ws_commands_path = ws_channel_dir.join("commands.toml");
        let global_commands_path = h.profile_dir().join("commands.toml");

        // Open Edit commands in a workspace: opens workspace commands.toml with `[workspace]` scope.
        h.press_key(egui::Key::P, egui::Modifiers::COMMAND);
        click_row(&mut h, CommandRowId::Command(PaletteCommand::EditCommands));
        assert_eq!(h.pane_count(), 2);

        let editor_pane_id = h
            .app
            .find_open_text_editor_pane_any_window(&ws_commands_path)
            .expect("workspace commands.toml open in editor");
        assert_eq!(
            h.text_editor_mut(editor_pane_id).commands_scope(),
            Some(crate::cli::UserCommandScope::Workspace)
        );
        assert_eq!(
            h.app.windows[0].panes[&editor_pane_id]
                .as_view()
                .unwrap()
                .name,
            "commands.toml [workspace]"
        );

        // Switch scope via `PaletteCommand::SwitchCommandsScope`: switches in-place to global
        // commands.toml without opening a second editor pane.
        h.focus_pane(editor_pane_id);
        h.app
            .run_palette_selection(CommandRowId::Command(PaletteCommand::SwitchCommandsScope));
        assert_eq!(
            h.pane_count(),
            2,
            "switching commands scope must replace the buffer in-place, not open a third pane"
        );
        assert!(
            global_commands_path.exists(),
            "switching to global scope must ensure global commands.toml exists"
        );
        assert_eq!(
            h.text_editor_mut(editor_pane_id).commands_scope(),
            Some(crate::cli::UserCommandScope::Global)
        );
        assert_eq!(
            h.app.windows[0].panes[&editor_pane_id]
                .as_view()
                .unwrap()
                .name,
            "commands.toml [global]"
        );

        // Save an edit to global commands.toml via Cmd+S: `take_config_saved()` triggers `reload_config()`
        // and the new global command is immediately available.
        h.text_editor_mut(editor_pane_id)
            .test_set_unsaved("[commands]\nglobal_hi = \"echo from_global\"\n");
        h.press_key(egui::Key::S, egui::Modifiers::COMMAND);
        h.run_frames(1);

        h.press_key(egui::Key::P, egui::Modifiers::COMMAND);
        let global_cmd = h
            .app
            .palette_commands
            .iter()
            .find(|c| c.name == "global_hi")
            .expect("saved global commands.toml must immediately populate palette_commands");
        assert_eq!(global_cmd.scope, crate::cli::UserCommandScope::Global);
        assert_eq!(global_cmd.run.as_deref(), Some("echo from_global"));
        h.app.close_command_palette();

        // Switch back to workspace scope in-place.
        assert!(h
            .text_editor_mut(editor_pane_id)
            .switch_commands_scope_in_place());
        assert_eq!(
            h.text_editor_mut(editor_pane_id).commands_scope(),
            Some(crate::cli::UserCommandScope::Workspace)
        );
    }

    #[test]
    fn single_line_user_command_runs_in_focused_idle_shell_and_splits_when_busy_agent_view_or_split_true(
    ) {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let term = h.add_focused_terminal();
        let view_pane = *h.app.windows[0]
            .panes
            .keys()
            .find(|&&id| id != term)
            .expect("initial view pane");

        let ws_root = h.workspace_root().to_path_buf();
        let ws_commands = ws_root
            .join(crate::config::workspace_channel_dir())
            .join("commands.toml");
        std::fs::create_dir_all(ws_commands.parent().unwrap()).unwrap();
        std::fs::write(
            &ws_commands,
            r#"
[commands]
in_place = "echo IN_PLACE_0879"

[commands.forced_split]
run = "echo SPLIT_0879"
split = true
"#,
        )
        .unwrap();
        h.app.palette_workspace_root = Some(ws_root.clone());
        h.app.palette_commands = crate::cli::resolve_user_commands(Some(&ws_root));

        // 1. Focused pane is an idle shell terminal: `in_place` runs in the current pane (no split).
        h.focus_pane(term).run_frames(2);
        assert_eq!(
            h.app.idle_shell_pane_id(term),
            Some(term),
            "settled terminal pane with no child process must be recognized as an idle shell"
        );
        let count_before = h.pane_count();
        h.terminal_backend(term).enable_input_tap();
        h.app.run_palette_selection(CommandRowId::UserCommand {
            name: "in_place".to_string(),
            scope: crate::cli::UserCommandScope::Workspace,
        });
        h.hidden_frame();
        assert_eq!(
            h.pane_count(),
            count_before,
            "single-line user command on an idle shell pane must NOT open a split pane"
        );
        let tapped = h.terminal_backend(term).take_input_tap();
        let tapped_str = String::from_utf8_lossy(&tapped).into_owned();
        assert!(
            tapped_str.contains("echo IN_PLACE_0879"),
            "idle shell pane must receive the command text directly; got: {tapped_str:?}"
        );

        // Wait for the submitted newline / in-flight submit to clear, then verify `split = true` splits.
        h.app.pending_submits.clear();
        let count_before_forced = h.pane_count();
        h.app.queue_user_command("forced_split", None);
        h.hidden_frame();
        assert_eq!(
            h.pane_count(),
            count_before_forced + 1,
            "`split = true` on a command definition must open a new split pane even from an idle shell"
        );

        // 2. When the focused terminal pane is busy (activity = Working) or an agent pane,
        //    running `in_place` falls back to splitting so we never type into a running program.
        h.focus_pane(term);
        h.app.windows[0]
            .panes
            .get_mut(&term)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .activity = Some(crate::protocol::AgentState::Working);
        assert_eq!(
            h.app.idle_shell_pane_id(term),
            None,
            "busy terminal must not be treated as an idle shell"
        );
        let count_before_busy = h.pane_count();
        h.app.queue_user_command("in_place", None);
        h.hidden_frame();
        assert_eq!(
            h.pane_count(),
            count_before_busy + 1,
            "running a user command while focused on a busy terminal must fall back to a split pane"
        );

        // 3. When the focused pane is a view pane, running `in_place` also falls back to splitting.
        h.focus_pane(view_pane);
        assert_eq!(h.app.idle_shell_pane_id(view_pane), None);
        let count_before_plugin = h.pane_count();
        h.app.queue_user_command("in_place", None);
        h.hidden_frame();
        assert_eq!(
            h.pane_count(),
            count_before_plugin + 1,
            "running a user command while focused on a view pane must fall back to a split pane"
        );
    }

    #[test]
    fn open_panes_pick_up_newly_added_edited_and_removed_commands_without_restart() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let pane_id = h.add_test_pane();
        h.focus_pane(pane_id);
        let ws_root = h.workspace_root().to_path_buf();
        let ws_commands = ws_root
            .join(crate::config::workspace_channel_dir())
            .join("commands.toml");
        std::fs::create_dir_all(ws_commands.parent().unwrap()).unwrap();
        std::fs::write(&ws_commands, "[commands]\nalpha = \"pane name AlphaV1\"\n").unwrap();

        // Deliberately leave `palette_workspace_root = None` to prove `[keys]` `run:<name>`
        // resolves from the focused pane's workspace rather than host process cwd.
        h.app.palette_workspace_root = None;
        h.app.queue_user_command("alpha", None);
        h.hidden_frame();
        assert_eq!(
            h.app.windows[0].panes[&pane_id].as_view().unwrap().name,
            "AlphaV1"
        );

        // While the pane stays open, add `beta` and edit `alpha` on disk.
        std::fs::write(
            &ws_commands,
            "[commands]\nalpha = \"pane name AlphaV2\"\nbeta = \"pane name BetaAdded\"\n",
        )
        .unwrap();

        // 1. Command Palette open (`Cmd+P`) picks up `beta` and updated `alpha`.
        h.press_key(egui::Key::P, egui::Modifiers::COMMAND);
        assert!(h
            .app
            .palette_commands
            .iter()
            .any(|c| c.name == "alpha" && c.run.as_deref() == Some("pane name AlphaV2")));
        assert!(h
            .app
            .palette_commands
            .iter()
            .any(|c| c.name == "beta" && c.run.as_deref() == Some("pane name BetaAdded")));
        h.app.close_command_palette();
        h.app.palette_workspace_root = None;

        // 2. `[keys]` `run:<name>` (`queue_user_command`) executes updated `alpha` and new `beta`.
        h.app.queue_user_command("alpha", None);
        h.hidden_frame();
        assert_eq!(
            h.app.windows[0].panes[&pane_id].as_view().unwrap().name,
            "AlphaV2"
        );
        h.app.queue_user_command("beta", None);
        h.hidden_frame();
        assert_eq!(
            h.app.windows[0].panes[&pane_id].as_view().unwrap().name,
            "BetaAdded"
        );

        // 3. Remove `beta` from `commands.toml` while the pane stays open:
        //    palette, `queue_user_command`, and `plexi run` all stop resolving `beta`.
        std::fs::write(&ws_commands, "[commands]\nalpha = \"pane name AlphaV2\"\n").unwrap();
        h.press_key(egui::Key::P, egui::Modifiers::COMMAND);
        assert!(!h.app.palette_commands.iter().any(|c| c.name == "beta"));
        h.app.close_command_palette();
        h.app.palette_workspace_root = None;
        h.app.queue_user_command("beta", None);
        h.hidden_frame();
        assert_eq!(
            h.app.windows[0].panes[&pane_id].as_view().unwrap().name,
            "BetaAdded"
        );
        let cli_removed = crate::cli::run_command_for_root(Some(&ws_root), &ws_root, "beta", &[]);
        assert_eq!(cli_removed.exit_code, 1);
        assert_eq!(cli_removed.total_steps, 0);
    }

    /// Owner ruling 2026-11-12 (ledger row 48): a saved command whose first
    /// word is a retired short form never runs, whether it is a single-line
    /// `run =` or a step, from the palette or `plexi run`; the user is told.
    #[test]
    fn retired_short_form_command_is_refused_from_the_palette_and_the_cli() {
        use crate::testing::HostHarness;

        let mut h = HostHarness::new();
        let invoker = h.add_test_pane();
        h.focus_pane(invoker);
        h.app.notifications_enabled = true;
        let ws_root = h.workspace_root().to_path_buf();
        let ws_commands = ws_root
            .join(crate::config::workspace_channel_dir())
            .join("commands.toml");
        std::fs::create_dir_all(ws_commands.parent().unwrap()).unwrap();
        std::fs::write(
            &ws_commands,
            "[commands]\nold = \"focus || touch ran.txt\"\n\n[commands.stepped]\nsteps = [\"focus || touch ran.txt\"]\n",
        )
        .unwrap();
        let panes_before = h.app.windows[0].panes.len();

        for name in ["old", "stepped"] {
            h.app.queue_user_command(name, None);
            h.hidden_frame();
            assert!(
                h.app
                    .pending_notifications
                    .iter()
                    .any(|n| n.body.contains("plexi pane focus")),
                "{name}"
            );
        }
        h.run_frames(2);
        assert_eq!(
            h.app.windows[0].panes.len(),
            panes_before,
            "no shell pane opened"
        );
        assert!(
            h.app
                .pending_notifications
                .iter()
                .any(|n| n.title == "Command 'old' not run"),
            "the palette refusal is shown, not only logged"
        );

        let cli = crate::cli::run_command_for_root(Some(&ws_root), &ws_root, "old", &[]);
        assert_eq!(
            cli.exit_code, 2,
            "plexi run refuses the single-line form too"
        );
        assert!(!ws_root.join("ran.txt").exists(), "nothing ran");
    }

    #[test]
    fn stepped_command_splits_anchor_to_invoking_pane_on_every_trigger_unless_explicit_from() {
        use crate::testing::HostHarness;
        use std::io::BufRead;

        let mut h = HostHarness::new();
        let invoker = h.add_test_pane();
        h.focus_pane(invoker);

        let ws_root = h.workspace_root().to_path_buf();
        let ws_commands = ws_root
            .join(crate::config::workspace_channel_dir())
            .join("commands.toml");
        std::fs::create_dir_all(ws_commands.parent().unwrap()).unwrap();
        std::fs::write(
            &ws_commands,
            r#"
[commands.layout2]
steps = [
  "pane new --right --name step-right -- echo r",
  "pane new --down --name step-down -- echo d",
]
"#,
        )
        .unwrap();
        h.app.palette_workspace_root = Some(ws_root.clone());

        // Trigger 1 via host step runner (palette / hotkey):
        // Step 1 splits right of `invoker` (creating a Horizontal container [invoker, step-right]).
        // Step 2 splits down from `invoker` (anchoring to `invoker`, NOT `step-right`!).
        // Therefore `invoker` and `step-down` share a Vertical container, while `step-right` is the
        // right sibling of that Vertical container!
        h.app.queue_user_command("layout2", None);
        h.hidden_frame();
        assert_eq!(h.pane_count(), 3);

        let step_down_id = *h.app.windows[0]
            .panes
            .iter()
            .find_map(|(id, p)| {
                (p.as_terminal().and_then(|t| t.name.as_deref()) == Some("step-down")).then_some(id)
            })
            .expect("step-down pane");
        let step_right_id = *h.app.windows[0]
            .panes
            .iter()
            .find_map(|(id, p)| {
                (p.as_terminal().and_then(|t| t.name.as_deref()) == Some("step-right"))
                    .then_some(id)
            })
            .expect("step-right pane");

        // The split container holding a pane. A settled tree wraps each pane
        // in its own Tabs container (stints 1440, 1511), so the split sits one
        // level above that Tabs.
        let split_parent = |pane: u64, what: &str| {
            let window = &h.app.windows[0];
            let tile = window.tree.tiles.find_pane(&pane).unwrap();
            let held = window
                .find_ancestor_tabs(tile)
                .map_or(tile, |(tabs, _)| tabs);
            window
                .tree
                .tiles
                .parent_of(held)
                .unwrap_or_else(|| panic!("{what} split container"))
        };
        let invoker_parent = split_parent(invoker, "invoker");
        let down_parent = split_parent(step_down_id, "step-down");
        let right_parent = split_parent(step_right_id, "step-right");
        assert_eq!(
            invoker_parent, down_parent,
            "step 2 (--down) must anchor to the invoking pane on the first trigger, not chain off step 1 (--right)"
        );
        assert_ne!(
            right_parent, down_parent,
            "step-right must not be the split anchor for step-down"
        );

        // Trigger 2 via CLI (`run_command_for_root_from_pane`) from `step_right_id` with an explicit
        // `--from` override on step 2: verify step 1 anchors to `step_right_id` and step 2 honors `--from`.
        std::fs::write(
            &ws_commands,
            format!(
                r#"
[commands.cli_anchor]
steps = [
  "pane new --right --name cli-s1 -- echo 1",
  "pane new --down --from {invoker} --name cli-s2 -- echo 2",
]
"#
            ),
        )
        .unwrap();

        let sock_dir = tempfile::tempdir().unwrap();
        let sock_path = sock_dir.path().join("notify.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let _sg = crate::cli::test_env::socket_env_guard();
        crate::cli::set_command_socket_override(sock_path.clone());

        let ws_for_cli = ws_root.clone();
        let cli_handle = std::thread::spawn(move || {
            crate::cli::run_command_for_root_from_pane(
                Some(&ws_for_cli),
                &ws_for_cli,
                "cli_anchor",
                &[],
                Some(step_right_id),
            )
        });

        let mut seen_from_ids = Vec::new();
        while !cli_handle.is_finished() {
            if let Ok((stream, _)) = listener.accept() {
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() && !line.trim().is_empty() {
                    if let Ok(req) =
                        serde_json::from_str::<crate::protocol::HostRequest>(line.trim())
                    {
                        if let crate::protocol::HostRequest::SpawnPane { from_pane_id, .. } = &req {
                            seen_from_ids.push(*from_pane_id);
                        }
                        // Classify the peer the way the socket entry does: the
                        // CLI runs on a thread of this test process.
                        let origin = h.app.socket_origin_for_pids(&[std::process::id()]);
                        h.app.handle_pane_ipc_request(origin, req);
                    }
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let cli_report = cli_handle.join().unwrap();
        crate::cli::clear_command_socket_override();
        assert_eq!(cli_report.exit_code, 0);
        assert_eq!(
            seen_from_ids,
            vec![Some(step_right_id), Some(invoker)],
            "step 1 must default to invoking pane ({step_right_id}) and step 2 must honor explicit --from ({invoker})"
        );
    }
}
