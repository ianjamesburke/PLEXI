//! Pane / plugin / agent creation and launch entry points.
//!
//! These methods own the "make a new pane appear" side of the pane API —
//! WASM plugin runtime wiring, builtin plugin wiring, single-pane tree creation for new
//! contexts, and the launch helpers called from the command palette and
//! HostRequest routing. The actual tile-tree mutation is delegated to
//! [`PlexiApp::split_with_new_pane_at`] in [`super::layout`].

use super::construct::{LaunchCommand, McpBearer, TerminalLaunch};
use crate::app::view_trait::View;
use crate::app::PlexiApp;
use crate::host::command::ShareRatio;
use crate::host::pane::{Pane, TerminalPane};
use crate::spatial::tiling::PaneId;
use egui_tiles::{Tile, TileId, Tree};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A fully-built context window: tile tree, panes, root tile, spawn order.
type ContextPaneSet = (Tree<PaneId>, HashMap<PaneId, Pane>, TileId, Vec<PaneId>);

/// Context identity stamped into a new pane's `PLEXI_CONTEXT_*` environment.
///
/// Carried explicitly so a pane can be created for a context that is not the
/// active one — including one not yet pushed to the router, which is the case
/// while a child context's window is still being built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneContextEnv {
    pub context_id: u64,
    pub name: String,
    pub description: String,
    pub root: Option<PathBuf>,
    pub depth: u32,
}

/// Derive a placeholder header title for a new terminal pane from its context and cwd.
pub(crate) fn derive_context_placeholder_title(
    context_name: &str,
    cwd: Option<&Path>,
    context_root: Option<&Path>,
) -> Option<String> {
    let trimmed = context_name.trim();
    if !trimmed.is_empty() {
        return Some(trimmed.to_string());
    }
    if let Some(cwd) = cwd.or(context_root) {
        if let Some(name) = cwd.file_name().and_then(|n| n.to_str()) {
            let name_trimmed = name.trim();
            if !name_trimmed.is_empty() {
                return Some(name_trimmed.to_string());
            }
        }
    }
    Some("Terminal".to_string())
}

/// Where a split lands: beside `target_tile` in window `win_idx`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SplitPlacement {
    pub win_idx: usize,
    pub target_tile: egui_tiles::TileId,
    pub vertical: bool,
    pub new_pane_first: bool,
    pub keep_focus: bool,
}

/// Where an agent worker lands, resolved read-only before construction.
#[derive(Clone, Debug)]
pub(crate) struct AgentPlacement {
    pub(crate) parent_pane_id: Option<u64>,
    pub(crate) effective_cwd: PathBuf,
    pub(crate) target: AgentTarget,
}

#[derive(Clone, Debug)]
pub(crate) enum AgentTarget {
    /// An existing window: the parent's own, or an existing sub-context's.
    /// The worker splits beside `split_at`, or seeds the empty tree when it
    /// is `None`; installation never re-reads focus to choose.
    Window {
        win_idx: usize,
        context_id: u64,
        split_at: Option<egui_tiles::TileId>,
    },
    /// A sub-context created under `parent_context_id` when the worker is
    /// installed.
    NewSubcontext {
        name: String,
        parent_win_idx: usize,
        parent_context_id: u64,
        depth: u32,
        conjoined: bool,
    },
}

/// A placement's context identity, with a new sub-context's ids reserved.
enum StagedAgentDestination {
    Window(PaneContextEnv),
    NewSubcontext {
        context: PaneContextEnv,
        window_id: u64,
    },
}

/// What an agent worker runs and under which ceiling.
pub(crate) struct AgentWorkerLaunch<'a> {
    pub(crate) grant: AgentWorkerGrant,
    /// A reserved id (a handoff successor), or `None` to allocate one.
    pub(crate) pane_id: Option<PaneId>,
    /// Provenance decided at admission (`team:...`); `None` derives it from
    /// the verified parent.
    pub(crate) spawned_by: Option<String>,
    pub(crate) role: &'a str,
    pub(crate) harness: &'a str,
    pub(crate) command: &'a str,
    pub(crate) claude_memory: Option<crate::agent::memory::ClaudeMemoryIdentity>,
    pub(crate) task: &'a str,
    pub(crate) ephemeral: bool,
    pub(crate) result_schema: Option<serde_json::Value>,
    pub(crate) run_token: Option<&'a str>,
}

/// One host decision for a configured agent's command, grant and memory.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConfiguredAgentLaunch {
    pub(crate) role: crate::broker::pane_authority::AdmittedRole,
    pub(crate) harness: crate::cli::AgentHarness,
    pub(crate) command: String,
    pub(crate) claude_memory: Option<crate::agent::memory::ClaudeMemoryIdentity>,
    pub(crate) ephemeral: bool,
    pub(crate) result_schema: Option<serde_json::Value>,
}

impl ConfiguredAgentLaunch {
    pub(crate) fn resolve(
        role: &str,
        cwd: &Path,
        task: Option<&str>,
        ephemeral: Option<bool>,
    ) -> Result<Self, String> {
        let stops = crate::app::registry::workspace_walk_stops()?;
        Self::resolve_in(
            role,
            cwd,
            task,
            ephemeral,
            &crate::config::config_dir(),
            &stops,
        )
    }

    pub(crate) fn resolve_in(
        role: &str,
        cwd: &Path,
        task: Option<&str>,
        ephemeral: Option<bool>,
        profile: &Path,
        stops: &[PathBuf],
    ) -> Result<Self, String> {
        use crate::agent::memory::{ClaudeMemoryIdentity, ensure_memory_dir};
        use crate::cli::agent::{
            build_agent_start_command, resolve_agent_definition_in,
            resolve_agent_workspace_root_for, resolve_harness_adapter, with_auto_state_hooks,
            with_claude_auto_memory,
        };

        crate::agent::validate_agent_id(role)?;
        let workspace = resolve_agent_workspace_root_for(role, cwd, stops)
            .map(|root| {
                root.canonicalize()
                    .map_err(|error| format!("resolve agent workspace {}: {error}", root.display()))
            })
            .transpose()?;
        let (definition, _) = resolve_agent_definition_in(role, profile, workspace.as_deref())?;
        let config = definition.harness.as_ref().ok_or_else(|| {
            format!("agent '{role}' has no [harness] table in settings.toml (it is an Assistant persona and cannot be started as a CLI harness session)")
        })?;
        let adapter = resolve_harness_adapter(config)?;
        let ephemeral = ephemeral.unwrap_or(config.ephemeral);
        let result_schema = config.result_schema.clone();
        let mut claude_memory = None;
        let mut prompt_memory = None;
        if !ephemeral {
            if adapter.harness == crate::cli::AgentHarness::Claude {
                claude_memory = Some(ClaudeMemoryIdentity::from_canonical_workspace(
                    &definition,
                    workspace.as_deref(),
                )?);
            } else {
                prompt_memory = Some(ensure_memory_dir(&definition)?);
            }
        }
        let command =
            build_agent_start_command(&definition, adapter, task, prompt_memory.as_deref())?;
        let command = match &claude_memory {
            Some(memory) => with_claude_auto_memory(&command, &memory.ensure_dir()?)?,
            None => with_auto_state_hooks(adapter, command),
        };
        Ok(Self {
            role: PlexiApp::admitted_worker_role(definition),
            harness: adapter.harness,
            command,
            claude_memory,
            ephemeral,
            result_schema,
        })
    }
}

/// Where an agent spawn hangs: see [`PlexiApp::agent_spawn_parent`].
pub(crate) struct AgentSpawnParent {
    pub(crate) pane_id: Option<u64>,
    pub(crate) win_idx: usize,
    pub(crate) context_id: u64,
    pub(crate) root: PathBuf,
    pub(crate) depth: u32,
}

/// The sub-context an agent spawn lands in, under its parent's context.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SubcontextTarget<'a> {
    /// Reuse the sub-context of this name, or create it.
    Named(&'a str),
    /// The sub-context admitted earlier; a same-named replacement refuses.
    Existing { name: &'a str, context_id: u64 },
    /// Create the sub-context under the parent context admitted with it;
    /// one that appeared since, or a parent that moved, refuses.
    Create {
        name: &'a str,
        parent_context_id: u64,
    },
}

/// Where an agent spawn without a live parent pane lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentOrigin {
    /// The active window at spawn time (direct, unplanned spawns).
    Active,
    /// The first window of a context.
    Context(u64),
    /// The window the caller selected, still in the context it was in.
    Window { window_id: u64, context_id: u64 },
}

impl<'a> SubcontextTarget<'a> {
    pub(crate) fn name(self) -> &'a str {
        match self {
            Self::Named(name) | Self::Create { name, .. } | Self::Existing { name, .. } => name,
        }
    }
}

/// What an agent worker's grant is admitted from, prepared before the pane
/// exists (stint 1632): the verified source that bounds it, the role it was
/// admitted for, and any recorded restriction a relaunch must keep.
#[derive(Clone, Debug)]
pub(crate) struct AgentWorkerGrant {
    pub(crate) source: crate::broker::pane_authority::GrantSource,
    /// `None` only for a pane that runs no named role (a team coordinator,
    /// a reopened harness run with no role).
    pub(crate) role: Option<crate::broker::pane_authority::AdmittedRole>,
    pub(crate) recorded: Option<crate::broker::pane_authority::PaneAuthority>,
}

pub(crate) struct IdViewLaunchOptions {
    pub target_win: usize,
    pub bypass_on_launch: bool,
    pub follow_file_browser_cwd: bool,
}

struct BuiltinPlacement<'a> {
    window: usize,
    hint: Option<&'a str>,
    share: Option<f32>,
}

struct WasmViewInstanceSpec<'a> {
    target_win: usize,
    plugin_id: &'a str,
    app: crate::host::wasm_view::WasmView,
    snapshot: crate::host::wasm_view::StateSnapshot,
    workspace_root: PathBuf,
    permissions: crate::app::permissions::PluginPermissions,
    remembered: Option<(
        crate::app::permissions::PermissionStore,
        HashSet<String>,
        HashSet<String>,
    )>,
    layout: Option<&'a str>,
    launch_args: Vec<String>,
}

/// The text editor on `path`, its workspace resolved from `cwd`. Shared by
/// the string-argument factory and typed launches that own a native path.
fn text_editor_view(path: PathBuf, cwd: &Path) -> Box<dyn View> {
    let read = TextEditorRead::read(&path, cwd);
    text_editor_from_read(path, read)
}

/// What a text editor reads from disk before it opens: the file, and the
/// workspace its `commands.toml` resolves against. Gathered in one call so a
/// caller can run it off the UI thread (`app::file_open::prepare_open`).
pub(crate) struct TextEditorRead {
    pub(crate) read: std::io::Result<String>,
    pub(crate) workspace_root: Option<PathBuf>,
}

impl TextEditorRead {
    pub(crate) fn read(path: &Path, cwd: &Path) -> Self {
        Self {
            read: std::fs::read_to_string(path),
            workspace_root: crate::cli::resolve_commands_workspace_root(cwd)
                .or_else(|| Some(cwd.to_path_buf())),
        }
    }
}

/// The text editor built from a read already made. Touches no disk.
pub(crate) fn text_editor_from_read(path: PathBuf, read: TextEditorRead) -> Box<dyn View> {
    Box::new(crate::app::text_editor_view::TextEditorView::from_read(
        path,
        read.workspace_root,
        read.read,
    ))
}

/// Which live pane a lookup by plugin id accepts (stint 1733): it must also
/// run the origin the launch would start.
#[derive(Debug, Clone, Copy)]
pub(crate) enum OriginQuery<'a> {
    /// The Rust builtin compiled into this binary.
    Builtin,
    /// The guest started from this entry file.
    Entry(&'a Path),
    /// The guest admitted as the bundled copy of its id. Bundled-id reuse
    /// never matches by path (stint 1733).
    Bundled,
    /// Any pane with the id, whatever runs it. Tests only.
    #[cfg(test)]
    Any,
}

/// Instantiate a builtin `App` by id, checked before the process registry.
///
/// Builtins are compiled-in and never require disk access. Add one line here
/// when introducing a new builtin plugin type; the dispatch path in
/// `open_view_by_id_with_layout` requires no further changes.
///
/// `"terminal"` is intentionally absent — it is a PTY pane, not an `App`.
pub(crate) fn builtin_factory(id: &str, cwd: &Path, args: &[String]) -> Option<Box<dyn View>> {
    match id {
        "agents" => Some(Box::new(crate::app::agents_view::AgentsPaneView::default())),
        "image-viewer" => {
            let path = args.first().map(PathBuf::from)?;
            Some(Box::new(
                crate::app::image_viewer_view::ImageViewerView::new(path),
            ))
        }
        "video-player" => {
            let path = args.first().map(PathBuf::from)?;
            Some(Box::new(
                crate::app::video_player_view::VideoPlayerView::new(path),
            ))
        }
        "audio-player" => {
            let path = args.first().map(PathBuf::from)?;
            Some(Box::new(
                crate::app::audio_player_view::AudioPlayerView::new(path),
            ))
        }
        "text-editor" => {
            // No explicit path → a fresh scratch note in the global tier. The
            // file is only created once content is typed (empty notes are
            // deleted).
            let path = args
                .first()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(new_scratch_note_path);
            Some(text_editor_view(path, cwd))
        }
        "mcp-renderer" => Some(Box::new(
            crate::render::mcp_renderer_view::McpRendererView::new(args, cwd.to_path_buf()),
        )),
        "cli-renderer" => {
            let path = args.first().cloned().unwrap_or_default();
            Some(Box::new(
                crate::render::cli_renderer_view::CliRendererView::new(path),
            ))
        }
        "file_browser" => Some(Box::new(crate::file_browser::FileBrowserView::new(
            cwd.to_path_buf(),
        ))),
        "secrets_manager" => Some(Box::new(crate::app::secrets_view::SecretsView::new(
            cwd.to_path_buf(),
        ))),
        "agents-dashboard" => Some(Box::new(
            crate::app::agents_dashboard_view::AgentsDashboardView::new(cwd.to_path_buf()),
        )),
        _ => None,
    }
}

/// Whether launching `id` with a file path as its first argument opens that
/// file. The built-in document viewers do; every other built-in pane type
/// (`builtin_factory`, plus `terminal` and `assistant`) treats the argument
/// differently or ignores it. An installed plugin receives its arguments as
/// launch args and is trusted to open them.
pub(crate) fn opens_path_argument(id: &str) -> bool {
    match id {
        "text-editor" | "image-viewer" | "video-player" | "audio-player" => true,
        "terminal" => false,
        id => !is_builtin_view_id(id),
    }
}

/// The view ids compiled into this binary: everything [`builtin_factory`]
/// builds, plus the Assistant, whose constructor needs the live broker. A
/// launch of one of these ids opens the builtin and never a registry plugin
/// that reuses the id (stint 1733).
pub(crate) fn is_builtin_view_id(id: &str) -> bool {
    matches!(
        id,
        "assistant"
            | "agents"
            | "image-viewer"
            | "video-player"
            | "audio-player"
            | "text-editor"
            | "mcp-renderer"
            | "cli-renderer"
            | "file_browser"
            | "secrets_manager"
            | "agents-dashboard"
    )
}

fn release_feature_for_plugin_id(id: &str) -> Option<crate::release::ReleaseFeature> {
    match id {
        "audio-player" | "video-player" => Some(crate::release::ReleaseFeature::MediaIo),
        "com.plexi.daw-engine-poc" => Some(crate::release::ReleaseFeature::Daw),
        "agents-dashboard" => Some(crate::release::ReleaseFeature::AgentsDashboard),
        "mcp-renderer" => Some(crate::release::ReleaseFeature::McpClient),
        _ => None,
    }
}

/// Native WASM component plugins (`ViewRuntime::Wasm`) are an experiment. The
/// Python runtime also runs in Wasmtime but is core, so it never calls this.
fn require_wasm_experiment() -> Result<(), String> {
    if crate::release::feature_enabled(crate::release::ReleaseFeature::WasmApps) {
        Ok(())
    } else {
        Err(crate::release::feature_unavailable_message(
            crate::release::ReleaseFeature::WasmApps,
        ))
    }
}

pub(crate) const FILE_BROWSER_CWD_GROUP: &str = "file-browser-cwd";

fn builtin_default_group(type_id: &str) -> Option<String> {
    match type_id {
        "file_browser" => Some(FILE_BROWSER_CWD_GROUP.to_string()),
        _ => None,
    }
}

fn builtin_restore_args(
    plugin_id: &str,
    plugin_state: Option<&serde_json::Value>,
) -> Option<Vec<String>> {
    match plugin_id {
        "text-editor" | "image-viewer" | "video-player" | "audio-player" => plugin_state
            .and_then(|state| state.get("path"))
            .and_then(|path| path.as_str())
            .map(|path| vec![path.to_string()]),
        "agents" | "file_browser" | "secrets_manager" | "agents-dashboard" => Some(vec![]),
        _ => None,
    }
}

pub(crate) fn restore_builtin_view_pane(
    plugin_id: &str,
    pane_id: PaneId,
    cwd: PathBuf,
    plugin_state: Option<&serde_json::Value>,
) -> Option<Pane> {
    if let Some(feature) = release_feature_for_plugin_id(plugin_id) {
        if !crate::release::feature_enabled(feature) {
            crate::release::log_feature_blocked(feature);
            return None;
        }
    }
    let args = builtin_restore_args(plugin_id, plugin_state)?;
    let mut app = builtin_factory(plugin_id, &cwd, &args)?;
    if let Some(state) = plugin_state {
        app.restore_state(state);
    }
    let runtime_id = app.type_id().to_string();
    let name = app.display_name();
    log::info!(
        "workspace_restore: restored builtin plugin_id={plugin_id} runtime_id={runtime_id} pane_id={pane_id} cwd={}",
        cwd.display()
    );
    Some(Pane::View(Box::new(crate::host::pane::ViewPane {
        incarnation: Default::default(),
        pip_status: None,
        id: pane_id,
        runtime: crate::host::pane::ViewRuntime::Builtin(app),
        workspace_root: cwd,
        permissions: crate::app::permissions::PluginPermissions::builtin(),
        manifest_id: runtime_id.clone(),
        name,
        pane_group: builtin_default_group(&runtime_id),
        linked_pane_id: None,
        overlay_replaced: None,
        agent: None,
        slots: std::collections::HashMap::new(),
        semantic_state: Default::default(),
    })))
}

/// Rebuild the Assistant pane during workspace restore so a reopened plugin comes
/// back to the same conversation, focused. The Assistant is a builtin that
/// needs a broker and profile dir, so the broker-free [`builtin_factory`]
/// cannot construct it — the caller supplies the broker (built from
/// `config.ai`). `AssistantView::new` auto-resumes the active conversation from
/// disk, so no `plugin_state` is required.
pub(crate) fn restore_assistant_pane(
    pane_id: PaneId,
    workspace_root: PathBuf,
    broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker>,
    profile_dir: &std::path::Path,
    context_id: u64,
) -> Pane {
    let app = Box::new(crate::assistant::AssistantView::new(
        workspace_root.clone(),
        broker,
        profile_dir,
        context_id,
    ));
    let runtime_id = app.type_id().to_string();
    let name = app.display_name();
    log::info!(
        "workspace_restore: restored assistant pane {pane_id} cwd={}",
        workspace_root.display()
    );
    Pane::View(Box::new(crate::host::pane::ViewPane {
        incarnation: Default::default(),
        pip_status: None,
        id: pane_id,
        runtime: crate::host::pane::ViewRuntime::Builtin(app),
        workspace_root,
        permissions: crate::app::permissions::PluginPermissions::builtin(),
        manifest_id: runtime_id.clone(),
        name,
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: None,
        agent: None,
        slots: std::collections::HashMap::new(),
        semantic_state: Default::default(),
    }))
}

/// Shared construction site for a live CPython-WASM runtime, run inside an
/// admitted start. Plans the manifest's capability rows and writes them only
/// once the guest has started, so a refused or failed start keeps none
/// (stint 1733); fills the launch-time fields only the caller knows (workspace root,
/// live context root, launch args, theme) and launches. Both a fresh open
/// (`open_python_wasm_view_pane`) and a workspace restore (`restore_view_pane`)
/// call this, so there is exactly one place that builds the runtime.
fn build_python_runtime(
    admitted: crate::host::plugin_admission::AdmittedPlugin<'_>,
    mut config: crate::host::wasm_python::PythonLaunchConfig,
    profile_dir: &Path,
    workspace_root: PathBuf,
    context_root: PathBuf,
    launch_args: Vec<String>,
    theme: &crate::ui::theme::Colors,
) -> Result<
    (
        crate::host::wasm_python::LivePythonPane,
        crate::app::permissions::PluginPermissions,
    ),
    String,
> {
    let (permissions, mints) = plan_plugin_permissions_in_profile(
        profile_dir,
        admitted.owner(),
        &workspace_root,
        &config.declared_capabilities,
        config.allowed_hosts.clone(),
    );
    config.workspace_root = workspace_root;
    config.context_root = context_root;
    config.launch_args = launch_args;
    config.capabilities = permissions
        .capabilities
        .iter()
        .copied()
        .map(|capability| capability.as_str().to_string())
        .collect();
    config.allowed_hosts = permissions.allowed_hosts.clone();
    config.theme = theme.to_theme_map();
    let runtime = crate::host::wasm_python::LivePythonPane::launch(admitted, config)
        .map_err(|error| error.to_string())?;
    mints.commit();
    Ok((runtime, permissions))
}

/// The Python launch config for `resolved`, from the manifest its resolver
/// parsed.
fn python_launch_config(
    resolved: &crate::app::registry::ResolvedPlugin,
) -> Result<crate::host::wasm_python::PythonLaunchConfig, String> {
    crate::host::wasm_python::PythonLaunchConfig::from_resolved(resolved)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "manifest does not contain a Python entry".to_string())
}

/// The saved identity a restored view pane keeps: the caller owns the pane id,
/// context, and placement; restore only rebuilds the runtime behind them.
pub(crate) struct RestoreTarget<'a> {
    pub pane_id: PaneId,
    pub context_id: u64,
    pub workspace_root: PathBuf,
    pub context_root: &'a Path,
    pub saved_name: Option<&'a str>,
}

/// What a saved view pane relaunches as.
pub(crate) struct RestoredViewPane {
    pub pane: Pane,
    pub state_paths: Vec<(crate::host::state_scope::StateScope, PathBuf)>,
    /// Hot-reload watch target (the Python `plugin_dir`); empty for WASM.
    pub hot_reload_dir: PathBuf,
}

/// A saved layout's plugin id is only an assertion: refuse the restore when
/// the plugin on disk now has another owner (stint 1398).
fn check_saved_owner(
    saved_id: &str,
    resolved: &crate::app::registry::ResolvedPlugin,
) -> Result<(), String> {
    if resolved.owner == saved_id {
        return Ok(());
    }
    let message = format!(
        "the saved layout names plugin '{}', but {} belongs to '{}'",
        crate::host::guest_text::shown(saved_id),
        resolved.entry.display(),
        crate::host::guest_text::shown(&resolved.owner)
    );
    log::info!("workspace_restore: refused: {message}");
    Err(message)
}

/// Record the imports a person allowed in the raw-WASM review dialog, in both
/// stores. Called only once an admitted component has instantiated, while its
/// start still holds the admission lock (stints 1398, 1733).
fn record_reviewed_wasm_imports(
    decision: crate::broker::consent::CoveredDecision<'_>,
    plugin_id: &str,
    workspace_root: &Path,
    imports: &[String],
) -> Result<(), String> {
    let profile = crate::config::config_dir();
    let mut store = crate::app::permissions::PermissionStore::load_or_default(&profile)
        .map_err(|error| format!("raw_wasm_review: approval not recorded: {error}"))?;
    let monitor = crate::broker::gate::PermissionMonitor::for_profile(&profile)
        .map_err(|error| format!("raw_wasm_review: approval not recorded: {error}"))?;
    for capability in imports {
        store.set_wasm(
            plugin_id,
            workspace_root,
            capability,
            crate::app::permissions::PermissionState::Green,
        );
    }
    store.save();
    for capability in imports {
        monitor.grant_capability_id(
            decision,
            plugin_id,
            workspace_root,
            capability,
            crate::broker::Decision::Allow,
        );
    }
    log::info!(
        "raw_wasm_review: approved {} imports for plugin_id={plugin_id}",
        imports.len()
    );
    Ok(())
}

/// Resolve the plugin at `dir` from disk now; the manifest the caller looked up
/// must still own that folder (stint 1398).
fn resolve_owned_plugin_dir(
    dir: &Path,
    expected_owner: &str,
) -> Result<crate::app::registry::ResolvedPlugin, String> {
    let resolved = crate::app::registry::resolve_plugin_dir(dir)?;
    if resolved.owner != expected_owner {
        return Err(format!(
            "{} now holds plugin '{}', not '{}'",
            resolved.root.display(),
            crate::host::guest_text::shown(&resolved.owner),
            crate::host::guest_text::shown(expected_owner)
        ));
    }
    Ok(resolved)
}

/// The admission check for `profile_dir`'s own plugins folder.
fn profile_admission(profile_dir: &Path) -> crate::host::plugin_admission::PluginAdmission {
    crate::host::plugin_admission::PluginAdmission::new(profile_dir, &profile_dir.join("plugins"))
}

/// Restore a saved Python or WASM view pane by relaunching it through the same
/// runtime-construction path a fresh open uses — never through
/// `place_view_pane`, which allocates a new pane id and mutates the tile tree.
/// The caller (the restore loop in `restore_workspace`) keeps the saved pane id,
/// tree, and placement; this only builds the runtime and its `ViewPane`
/// envelope.
///
/// `Err` means the plugin no longer resolves (manifest gone, permission review
/// required, component failed to load) — the caller must render a
/// launch-failed pane, never silently substitute a terminal.
#[cfg(test)]
pub(crate) fn restore_view_pane(
    manifest_id: &str,
    launch: &crate::workspace::SavedViewLaunch,
    target: RestoreTarget<'_>,
    theme: &crate::ui::theme::Colors,
) -> Result<RestoredViewPane, String> {
    restore_view_pane_in_profile(
        manifest_id,
        launch,
        target,
        theme,
        &crate::config::config_dir(),
    )
}

/// Profile-explicit variant of [`restore_view_pane`] used by [`crate::app::restore_workspace`]
/// so workspace restore does not read an ambient `config_dir()`.
pub(crate) fn restore_view_pane_in_profile(
    manifest_id: &str,
    launch: &crate::workspace::SavedViewLaunch,
    target: RestoreTarget<'_>,
    theme: &crate::ui::theme::Colors,
    profile_dir: &Path,
) -> Result<RestoredViewPane, String> {
    let RestoreTarget {
        pane_id,
        context_id,
        workspace_root,
        context_root,
        saved_name,
    } = target;
    match launch {
        crate::workspace::SavedViewLaunch::Python { plugin_dir, args } => {
            if !plugin_dir.is_dir() {
                return Err(format!(
                    "plugin directory not found at {}",
                    plugin_dir.display()
                ));
            }
            let resolved = crate::app::registry::resolve_plugin_dir(plugin_dir)?;
            check_saved_owner(manifest_id, &resolved)?;
            let config = python_launch_config(&resolved)?;
            let (mut runtime, permissions) =
                profile_admission(profile_dir).start_live_plugin(&resolved, |admitted| {
                    build_python_runtime(
                        admitted,
                        config,
                        profile_dir,
                        workspace_root.clone(),
                        context_root.to_path_buf(),
                        args.clone(),
                        theme,
                    )
                })?;
            let plugin_id = runtime.origin().owner().to_string();
            runtime.set_pipe_owner(crate::host::scope::Scope::Pane {
                pane_id,
                context_id,
            });
            let state_paths = runtime.state_paths();
            let name = saved_name
                .map(String::from)
                .unwrap_or_else(|| runtime.display_name());
            log::info!(
                "workspace_restore: restored python plugin_id={plugin_id} pane_id={pane_id}"
            );
            let pane = Pane::View(Box::new(crate::host::pane::ViewPane {
                incarnation: Default::default(),
                pip_status: None,
                id: pane_id,
                runtime: crate::host::pane::ViewRuntime::Python(Box::new(runtime)),
                workspace_root,
                permissions,
                manifest_id: plugin_id,
                name,
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            }));
            Ok(RestoredViewPane {
                pane,
                state_paths,
                hot_reload_dir: plugin_dir.clone(),
            })
        }
        crate::workspace::SavedViewLaunch::Wasm { wasm_path, args } => {
            use crate::app::registry::ManifestType;
            use crate::host::wasm_pane::{LiveWasmPane, SysinfoStats, WasmPane};
            use crate::host::wasm_view::{StateStore, WasmView};

            require_wasm_experiment()?;
            if let Some(feature) = release_feature_for_plugin_id(manifest_id) {
                if !crate::release::feature_enabled(feature) {
                    return Err(crate::release::feature_unavailable_message(feature));
                }
            }
            if !wasm_path.is_file() {
                return Err(format!(
                    "WASM component not found at {}",
                    wasm_path.display()
                ));
            }
            // The saved id is an assertion: the component's actual owner is
            // resolved from disk and must match before any grant, state or
            // import under that id is touched (stint 1398).
            let resolved = crate::app::registry::resolve_component(wasm_path)?;
            check_saved_owner(manifest_id, &resolved)?;
            match resolved.installed.as_ref() {
                Some(installed) if installed.manifest.manifest_type == ManifestType::Wasm => {
                    // Mint non-sensitive manifest capabilities before the WASM
                    // sets collect stored rows, just as an installed launch does.
                    // Import-derived grants below still require sensitive review.
                    wasm_manifest_permissions(
                        profile_dir,
                        manifest_id,
                        &workspace_root,
                        &installed.manifest.capabilities.capabilities,
                        installed.manifest.capabilities.allowed_hosts.clone(),
                    )?
                    .1
                    .commit();
                }
                installed => {
                    let reason = if installed.is_some() {
                        "installed manifest is not WASM"
                    } else {
                        "standalone component with no manifest"
                    };
                    log::info!(
                        "plugin::{manifest_id}: restore minted no manifest capabilities: {reason}"
                    );
                }
            }
            let permission_store =
                crate::app::permissions::PermissionStore::load_or_default(profile_dir)
                    .map_err(|error| error.to_string())?;
            let required_grants = WasmView::inspect_required_grants(wasm_path)
                .map_err(|e| format!("inspect {}: {e}", wasm_path.display()))?;
            let required_caps = required_grants.capability_ids();
            let declared: std::collections::HashSet<String> =
                required_caps.iter().cloned().collect();
            let (remembered_grants, remembered_blocks) =
                gate_wasm_sets_in_profile(profile_dir, manifest_id, &workspace_root, &declared);
            let missing: Vec<String> = required_caps
                .iter()
                .filter(|cap| !remembered_grants.contains(*cap))
                .cloned()
                .collect();
            if !missing.is_empty() {
                return Err(format!(
                    "restore requires permission review for: {}",
                    missing.join(", ")
                ));
            }
            // Persistent, not ephemeral — restore must re-address the same
            // state file a fresh installed-plugin launch would (stint 0678).
            let store = StateStore::persistent(wasm_state_path_in_profile(
                profile_dir,
                manifest_id,
                &workspace_root,
            ))
            .map_err(|e| format!("open wasm state for {manifest_id}: {e}"))?;
            let snapshot = store.snapshot();
            let app = profile_admission(profile_dir).start_live_plugin(&resolved, |admitted| {
                WasmView::load_with_grants(admitted, store, required_grants)
                    .map_err(|e| format!("load {}: {e}", wasm_path.display()))
            })?;
            let pane = WasmPane::new(app, Box::new(SysinfoStats::new()))
                .with_remembered_capabilities(
                    workspace_root.clone(),
                    permission_store,
                    remembered_grants,
                    remembered_blocks,
                );
            let mut live =
                LiveWasmPane::new(pane, manifest_id, snapshot, args.clone());
            live.set_pane_id(pane_id);
            live.set_context_id(context_id);
            let name = saved_name
                .map(String::from)
                .unwrap_or_else(|| live.display_name());
            log::info!(
                "workspace_restore: restored wasm plugin_id={manifest_id} pane_id={pane_id}",
                manifest_id = crate::host::guest_text::shown(manifest_id)
            );
            let pane = Pane::View(Box::new(crate::host::pane::ViewPane {
                incarnation: Default::default(),
                pip_status: None,
                id: pane_id,
                runtime: crate::host::pane::ViewRuntime::Wasm(Box::new(live)),
                workspace_root,
                permissions: crate::app::permissions::PluginPermissions::default(),
                manifest_id: manifest_id.to_string(),
                name,
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            }));
            Ok(RestoredViewPane {
                pane,
                state_paths: Vec::new(),
                hot_reload_dir: PathBuf::new(),
            })
        }
    }
}

/// Build a `launch_failed` builtin pane for a saved plugin that no longer
/// resolves (uninstalled, manifest gone, permission review required). Mirrors
/// `PlexiApp::open_launch_failed_pane`'s `ViewPane` literal, minus the
/// placement/tile-tree mutation — the restore loop keeps the saved pane id
/// and tree, so this only builds the `Pane` to slot in.
pub(crate) fn restore_launch_failed_pane(
    plugin_id: &str,
    pane_id: PaneId,
    workspace_root: PathBuf,
    reason: String,
) -> Pane {
    log::info!(
        "workspace_restore: launch failed for plugin_id={plugin_id} pane_id={pane_id}: {reason}"
    );
    Pane::View(Box::new(crate::host::pane::ViewPane {
        incarnation: Default::default(),
        pip_status: None,
        id: pane_id,
        runtime: crate::host::pane::ViewRuntime::Builtin(Box::new(
            crate::host::launch_failed::LaunchFailedView {
                plugin_id: plugin_id.to_string(),
                missing: vec![reason],
            },
        )),
        workspace_root,
        permissions: crate::app::permissions::PluginPermissions::default(),
        manifest_id: plugin_id.to_string(),
        name: format!("Cannot launch {plugin_id}"),
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: None,
        agent: None,
        slots: std::collections::HashMap::new(),
        semantic_state: Default::default(),
    }))
}

impl PlexiApp {
    /// Resolve the environment identity for a pane owned by `win_idx`.
    ///
    /// The caller supplies the target window explicitly; this must never use
    /// `active_window` or `router.active()`, since a command can create panes
    /// outside the currently visible context.
    pub(crate) fn pane_context_env_for_window(&self, win_idx: usize) -> PaneContextEnv {
        let context_id = self.windows[win_idx].context_id;
        self.pane_context_env_for_context(context_id)
    }

    /// Resolve a registered context's identity for a pane about to spawn.
    pub(crate) fn pane_context_env_for_context(&self, context_id: u64) -> PaneContextEnv {
        PaneContextEnv {
            context_id,
            name: self.context_name_for(context_id),
            description: self.context_description_for(context_id),
            root: self.context_root_for(context_id),
            depth: self.context_depth_for(context_id),
        }
    }
    /// Convert a manifest-declared share fraction (0.0..1.0 exclusive) to a `ShareRatio`.
    /// Validates the range and falls back to 0.5 (1:1) on invalid input, logging a warning.
    fn share_ratio_from_fraction(plugin_id: &str, fraction: Option<f32>) -> ShareRatio {
        let f = fraction.unwrap_or(0.5);
        if f <= 0.0 || f >= 1.0 {
            log::warn!(
                "open_pane_layout({plugin_id}): initial_share {f} out of range (0.0, 1.0); defaulting to 0.5"
            );
            ShareRatio::new(1.0, 1.0).expect("1:1 is valid")
        } else {
            ShareRatio::new(f, 1.0 - f).expect("validated fraction in (0, 1)")
        }
    }

    /// Return the focused terminal's PaneId, if the currently focused pane is a terminal.
    /// Used to record which terminal a plugin was spawned alongside, so CdRequest can
    /// route directly to it without a tile-tree walk.
    fn focused_terminal_id(&self, active: usize) -> Option<PaneId> {
        let ctx = &self.windows[active];
        let tile_id = ctx.focused_pane?;
        let pane_id = match ctx.tree.tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(id)) => *id,
            _ => return None,
        };
        if ctx.panes.get(&pane_id)?.as_terminal().is_some() {
            Some(pane_id)
        } else {
            None
        }
    }

    /// Allocate the new view pane's id from `HostModel` and derive
    /// `(pane_id, vertical, new_pane_first)` from the layout `hint`.
    fn open_pane_layout(&mut self, plugin_id: &str, hint: Option<&str>) -> (PaneId, bool, bool) {
        let new_pane_first = matches!(hint, Some("split_above") | Some("split_left"));
        let vertical = matches!(
            hint,
            Some("split_h") | Some("split_right") | Some("split_left")
        );
        let pane_id = self.host.alloc_pane_id();
        log::debug!(
            "open_pane_layout({plugin_id}): pane_id={pane_id} vertical={vertical} new_pane_first={new_pane_first}"
        );
        (pane_id, vertical, new_pane_first)
    }

    /// Single placement authority for a freshly-built view pane. Honors the
    /// layout `hint` identically for every runtime (builtin, Python-WASM, native
    /// WASM) so no open path can silently diverge:
    ///
    /// * `Some("overlay")` — replace the focused pane in place, reusing its pane
    ///   id and stashing the displaced pane in `overlay_replaced` so closing the
    ///   plugin restores it (see `PlexiApp::restore_overlay_replacement`). Falls through to
    ///   the root/split path when the context is empty.
    /// * `Some("tab")` — add a tab beside the focused pane; an empty context
    ///   gets a root pane.
    /// * any other hint — split beside the focused pane, or install as the tree
    ///   root when no pane is focused.
    ///
    /// `build(pane_id, linked_pane_id, overlay_replaced) -> Pane` constructs the
    /// runtime-specific pane once its final id is known — overlay reuses the
    /// focused id; every other path allocates a fresh one via
    /// [`Self::open_pane_layout`]. Returns the installed pane id, or `None` when
    /// an overlay was requested against a degenerate focus target (the focused
    /// tile is not a pane, or its pane is missing from the map); callers treat
    /// that as a failed launch.
    fn place_view_pane<F>(
        &mut self,
        target_win: usize,
        plugin_id: &str,
        hint: Option<&str>,
        share_hint: Option<f32>,
        build: F,
    ) -> Option<PaneId>
    where
        F: FnOnce(PaneId, Option<PaneId>, Option<Box<Pane>>) -> Pane,
    {
        let active = target_win;

        if matches!(hint, Some("overlay")) {
            if let Some(focused_tile) = self.windows[active].focused_pane {
                let Some(Tile::Pane(focused_pane_id)) =
                    self.windows[active].tree.tiles.get(focused_tile)
                else {
                    log::warn!(
                        "plugin::{plugin_id}: overlay launch skipped — focused tile is not a pane"
                    );
                    return None;
                };
                let pane_id = *focused_pane_id;
                if !self.windows[active].push_overlay(pane_id, |covered| {
                    build(pane_id, None, Some(Box::new(covered)))
                }) {
                    log::warn!(
                        "plugin::{plugin_id}: overlay launch skipped — pane {pane_id} missing from pane map"
                    );
                    return None;
                }
                self.set_window_focused_pane(active, focused_tile);
                log::info!("plugin::{plugin_id}: launched as overlay on pane {pane_id}");
                return Some(pane_id);
            }
            log::info!(
                "plugin::{plugin_id}: overlay requested but empty context — launching as root pane"
            );
            // fall through to the root/split path below
        }

        if let Some(focused) = self.windows[active].focused_pane {
            let window = &self.windows[active];
            let valid = window
                .find_first_pane_in(focused)
                .and_then(|tile| window.tree.tiles.get(tile))
                .is_some_and(
                    |tile| matches!(tile, Tile::Pane(id) if window.panes.contains_key(id)),
                );
            if !valid {
                log::warn!("plugin::{plugin_id}: launch skipped — stale focus target");
                return None;
            }
        }

        if self.windows[active].zoomed_pane.take().is_some() {
            log::info!("plugin::{plugin_id}: cleared zoom before launch");
        }

        // Record which terminal we're splitting from before focus moves.
        let linked_pane_id = self.focused_terminal_id(active);
        if matches!(hint, Some("tab")) {
            let focused = self.windows[active]
                .focused_pane
                .and_then(|tile| self.windows[active].find_first_pane_in(tile));
            if let Some(focused) = focused {
                let new_id = self.host.alloc_pane_id();
                let pane = build(new_id, linked_pane_id, None);
                if self.windows[active].panes.insert(new_id, pane).is_err() {
                    return None;
                }
                // Same history step a split records, so Back returns here.
                let window_id = self.windows[active].window_id;
                let old_focus = self.windows[active].focused_pane;
                self.push_focus_history(window_id, old_focus);
                self.insert_pane_as_tab(active, focused, new_id);
                log::info!("plugin::{plugin_id}: launched as tab pane {new_id}");
                return Some(new_id);
            }
        }
        let share = Self::share_ratio_from_fraction(plugin_id, share_hint);
        let (new_id, vertical, new_pane_first) = self.open_pane_layout(plugin_id, hint);
        let pane = build(new_id, linked_pane_id, None);
        if self.windows[active].panes.insert(new_id, pane).is_err() {
            return None;
        }

        // Empty context: no focused pane means no existing tile to split against —
        // install the new pane directly as the tree root.
        if self.windows[active].focused_pane.is_none() {
            let ctx = &mut self.windows[active];
            let root_tile = ctx.tree.tiles.insert_pane(new_id);
            ctx.tree.root = Some(root_tile);
            ctx.focused_pane = Some(root_tile);
            ctx.simplify_and_reconcile();
            log::info!("plugin::{plugin_id}: launched as root pane {new_id} (empty context)");
            return Some(new_id);
        }
        let _ =
            self.split_with_new_pane_at(target_win, new_id, vertical, share, new_pane_first, false);
        log::info!("plugin::{plugin_id}: launched as split pane {new_id}");
        Some(new_id)
    }

    /// Open a built-in error tile pane when a capability pre-flight check fails.
    /// Uses `ViewRuntime::Builtin` — no plugin runtime is spawned.
    fn open_launch_failed_pane(
        &mut self,
        target_win: usize,
        plugin_id: &str,
        hint: Option<&str>,
        missing: Vec<String>,
        workspace_root: std::path::PathBuf,
    ) {
        let active = target_win;
        let share = Self::share_ratio_from_fraction(plugin_id, None);
        let (new_id, vertical, new_pane_first) = self.open_pane_layout(plugin_id, hint);
        if self.windows[active]
            .panes
            .insert(
                new_id,
                crate::host::pane::Pane::View(Box::new(crate::host::pane::ViewPane {
                    incarnation: Default::default(),
                    pip_status: None,
                    id: new_id,
                    runtime: crate::host::pane::ViewRuntime::Builtin(Box::new(
                        crate::host::launch_failed::LaunchFailedView {
                            plugin_id: plugin_id.to_string(),
                            missing,
                        },
                    )),
                    workspace_root,
                    permissions: crate::app::permissions::PluginPermissions::default(),
                    manifest_id: plugin_id.to_string(),
                    name: format!("Cannot launch {plugin_id}"),
                    pane_group: None,
                    linked_pane_id: None,
                    overlay_replaced: None,
                    agent: None,
                    slots: std::collections::HashMap::new(),
                    semantic_state: Default::default(),
                })),
            )
            .is_err()
        {
            return;
        }
        if self.windows[active].focused_pane.is_none() {
            let ctx = &mut self.windows[active];
            let root_tile = ctx.tree.tiles.insert_pane(new_id);
            ctx.tree.root = Some(root_tile);
            ctx.focused_pane = Some(root_tile);
            ctx.simplify_and_reconcile();
            log::info!("plugin::{plugin_id}: launch-failed tile inserted as root pane {new_id}");
            return;
        }
        let _ =
            self.split_with_new_pane_at(target_win, new_id, vertical, share, new_pane_first, false);
        log::info!("plugin::{plugin_id}: launch-failed tile inserted as pane {new_id}");
    }

    /// Open a sandboxed WASM component plugin in a new pane (the `run` primitive,
    /// G6). Loads `wasm_path` into a per-pane `wasmtime::Store`, links only the
    /// standard grants, and drives it through the synchronous effect loop.
    /// State is ephemeral — the namespace lives only as long as the pane, per
    /// the spec's default `run` lifecycle (`--persist` is a later addition).
    /// Returns the new pane id, or an error string if the component fails to
    /// load or instantiate (surfaced to the caller rather than crashing).
    ///
    /// `reviewed` holds imports the person just allowed in the review dialog,
    /// with the evidence of that click (stint 1398). They are recorded only
    /// inside the admitted start,
    /// so a plugin whose trust was revoked while the dialog waited gets neither
    /// a guest nor the grants.
    pub(crate) fn open_wasm_view_pane(
        &mut self,
        plugin_id: &str,
        wasm_path: &std::path::Path,
        workspace_root: PathBuf,
        launch_args: Vec<String>,
        reviewed: Option<(&[String], crate::broker::consent::CoveredDecision<'_>)>,
    ) -> Result<PaneId, String> {
        let reviewed_imports = reviewed.map_or(&[][..], |(imports, _)| imports);
        self.require_plugin(plugin_id)?;
        use crate::host::wasm_view::{StateStore, WasmView};

        require_wasm_experiment()?;
        if let Some(feature) = release_feature_for_plugin_id(plugin_id) {
            if !crate::release::feature_enabled(feature) {
                return Err(crate::release::feature_unavailable_message(feature));
            }
        }

        // Resolve the component's owner from disk now (stint 1398): the id the
        // caller passed must still own it.
        let resolved = crate::app::registry::resolve_component(wasm_path)?;
        if resolved.owner != plugin_id {
            return Err(format!(
                "{} now belongs to '{}', not '{}'",
                wasm_path.display(),
                crate::host::guest_text::shown(&resolved.owner),
                crate::host::guest_text::shown(plugin_id)
            ));
        }
        let store = StateStore::ephemeral();
        let snapshot = store.snapshot();
        let permission_store =
            crate::app::permissions::PermissionStore::load_or_default(&crate::config::config_dir())
                .map_err(|error| error.to_string())?;
        let required_grants = WasmView::inspect_required_grants(wasm_path)
            .map_err(|e| format!("inspect {}: {e}", wasm_path.display()))?;
        let required_caps = required_grants.capability_ids();
        let declared: std::collections::HashSet<String> = required_caps.iter().cloned().collect();
        let (remembered_grants, remembered_blocks) =
            gate_wasm_sets(plugin_id, &workspace_root, &declared);
        let missing: Vec<String> = required_caps
            .iter()
            .filter(|cap| !remembered_grants.contains(*cap) && !reviewed_imports.contains(cap))
            .cloned()
            .collect();
        if !missing.is_empty() {
            log::warn!(
                "plugin::{plugin_id}: raw wasm launch requires review for {missing:?} path={}",
                wasm_path.display()
            );
            return Err(format!(
                "raw WASM launch requires review for: {}",
                missing.join(", ")
            ));
        }
        let app = crate::host::plugin_admission::PluginAdmission::current().start_live_plugin(
            &resolved,
            |admitted| {
                // The reviewed imports are recorded only once the component
                // has instantiated, before the start releases the admission
                // lock: a failed load leaves no grants (stint 1733).
                WasmView::load_with_grants_then(admitted, store, required_grants, || {
                    let Some((imports, decision)) = reviewed else {
                        return Ok(());
                    };
                    record_reviewed_wasm_imports(decision, plugin_id, &workspace_root, imports)
                })
                .map_err(|e| format!("load {}: {e}", wasm_path.display()))
            },
        )?;
        let (permission_store, remembered_grants, remembered_blocks) =
            if reviewed_imports.is_empty() {
                (permission_store, remembered_grants, remembered_blocks)
            } else {
                let (granted, blocked) = gate_wasm_sets(plugin_id, &workspace_root, &declared);
                let store = crate::app::permissions::PermissionStore::load_or_default(
                    &crate::config::config_dir(),
                )
                .map_err(|error| error.to_string())?;
                (store, granted, blocked)
            };
        self.open_wasm_view_instance(WasmViewInstanceSpec {
            target_win: self.active_window,
            plugin_id,
            app,
            snapshot,
            workspace_root,
            permissions: crate::app::permissions::PluginPermissions::default(),
            remembered: Some((permission_store, remembered_grants, remembered_blocks)),
            layout: None,
            launch_args,
        })
    }

    /// Open the Python plugin `resolved` names. Its manifest was parsed once,
    /// by the resolver, and drives config and capabilities together; rows are
    /// written only once admission decides the start (stint 1733).
    fn open_python_wasm_view_pane(
        &mut self,
        target_win: usize,
        opened: crate::host::plugin_admission::OpenedBy,
        resolved: crate::app::registry::ResolvedPlugin,
        workspace_root: PathBuf,
        layout: Option<&str>,
        launch_args: Vec<String>,
    ) -> Result<PaneId, String> {
        let plugin_dir = resolved.root.clone();
        let config = python_launch_config(&resolved)?;
        // Seed the pane's live context root from the launching context; the
        // render loop refreshes it every frame thereafter (stint 0652).
        let context_root = self
            .context_root_for(self.windows[target_win].context_id)
            .ok_or_else(|| "launch context not found".to_string())?;
        let colors = self.colors;
        let profile_dir = crate::config::config_dir();
        let (mut runtime, permissions) = crate::host::plugin_admission::PluginAdmission::current()
            .start_opened(opened, &resolved, |admitted| {
                build_python_runtime(
                    admitted,
                    config,
                    &profile_dir,
                    workspace_root.clone(),
                    context_root,
                    launch_args,
                    &colors,
                )
            })?;
        let plugin_id = runtime.origin().owner().to_string();
        // Captured before the runtime moves into the pane closure; the drain
        // pass re-syncs these if the pane's context root changes later.
        let state_paths = runtime.state_paths();
        let manifest_id = plugin_id.clone();
        let name = plugin_id.clone();
        let context_id = self.windows[target_win].context_id;
        let new_id = self
            .place_view_pane(
                target_win,
                &plugin_id,
                layout,
                None,
                move |new_id, linked_pane_id, overlay_replaced| {
                    runtime.set_pipe_owner(crate::host::scope::Scope::Pane {
                        pane_id: new_id,
                        context_id,
                    });
                    Pane::View(Box::new(crate::host::pane::ViewPane {
                        incarnation: Default::default(),
                        pip_status: None,
                        id: new_id,
                        runtime: crate::host::pane::ViewRuntime::Python(Box::new(runtime)),
                        workspace_root,
                        permissions,
                        manifest_id,
                        name,
                        pane_group: None,
                        linked_pane_id,
                        overlay_replaced,
                        agent: None,
                        slots: std::collections::HashMap::new(),
                        semantic_state: Default::default(),
                    }))
                },
            )
            .ok_or_else(|| format!("overlay launch target unavailable for {plugin_id}"))?;
        self.hot_reload.watch(new_id, &plugin_dir);
        // State files (stint 0644): watch for external writes so CLI/agent
        // edits reach the running plugin without a relaunch.
        self.state_watch.watch(new_id, &state_paths);
        let context_root = self
            .origin_for_pane(new_id)
            .map(|origin| origin.context_root);
        crate::host::event_log::emit_scoped(
            crate::host::event_log::HostEvent::ViewOpened {
                plugin_id: plugin_id.clone(),
                type_id: "python-wasm".to_string(),
                pane_id: new_id,
                timestamp: crate::host::event_log::now_timestamp(),
            },
            context_root.as_deref(),
        );
        log::info!("plugin::{plugin_id}: launched CPython WASM pane {new_id}");
        Ok(new_id)
    }

    fn review_or_open_raw_wasm_view_pane(
        &mut self,
        plugin_id: &str,
        wasm_path: &std::path::Path,
        workspace_root: PathBuf,
        launch_args: Vec<String>,
    ) -> Result<(), String> {
        self.require_plugin(plugin_id)?;
        let required_grants = crate::host::wasm_view::WasmView::inspect_required_grants(wasm_path)
            .map_err(|e| format!("inspect {}: {e}", wasm_path.display()))?;
        let required_caps = required_grants.capability_ids();
        let declared: HashSet<String> = required_caps.iter().cloned().collect();
        let (remembered_grants, remembered_blocks) =
            gate_wasm_sets(plugin_id, &workspace_root, &declared);

        let blocked: Vec<String> = required_caps
            .iter()
            .filter(|cap| remembered_blocks.contains(*cap))
            .cloned()
            .collect();
        if !blocked.is_empty() {
            log::warn!(
                "raw_wasm_review: launch blocked by remembered denial plugin_id={plugin_id} path={} blocked={blocked:?}",
                wasm_path.display()
            );
            return Err(format!(
                "raw WASM launch blocked by remembered denial for: {}",
                blocked.join(", ")
            ));
        }

        let missing: Vec<String> = required_caps
            .iter()
            .filter(|cap| !remembered_grants.contains(*cap))
            .cloned()
            .collect();
        if missing.is_empty() {
            return self
                .open_wasm_view_pane(plugin_id, wasm_path, workspace_root, launch_args, None)
                .map(|_| ());
        }

        let already_pending = self.pending_raw_wasm_launches.iter().any(|pending| {
            pending.plugin_id == plugin_id
                && pending.wasm_path == wasm_path
                && pending.workspace_root == workspace_root
        });
        if !already_pending {
            log::info!(
                "raw_wasm_review: queued plugin_id={plugin_id} path={} missing={missing:?}",
                wasm_path.display()
            );
            self.pending_raw_wasm_launches
                .push_back(crate::app::PendingRawWasmLaunch {
                    request_id: crate::broker::consent::next_consent_request_id(),
                    plugin_id: plugin_id.to_string(),
                    wasm_path: wasm_path.to_path_buf(),
                    workspace_root,
                    missing_capabilities: missing,
                    launch_args,
                });
        }
        Ok(())
    }

    fn open_wasm_view_instance(
        &mut self,
        spec: WasmViewInstanceSpec<'_>,
    ) -> Result<PaneId, String> {
        use crate::host::wasm_pane::{LiveWasmPane, SysinfoStats, WasmPane};
        let WasmViewInstanceSpec {
            target_win,
            plugin_id,
            app,
            snapshot,
            workspace_root,
            permissions,
            remembered,
            layout,
            launch_args,
        } = spec;

        let pane = WasmPane::new(app, Box::new(SysinfoStats::new()));
        let pane = if let Some((store, granted, blocked)) = remembered {
            pane.with_remembered_capabilities(workspace_root.clone(), store, granted, blocked)
        } else {
            pane
        };
        let mut live = LiveWasmPane::new(pane, plugin_id, snapshot, launch_args);

        let manifest_id = plugin_id.to_string();
        let name = plugin_id.to_string();
        // Runtime scope and pane placement share the explicit target.
        let context_id = self.windows[target_win].context_id;
        let new_id = self
            .place_view_pane(
                target_win,
                plugin_id,
                layout,
                None,
                move |new_id, linked_pane_id, overlay_replaced| {
                    live.set_pane_id(new_id);
                    live.set_context_id(context_id);
                    Pane::View(Box::new(crate::host::pane::ViewPane {
                        incarnation: Default::default(),
                        pip_status: None,
                        id: new_id,
                        runtime: crate::host::pane::ViewRuntime::Wasm(Box::new(live)),
                        workspace_root,
                        permissions,
                        manifest_id,
                        name,
                        pane_group: None,
                        linked_pane_id,
                        overlay_replaced,
                        agent: None,
                        slots: std::collections::HashMap::new(),
                        semantic_state: Default::default(),
                    }))
                },
            )
            .ok_or_else(|| format!("overlay launch target unavailable for {plugin_id}"))?;
        let context_root = self
            .origin_for_pane(new_id)
            .map(|origin| origin.context_root);
        crate::host::event_log::emit_scoped(
            crate::host::event_log::HostEvent::ViewOpened {
                plugin_id: plugin_id.to_string(),
                type_id: "wasm".to_string(),
                pane_id: new_id,
                timestamp: crate::host::event_log::now_timestamp(),
            },
            context_root.as_deref(),
        );
        log::info!("wasm::{plugin_id}: spawned pane {new_id}");
        Ok(new_id)
    }

    fn open_installed_wasm_view_pane(
        &mut self,
        target_win: usize,
        opened: crate::host::plugin_admission::OpenedBy,
        installed: crate::app::registry::InstalledPlugin,
        workspace_root: PathBuf,
        layout: Option<&str>,
        launch_args: Vec<String>,
    ) -> Result<PaneId, String> {
        use crate::app::permissions::{Capability, PermissionStore};
        use crate::host::wasm_view::{Grants, StateStore, WasmView};

        let plugin_id = installed.manifest.id.clone();

        require_wasm_experiment()?;
        if let Some(feature) = release_feature_for_plugin_id(&plugin_id) {
            if !crate::release::feature_enabled(feature) {
                return Err(crate::release::feature_unavailable_message(feature));
            }
        }

        // Cloud/remote execution is future work (stints 0286/0287): no
        // server-side wasmtime runtime or thin-client streaming exists yet.
        // Fail loudly at the launch boundary rather than silently running a
        // `execution = "cloud"`/`"preferred-local"` plugin locally — the Python
        // launch path already rejects the same shape in
        // `PythonLaunchConfig::from_manifest_file`.
        if installed.runtime.execution != crate::app::registry::RuntimeExecution::Local {
            return Err(format!(
                "plugin '{plugin_id}' declares runtime execution '{}', which is not available yet — \
                 only local execution is supported (cloud streaming is future work)",
                installed.runtime.execution.as_str()
            ));
        }
        // Resolve the component's owner from disk now (stint 1398).
        let resolved = crate::app::registry::resolve_component(&installed.bin_path)?;
        if resolved.owner != plugin_id {
            return Err(format!(
                "{} now belongs to '{}', not '{}'",
                installed.bin_path.display(),
                crate::host::guest_text::shown(&resolved.owner),
                crate::host::guest_text::shown(&plugin_id)
            ));
        }
        let config_dir = crate::config::config_dir();
        let store = StateStore::persistent(wasm_state_path(&plugin_id, &workspace_root))
            .map_err(|e| format!("open wasm state for {plugin_id}: {e}"))?;
        let snapshot = store.snapshot();
        // Capability rows are planned inside the admitted start and written
        // only once the component starts, so a refused or failed start keeps
        // none (stint 1733).
        let (app, permissions, remembered_grants, remembered_blocks) =
            crate::host::plugin_admission::PluginAdmission::current().start_opened(
                opened,
                &resolved,
                |admitted| {
                    let (mut permissions, mut mints) = wasm_manifest_permissions(
                        &config_dir,
                        &plugin_id,
                        &workspace_root,
                        &installed.manifest.capabilities.capabilities,
                        installed.manifest.capabilities.allowed_hosts.clone(),
                    )?;
                    permissions
                        .capabilities
                        .retain(|cap| !permissions.blocked.contains(cap));
                    let wasm_declared =
                        wasm_runtime_capabilities_from_permissions(&permissions, &workspace_root);
                    let monitor = crate::broker::gate::PermissionMonitor::for_profile(&config_dir)
                        .map_err(|error| error.to_string())?;
                    let (mut remembered_grants, remembered_blocks, mint) = monitor.plan_wasm_sets(
                        &crate::broker::principal::PluginInstance::new(&plugin_id, None),
                        &workspace_root,
                        &wasm_declared,
                    );
                    // The manifest rows this start mints are not stored yet,
                    // so the stored-row read above cannot return them.
                    remembered_grants.extend(mints.capability_ids().map(str::to_string));
                    mints.add(monitor, mint);
                    let grants = Grants {
                        state: true,
                        pipes: permissions.capabilities.contains(&Capability::PipeOpen),
                        gpu: permissions.capabilities.contains(&Capability::GpuRender),
                        audio: permissions
                            .capabilities
                            .contains(&Capability::AudioPlayback),
                    };
                    // Rows are written once the component has instantiated,
                    // still under the admission lock (stint 1733).
                    let app = WasmView::load_with_grants_then(admitted, store, grants, || {
                        mints.commit();
                        Ok(())
                    })
                    .map_err(|e| format!("load {}: {e}", installed.bin_path.display()))?;
                    Ok((app, permissions, remembered_grants, remembered_blocks))
                },
            )?;
        let permission_store =
            PermissionStore::load_or_default(&config_dir).map_err(|error| error.to_string())?;

        log::info!(
            "wasm::{plugin_id}: manifest-backed launch entry={} remembered_grants={} remembered_blocks={}",
            installed.bin_path.display(),
            remembered_grants.len(),
            remembered_blocks.len()
        );
        self.open_wasm_view_instance(WasmViewInstanceSpec {
            target_win,
            plugin_id: &plugin_id,
            app,
            snapshot,
            workspace_root,
            permissions,
            remembered: Some((permission_store, remembered_grants, remembered_blocks)),
            layout,
            launch_args,
        })
    }

    /// Relaunch a CPython-in-WASM pane while preserving its pane envelope.
    pub(crate) fn reload_view_pane(&mut self, pane_id: PaneId, reason: &str) -> bool {
        let active = self.active_window;
        let Some(view_pane) = self.windows[active]
            .panes
            .get_mut(&pane_id)
            .and_then(|pane| pane.as_view_mut())
        else {
            return false;
        };
        let crate::host::pane::ViewRuntime::Python(runtime) = &mut view_pane.runtime else {
            return false;
        };
        log::info!(
            "plugin::{} reload triggered ({reason}) for CPython WASM pane {pane_id}",
            view_pane.manifest_id
        );
        // A reload is a new guest start: the pane's own plugin, resolved from
        // disk now, must still be admitted (its trust may have been revoked).
        // Edits in a trusted folder never ask again (stint 1398). A refusal
        // keeps the running incarnation.
        let relaunched =
            crate::app::registry::resolve_plugin_dir(runtime.origin().root()).and_then(|resolved| {
                crate::host::plugin_admission::PluginAdmission::current().start_live_plugin(
                    &resolved,
                    |admitted| {
                        runtime
                            .relaunch(admitted)
                            .map_err(|error| error.to_string())
                    },
                )
            });
        if let Err(error) = relaunched {
            log::info!("plugin::{} reload refused: {error}", view_pane.manifest_id);
            return false;
        }
        // A relaunched guest is a new emitter: requests the old guest left
        // pending are retired, and nothing answers them to the new one
        // (stint 1632).
        let retired = crate::app::view_trait::ViewEmitter {
            pane_id,
            incarnation: view_pane.incarnation,
        };
        let plugin_id = view_pane.manifest_id.clone();
        view_pane.incarnation = Default::default();
        self.retire_commands_from(retired);
        // Nor does the new guest inherit the old one's directed pipes.
        self.retire_directed_pipes_for(
            &[(retired, plugin_id)],
            crate::protocol::PipeCloseReason::PeerReloaded,
        );
        true
    }

    /// Drain pending `ReloadRequest`s from the watcher channel and reload
    /// the matching panes. Called once per frame from the host update loop.
    pub(crate) fn drain_hot_reload_requests(&mut self) {
        while let Ok(req) = self.hot_reload_rx.try_recv() {
            self.reload_view_pane(req.pane_id, "watcher");
        }
    }

    /// Drain pending `StateChangedNotice`s from the state watcher and re-read
    /// the affected scope on the matching pane (stint 0644). In the same pass,
    /// re-sync every live Python pane's watcher registration against its
    /// currently-resolved state paths — `plexi context set-root` moves
    /// context-scoped paths without any pane event, and this drain is the one
    /// place that observes both sides. Called once per `logic` pass.
    pub(crate) fn drain_state_change_requests(&mut self) {
        while let Ok(notice) = self.state_watch_rx.try_recv() {
            let mut found = false;
            for window in &mut self.windows {
                if let Some(view_pane) = window
                    .panes
                    .get_mut(&notice.pane_id)
                    .and_then(|pane| pane.as_view_mut())
                {
                    log::info!(
                        "state_watch: external change → pane {} plugin {} scope={}",
                        notice.pane_id,
                        view_pane.manifest_id,
                        notice.scope.as_str()
                    );
                    view_pane.runtime.apply_external_state(notice.scope);
                    found = true;
                    break;
                }
            }
            if !found {
                // Pane closed between the notice and the drain — the unwatch
                // on close already tore the watcher down; nothing to do.
                log::debug!(
                    "state_watch: notice for unknown pane {} dropped (pane closed)",
                    notice.pane_id
                );
            }
        }

        // Registration re-sync: covers context-root moves.
        let mut stale: Vec<(
            PaneId,
            Vec<(crate::host::state_scope::StateScope, std::path::PathBuf)>,
        )> = Vec::new();
        for window in &self.windows {
            for pane in window.panes.values() {
                let Some(view_pane) = pane.as_view() else {
                    continue;
                };
                let current = view_pane.runtime.state_paths();
                if current.is_empty() {
                    continue;
                }
                if self.state_watch.watched_entries(view_pane.id) != current.as_slice() {
                    stale.push((view_pane.id, current));
                }
            }
        }
        for (pane_id, entries) in stale {
            log::info!("state_watch: re-registering pane {pane_id} — resolved state paths changed");
            self.state_watch.watch(pane_id, &entries);
        }
    }

    /// Detect crashes on watched panes and restart them after a brief delay
    /// so the developer can read the crash overlay. Called once per frame
    /// alongside `drain_hot_reload_requests`.
    pub(crate) fn drain_crash_restarts(&mut self) {}

    /// Force-reload the focused view pane (manual trigger via Cmd+Option+R).
    /// No-op when the focused pane isn't a process-backed ViewPane.
    pub(crate) fn force_reload_focused_view(&mut self) {
        let active = self.active_window;
        let Some(focused_tile) = self.windows[active].focused_pane else {
            return;
        };
        let Some(Tile::Pane(pane_id)) = self.windows[active].tree.tiles.get(focused_tile) else {
            return;
        };
        let pane_id = *pane_id;
        if self.config.core_only.unwrap_or(false) {
            if let Some(plugin) = self.windows[active]
                .panes
                .get(&pane_id)
                .and_then(|pane| pane.as_view())
                .map(|pane| pane.manifest_id.clone())
            {
                self.plugin_blocked(&plugin);
                return;
            }
        }
        self.reload_view_pane(pane_id, "manual");
    }

    pub(crate) fn open_builtin_view_pane(
        &mut self,
        app: Box<dyn View>,
        permissions: crate::app::permissions::PluginPermissions,
        workspace_root: PathBuf,
        group: Option<String>,
        hint: Option<&str>,
        share: Option<f32>,
    ) {
        if let Err(reason) = self.open_builtin_view_pane_at(
            BuiltinPlacement {
                window: self.active_window,
                hint,
                share,
            },
            app,
            permissions,
            workspace_root,
            group,
        ) {
            log::info!("open_builtin_view_pane: not opened: {reason}");
        }
    }

    /// Place a builtin view and return the pane it landed in, or why it did
    /// not open (core-only refusal, no pane to cover).
    fn open_builtin_view_pane_at(
        &mut self,
        placement: BuiltinPlacement<'_>,
        mut app: Box<dyn View>,
        permissions: crate::app::permissions::PluginPermissions,
        workspace_root: PathBuf,
        group: Option<String>,
    ) -> Result<PaneId, String> {
        self.require_plugin(app.type_id())?;
        let BuiltinPlacement {
            window: target_win,
            hint,
            share,
        } = placement;
        let plugin_type_id = app.type_id().to_string();
        let app_name = app.display_name();
        let manifest_id = plugin_type_id.clone();
        let context_id = self.windows[target_win].context_id;
        let placed = self.place_view_pane(
            target_win,
            &plugin_type_id,
            hint,
            share,
            move |id, linked_pane_id, overlay_replaced| {
                app.bind_host_identity(id, context_id);
                Pane::View(Box::new(crate::host::pane::ViewPane {
                    incarnation: Default::default(),
                    pip_status: None,
                    id,
                    runtime: crate::host::pane::ViewRuntime::Builtin(app),
                    workspace_root,
                    permissions,
                    manifest_id,
                    name: app_name,
                    pane_group: group,
                    linked_pane_id,
                    overlay_replaced,
                    agent: None,
                    slots: std::collections::HashMap::new(),
                    semantic_state: Default::default(),
                }))
            },
        );
        let pane_id = placed.ok_or_else(|| {
            format!("the {plugin_type_id} view had no pane to open in")
        })?;
        let context_root = self
            .origin_for_pane(pane_id)
            .map(|origin| origin.context_root);
        crate::host::event_log::emit_scoped(
            crate::host::event_log::HostEvent::ViewOpened {
                plugin_id: plugin_type_id.clone(),
                type_id: plugin_type_id.clone(),
                pane_id,
                timestamp: crate::host::event_log::now_timestamp(),
            },
            context_root.as_deref(),
        );
        Ok(pane_id)
    }

    /// Build a window's worth of terminal panes for a context that is **not**
    /// the active one — typically one that does not exist in the router yet.
    ///
    /// [`create_single_pane_tree`](Self::create_single_pane_tree) stamps
    /// `PLEXI_CONTEXT_*` from `active_window`, which is wrong for a child
    /// context being constructed: its panes would advertise the parent's id.
    /// This takes the context identity explicitly instead, per the root
    /// AGENTS.md rule against steering helpers by mutating global focus state.
    ///
    /// One pane per entry in `commands`, in order; `None` launches a plain
    /// shell. Returns the tiled tree, the pane map, the first pane's tile (the
    /// window's initial focus), and the pane ids in creation order.
    ///
    /// Returns `None` if any terminal fails to spawn — partial squads are never
    /// installed. Panes already built are dropped, closing their PTYs.
    pub(super) fn create_context_pane_set(
        &mut self,
        context: &PaneContextEnv,
        cwd: PathBuf,
        commands: &[Option<String>],
        layout: crate::protocol::SubContextLayout,
        source: &crate::broker::pane_authority::GrantSource,
    ) -> Option<ContextPaneSet> {
        if commands.is_empty() {
            log::error!("create_context_pane_set: refusing to build a window with zero panes");
            return None;
        }
        let total = commands.len();
        let mut panes: HashMap<PaneId, Pane> = HashMap::new();
        let mut pane_ids: Vec<PaneId> = Vec::with_capacity(total);
        for (idx, initial_cmd) in commands.iter().enumerate() {
            let launch = self.terminal_launch(
                source.clone(),
                context.clone(),
                Some(cwd.clone()),
                initial_cmd.as_ref().map(|command| LaunchCommand {
                    command: command.clone(),
                    close_on_exit: false,
                }),
            );
            let new_id = launch.pane_id();
            // A failure drops the panes already built: their credentials are
            // revoked and their shells released. No partial squad installs.
            let mut pane = match self.construct_terminal(launch) {
                Ok(pane) => pane,
                Err(error) => {
                    log::error!(
                        "create_context_pane_set: construction failed for pane {} of {total} \
                     (pane_id={new_id} context_id={}) — discarding \
                     {} already-created pane(s) {:?}, no partial squad is installed: {error}",
                        idx + 1,
                        context.context_id,
                        pane_ids.len(),
                        pane_ids
                    );
                    return None;
                }
            };
            let pane_name = super::layout::default_squad_pane_name(&context.name, idx);
            pane.name = Some(pane_name.clone());
            pane.name_locked = true;
            log::info!(
                "create_context_pane_set: spawned pane {} of {total} pane_id={new_id} \
                 name={pane_name:?} context_id={}",
                idx + 1,
                context.context_id
            );
            panes.insert(new_id, Pane::Terminal(Box::new(pane)));
            pane_ids.push(new_id);
        }
        let (tree, first_tile) = super::layout::build_squad_tree(&pane_ids, layout);
        log::info!(
            "create_context_pane_set: context_id={} panes={:?} layout={layout:?} cwd={}",
            context.context_id,
            pane_ids,
            cwd.display()
        );
        Some((tree, panes, first_tile, pane_ids))
    }

    /// Construct `launch` into a single-pane tree. `None` when construction
    /// fails; nothing it acquired survives.
    pub(super) fn create_single_pane_tree(
        &mut self,
        launch: TerminalLaunch,
    ) -> Option<(Tree<PaneId>, HashMap<PaneId, Pane>, TileId)> {
        let new_id = launch.pane_id();
        let pane = match self.construct_terminal(launch) {
            Ok(pane) => pane,
            Err(error) => {
                log::error!("create_single_pane_tree: pane_id={new_id}: {error}");
                return None;
            }
        };
        let mut panes = HashMap::new();
        panes.insert(new_id, Pane::Terminal(Box::new(pane)));
        let mut tiles = egui_tiles::Tiles::default();
        let root_tile = tiles.insert_pane(new_id);
        let tree = Tree::new("plexi", root_tile, tiles);
        Some((tree, panes, root_tile))
    }

    /// Construct `launch` and install it beside `at.target_tile`.
    pub(crate) fn spawn_terminal_pane_at(
        &mut self,
        at: SplitPlacement,
        launch: TerminalLaunch,
    ) -> Result<PaneId, String> {
        let pane = self
            .construct_terminal(launch)
            .map_err(|error| error.to_string())?;
        self.install_split(at, pane)
    }

    /// Install a constructed terminal beside `at.target_tile` in `at.win_idx`.
    /// Reads and writes no `active_window`; `keep_focus` leaves the window's
    /// focus alone. Returns the installed pane's id.
    pub(crate) fn install_split(
        &mut self,
        at: SplitPlacement,
        pane: TerminalPane,
    ) -> Result<PaneId, String> {
        let SplitPlacement {
            win_idx,
            target_tile,
            vertical,
            new_pane_first,
            keep_focus,
        } = at;
        let new_id = pane.id;
        log::info!(
            "install_split: win_idx={win_idx} target_tile={target_tile:?} pane_id={new_id} \
             vertical={vertical} keep_focus={keep_focus}"
        );
        self.windows[win_idx]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(pane)))
            .map_err(|_| format!("pane id {new_id} is already installed"))?;
        let share =
            crate::host::command::ShareRatio::new(1.0, 1.0).expect("1:1 is a valid ShareRatio");
        let new_tile = super::layout::insert_split_tile(
            &mut self.windows[win_idx].tree,
            Some(target_tile),
            new_id,
            vertical,
            share,
            new_pane_first,
        );
        if !keep_focus {
            self.set_window_focused_pane(win_idx, new_tile);
        }
        // Saved settled like every other split caller (stints 1440, 1511).
        self.windows[win_idx].simplify_and_reconcile();
        Ok(new_id)
    }

    /// OSC 7501 hierarchical id for an agent spawned under `parent_pane_id`:
    /// ancestor agent labels (root lead first) then `role`. Ancestors stop at
    /// the first pane that is not an agent.
    fn agent_tree_id(&self, parent_pane_id: Option<u64>, role: &str) -> Option<String> {
        let mut labels = vec![role.to_string()];
        let mut next = parent_pane_id;
        while let Some(pid) = next {
            let Some(label) = self
                .derive_spawned_by_for_parent(Some(pid))
                .strip_prefix("agent:")
                .map(str::to_owned)
            else {
                break;
            };
            labels.push(label);
            if labels.len() > 8 {
                break;
            }
            next = self
                .windows
                .iter()
                .find_map(|w| w.panes.get(&pid))
                .and_then(|p| p.as_terminal())
                .and_then(|t| t.parent_pane_id);
        }
        labels.reverse();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        crate::program_status::emit::tree_id(&refs)
    }

    /// Derive the unspoofable `spawned_by` provenance string for a parent pane ID.
    /// A pane created by a process in another pane is that pane's, never the
    /// person's; with no parent the spawner is not known here (stint 1285).
    pub(crate) fn derive_spawned_by_for_parent(&self, parent_pane_id: Option<u64>) -> String {
        let Some(pid) = parent_pane_id else {
            return "unknown".to_string();
        };
        if let Some(term) = self.terminal_incarnation(pid) {
            if let Some(ref role) = term.agent_role {
                if !role.trim().is_empty() {
                    return format!("agent:{}", role.trim());
                }
            }
            if let Some(ref st) = term.agent {
                if !st.agent.trim().is_empty() && st.agent != "unknown" {
                    return format!("agent:{}", st.agent.trim());
                }
            }
            if let Some(ref obs) = term.observed_agent {
                if !obs.agent.trim().is_empty() && obs.agent != "unknown" {
                    return format!("agent:{}", obs.agent.trim());
                }
            }
            if let Some(ref h) = term.agent_harness {
                if !h.trim().is_empty() {
                    return format!("agent:{}", h.trim());
                }
            }
        }
        format!("pane:{pid}")
    }

    /// Record a constructed terminal's provenance in the session trace, read
    /// from its installed authority binding. Provenance is supplied at
    /// admission and stamped at construction; nothing sets it afterwards
    /// (stint 1632).
    pub(crate) fn record_pane_provenance(&self, pane_id: u64) {
        let Some((win_idx, _)) = self.find_pane_in_any_window(pane_id) else {
            return;
        };
        let window = &self.windows[win_idx];
        let Some(term) = window.panes.get(&pane_id).and_then(|p| p.as_terminal()) else {
            return;
        };
        let spawned_by = term
            .authority()
            .map(|a| a.grant().spawned_by.clone())
            .or_else(|| term.spawned_by.clone());
        crate::host::flight_recorder::record(
            "pane_provenance",
            Some(window.context_id),
            Some(pane_id),
            None,
            serde_json::json!({
                "pane_id": pane_id,
                "spawned_by": spawned_by,
                "parent_pane_id": term.parent_pane_id,
            }),
        );
    }

    /// The parent an agent spawn hangs under: the live `from_pane_id`, else the
    /// origin context's first window (or the active window). Read-only, so the
    /// authority a spawn installs and the place it lands derive from the same
    /// parent and root.
    pub(crate) fn agent_spawn_parent(
        &self,
        from_pane_id: Option<u64>,
        origin: AgentOrigin,
    ) -> Result<AgentSpawnParent, String> {
        let origin_win = match origin {
            AgentOrigin::Active => self.active_window,
            AgentOrigin::Context(id) => self
                .windows
                .iter()
                .position(|w| w.context_id == id)
                .ok_or_else(|| format!("origin context {id} not found"))?,
            AgentOrigin::Window {
                window_id,
                context_id,
            } => self
                .windows
                .iter()
                .position(|w| w.window_id == window_id && w.context_id == context_id)
                .ok_or_else(|| {
                    format!("origin window {window_id} is no longer in context {context_id}")
                })?,
        };
        let parent_lookup = from_pane_id.and_then(|pid| {
            self.find_pane_in_any_window(pid)
                .map(|(w_idx, _)| (pid, w_idx, self.windows[w_idx].context_id))
        });
        let (parent_pane_id, parent_win_idx, parent_ctx_id) = match parent_lookup {
            Some((pid, w_idx, c_id)) => (Some(pid), w_idx, c_id),
            None => (
                from_pane_id,
                origin_win,
                self.windows[origin_win].context_id,
            ),
        };
        let parent_root = self
            .router
            .position(|c| c.context_id == parent_ctx_id)
            .map(|idx| self.router.get(idx).root.clone())
            .unwrap_or_else(|| self.windows[parent_win_idx].path.clone());
        let parent_depth = self
            .router
            .position(|c| c.context_id == parent_ctx_id)
            .map(|idx| self.router.get(idx).depth)
            .unwrap_or(0);
        Ok(AgentSpawnParent {
            pane_id: parent_pane_id,
            win_idx: parent_win_idx,
            context_id: parent_ctx_id,
            root: parent_root,
            depth: parent_depth,
        })
    }

    /// The sub-context named `name` directly under `parent_ctx_id`, if any.
    pub(crate) fn agent_subcontext_under(&self, parent_ctx_id: u64, name: &str) -> Option<u64> {
        self.router
            .iter()
            .find(|c| {
                c.parent_id == Some(parent_ctx_id) && c.name.displayed().eq_ignore_ascii_case(name)
            })
            .map(|c| c.context_id)
    }

    /// Where a spawned agent pane lands, resolved without changing anything.
    /// Every check an admitted sub-context needs happens here; a sub-context
    /// that must be created is only described, and exists once the worker is
    /// installed ([`Self::install_agent_worker`]).
    pub(crate) fn resolve_agent_spawn_location(
        &self,
        subcontext: Option<SubcontextTarget<'_>>,
        from_pane_id: Option<u64>,
        cwd_override: Option<std::path::PathBuf>,
        conjoined: bool,
        origin: AgentOrigin,
    ) -> Result<AgentPlacement, String> {
        let AgentSpawnParent {
            pane_id: parent_pane_id,
            win_idx: parent_win_idx,
            context_id: parent_ctx_id,
            root: parent_root,
            depth: parent_depth,
        } = self.agent_spawn_parent(from_pane_id, origin)?;
        let effective_cwd = cwd_override.unwrap_or_else(|| parent_root.clone());

        let target = if let Some(target) = subcontext {
            let sub_name = target.name();
            let existing = self.agent_subcontext_under(parent_ctx_id, sub_name);
            // An admitted sub-context is the incarnation the caller saw: a
            // same-named replacement, or one that appeared after a create
            // intent, is a different destination (stint 1632).
            match (target, existing) {
                (SubcontextTarget::Existing { context_id, .. }, found)
                    if found != Some(context_id) =>
                {
                    return Err(format!(
                        "sub-context '{sub_name}' (id={context_id}) is no longer under context {parent_ctx_id}"
                    ));
                }
                (SubcontextTarget::Create { .. }, Some(found)) => {
                    return Err(format!(
                        "sub-context '{sub_name}' (id={found}) appeared after the launch was admitted"
                    ));
                }
                (
                    SubcontextTarget::Create {
                        parent_context_id, ..
                    },
                    None,
                ) if parent_context_id != parent_ctx_id => {
                    return Err(format!(
                        "sub-context '{sub_name}' was admitted under context {parent_context_id}, but its parent is now in context {parent_ctx_id}"
                    ));
                }
                _ => {}
            }
            match existing {
                Some(existing_ctx_id) => {
                    let win_idx = self
                        .windows
                        .iter()
                        .position(|w| w.context_id == existing_ctx_id)
                        .ok_or_else(|| {
                            format!("sub-context '{sub_name}' (id={existing_ctx_id}) has no window")
                        })?;
                    AgentTarget::Window {
                        win_idx,
                        context_id: existing_ctx_id,
                        split_at: self.agent_split_at(win_idx, parent_pane_id),
                    }
                }
                None => {
                    if conjoined && parent_pane_id.is_none() {
                        return Err("conjoined children require a parent pane".into());
                    }
                    AgentTarget::NewSubcontext {
                        name: sub_name.to_string(),
                        parent_win_idx,
                        parent_context_id: parent_ctx_id,
                        depth: parent_depth + 1,
                        conjoined,
                    }
                }
            }
        } else {
            AgentTarget::Window {
                win_idx: parent_win_idx,
                context_id: parent_ctx_id,
                split_at: self.agent_split_at(parent_win_idx, parent_pane_id),
            }
        };
        Ok(AgentPlacement {
            parent_pane_id,
            effective_cwd,
            target,
        })
    }

    /// Where a worker lands in an existing window now: beside its parent pane
    /// when that is in the window, else beside the window's focus. A planned
    /// launch replaces this with the anchor it bound at admission.
    pub(crate) fn agent_split_at(
        &self,
        win_idx: usize,
        parent_pane_id: Option<u64>,
    ) -> Option<egui_tiles::TileId> {
        let win = &self.windows[win_idx];
        parent_pane_id
            .and_then(|pid| win.tree.tiles.find_pane(&pid))
            .or(win.focused_pane)
    }

    /// The context identity a worker at `placement` is constructed with,
    /// reserving the ids of a sub-context it will create. Reserved ids that
    /// end up unused (a failed construction) are harmless: allocation is
    /// monotonic.
    fn stage_agent_destination(&mut self, placement: &AgentPlacement) -> StagedAgentDestination {
        match &placement.target {
            AgentTarget::Window { win_idx, .. } => {
                StagedAgentDestination::Window(self.pane_context_env_for_window(*win_idx))
            }
            AgentTarget::NewSubcontext { name, depth, .. } => {
                let context_id = self.next_window_id;
                let window_id = self.next_window_id + 1;
                self.next_window_id += 2;
                StagedAgentDestination::NewSubcontext {
                    context: PaneContextEnv {
                        context_id,
                        name: name.clone(),
                        description: String::new(),
                        root: Some(placement.effective_cwd.clone()),
                        depth: *depth,
                    },
                    window_id,
                }
            }
        }
    }

    /// Install a constructed worker at its staged destination, creating the
    /// sub-context (and its portal, unless conjoined) only now. Returns the
    /// worker's window index and context id.
    fn install_agent_worker(
        &mut self,
        placement: &AgentPlacement,
        staged: StagedAgentDestination,
        pane: TerminalPane,
    ) -> Result<(usize, u64), String> {
        let new_id = pane.id;
        let (target_win_idx, target_ctx_id, window_split_at) = match (&placement.target, staged) {
            (
                AgentTarget::Window {
                    win_idx,
                    context_id,
                    split_at,
                },
                StagedAgentDestination::Window(_),
            ) => (*win_idx, *context_id, Some(*split_at)),
            (
                AgentTarget::NewSubcontext {
                    name,
                    parent_win_idx,
                    parent_context_id,
                    depth,
                    conjoined,
                },
                StagedAgentDestination::NewSubcontext { context, window_id },
            ) => {
                let ctx_id = context.context_id;
                if *conjoined {
                    let parent = placement
                        .parent_pane_id
                        .ok_or("conjoined children require a parent pane")?;
                    self.conjoined_contexts.insert(ctx_id, parent);
                } else {
                    // A PortalPane in the parent window, beside the caller.
                    let portal_pane_id = self.host.alloc_pane_id();
                    self.windows[*parent_win_idx]
                        .panes
                        .insert(
                            portal_pane_id,
                            Pane::Portal(Box::new(crate::host::pane::PortalPane {
                                pane_id: portal_pane_id,
                                target_context_id: ctx_id,
                                context_state: None,
                            })),
                        )
                        .map_err(|_| format!("portal pane {portal_pane_id} id is taken"))?;
                    let split_target = placement
                        .parent_pane_id
                        .and_then(|pid| self.windows[*parent_win_idx].tree.tiles.find_pane(&pid))
                        .or(self.windows[*parent_win_idx].focused_pane);
                    super::layout::insert_split_tile(
                        &mut self.windows[*parent_win_idx].tree,
                        split_target,
                        portal_pane_id,
                        true,
                        crate::host::command::ShareRatio {
                            numerator: 1.0,
                            denominator: 1.0,
                        },
                        false,
                    );
                    // The parent window is saved settled (stint 1440).
                    self.windows[*parent_win_idx].simplify_and_reconcile();
                }
                self.router.push(crate::host::context::Context {
                    name: crate::host::context::ContextName::custom(name.clone()),
                    root: placement.effective_cwd.clone(),
                    description: None,
                    context_id: ctx_id,
                    parent_id: Some(*parent_context_id),
                    depth: *depth,
                    parked: false,
                });
                self.windows.push(crate::host::context::Window {
                    name: String::new(),
                    path: placement.effective_cwd.clone(),
                    tree: egui_tiles::Tree::empty("plexi"),
                    panes: Default::default(),
                    focused_pane: None,
                    zoomed_pane: None,
                    grid_x: 0,
                    grid_y: 0,
                    window_id,
                    context_id: ctx_id,
                    tree_history: Default::default(),
                });
                self.context_active_window.insert(ctx_id, window_id);
                (self.windows.len() - 1, ctx_id, None)
            }
            _ => return Err("agent destination changed between staging and install".into()),
        };
        self.windows[target_win_idx]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(pane)))
            .map_err(|_| format!("agent worker pane {new_id} id is taken"))?;
        // An existing window's anchor was resolved with the placement; a new
        // sub-context's window is empty.
        let split_target = window_split_at.flatten();
        let share =
            crate::host::command::ShareRatio::new(1.0, 1.0).expect("1:1 is a valid ShareRatio");
        let new_tile = super::layout::insert_split_tile(
            &mut self.windows[target_win_idx].tree,
            split_target,
            new_id,
            true,
            share,
            false,
        );
        if self.windows[target_win_idx].focused_pane.is_none() {
            self.windows[target_win_idx].focused_pane = Some(new_tile);
        }
        self.windows[target_win_idx].simplify_and_reconcile();
        Ok((target_win_idx, target_ctx_id))
    }

    /// Hand focus to a freshly started interactive agent pane (stint 1117), through
    /// the one focus authority ([`pane_navigate`](Self::pane_navigate)).
    ///
    /// Only a live caller in the user's active window may hand focus over: a stale
    /// caller id, or a caller in a background context, leaves focus untouched so an
    /// unattended launch can never steal it.
    pub(crate) fn focus_started_agent_pane(&mut self, new_pane_id: PaneId, caller: Option<u64>) {
        let active = &self.windows[self.active_window];
        let caller_in_active = caller.is_none_or(|pid| active.panes.contains_key(&pid));
        if !caller_in_active || !active.panes.contains_key(&new_pane_id) {
            log::info!(
                "agent_start: focus handoff skipped new_pane={new_pane_id} caller={caller:?} (caller or pane not in the active window)"
            );
            return;
        }
        let navigated = self.pane_navigate(new_pane_id);
        log::info!(
            "agent_start: focus handoff new_pane={new_pane_id} caller={caller:?} navigated={navigated}"
        );
    }

    /// Spawn a worker terminal pane for `plexi agent spawn` / `plexi agent start` / team / GUI,
    /// creating or reusing a sub-context and linking a `PortalPane` in the parent context when
    /// `subcontext` is specified.
    ///
    /// The role's permission ceiling is resolved once, before placement, from
    /// the same parent and working directory the pane will use.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_agent_worker_pane(
        &mut self,
        role: &str,
        harness: &str,
        cmd: &str,
        task: &str,
        subcontext: Option<&str>,
        ephemeral: bool,
        from_pane_id: Option<u64>,
        cwd_override: Option<std::path::PathBuf>,
        result_schema: Option<serde_json::Value>,
        run_token: Option<&str>,
        conjoined: bool,
        origin_context_id: Option<u64>,
    ) -> Result<(crate::spatial::tiling::PaneId, u64), String> {
        self.spawn_agent_worker_attributed(
            WorkerAttribution {
                spawned_by: None,
                anchor_pane: None,
            },
            WorkerSpawn {
                role,
                command: WorkerCommand::Raw {
                    harness,
                    command: cmd,
                    ephemeral,
                    result_schema,
                },
                task,
                subcontext,
                from_pane_id,
                cwd_override,
                run_token,
                conjoined,
                origin_context_id,
            },
        )
        .map(|worker| (worker.pane_id, worker.context_id))
    }

    /// [`Self::spawn_agent_worker_pane`] with the attribution and placement
    /// anchor its admission already decided. Neither grants anything.
    pub(crate) fn spawn_agent_worker_attributed(
        &mut self,
        attribution: WorkerAttribution,
        spawn: WorkerSpawn<'_>,
    ) -> Result<SpawnedWorker, String> {
        let WorkerAttribution {
            spawned_by,
            anchor_pane,
        } = attribution;
        let WorkerSpawn {
            role,
            command,
            task,
            subcontext,
            from_pane_id,
            cwd_override,
            run_token,
            conjoined,
            origin_context_id,
        } = spawn;
        let anchor_at = anchor_pane.and_then(|pane| {
            let (window, _) = self.find_pane_in_any_window(pane)?;
            Some((pane, window, self.windows[window].context_id))
        });
        let origin = origin_context_id
            .or(anchor_at.map(|(_, _, context_id)| context_id))
            .map_or(AgentOrigin::Active, AgentOrigin::Context);
        let mut placement = self.resolve_agent_spawn_location(
            subcontext.map(SubcontextTarget::Named),
            from_pane_id,
            cwd_override,
            conjoined,
            origin,
        )?;
        if let (Some((pane, anchor_window, anchor_context)), AgentTarget::Window { win_idx, context_id, split_at }) =
            (anchor_at, &mut placement.target)
        {
            if *context_id == anchor_context {
                *win_idx = anchor_window;
                *split_at = self.agent_split_at(anchor_window, Some(pane));
            }
        }
        // The worker's source is its verified parent's grant, a team run, or
        // an outside caller; a named role must resolve or nothing starts.
        let source = match (run_token, placement.parent_pane_id) {
            (_, Some(parent)) => self.caller_grant_source(parent),
            (Some(_), None) => crate::broker::pane_authority::GrantSource::Team,
            (None, None) => crate::broker::pane_authority::GrantSource::OutsideProcess,
        };
        let resolved;
        let (role, harness, command, ephemeral, result_schema, claude_memory, role_grant) =
            match command {
                WorkerCommand::Raw {
                    harness,
                    command,
                    ephemeral,
                    result_schema,
                } => {
                    let role_grant =
                        self.admit_defined_worker_role(role, &placement.effective_cwd)?;
                    (
                        role,
                        harness,
                        command,
                        ephemeral,
                        result_schema,
                        None,
                        role_grant,
                    )
                }
                WorkerCommand::Configured {
                    ephemeral,
                    admitted,
                } => {
                    let launch = match admitted {
                        Some(launch) => launch,
                        None => {
                            resolved = ConfiguredAgentLaunch::resolve(
                                role,
                                &placement.effective_cwd,
                                Some(task),
                                ephemeral,
                            )?;
                            &resolved
                        }
                    };
                    (
                        &*launch.role.id,
                        launch.harness.as_str(),
                        &*launch.command,
                        launch.ephemeral,
                        launch.result_schema.clone(),
                        launch.claude_memory.clone(),
                        Some(launch.role.clone()),
                    )
                }
            };
        let (pane_id, context_id) = self.launch_agent_worker(
            AgentWorkerLaunch {
                grant: AgentWorkerGrant {
                    source,
                    role: role_grant,
                    recorded: None,
                },
                pane_id: None,
                spawned_by,
                role,
                harness,
                command,
                claude_memory,
                task,
                ephemeral,
                result_schema,
                run_token,
            },
            placement,
        )?;
        Ok(SpawnedWorker {
            pane_id,
            context_id,
            harness: harness.to_owned(),
            command: command.to_owned(),
        })
    }

    /// `role`'s definition if one exists at `lookup_cwd`; a label with no
    /// definition runs unconfigured, bounded by its source's grant. A
    /// definition that exists but does not load refuses.
    pub(crate) fn admit_defined_worker_role(
        &self,
        role: &str,
        lookup_cwd: &Path,
    ) -> Result<Option<crate::broker::pane_authority::AdmittedRole>, String> {
        Self::resolve_defined_worker_role(role, lookup_cwd)
            .map(|definition| definition.map(Self::admitted_worker_role))
    }

    fn resolve_defined_worker_role(
        role: &str,
        lookup_cwd: &Path,
    ) -> Result<Option<crate::agent::AgentDefinition>, String> {
        let lookup_root = crate::app::registry::resolve_agent_lookup_workspace_root(lookup_cwd);
        let profile_dir = crate::config::config_dir();
        if !crate::cli::agent::agent_definition_exists(
            role,
            &profile_dir,
            lookup_root.as_deref(),
        ) {
            log::info!("agent_spawn: role={role} has no definition; unconfigured worker");
            return Ok(None);
        }
        crate::cli::agent::resolve_agent_definition_in(role, &profile_dir, lookup_root.as_deref())
            .map(|(definition, _)| Some(definition))
            .map_err(|error| {
                log::info!("agent_spawn: role={role} not admitted: {error}");
                format!("permission_denied: agent role '{role}' did not resolve: {error}")
            })
    }

    fn admitted_worker_role(
        definition: crate::agent::AgentDefinition,
    ) -> crate::broker::pane_authority::AdmittedRole {
        crate::broker::pane_authority::AdmittedRole {
            id: definition.id,
            posture: definition.posture,
            can_call: definition.can_call,
            can_handoff: definition.can_handoff,
        }
    }

    /// Construct and install an agent worker at `placement` under an already
    /// prepared ceiling. Nothing here re-reads a role definition: a worker
    /// started in a new checkout cannot pick up a broader definition from it
    /// (stint 1632). The sub-context a placement creates exists only once the
    /// worker is installed.
    pub(crate) fn launch_agent_worker(
        &mut self,
        worker: AgentWorkerLaunch<'_>,
        placement: AgentPlacement,
    ) -> Result<(PaneId, u64), String> {
        let AgentWorkerLaunch {
            grant,
            pane_id,
            spawned_by,
            role,
            harness,
            command,
            claude_memory,
            task,
            ephemeral,
            result_schema,
            run_token,
        } = worker;
        let parent_pane_id = placement.parent_pane_id;
        let spawned_by =
            spawned_by.unwrap_or_else(|| self.derive_spawned_by_for_parent(parent_pane_id));
        let mut env = Vec::new();
        if let Some(pid) = parent_pane_id {
            env.push(("PLEXI_PARENT_PANE".to_string(), pid.to_string()));
        }
        env.push(("PLEXI_SPAWNED_BY".to_string(), spawned_by.clone()));
        if ephemeral {
            env.push(("PLEXI_EPHEMERAL".to_string(), "1".to_string()));
        }
        env.push(("PLEXI_AGENT_HARNESS".to_string(), harness.to_string()));
        env.push(("PLEXI_AGENT_ROLE".to_string(), role.to_string()));
        if let Some(id) = self.agent_tree_id(parent_pane_id, role) {
            env.push(("PLEXI_AGENT_ID".to_string(), id));
        }
        env.push(("PLEXI_AGENT_TASK".to_string(), task.to_string()));
        let worker_command = if harness == "team" {
            command.to_string()
        } else {
            let exe =
                std::env::current_exe().map_err(|error| format!("worker executable: {error}"))?;
            let reporter = format!(
                "{} --socket {}",
                crate::host::shell::shell_quote(&exe.to_string_lossy()),
                crate::host::shell::shell_quote(&crate::platform::ipc::endpoint())
            );
            crate::cli::agent::supervised_worker_command(command, &reporter, harness)
        };

        let staged = self.stage_agent_destination(&placement);
        let context = match &staged {
            StagedAgentDestination::Window(context)
            | StagedAgentDestination::NewSubcontext { context, .. } => context.clone(),
        };
        let pane_id = pane_id.unwrap_or_else(|| self.host.alloc_pane_id());
        let scope = super::grant::credential_scope(&context, Some(&placement.effective_cwd));
        let authority = crate::broker::pane_authority::LivePaneAuthority::admitted(
            crate::broker::pane_authority::PaneAuthority::admit(
                &crate::config::config_dir(),
                crate::broker::pane_authority::GrantRequest {
                    pane_id,
                    source: grant.source,
                    parent: parent_pane_id,
                    role: grant.role,
                    scope,
                    secret_directory: None,
                    folder_secrets: false,
                    agent_command: true,
                    spawned_by: spawned_by.clone(),
                },
            )
            .narrowed_by(grant.recorded.as_ref()),
        );
        let launch = TerminalLaunch {
            authority,
            context,
            cwd: Some(placement.effective_cwd.clone()),
            // Completion closes ephemeral workers after the exit report;
            // failures retain the shell and its output for inspection.
            command: Some(LaunchCommand {
                command: worker_command,
                close_on_exit: false,
            }),
            ephemeral: false,
            env,
            mcp: match run_token {
                Some(token) => McpBearer::RunToken(token.to_string()),
                None => McpBearer::Pane,
            },
            profile_dir: crate::config::config_dir(),
        };
        let new_id = launch.pane_id();
        let mut pane = self.construct_terminal(launch).map_err(|error| {
            log::error!("{error}");
            format!("failed to create terminal pane for agent role '{role}': {error}")
        })?;
        pane.ephemeral = ephemeral;
        pane.parent_owner = parent_pane_id.and_then(|id| self.pane_principal(id));
        pane.parent_pane_id = parent_pane_id;
        pane.agent_harness = Some(harness.to_string());
        pane.agent_role = Some(role.to_string());
        pane.claude_memory = claude_memory;
        pane.begin_agent_launch(crate::host::pane::AgentLaunchKind::Command);
        if let Ok(Some(posture)) = pane.permission_ceiling() {
            log::info!(
                "agent_spawn: posture snapshot for role={role} pane={new_id}: default={}",
                posture.default_posture.as_str()
            );
        }
        pane.name = Some(role.to_string());
        pane.name_locked = true;
        pane.agent = Some(crate::protocol::PaneAgentState {
            pane_id: new_id,
            state: crate::protocol::AgentState::Working,
            agent: role.to_string(),
            detail: (!task.is_empty()).then(|| task.to_string()),
            session_id: None,
        });

        let (_, target_ctx_id) = self.install_agent_worker(&placement, staged, pane)?;

        if let Some(schema) = result_schema {
            if let Err(err) = self
                .pane_results
                .entry(new_id)
                .or_default()
                .set_schema(schema.clone(), Some(&placement.effective_cwd))
            {
                log::warn!(
                    "agent_spawn: result_schema set_schema fallback for pane {new_id}: {err}"
                );
                self.pane_results.entry(new_id).or_default().schema = Some(schema);
            }
        }
        let _ = self.ensure_pane_lifecycle(new_id);
        self.emit_pane_lifecycle(
            new_id,
            crate::host::pane_lifecycle::PaneLifecycleEvent::AgentWorking {
                provenance: crate::host::pane_lifecycle::Provenance {
                    source: crate::host::pane_lifecycle::Source::Hook,
                    agent_label: harness.to_string(),
                    session_id: None,
                    raw_event: Some("AgentSpawn".into()),
                },
            },
        );
        crate::host::flight_recorder::record(
            "pane_spawned",
            Some(target_ctx_id),
            Some(new_id),
            None,
            serde_json::json!({
                "pane_id": new_id,
                "role": role,
                "harness": harness,
                "spawned_by": spawned_by,
                "parent_pane_id": parent_pane_id,
                "context_id": target_ctx_id,
            }),
        );
        self.refresh_portal_context_states();
        self.mark_workspace_dirty();
        Ok((new_id, target_ctx_id))
    }

    /// Toggle the file browser: if the focused pane has a file browser open,
    /// close it. Otherwise, open one.
    pub(crate) fn open_file_browser(&mut self) {
        if self.plugin_blocked("file_browser") {
            return;
        }
        // Check if the focused pane (or its linked view pane above) already has
        // a file browser open. If so, close it.
        let ctx = &self.windows[self.active_window];
        if let Some(focused) = ctx.focused_pane {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                let pane_id = *pane_id;
                if let Some(pane) = ctx.panes.get(&pane_id) {
                    if let Some(a) = pane.as_view() {
                        if a.runtime.type_id() == "file_browser" {
                            self.close_focused_view();
                            return;
                        }
                    }
                }
            }
        }

        // Bind the invoking pane and its owning context before opening the overlay.
        let caller = ctx
            .focused_pane
            .and_then(|tile| ctx.find_first_pane_in(tile))
            .and_then(|tile| match ctx.tree.tiles.get(tile) {
                Some(egui_tiles::Tile::Pane(id)) => Some(*id),
                _ => None,
            });
        let cwd = self.file_browser_start_directory(caller, ctx.context_id);

        let app: Box<dyn View> = Box::new(crate::file_browser::FileBrowserView::new(cwd.clone()));

        // Track only the invoking terminal, including its overlay replacement.
        let perms = crate::app::permissions::PluginPermissions::builtin();
        self.open_builtin_view_pane(
            app,
            perms,
            cwd,
            Some(FILE_BROWSER_CWD_GROUP.to_string()),
            Some("overlay"),
            None,
        );
    }

    /// Fresh Explorer launches follow the invoking live terminal's current cwd.
    /// Nonterminal, standby, exited, missing or deleted cwd falls back to the
    /// caller's context root, then home. Existing Explorer state is not retargeted.
    pub(crate) fn file_browser_start_directory(
        &self,
        caller: Option<PaneId>,
        context_id: u64,
    ) -> PathBuf {
        let terminal = caller.and_then(|id| {
            self.windows
                .iter()
                .filter(|window| window.context_id == context_id)
                .find_map(|window| window.panes.get(&id))
                .and_then(Pane::as_terminal)
        });
        if let Some(terminal) = terminal {
            if !terminal.exited && terminal.standby_session.is_none() {
                if let Some(cwd) =
                    crate::host::shell::get_pid_cwd_fresh(terminal.backend.child_pid())
                {
                    match cwd.try_exists() {
                        Ok(true) if cwd.is_dir() => {
                            log::info!("file_browser: launch caller={caller:?} context={context_id} cwd={} source=terminal", cwd.display());
                            return cwd;
                        }
                        result => log::warn!(
                            "file_browser: unusable terminal cwd={} caller={caller:?}: {result:?}",
                            cwd.display()
                        ),
                    }
                } else {
                    log::warn!("file_browser: current cwd unavailable caller={caller:?} context={context_id}");
                }
            } else {
                log::info!(
                    "file_browser: caller={caller:?} has no live terminal cwd; using fallback"
                );
            }
        }
        let cwd = self
            .context_root_for(context_id)
            .filter(|root| root != &PathBuf::from("/") && root.is_dir())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        log::info!(
            "file_browser: launch caller={caller:?} context={context_id} cwd={} source=fallback",
            cwd.display()
        );
        cwd
    }

    /// Locate an open instance of plugin `manifest_id` whose origin is
    /// `origin`, including one currently buried under overlay replacements.
    /// When `context_id` is `Some`, only panes whose owning window belongs to
    /// that context match; `None` searches every window in every context.
    /// Returns `(window_index, slot_pane_id, overlay_depth)` — depth 0 means
    /// the instance is the visible pane at the slot, depth N means N overlays
    /// sit on top of it and must be popped to reveal it. Backs the `[launch]
    /// on_launch` resolver (#0336).
    ///
    /// Identity is `ViewPane::manifest_id` plus the pane's origin, fixed when
    /// it was constructed (stint 1733), at every depth of the overlay chain: a
    /// development copy under a bundled id, a registry plugin reusing a
    /// builtin's id, and the builtin itself are different plugins. Pane lookup
    /// is global by design (#878); context scoping comes purely from the
    /// owning window's `context_id`.
    pub(crate) fn locate_view_instance(
        &self,
        manifest_id: &str,
        context_id: Option<u64>,
        origin: OriginQuery<'_>,
    ) -> Option<(usize, PaneId, usize)> {
        self.locate_view_instance_where(manifest_id, context_id, origin, &|_| true)
    }

    /// [`Self::locate_view_instance`] counting only instances `accept` takes,
    /// at every depth of each overlay chain.
    pub(crate) fn locate_view_instance_where(
        &self,
        manifest_id: &str,
        context_id: Option<u64>,
        origin: OriginQuery<'_>,
        accept: &dyn Fn(&crate::host::pane::ViewPane) -> bool,
    ) -> Option<(usize, PaneId, usize)> {
        let wanted = match origin {
            OriginQuery::Entry(entry) => Some(crate::platform::path::canonical_or_self(entry)),
            _ => None,
        };
        let runs = |app: &crate::host::pane::ViewPane| match (origin, app.runtime.origin()) {
            (OriginQuery::Builtin, crate::host::pane::PaneOrigin::Builtin) => true,
            (OriginQuery::Entry(_), crate::host::pane::PaneOrigin::Guest(guest)) => {
                wanted.as_deref() == Some(guest.entry())
            }
            (OriginQuery::Bundled, crate::host::pane::PaneOrigin::Guest(guest)) => {
                guest.is_bundled()
            }
            #[cfg(test)]
            (OriginQuery::Any, _) => true,
            _ => false,
        };
        // Walk a pane's overlay-replacement chain, returning how many overlays
        // sit above the first pane that matches (`0` = the pane itself
        // matches). The replaced pane an overlay stores can be any variant;
        // only view panes carry an id and a further chain.
        let overlay_depth = |pane: &Pane| -> Option<usize> {
            let mut current = pane;
            let mut depth = 0;
            loop {
                let app = current.as_view()?;
                if app.manifest_id == manifest_id && runs(app) && accept(app) {
                    return Some(depth);
                }
                current = app.overlay_replaced.as_deref()?;
                depth += 1;
            }
        };

        self.windows.iter().enumerate().find_map(|(win_idx, win)| {
            if context_id.is_some_and(|c| c != win.context_id) {
                return None;
            }
            win.panes
                .iter()
                .find_map(|(slot, pane)| overlay_depth(pane).map(|d| (win_idx, *slot, d)))
        })
    }

    /// Convenience wrapper returning `(slot_pane_id, window_index)` for the
    /// first open instance of `manifest_id`, discarding overlay depth. Used by
    /// tests that only assert which instance was matched.
    #[cfg(test)]
    pub(crate) fn find_view_pane_by_type(
        &self,
        manifest_id: &str,
        context_id: Option<u64>,
    ) -> Option<(PaneId, usize)> {
        self.locate_view_instance(manifest_id, context_id, OriginQuery::Any)
            .map(|(win_idx, slot, _)| (slot, win_idx))
    }

    /// Resolve a registry plugin's `[launch] on_launch` policy (#0336) against
    /// the open panes running its entry before spawning. Builtins never reach
    /// here: they are resolved first and always open fresh (stint 1733).
    /// Returns `Some(existing_pane_id)` when the
    /// launch was satisfied by focusing a live instance — the caller must then
    /// NOT spawn, and must report *that* pane id (never a predicted one).
    /// Returns `None` when no instance matched and the caller should spawn.
    ///
    /// `caller_context_id` is the context the launch originates in (the active
    /// window's context). Cross-context focus for `focus_existing` uses the
    /// sanctioned [`jump_to_context`](Self::jump_to_context) path; it never
    /// mutates `active_window`/`router.active` to steer the focus. An instance
    /// buried under overlays is revealed by popping those overlays via the
    /// sanctioned [`restore_overlay_replacement`](Self::restore_overlay_replacement)
    /// path — the same restore Esc performs — before focusing. Revealing a
    /// buried instance therefore pops (and closes) every plugin overlay stacked
    /// above it, exactly as pressing Esc that many times would; this is
    /// intentional, not incidental.
    ///
    /// `args`/`cwd_override` are the relaunch payload. When a dedup focuses an
    /// existing instance they are *dropped* — the running plugin never receives
    /// them (delivering relaunch args to a live instance via a bus event is
    /// future work). The drop is logged at `warn` so it is observable.
    #[cfg(test)]
    pub(crate) fn resolve_on_launch_policy(
        &mut self,
        id: &str,
        caller_context_id: u64,
        args: &[String],
        cwd_override: Option<&Path>,
    ) -> Option<PaneId> {
        self.resolve_on_launch_policy_at(id, caller_context_id, args, cwd_override, None)
    }

    fn resolve_on_launch_policy_at(
        &mut self,
        id: &str,
        caller_context_id: u64,
        args: &[String],
        cwd_override: Option<&Path>,
        registry_root: Option<&Path>,
    ) -> Option<PaneId> {
        let view = self.launch_registry(caller_context_id, registry_root);
        let on_launch = view.on_launch_for(id);
        let entry = view.get(id)?.bin_path.clone();
        // A bundled id names the bundled plugin: only a pane admitted as the
        // bundled copy satisfies it, whatever its cached entry resolves to now.
        let origin = if crate::cli::install_host::is_bundled_plugin_id(id) {
            OriginQuery::Bundled
        } else {
            OriginQuery::Entry(&entry)
        };
        self.focus_for_on_launch(on_launch, id, origin, caller_context_id, args, cwd_override)
    }

    /// Focus a live instance when `policy` says this launch must not spawn.
    pub(crate) fn focus_for_on_launch(
        &mut self,
        on_launch: crate::app::registry::OnLaunchPolicy,
        id: &str,
        origin: OriginQuery<'_>,
        caller_context_id: u64,
        args: &[String],
        cwd_override: Option<&Path>,
    ) -> Option<PaneId> {
        use crate::app::registry::OnLaunchPolicy;
        let (scope, target) = match on_launch {
            OnLaunchPolicy::AlwaysNew => return None,
            OnLaunchPolicy::FocusExisting => (
                "focus_existing",
                self.locate_view_instance(id, None, origin),
            ),
            OnLaunchPolicy::FocusExistingInContext => (
                "focus_existing_in_context",
                self.locate_view_instance(id, Some(caller_context_id), origin),
            ),
        };
        let Some((win_idx, slot, overlay_depth)) = target else {
            log::info!(
                "on_launch[{scope}]: no existing '{id}' instance to focus (context {caller_context_id}) — spawning", id = crate::host::guest_text::shown(id)
            );
            return None;
        };
        let ctx_id = self.windows[win_idx].context_id;
        log::info!(
            "on_launch[{scope}]: '{id}' already open at slot {slot} in context {ctx_id} (overlay_depth={overlay_depth}) — focusing instead of spawning", id = crate::host::guest_text::shown(id)
        );
        if !args.is_empty() || cwd_override.is_some() {
            log::warn!(
                "on_launch[{scope}]: '{id}' relaunch carried argc={} cwd={cwd_override:?} \
                 but the policy focused the existing instance — these are NOT delivered to it \
                 (relaunch arg delivery to a running instance is future work)",
                args.len(),
                id = crate::host::guest_text::shown(id)
            );
        }
        self.reveal_and_focus_instance(win_idx, slot, overlay_depth);
        Some(slot)
    }

    /// Reveal a located plugin instance and focus it: pop any overlays covering
    /// it and jump to its window via the sanctioned
    /// [`jump_to_context`](Self::jump_to_context) path — never by mutating
    /// `active_window`/`router.active` directly. Popping overlays closes every
    /// plugin overlay stacked above the instance, exactly as pressing Esc that
    /// many times would.
    fn reveal_and_focus_instance(&mut self, win_idx: usize, slot: PaneId, overlay_depth: usize) {
        let win_id = self.windows[win_idx].window_id;
        for _ in 0..overlay_depth {
            self.restore_overlay_replacement(win_idx, slot);
        }
        self.jump_to_context(win_idx, win_id, Some(slot));
    }

    /// Launch the app discovered in the palette's workspace registry.
    pub(crate) fn launch_discovered_app(
        &mut self,
        target_win: usize,
        id: &str,
        source: crate::app::registry::RegistrySource,
        workspace: Option<&Path>,
    ) -> Result<Option<PaneId>, String> {
        let home = dirs::home_dir();
        let registry_root = workspace.or(home.as_deref()).unwrap_or(Path::new("/"));
        self.launch_view_by_id_in_registry(
            IdViewLaunchOptions {
                target_win,
                bypass_on_launch: false,
                follow_file_browser_cwd: true,
            },
            id,
            None,
            &[],
            None,
            Some((registry_root, source)),
        )
    }

    /// Open a config file (global or workspace) in a Plexi text-editor pane
    /// split from the focused pane. The one path behind the `open_config`
    /// keybinding, the palette entries, and the macOS menu item. The file
    /// opens through the editor's ordinary path launch argument; an already
    /// open config is focused rather than duplicated. File buffers save only
    /// on Cmd+S, so opening a real config here cannot silently mutate it.
    pub(crate) fn open_config_in_editor(&mut self, path: PathBuf) {
        self.open_config_in_editor_with_cwd(path, None);
    }

    pub(crate) fn open_config_in_editor_with_cwd(
        &mut self,
        path: PathBuf,
        cwd_override: Option<PathBuf>,
    ) {
        if self.plugin_blocked("text-editor") {
            return;
        }
        if let Some(pane_id) = self.find_open_text_editor_pane_any_window(&path) {
            log::info!(
                "open_config: {} already open in pane {pane_id}, focusing",
                path.display()
            );
            self.pane_navigate(pane_id);
            return;
        }
        let args = [path.display().to_string()];
        let pane_id = self.host.next_pane_id();
        match self.launch_view_by_id_with_layout(
            "text-editor",
            Some("split_h".to_string()),
            &args,
            cwd_override,
        ) {
            Ok(existing) => log::info!(
                "open_config: {} opened in text-editor pane {}",
                path.display(),
                existing.unwrap_or(pane_id)
            ),
            Err(e) => log::error!(
                "open_config: could not open {} in the text editor: {e}",
                path.display()
            ),
        }
    }

    pub(crate) fn switch_commands_editor_scope(
        &mut self,
        window_id: u64,
        pane_id: Option<PaneId>,
    ) -> bool {
        let Some(pane_id) = pane_id else {
            return false;
        };
        let Some(view_pane) = self
            .windows
            .iter_mut()
            .find(|window| window.window_id == window_id)
            .and_then(|window| window.panes.get_mut(&pane_id))
            .and_then(|p| p.as_view_mut())
        else {
            return false;
        };
        let switched = view_pane.runtime.switch_commands_scope();
        if switched {
            view_pane.name = view_pane.runtime.display_name();
            self.mark_workspace_dirty();
        }
        switched
    }

    /// Launch an installed plugin with an explicit layout and args override.
    ///   "overlay" (default) — full pane takeover; Esc restores the original pane
    ///   "split_h"           — horizontal split, new pane to the right
    ///   "split_v"           — vertical split, new pane below
    ///
    /// Honors the plugin's `[launch] on_launch` dedup policy: returns
    /// `Ok(Some(existing_pane_id))` when a live instance satisfied the launch
    /// (nothing was spawned), and `Ok(None)` when a fresh pane was spawned (the
    /// caller's predicted `next_pane_id()` is then the real id).
    pub(crate) fn launch_view_by_id_with_layout(
        &mut self,
        id: &str,
        layout: Option<String>,
        args: &[String],
        cwd_override: Option<PathBuf>,
    ) -> Result<Option<PaneId>, String> {
        let follow_file_browser_cwd = cwd_override.is_none();
        self.launch_view_by_id_with_layout_at(
            IdViewLaunchOptions {
                target_win: self.active_window,
                bypass_on_launch: false,
                follow_file_browser_cwd,
            },
            id,
            layout,
            args,
            cwd_override,
        )
    }

    /// Like [`open_view_by_id_with_layout`](Self::open_view_by_id_with_layout)
    /// but bypasses the `[launch] on_launch` dedup policy so a fresh instance is
    /// always spawned. Used by the split-mirror path (#0336), whose whole point
    /// is to duplicate a pane — with a singleton plugin the policy would otherwise
    /// silently no-op the mirror.
    pub(crate) fn launch_view_by_id_with_layout_forced(
        &mut self,
        id: &str,
        layout: Option<String>,
        args: &[String],
        cwd_override: Option<PathBuf>,
    ) -> Result<Option<PaneId>, String> {
        let follow_file_browser_cwd = cwd_override.is_none();
        self.launch_view_by_id_with_layout_at(
            IdViewLaunchOptions {
                target_win: self.active_window,
                bypass_on_launch: true,
                follow_file_browser_cwd,
            },
            id,
            layout,
            args,
            cwd_override,
        )
    }

    /// The id a launch of `id` in context `context_id` dispatches on: the
    /// owning plugin of an installed plugin or contributed view, else `id`
    /// itself (a built-in). Callers that gate on the handler decide on this.
    pub(crate) fn resolve_launch_id(
        &mut self,
        context_id: u64,
        id: &str,
    ) -> Result<String, String> {
        self.resolve_launch_id_at(context_id, id, None)
    }

    fn launch_registry(
        &mut self,
        context_id: u64,
        registry_root: Option<&Path>,
    ) -> &crate::app::registry::PluginRegistry {
        match registry_root {
            Some(root) => self.registries.view_for_root(root),
            None => self.registries.view_for_context(context_id, &self.router),
        }
    }

    fn resolve_launch_id_at(
        &mut self,
        context_id: u64,
        id: &str,
        registry_root: Option<&Path>,
    ) -> Result<String, String> {
        // A builtin id never resolves to a registry plugin (stint 1733).
        if is_builtin_view_id(id) {
            return Ok(id.to_string());
        }
        let registry = self.launch_registry(context_id, registry_root);
        let contributes = |plugin: &&crate::app::registry::InstalledPlugin| {
            plugin.contributes.views.iter().any(|view| view.id == id)
        };
        // A bundled plugin's contributed view names are reserved like its id:
        // an exact personal plugin id cannot take one (stint 1733). The
        // registry indexes a bundled id only from its verified profile root.
        let plugins = registry.list();
        let bundled_view = plugins
            .iter()
            .copied()
            .filter(contributes)
            .find(|plugin| crate::cli::install_host::is_bundled_plugin_id(&plugin.manifest.id));
        let installed = bundled_view
            .or_else(|| registry.get(id))
            .or_else(|| plugins.iter().copied().find(contributes));
        match installed {
            Some(plugin) => {
                let view = plugin.contributes.resolve_view(&plugin.manifest.id, id)?;
                log::info!(
                    "view_open: plugin_id={} view_id={} context_id={context_id}",
                    plugin.manifest.id,
                    crate::host::guest_text::shown(&view.id)
                );
                Ok(plugin.manifest.id.clone())
            }
            None => Ok(id.to_string()),
        }
    }

    /// The working directory a launch in `context_id` uses.
    pub(crate) fn launch_cwd(&self, context_id: u64, cwd_override: Option<PathBuf>) -> PathBuf {
        cwd_override
            .or_else(|| self.context_root_for(context_id))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
    }

    /// Place a constructed builtin the way every builtin launch does.
    fn open_launched_builtin(
        &mut self,
        target_win: usize,
        layout: Option<String>,
        app: Box<dyn View>,
        cwd: PathBuf,
        group: Option<String>,
    ) -> Result<PaneId, String> {
        let hint = layout.unwrap_or_else(|| "overlay".to_string());
        self.open_builtin_view_pane_at(
            BuiltinPlacement {
                window: target_win,
                hint: Some(&hint),
                share: None,
            },
            app,
            crate::app::permissions::PluginPermissions::builtin(),
            cwd,
            group,
        )
    }

    /// Open the text editor on a native `path` in the active window, through
    /// the builtin launch gate, placement and workspace resolution. The path
    /// is never spelled as a string, so a non-UTF-8 note keeps its bytes.
    /// Place a text editor already built (`text_editor_from_read`) in
    /// `target_win` with `layout`, through the builtin gate and placement.
    pub(crate) fn open_built_text_editor(
        &mut self,
        target_win: usize,
        layout: Option<String>,
        app: Box<dyn View>,
        cwd: PathBuf,
    ) -> Result<PaneId, String> {
        self.require_plugin("text-editor")?;
        let group = builtin_default_group(app.type_id());
        self.open_launched_builtin(target_win, layout, app, cwd, group)
    }

    pub(crate) fn open_text_editor_at_path(&mut self, path: PathBuf) -> Result<(), String> {
        const ID: &str = "text-editor";
        self.require_plugin(ID)?;
        let target_win = self.active_window;
        let context_id = self.windows[target_win].context_id;
        let layout = self
            .registries
            .view_for_context(context_id, &self.router)
            .placement_for(ID);
        let cwd = self.launch_cwd(context_id, None);
        log::info!("open_text_editor_at_path: typed note launch context_id={context_id}");
        let app = text_editor_view(path, &cwd);
        self.open_built_text_editor(target_win, layout, app, cwd)?;
        Ok(())
    }

    pub(crate) fn launch_view_by_id_with_layout_at(
        &mut self,
        options: IdViewLaunchOptions,
        id: &str,
        layout: Option<String>,
        args: &[String],
        cwd_override: Option<PathBuf>,
    ) -> Result<Option<PaneId>, String> {
        self.launch_view_by_id_in_registry(options, id, layout, args, cwd_override, None)
    }

    fn launch_view_by_id_in_registry(
        &mut self,
        options: IdViewLaunchOptions,
        id: &str,
        layout: Option<String>,
        args: &[String],
        cwd_override: Option<PathBuf>,
        discovered: Option<(&Path, crate::app::registry::RegistrySource)>,
    ) -> Result<Option<PaneId>, String> {
        // A plugin can ask for any id (SpawnView, SpawnPane). No real plugin
        // or view id holds a control, bidi or invisible character, so such a
        // request is refused here, before any log or error repeats it raw
        // (stint 1344). Length is not checked: manifests set no limit on view
        // ids, and a display cap must not become an identity limit; logs
        // cap the shown id instead.
        if id.chars().any(crate::host::guest_text::is_unsafe_to_print) {
            let shown = crate::host::guest_text::shown(id);
            log::info!("launch refused: requested id '{shown}' is not a plugin or view id");
            return Err(format!("no plugin or view '{shown}'"));
        }
        // A terminal is not a view. Each producer admits its terminal with
        // the source it is (desktop input, a view, the scheduler), so no
        // route that names "terminal" here can open one (stint 1666).
        if id == "terminal" {
            log::info!("launch refused: 'terminal' is not a view; admit it through its caller's launch route");
            return Err("'terminal' is not a view".to_string());
        }
        self.require_plugin(id)?;
        let IdViewLaunchOptions {
            target_win,
            bypass_on_launch,
            follow_file_browser_cwd,
        } = options;
        let context_id = self.windows[target_win].context_id;
        let registry_root = discovered.map(|(root, _)| root);
        let requested = id;
        let resolved = self.resolve_launch_id_at(context_id, id, registry_root)?;
        if let Some((root, source)) = discovered {
            if resolved != id
                || self
                    .registries
                    .view_for_root(root)
                    .get(id)
                    .is_none_or(|app| {
                        app.source != source
                            || (source != crate::app::registry::RegistrySource::Global
                                && app.workspace_root.as_deref() != Some(root))
                            || !app.plugin_dir.is_dir()
                    })
            {
                return Err("The app is no longer available in the invoking scope".into());
            }
        }
        let id = resolved.as_str();
        // Resolve an implicit Explorer caller before on-launch policy can move focus.
        let cwd_override =
            if id == "file_browser" && follow_file_browser_cwd && cwd_override.is_none() {
                Some(self.file_browser_start_directory(
                    self.focused_terminal_id(target_win),
                    self.windows[target_win].context_id,
                ))
            } else {
                cwd_override
            };
        // The Assistant is a compiled-in plugin whose constructor needs the live
        // broker and channel profile, so it cannot participate in the
        // broker-free `builtin_factory`. Keep it on the same generic plugin-open
        // route as every other builtin so scene and CLI callers do not need an
        // plugin-specific command.
        if id == "assistant" {
            if !crate::release::feature_enabled(crate::release::ReleaseFeature::Assistant) {
                crate::release::log_feature_blocked(crate::release::ReleaseFeature::Assistant);
                return Err(crate::release::feature_unavailable_text(
                    crate::release::ReleaseFeature::Assistant,
                ));
            }
            let pane_id = self
                .open_assistant_pane_with_hint_at(
                    target_win,
                    layout.as_deref().unwrap_or("overlay"),
                )
                .ok_or_else(|| "assistant pane could not be opened".to_string())?;
            log::info!(
                "open_view_by_id_with_layout: 'assistant' resolved as builtin pane_id={pane_id}"
            );
            return Ok(Some(pane_id));
        }

        if let Some(feature) = release_feature_for_plugin_id(id) {
            if !crate::release::feature_enabled(feature) {
                return Err(crate::release::feature_unavailable_message(feature));
            }
        }

        // The caller resolves ownership before entering this launch pipeline.
        let context_id = self.windows[target_win].context_id;

        // Builtins resolve before any registry metadata (placement, on_launch,
        // entry), never touch disk, and always open fresh: a registry plugin
        // reusing a builtin id never stands in for it (stint 1733).
        if is_builtin_view_id(id) {
            let cwd = self.launch_cwd(context_id, cwd_override);
            let Some(app) = builtin_factory(id, &cwd, args) else {
                let shown = crate::host::guest_text::shown(id);
                log::info!("open_view_by_id_with_layout: builtin '{shown}' needs a file argument");
                return Err(format!("'{shown}' opens a file: pass its path"));
            };
            log::info!(
                "open_view_by_id_with_layout: '{id}' resolved as builtin",
                id = crate::host::guest_text::shown(id)
            );
            let group = if id == "file_browser" && !follow_file_browser_cwd {
                None
            } else {
                builtin_default_group(app.type_id())
            };
            // A placement failure is the caller's error, never a silent
            // `Ok(None)` the caller reads as "opened" (stint 1673).
            self.open_launched_builtin(target_win, layout, app, cwd, group)?;
            return Ok(None);
        }

        // When the caller does not specify a placement, fall back to the plugin's
        // manifest-declared `[launch] placement` before the host default
        // (`overlay`). One resolution point so every downstream hint inherits
        // it (#2283). Builtins are not in the registry → None → `overlay`.
        let layout = layout.or_else(|| {
            self.launch_registry(context_id, registry_root)
                .placement_for(id)
        });

        // #0336: honor the plugin's `[launch] on_launch` dedup policy before any
        // spawn path (unless the caller forces a fresh instance, e.g. the
        // split-mirror). If a live instance already satisfies the policy we
        // focus it and return its real pane id; otherwise we fall through and
        // spawn as usual. This sits ahead of the background re-attach below on
        // purpose: the policy only matches *open* panes, so a `focus_existing`
        // plugin whose pane was closed (its process parked in `background_apps`)
        // finds no pane here, falls through, and re-attaches — the two compose.
        if !bypass_on_launch {
            if let Some(existing_id) = self.resolve_on_launch_policy_at(
                id,
                context_id,
                args,
                cwd_override.as_deref(),
                registry_root,
            ) {
                return Ok(Some(existing_id));
            }
        }

        let cwd_explicit = cwd_override.is_some();
        let cwd = self.launch_cwd(context_id, cwd_override);
        log::info!(
            "open_view_by_id_with_layout: id={id} cwd={cwd:?} cwd_source={} context_root={:?}",
            if cwd_explicit { "explicit" } else { "resolved" },
            self.context_root_for(context_id),
            id = crate::host::guest_text::shown(id)
        );

        // Ensure the registry is up-to-date: rescan if the plugin was added mid-session
        // via `plexi plugin init` and wasn't present at startup.
        if discovered.is_none()
            && self
                .launch_registry(context_id, registry_root)
                .get(id)
                .is_none()
        {
            log::info!(
                "open_view_by_id: '{id}' not in startup registry — rescanning from disk",
                id = crate::host::guest_text::shown(id)
            );
            // Rescan the CONTEXT's own root, not `cwd` — `view_for_context`
            // below looks the plugin up by that same key, so rescanning any
            // other path (even one that happens to resolve to the same
            // workspace via `cwd`'s internal walk-up) would leave the cache
            // entry `view_for_context` actually reads unchanged.
            let context_root = self
                .context_root_for(context_id)
                .unwrap_or_else(|| cwd.clone());
            self.registries.rescan_root(&context_root);
        }
        // Resolve the id the caller asked for again against the refreshed
        // view: a contributed view whose owner was not yet discovered now
        // resolves to that owner (stint 1398).
        let refreshed = self.resolve_launch_id_at(context_id, requested, registry_root)?;
        let id = refreshed.as_str();

        // Pre-flight: check config-level capability requirements before spawning.
        let config = &self.config;
        let registry = match registry_root {
            Some(root) => self.registries.view_for_root(root),
            None => self.registries.view_for_context(context_id, &self.router),
        };
        let missing = registry.check_config_capabilities(id, config);
        let installed = registry.get(id).cloned();
        if !missing.is_empty() {
            log::warn!(
                "pre-flight: '{id}' cannot launch — missing: {missing:?}",
                id = crate::host::guest_text::shown(id)
            );
            let fail_hint = layout.or_else(|| Some("overlay".to_string()));
            self.open_launch_failed_pane(target_win, id, fail_hint.as_deref(), missing, cwd);
            return Ok(None);
        }

        // Query group/hint after any registry reload so metadata reflects the
        // actual registry that found the plugin.
        let hint = layout.or_else(|| Some("overlay".to_string()));
        if let Some(installed) = &installed {
            if installed.manifest.manifest_type == crate::app::registry::ManifestType::Wasm {
                let workspace_root = installed
                    .workspace_root
                    .clone()
                    .unwrap_or_else(|| cwd.clone());
                self.open_installed_wasm_view_pane(
                    target_win,
                    crate::host::plugin_admission::OpenedBy::Id,
                    installed.clone(),
                    workspace_root,
                    hint.as_deref(),
                    args.to_vec(),
                )?;
                return Ok(None);
            }
            if matches!(
                installed.bin_path.extension().and_then(|ext| ext.to_str()),
                Some("py" | "pyc")
            ) {
                let workspace_root = installed
                    .workspace_root
                    .clone()
                    .unwrap_or_else(|| cwd.clone());
                // The current owner on disk, resolved before anything is
                // granted; its parsed manifest drives the launch (stint 1733).
                self.open_python_wasm_view_pane(
                    target_win,
                    crate::host::plugin_admission::OpenedBy::Id,
                    resolve_owned_plugin_dir(&installed.plugin_dir, id)?,
                    workspace_root,
                    hint.as_deref(),
                    args.to_vec(),
                )?;
                return Ok(None);
            }
        }

        // Dev-pack plugins (plugins/dev/*) are intentionally excluded from the
        // id-addressable registry (plugins/AGENTS.md) — install.sh never syncs
        // them into a profile. When the caller passed an explicit cwd, `cwd`
        // already is that value; otherwise prefer the focused pane's actual
        // cwd over `cwd` (which may have resolved to the context root ahead
        // of it — see stint 0399) so this fires for a developer sitting in a
        // repo checkout.
        let search_start = if cwd_explicit {
            cwd.clone()
        } else {
            self.windows[target_win]
                .focused_pane
                .and_then(|f| self.windows[target_win].get_focused_pane_cwd(f))
                .unwrap_or_else(|| cwd.clone())
        };
        if let Some(dev_plugin_dir) = crate::app::registry::find_dev_pack_plugin(id, &search_start)
        {
            log::warn!(
                "open_view_by_id: '{id}' is a dev-pack plugin ({dev_plugin_dir:?}) — not id-openable", id = crate::host::guest_text::shown(id)
            );
            return Err(format!(
                "'{id}' is a dev-pack plugin ({}) — dev plugins are path-open only. Use `plexi view open {}` instead.",
                dev_plugin_dir.display(),
                dev_plugin_dir.display(), id = crate::host::guest_text::shown(id)
            ));
        }

        match installed {
            None => {
                if let Some(skipped) = self
                    .launch_registry(context_id, registry_root)
                    .skipped_reason(id)
                {
                    log::info!(
                        "open_view_by_id: plugin '{id}' was skipped at registry load ({}): {}",
                        skipped.dir.display(),
                        skipped.error,
                        id = crate::host::guest_text::shown(id)
                    );
                    return Err(skipped.load_error(id));
                }
                log::warn!(
                    "open_view_by_id: plugin '{id}' not found in context_id={context_id}'s registry", id = crate::host::guest_text::shown(id)
                );
                Err(format!(
                    "plugin '{id}' not found in this context's registry",
                    id = crate::host::guest_text::shown(id)
                ))
            }
            Some(installed) => {
                log::warn!(
                    "open_view_by_id: plugin '{id}' has manifest type {:?} with no launch runtime for it",
                    installed.manifest.manifest_type, id = crate::host::guest_text::shown(id)
                );
                Err(format!(
                    "plugin '{id}' has manifest type {:?} with no launch runtime for it",
                    installed.manifest.manifest_type,
                    id = crate::host::guest_text::shown(id)
                ))
            }
        }
    }

    /// Trust the folder holding the plugin at `path`, as a desktop click on
    /// its ask does (stint 1398). For tests that path-launch fixtures.
    #[cfg(test)]
    pub(crate) fn trust_plugin_path_for_test(&self, path: impl AsRef<str>) {
        let path = crate::platform::path::canonical_or_self(Path::new(path.as_ref()));
        if let Some(folder) = path.parent() {
            crate::host::plugin_admission::PluginAdmission::current().trust_folder_for_test(folder);
        }
    }

    /// Launch a plugin directly from a filesystem path without looking it up in the registry.
    ///
    /// The plugin runs in-place; its own directory is used as `workspace_root`.
    /// A raw `.wasm` with unreviewed imports queues the host review modal.
    pub(crate) fn launch_view_by_path_with_layout(
        &mut self,
        app_path: &str,
        layout: Option<String>,
        workspace_root_override: Option<std::path::PathBuf>,
        args: &[String],
    ) -> Result<Option<PaneId>, String> {
        self.launch_view_by_path_with_layout_inner(
            app_path,
            layout,
            workspace_root_override,
            args,
            true,
            true,
        )
    }

    /// Spawn another instance even when the manifest says to focus the open one.
    /// The split-mirror and the multi-instance routing test use this.
    #[cfg(test)]
    pub(crate) fn launch_view_by_path_forced(
        &mut self,
        app_path: &str,
        workspace_root_override: Option<std::path::PathBuf>,
        args: &[String],
    ) -> Result<Option<PaneId>, String> {
        self.launch_view_by_path_with_layout_inner(
            app_path,
            None,
            workspace_root_override,
            args,
            true,
            false,
        )
    }

    #[cfg(test)]
    pub(crate) fn launch_view_by_path_with_layout_no_review_modal(
        &mut self,
        app_path: &str,
        layout: Option<String>,
        workspace_root_override: Option<std::path::PathBuf>,
        args: &[String],
    ) -> Result<Option<PaneId>, String> {
        self.launch_view_by_path_with_layout_inner(
            app_path,
            layout,
            workspace_root_override,
            args,
            false,
            true,
        )
    }

    fn launch_view_by_path_with_layout_inner(
        &mut self,
        app_path: &str,
        layout: Option<String>,
        workspace_root_override: Option<std::path::PathBuf>,
        args: &[String],
        queue_raw_wasm_review: bool,
        honor_on_launch: bool,
    ) -> Result<Option<PaneId>, String> {
        self.require_plugin(app_path)?;
        let plugin_dir = PathBuf::from(app_path);
        log::info!("open_view_by_path_with_layout: path={app_path}");
        // Path-open launches resolve against the active window's context —
        // there is no explicit target context for this entry point.
        let context_id = self.windows[self.active_window].context_id;

        // A `.wasm` file is a sandboxed component plugin (the run primitive, G6),
        // not a manifest-backed process plugin. Route it to the wasmtime path.
        if plugin_dir.extension().and_then(|e| e.to_str()) == Some("wasm") {
            require_wasm_experiment()?;
            // Ownership comes from disk, registered or not; admission is
            // decided when the component starts (stint 1398).
            let plugin_id = crate::app::registry::resolve_component(&plugin_dir)?.owner;
            let workspace_root = workspace_root_override.unwrap_or_else(|| {
                plugin_dir
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."))
            });
            if queue_raw_wasm_review {
                return self
                    .review_or_open_raw_wasm_view_pane(
                        &plugin_id,
                        &plugin_dir,
                        workspace_root,
                        args.to_vec(),
                    )
                    .map(|_| None);
            }
            return self
                .open_wasm_view_pane(&plugin_id, &plugin_dir, workspace_root, args.to_vec(), None)
                .map(Some);
        }

        // One parse: the resolver's manifest drives view, placement, reuse,
        // config and capabilities (stint 1733).
        let (resolved, installed) = match crate::app::registry::resolve_plugin_dir(&plugin_dir)
            .and_then(|resolved| {
                let installed = resolved
                    .installed
                    .clone()
                    .ok_or_else(|| "no manifest".to_string())?;
                Ok((resolved, installed))
            }) {
            Ok(found) => found,
            Err(e) => {
                log::warn!(
                    "open_view_by_path_with_layout: failed to load manifest at {app_path}: {e}"
                );
                return Err(format!("failed to load plugin at '{app_path}': {e}"));
            }
        };
        let view = installed
            .contributes
            .resolve_view(&installed.manifest.id, &installed.manifest.id)?;
        log::info!(
            "view_open_path: plugin_id={} view_id={} context_id={context_id}",
            installed.manifest.id,
            view.id
        );
        let plugin_id = installed.manifest.id.clone();

        let manifest_placement = installed
            .launch
            .placement
            .clone()
            .filter(|p| crate::app::registry::VALID_PLACEMENTS.contains(&p.as_str()));
        let layout_hint = Some(cli_open_placement(layout, manifest_placement));

        let workspace_root = workspace_root_override.unwrap_or_else(|| plugin_dir.clone());
        log::info!(
            "open_view_by_path_with_layout: workspace_root={}",
            workspace_root.display()
        );
        if honor_on_launch {
            let policy = installed
                .launch
                .on_launch
                .as_deref()
                .and_then(|mode| crate::app::registry::OnLaunchPolicy::parse(mode).ok())
                .unwrap_or_default();
            if let Some(existing) = self.focus_for_on_launch(
                policy,
                &plugin_id,
                OriginQuery::Entry(&resolved.entry),
                context_id,
                args,
                None,
            ) {
                log::info!(
                    "open_view_by_path_with_layout: focused existing {plugin_id} pane {existing}"
                );
                return Ok(Some(existing));
            }
        }
        if installed.manifest.manifest_type == crate::app::registry::ManifestType::Wasm {
            let pane_id = self.open_installed_wasm_view_pane(
                self.active_window,
                crate::host::plugin_admission::OpenedBy::Path,
                installed,
                workspace_root,
                layout_hint.as_deref(),
                args.to_vec(),
            )?;
            return Ok(Some(pane_id));
        }
        if installed.bin_path.extension().and_then(|ext| ext.to_str()) == Some("py")
            || installed.bin_path.extension().and_then(|ext| ext.to_str()) == Some("pyc")
        {
            return self
                .open_python_wasm_view_pane(
                    self.active_window,
                    crate::host::plugin_admission::OpenedBy::Path,
                    resolved,
                    workspace_root,
                    layout_hint.as_deref(),
                    args.to_vec(),
                )
                .map(Some);
        }
        Err(format!("plugin '{plugin_id}' has no WASM runtime"))
    }

    /// Open the secrets manager (read-only vault viewer, full pane, no terminal split).
    pub(crate) fn open_secrets_manager(&mut self) {
        if self.plugin_blocked("secrets_manager") {
            return;
        }
        // Toggle: if already open, close it.
        let ctx = &self.windows[self.active_window];
        if let Some(focused) = ctx.focused_pane {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = ctx.tree.tiles.get(focused) {
                if let Some(pane) = ctx.panes.get(pane_id) {
                    if let Some(a) = pane.as_view() {
                        if a.runtime.type_id() == "secrets_manager" {
                            self.close_focused_view();
                            return;
                        }
                    }
                }
            }
        }

        let cwd = {
            let ctx = &self.windows[self.active_window];
            ctx.focused_pane
                .and_then(|tile_id| ctx.get_focused_pane_cwd(tile_id))
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        };

        let app = Box::new(crate::app::secrets_view::SecretsView::new(cwd.clone()));
        let perms = crate::app::permissions::PluginPermissions::builtin();
        self.open_builtin_view_pane(app, perms, cwd, None, Some("overlay"), None);
    }

    /// Open the command view pane for `workspace`. One pane per context.
    pub(crate) fn open_command_view(&mut self, workspace: &std::path::Path) -> Result<u64, String> {
        self.require_plugin("agents-dashboard")?;
        if !crate::release::feature_enabled(crate::release::ReleaseFeature::Assistant) {
            return Err(crate::release::feature_unavailable_text(
                crate::release::ReleaseFeature::Assistant,
            ));
        }
        let context_id = self.windows[self.active_window].context_id;
        let workspace = crate::platform::path::canonical_or_self(workspace);
        // Only a dashboard already showing this workspace is reused, found
        // among every dashboard in the context: another one's pane would
        // serve that workspace's projection.
        let showing = |view: &crate::host::pane::ViewPane| {
            crate::platform::path::canonical_or_self(&view.workspace_root) == workspace
        };
        if let Some((win_idx, slot, depth)) = self.locate_view_instance_where(
            "agents-dashboard",
            Some(context_id),
            OriginQuery::Builtin,
            &showing,
        ) {
            self.reveal_and_focus_instance(win_idx, slot, depth);
            return Ok(slot);
        }
        let app = Box::new(crate::app::agents_dashboard_view::AgentsDashboardView::new(
            workspace.to_path_buf(),
        ));
        let perms = crate::app::permissions::PluginPermissions::builtin();
        let predicted = self.host.next_pane_id();
        log::info!("command_view: opening pane for {}", workspace.display());
        self.open_builtin_view_pane(
            app,
            perms,
            workspace.to_path_buf(),
            None,
            Some("split_h"),
            None,
        );
        Ok(predicted)
    }

    /// Open (or focus) the host Assistant pane (Phase 1 of
    /// `docs/assistant-host-plugin.md`, reachable via Cmd+Ctrl+A and the
    /// Cmd+P palette). Opens as an overlay — it overtakes the focused pane
    /// like every other plugin launch. One Assistant pane per *context*: if one
    /// already exists in any window of the caller's context it is revealed
    /// and focused (cross-window) instead of spawning a duplicate.
    /// Conversation state is persisted per context on disk, so
    /// close-then-reopen resumes that context's conversation.
    ///
    /// Builtin exception to the manifest `[launch] on_launch` policy (#0336):
    /// the Assistant is a host builtin with its own entry point and is never
    /// routed through the registry resolver, so its dedup — matching
    /// `focus_existing_in_context` semantics — is kept hardcoded here.
    pub(crate) fn open_assistant_pane(&mut self) {
        self.open_assistant_pane_with_hint("overlay");
    }

    /// Returns the pane id of the focused-or-spawned Assistant, or `None`
    /// when the feature gate blocks the open.
    fn open_assistant_pane_with_hint(&mut self, hint: &str) -> Option<PaneId> {
        self.open_assistant_pane_with_hint_at(self.active_window, hint)
    }

    fn open_assistant_pane_with_hint_at(
        &mut self,
        target_win: usize,
        hint: &str,
    ) -> Option<PaneId> {
        if self.plugin_blocked("assistant") {
            return None;
        }
        if !crate::release::feature_enabled(crate::release::ReleaseFeature::Assistant) {
            crate::release::log_feature_blocked(crate::release::ReleaseFeature::Assistant);
            return None;
        }
        let caller_context_id = self.windows[target_win].context_id;
        // Focus an existing Assistant pane anywhere in this context instead
        // of opening a second one.
        if let Some((win_idx, slot, overlay_depth)) =
            self.locate_view_instance("assistant", Some(caller_context_id), OriginQuery::Builtin)
        {
            log::info!(
                "assistant: focusing existing instance at pane {slot} in context {caller_context_id}"
            );
            self.reveal_and_focus_instance(win_idx, slot, overlay_depth);
            return Some(slot);
        }

        let workspace_root = self
            .context_root_for(caller_context_id)
            .or_else(crate::config::active_workspace_root)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        log::info!(
            "assistant: opening pane for workspace {} in context {caller_context_id}",
            workspace_root.display()
        );
        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::with_model_modes(
                self.config.ai.clone(),
                self.config.resolved_model_modes(),
            ));
        let app = Box::new(crate::assistant::AssistantView::new(
            workspace_root.clone(),
            broker,
            &crate::config::config_dir(),
            caller_context_id,
        ));
        let perms = crate::app::permissions::PluginPermissions::builtin();
        match self.open_builtin_view_pane_at(
            BuiltinPlacement {
                window: target_win,
                hint: Some(hint),
                share: None,
            },
            app,
            perms,
            workspace_root,
            None,
        ) {
            Ok(pane) => Some(pane),
            Err(reason) => {
                log::info!("assistant: not opened: {reason}");
                None
            }
        }
    }

    /// Reuse an Assistant in `context_id` (or the active window's context), or
    /// insert a hidden one that is not in the tile tree. CLI sends call this when they did not name a pane, so `assistant send`
    /// works without `view open assistant`. A caller that names a missing pane
    /// must not land here — that stays `assistant_pane_not_found`.
    ///
    /// Returns `None` when the Assistant feature is off or the context has no
    /// window.
    pub(crate) fn ensure_headless_assistant(&mut self, context_id: Option<u64>) -> Option<PaneId> {
        if self.plugin_blocked("assistant") {
            return None;
        }
        if !crate::release::feature_enabled(crate::release::ReleaseFeature::Assistant) {
            crate::release::log_feature_blocked(crate::release::ReleaseFeature::Assistant);
            return None;
        }
        let win_idx = match context_id {
            Some(id) => self
                .windows
                .iter()
                .position(|window| window.context_id == id)?,
            None => self.active_window,
        };
        let caller_context_id = self.windows[win_idx].context_id;
        for window in &self.windows {
            if window.context_id != caller_context_id {
                continue;
            }
            for (id, pane) in window.panes.iter() {
                if pane
                    .as_view()
                    .is_some_and(|app| app.runtime.type_id() == "assistant")
                {
                    log::info!(
                        "assistant: reusing pane {id} in context {caller_context_id} for a headless send"
                    );
                    return Some(*id);
                }
            }
        }

        let workspace_root = self
            .context_root_for(caller_context_id)
            .or_else(crate::config::active_workspace_root)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::with_model_modes(
                self.config.ai.clone(),
                self.config.resolved_model_modes(),
            ));
        let app = Box::new(crate::assistant::AssistantView::new(
            workspace_root.clone(),
            broker,
            &crate::config::config_dir(),
            caller_context_id,
        ));
        let id = self.host.alloc_pane_id();
        log::info!(
            "assistant: created headless pane {id} (not in the tile tree) for a headless send in context {caller_context_id} workspace {}",
            workspace_root.display()
        );
        if self.windows[win_idx]
            .panes
            .insert(
                id,
                Pane::View(Box::new(crate::host::pane::ViewPane {
                    incarnation: Default::default(),
                    pip_status: None,
                    id,
                    runtime: crate::host::pane::ViewRuntime::Builtin(app),
                    workspace_root,
                    permissions: crate::app::permissions::PluginPermissions::builtin(),
                    manifest_id: "assistant".to_string(),
                    name: "Assistant".to_string(),
                    pane_group: None,
                    linked_pane_id: None,
                    overlay_replaced: None,
                    agent: None,
                    slots: std::collections::HashMap::new(),
                    semantic_state: Default::default(),
                })),
            )
            .is_err()
        {
            return None;
        }
        let context_root = self.context_root_for(caller_context_id);
        crate::host::event_log::emit_scoped(
            crate::host::event_log::HostEvent::ViewOpened {
                plugin_id: "assistant".to_string(),
                type_id: "assistant".to_string(),
                pane_id: id,
                timestamp: crate::host::event_log::now_timestamp(),
            },
            context_root.as_deref(),
        );
        Some(id)
    }

    /// Create a new scratch note in `notes/inbox/` and open it in a text-editor
    /// pane. Each press creates a fresh timestamped note — scratch notes share
    /// the inbox with quick notes and flow through the same triage. Notes whose
    /// body is still empty on close are deleted by the editor.
    ///
    /// Works from any focused pane type (terminal, plugin, file browser).
    pub(crate) fn open_scratchpad(&mut self) {
        if self.plugin_blocked("notes") {
            return;
        }
        log::info!("scratchpad: opening new scratch note");
        let active = self.active_window;
        let target_pane_id = self.windows[active].focused_pane.and_then(|tile_id| {
            match self.windows[active].tree.tiles.get(tile_id) {
                Some(Tile::Pane(pane_id)) => Some(*pane_id),
                _ => None,
            }
        });

        // Capture provenance the same way a quick note does.
        let cwd = self.windows[active]
            .focused_pane
            .and_then(|tile_id| self.windows[active].get_focused_pane_cwd(tile_id))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let ctx = crate::app::QuickNoteCtx {
            cwd,
            workspace_root: crate::config::active_workspace_root(),
        };
        let dir = crate::notes::context_notes_dir(&self.router.active().root);
        let Some(path) = Self::write_note("", "scratchpad", &dir, &ctx, &[]) else {
            log::warn!("scratchpad: failed to create scratch note in {:?}", dir);
            return;
        };
        self.invalidate_notes_index();

        let path_str = path.display().to_string();
        log::info!("scratchpad: opening text-editor pane for {:?}", path);
        match self.launch_view_by_id_with_layout(
            "text-editor",
            None,
            std::slice::from_ref(&path_str),
            None,
        ) {
            Ok(_) => {
                let pane_id = target_pane_id.or_else(|| {
                    self.windows[active].focused_pane.and_then(|tile_id| {
                        match self.windows[active].tree.tiles.get(tile_id) {
                            Some(Tile::Pane(pane_id)) => Some(*pane_id),
                            _ => None,
                        }
                    })
                });
                if let Some(pane_id) = pane_id {
                    let context_root = self
                        .origin_for_pane(pane_id)
                        .map(|origin| origin.context_root);
                    crate::host::event_log::emit_scoped(
                        crate::host::event_log::HostEvent::ScratchpadOpened {
                            pane_id,
                            path: path_str,
                            timestamp: crate::host::event_log::now_timestamp(),
                        },
                        context_root.as_deref(),
                    );
                } else {
                    log::warn!("scratchpad: opened note but could not resolve pane id");
                }
            }
            Err(e) => {
                log::warn!("scratchpad: failed to launch text-editor pane: {e}");
            }
        }
    }

    /// Open the quick note modal: capture context and push FocusKind::QuickNote.
    pub(crate) fn open_quick_note_modal(&mut self) {
        if self.plugin_blocked("notes") {
            return;
        }
        let active = self.active_window;
        let cwd = self.windows[active]
            .focused_pane
            .and_then(|tile_id| self.windows[active].get_focused_pane_cwd(tile_id))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let workspace_root = crate::config::active_workspace_root();
        self.quick_note_text = String::new();
        self.quick_note_attachments.clear();
        self.quick_note_ctx = crate::app::QuickNoteCtx {
            cwd,
            workspace_root,
        };
        self.push_focus_layer(crate::app::FocusKind::QuickNote);
        log::info!(
            "QuickNote: modal opened — cwd={}, workspace={:?}",
            self.quick_note_ctx.cwd.display(),
            self.quick_note_ctx.workspace_root
        );
    }

    /// Commit a quick note into the active context's notes tier. Returns
    /// `false` if the write fails.
    pub(crate) fn commit_quick_note(&mut self, text: &str) -> bool {
        let ctx = self.quick_note_ctx.clone();
        let dir = crate::notes::context_notes_dir(&self.router.active().root);
        log::info!(
            "QuickNote: committing to {} attachments={}",
            dir.display(),
            self.quick_note_attachments.len()
        );
        let wrote = Self::write_note(text, "quick-note", &dir, &ctx, &self.quick_note_attachments)
            .is_some();
        if wrote {
            self.invalidate_notes_index();
        }
        wrote
    }

    /// Validate and stage one local image for the open QuickNote modal. The
    /// file remains at its original path until the note is saved, preventing
    /// discarded modals from creating unattached collection assets.
    pub(crate) fn stage_quick_note_image(&mut self, source: PathBuf) -> Result<(), String> {
        let bytes = std::fs::read(&source)
            .map_err(|e| format!("failed to read dropped image {}: {e}", source.display()))?;
        image::load_from_memory(&bytes)
            .map_err(|e| format!("not a decodable image ({}): {e}", source.display()))?;
        if self
            .quick_note_attachments
            .iter()
            .any(|path| path == &source)
        {
            log::info!("QuickNote: drop already staged source={}", source.display());
            return Ok(());
        }
        log::info!(
            "QuickNote: staged image source={} bytes={}",
            source.display(),
            bytes.len()
        );
        self.quick_note_attachments.push(source);
        Ok(())
    }

    /// Write a timestamped note with capture-context frontmatter into `dir`.
    /// Returns the created path, or `None` when the write fails.
    /// Write a capture into `dir` (a notes tier), returning its path.
    ///
    /// Frontmatter carries an empty `title` the editor fills in, plus capture
    /// provenance as plain text. Provenance is not what locates a note — the tier
    /// it was written into is — so nothing parses these lines back.
    fn write_note(
        text: &str,
        source: &str,
        dir: &PathBuf,
        ctx: &crate::app::QuickNoteCtx,
        attachments: &[PathBuf],
    ) -> Option<PathBuf> {
        use chrono::Utc;
        let now = Utc::now();
        let captured_at = now.to_rfc3339();
        let stamp = now.format("%Y%m%d-%H%M%S%.3f").to_string();

        let cwd_str = ctx.cwd.to_string_lossy();
        let frontmatter = format!(
            "---\ntitle: \"\"\ncaptured_at: \"{captured_at}\"\nsource: \"{source}\"\ncwd: \"{cwd_str}\"\n---\n\n"
        );
        let persisted_attachments = match persist_quick_note_attachments(attachments, dir) {
            Ok(attachments) => attachments,
            Err(e) => {
                log::error!("QuickNote: failed to persist staged image: {e}");
                return None;
            }
        };
        let attachment_markdown = persisted_attachments
            .references
            .iter()
            .map(|reference| format!("![]({reference})"))
            .collect::<Vec<_>>()
            .join("\n");
        let body = match (text.trim(), attachment_markdown.is_empty()) {
            ("", true) => String::new(),
            ("", false) => attachment_markdown,
            (body, true) => body.to_string(),
            (body, false) => format!("{body}\n\n{attachment_markdown}"),
        };
        let content = format!("{frontmatter}{body}\n");

        if let Err(e) = std::fs::create_dir_all(dir) {
            rollback_quick_note_assets(&persisted_attachments.newly_written);
            log::error!(
                "QuickNote: failed to create notes dir {}: {e}",
                dir.display()
            );
            return None;
        }
        // Use O_CREAT|O_EXCL (create_new) so concurrent scratchpad calls can
        // never overwrite each other — the check-then-write pattern has a TOCTOU
        // window that is safe today but unsafe if notes are written in parallel.
        let mut path = dir.join(format!("note-{stamp}.md"));
        let mut n = 1u32;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write as _;
                    match file.write_all(content.as_bytes()) {
                        Ok(()) => {
                            log::info!("QuickNote: saved to {}", path.display());
                            return Some(path);
                        }
                        Err(e) => {
                            // create_new already created the inode; remove it so
                            // a partial write doesn't leave a ghost file blocking
                            // any future attempt at the same path.
                            let _ = std::fs::remove_file(&path);
                            rollback_quick_note_assets(&persisted_attachments.newly_written);
                            log::error!("QuickNote: save failed {}: {e}", path.display());
                            return None;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    path = dir.join(format!("note-{stamp}-{n}.md"));
                    n += 1;
                }
                Err(e) => {
                    rollback_quick_note_assets(&persisted_attachments.newly_written);
                    log::error!("QuickNote: save failed {}: {e}", path.display());
                    return None;
                }
            }
        }
    }
}

/// Persist QuickNote attachments into the collection-level `notes/assets/`
/// directory. Inbox notes live under `notes/inbox/`, so their Markdown paths
/// are always relative `../assets/<name>` references.
struct PersistedQuickNoteAttachments {
    references: Vec<String>,
    newly_written: Vec<PathBuf>,
}

/// Copy every staged image into `<tier_dir>/assets/`, returning the Markdown
/// references to embed. A tier is a flat directory that owns its own `assets/`,
/// so references are `assets/<name>` — relative to the note, and inside the
/// context root, which is what lets a project's notes and their attachments move
/// together.
fn persist_quick_note_attachments(
    attachments: &[PathBuf],
    tier_dir: &Path,
) -> Result<PersistedQuickNoteAttachments, String> {
    if attachments.is_empty() {
        return Ok(PersistedQuickNoteAttachments {
            references: Vec::new(),
            newly_written: Vec::new(),
        });
    }
    let assets_dir = tier_dir.join("assets");
    std::fs::create_dir_all(&assets_dir)
        .map_err(|e| format!("failed to create {}: {e}", assets_dir.display()))?;

    let mut references = Vec::with_capacity(attachments.len());
    let mut newly_written = Vec::new();
    for source in attachments {
        let bytes = match std::fs::read(source) {
            Ok(bytes) => bytes,
            Err(e) => {
                rollback_quick_note_assets(&newly_written);
                return Err(format!(
                    "failed to read staged image {}: {e}",
                    source.display()
                ));
            }
        };
        if let Err(e) = image::load_from_memory(&bytes) {
            rollback_quick_note_assets(&newly_written);
            return Err(format!("not a decodable image ({}): {e}", source.display()));
        }
        let format = match image::guess_format(&bytes) {
            Ok(format) => format,
            Err(e) => {
                rollback_quick_note_assets(&newly_written);
                return Err(format!(
                    "unrecognized image container ({}): {e}",
                    source.display()
                ));
            }
        };
        let ext = format.extensions_str().first().copied().unwrap_or("img");
        let hash = quick_note_asset_hash(&bytes);
        let stem = quick_note_asset_stem(source);
        let mut name = format!("{stem}-{hash:016x}.{ext}");
        let mut suffix = 2usize;
        let deduped = loop {
            let candidate = assets_dir.join(&name);
            if !candidate.exists() {
                if let Err(e) = std::fs::write(&candidate, &bytes) {
                    if let Err(cleanup_error) = std::fs::remove_file(&candidate) {
                        if cleanup_error.kind() != std::io::ErrorKind::NotFound {
                            log::error!(
                                "QuickNote: failed to remove partially written image asset {} after save failure: {cleanup_error}",
                                candidate.display()
                            );
                        }
                    }
                    rollback_quick_note_assets(&newly_written);
                    return Err(format!("failed to save {}: {e}", candidate.display()));
                }
                newly_written.push(candidate);
                break false;
            }
            match std::fs::read(&candidate) {
                Ok(existing) if existing == bytes => break true,
                Ok(_) => {
                    name = format!("{stem}-{hash:016x}-{suffix}.{ext}");
                    suffix += 1;
                }
                Err(e) => {
                    rollback_quick_note_assets(&newly_written);
                    return Err(format!(
                        "failed to inspect existing asset {}: {e}",
                        candidate.display()
                    ));
                }
            }
        };
        let reference = format!("assets/{name}");
        log::info!(
            "QuickNote: persisted image asset={reference} bytes={} hash={hash:016x} deduped={deduped}",
            bytes.len()
        );
        references.push(reference);
    }
    Ok(PersistedQuickNoteAttachments {
        references,
        newly_written,
    })
}

fn rollback_quick_note_assets(paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    log::info!(
        "QuickNote: rolling back {} newly persisted image assets",
        paths.len()
    );
    for path in paths {
        if let Err(e) = std::fs::remove_file(path) {
            log::error!(
                "QuickNote: failed to roll back image asset {}: {e}",
                path.display()
            );
        }
    }
}

fn quick_note_asset_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn quick_note_asset_stem(source: &Path) -> String {
    let raw = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("image");
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let stem = sanitized.trim_matches('-');
    if stem.is_empty() {
        "image".to_string()
    } else {
        stem.to_string()
    }
}

/// Resolve the pane placement for a CLI- or spawn-request-initiated view open.
///
/// The caller's pane is typically the agent's own terminal, so a CLI open must
/// never take over it (`overlay`). Precedence: an explicit layout from the
/// command flag, then the plugin's manifest `[launch] placement`, then a sibling
/// split (`split_h`). Manifest placement still overrides the default; only the
/// fallback changed from `overlay` to `split_h` (stint 0330). `manifest_placement`
/// must already be validated against [`crate::app::registry::VALID_PLACEMENTS`].
pub(crate) fn cli_open_placement(
    explicit: Option<String>,
    manifest_placement: Option<String>,
) -> String {
    explicit
        .or(manifest_placement)
        .unwrap_or_else(|| "split_h".to_string())
}

/// Build a fresh timestamped note path in the global notes tier. Does not create
/// the file.
///
/// The global tier, not a context tier, because the only caller is
/// `builtin_factory` — which resolves a plugin id with no host state to consult, so
/// it has no context to anchor to. `open_scratchpad` does have one and writes to
/// the active context's tier instead.
fn new_scratch_note_path() -> PathBuf {
    let filename = format!("note-{}.md", chrono::Utc::now().format("%Y%m%d-%H%M%S%.3f"));
    crate::notes::global_notes_dir().join(filename)
}

/// Rows a launch will mint once its guest has started (stint 1733).
/// Dropping it writes nothing.
#[derive(Default)]
#[must_use = "commit once the guest has started, or drop to write nothing"]
struct PendingMints(
    Vec<(
        std::sync::Arc<crate::broker::gate::PermissionMonitor>,
        crate::broker::gate::CapabilityMint,
    )>,
);

impl PendingMints {
    fn add(
        &mut self,
        monitor: std::sync::Arc<crate::broker::gate::PermissionMonitor>,
        mint: crate::broker::gate::CapabilityMint,
    ) {
        self.0.push((monitor, mint));
    }

    /// The capabilities these mints will grant.
    fn capability_ids(&self) -> impl Iterator<Item = &str> {
        self.0.iter().flat_map(|(_, mint)| mint.capability_ids())
    }

    /// Write the planned rows: the guest started.
    fn commit(self) {
        for (monitor, mint) in self.0 {
            monitor.commit_capability_mint(mint);
        }
    }
}

/// Validate the whole manifest before minting any grants, for launch and
/// restore. The rows come back planned, to commit once the guest starts.
fn wasm_manifest_permissions(
    profile_dir: &std::path::Path,
    plugin_id: &str,
    workspace_root: &std::path::Path,
    capability_strings: &[String],
    allowed_hosts: Vec<String>,
) -> Result<(crate::app::permissions::PluginPermissions, PendingMints), String> {
    let declared = crate::app::permissions::parse_capability_strings(capability_strings)
        .map_err(|error| format!("manifest lists {error}"))?;
    let monitor = crate::broker::gate::PermissionMonitor::for_profile(profile_dir)
        .map_err(|error| error.to_string())?;
    let (mut permissions, mint) = monitor.plan_plugin_permissions(
        &crate::broker::principal::PluginInstance::new(plugin_id, None),
        workspace_root,
        &declared,
        allowed_hosts,
    );
    let mut mints = PendingMints::default();
    mints.add(monitor, mint);
    apply_media_release_gate(plugin_id, &mut permissions);
    Ok((permissions, mints))
}

/// A Python plugin's launch set, with the rows it would mint planned, to
/// commit once the guest starts (stint 1733).
fn plan_plugin_permissions_in_profile(
    profile_dir: &std::path::Path,
    plugin_id: &str,
    workspace_root: &std::path::Path,
    capability_strings: &[String],
    allowed_hosts: Vec<String>,
) -> (crate::app::permissions::PluginPermissions, PendingMints) {
    let declared = capability_strings
        .iter()
        .filter_map(|capability| {
            crate::app::permissions::Capability::try_from(capability.as_str()).ok()
        })
        .collect();
    let mut mints = PendingMints::default();
    // Outside the host nothing is granted (stint 1291).
    let mut permissions = match crate::broker::gate::PermissionMonitor::for_profile(profile_dir) {
        Ok(monitor) => {
            let (permissions, mint) = monitor.plan_plugin_permissions(
                &crate::broker::principal::PluginInstance::new(plugin_id, None),
                workspace_root,
                &declared,
                allowed_hosts,
            );
            mints.add(monitor, mint);
            permissions
        }
        Err(error) => {
            log::error!("plugin::{plugin_id}: launched with no grants: {error}");
            crate::app::permissions::PluginPermissions::default()
        }
    };
    apply_media_release_gate(plugin_id, &mut permissions);
    (permissions, mints)
}

/// The one release policy for protocol media operations: unless the
/// `media_io` experiment is on, every media capability is withheld before the
/// plugin's permissions reach any dispatch path. Additive to the capability
/// model — it only removes grants.
fn apply_media_release_gate(
    plugin_id: &str,
    permissions: &mut crate::app::permissions::PluginPermissions,
) {
    if permissions.is_builtin
        || crate::release::feature_enabled(crate::release::ReleaseFeature::MediaIo)
    {
        return;
    }
    let withheld = permissions.withhold_media_io();
    if !withheld.is_empty() {
        log::info!(
            "release_gate: withheld media capabilities {withheld:?} from {plugin_id} (media_io off)"
        );
        crate::release::log_feature_blocked(crate::release::ReleaseFeature::MediaIo);
    }
}

fn gate_wasm_sets(
    plugin_id: &str,
    workspace_root: &std::path::Path,
    declared: &std::collections::HashSet<String>,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    gate_wasm_sets_in_profile(
        &crate::config::config_dir(),
        plugin_id,
        workspace_root,
        declared,
    )
}

fn gate_wasm_sets_in_profile(
    profile_dir: &std::path::Path,
    plugin_id: &str,
    workspace_root: &std::path::Path,
    declared: &std::collections::HashSet<String>,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    match crate::broker::gate::PermissionMonitor::for_profile(profile_dir) {
        Ok(monitor) => monitor.materialize_wasm_sets(
            &crate::broker::principal::PluginInstance::new(plugin_id, None),
            workspace_root,
            declared,
        ),
        Err(error) => {
            log::error!("plugin::{plugin_id}: no remembered WASM grants: {error}");
            Default::default()
        }
    }
}

fn wasm_runtime_capabilities_from_permissions(
    permissions: &crate::app::permissions::PluginPermissions,
    workspace_root: &std::path::Path,
) -> std::collections::HashSet<String> {
    use crate::app::permissions::Capability;

    let mut caps = std::collections::HashSet::new();
    if permissions.capabilities.contains(&Capability::FsRead) {
        caps.insert(format!("fs:read:{}", workspace_root.display()));
    }
    if permissions.capabilities.contains(&Capability::FsWrite) {
        caps.insert(format!("fs:write:{}", workspace_root.display()));
    }
    if permissions.capabilities.contains(&Capability::NetHttp) {
        if permissions.allowed_hosts.is_empty() {
            caps.insert("net:fetch:*".to_string());
        } else {
            caps.extend(
                permissions
                    .allowed_hosts
                    .iter()
                    .map(|host| format!("net:fetch:{host}")),
            );
        }
    }
    if permissions.capabilities.contains(&Capability::AiQuery) {
        caps.insert("ai.query".to_string());
    }
    caps
}

fn wasm_state_path(plugin_id: &str, workspace_root: &std::path::Path) -> PathBuf {
    wasm_state_path_in_profile(&crate::config::config_dir(), plugin_id, workspace_root)
}

fn wasm_state_path_in_profile(
    profile_dir: &std::path::Path,
    plugin_id: &str,
    workspace_root: &std::path::Path,
) -> PathBuf {
    profile_dir
        .join("wasm-state")
        .join(sanitize_wasm_path_component(plugin_id))
        .join(format!(
            "{}.json",
            sanitize_wasm_path_component(&workspace_root.display().to_string())
        ))
}

/// A bare `.wasm` launched without a manifest is identified by its file
/// name, and that id is its permission principal and state key. Rewriting the
/// name would let two files share one id and so one set of grants, so a name
/// that is not a valid plugin id is refused instead (stint 1344).
fn sanitize_wasm_path_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "root".to_string()
    } else {
        out
    }
}

/// Who an agent worker is attributed to and where its admission placed it,
/// both decided before the spawn. Neither grants anything.
pub(crate) struct WorkerAttribution {
    pub(crate) spawned_by: Option<String>,
    pub(crate) anchor_pane: Option<u64>,
}

/// What agent worker to start, and from where.
pub(crate) struct SpawnedWorker {
    pub(crate) pane_id: PaneId,
    pub(crate) context_id: u64,
    pub(crate) harness: String,
    pub(crate) command: String,
}

pub(crate) enum WorkerCommand<'a> {
    Raw {
        harness: &'a str,
        command: &'a str,
        ephemeral: bool,
        result_schema: Option<serde_json::Value>,
    },
    Configured {
        ephemeral: Option<bool>,
        admitted: Option<&'a ConfiguredAgentLaunch>,
    },
}

pub(crate) struct WorkerSpawn<'a> {
    pub(crate) role: &'a str,
    pub(crate) command: WorkerCommand<'a>,
    pub(crate) task: &'a str,
    pub(crate) subcontext: Option<&'a str>,
    pub(crate) from_pane_id: Option<u64>,
    pub(crate) cwd_override: Option<std::path::PathBuf>,
    pub(crate) run_token: Option<&'a str>,
    pub(crate) conjoined: bool,
    pub(crate) origin_context_id: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_plugin_error_truncates_long_id() {
        let mut h = HostHarness::new();
        let dir = crate::app::registry::plugins_dir().join("skipped");
        std::fs::create_dir_all(&dir).unwrap();
        let id = "a".repeat(2000);
        std::fs::write(dir.join("manifest.toml"), format!(
            "schema_version = 2\n[plugin]\nid = \"{id}\"\nname = \"Skipped\"\nversion = \"0.1.0\"\nwatch = true\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n"
        )).unwrap();
        let root = h.app.router.active().root.clone();
        h.app.registries =
            crate::app::registry_views::RegistryViews::new_with_global(dir.parent().unwrap());
        let error = h
            .app
            .launch_view_by_id_with_layout(&id, None, &[], Some(root))
            .expect_err("skipped plugin must not launch");
        assert!(error.contains("could not be loaded"), "{error}");
        assert!(
            error.contains(&crate::host::guest_text::shown(&id)),
            "{error}"
        );
        assert!(!error.contains(&id), "raw id must be truncated: {error}");
    }

    /// Stint 1344: a bare `.wasm` is named by its file, and that id is its
    /// permission principal. Two names that used to reduce to one id must not
    /// share it: an invalid name is refused, never rewritten.
    #[test]
    fn bare_wasm_ids_are_refused_not_rewritten() {
        let ok = crate::app::registry::bare_wasm_plugin_id(Path::new("/plugins/a_b.wasm"))
            .expect("valid name");
        assert_eq!(ok, "a_b");
        for name in ["a b", "a\u{1b}b", "..ab", "a..b", ".hidden"] {
            let error = crate::app::registry::bare_wasm_plugin_id(
                &Path::new("/plugins").join(format!("{name}.wasm")),
            )
            .expect_err(name);
            assert!(
                error.contains("rename the file or add a manifest.toml"),
                "{error}"
            );
            assert!(!error.contains('\u{1b}'), "{error:?}");
        }
    }
    use crate::app::permissions::PluginPermissions;
    use crate::app::registry::PluginRegistry;
    use crate::host::wasm_view::{StateSnapshot, StateStore, WasmView};
    use crate::testing::HostHarness;

    /// Stint 1440: a launch-failed tile beside a focused pane splits through
    /// `split_with_new_pane_at` and leaves the window settled.
    #[test]
    fn launch_failed_tile_beside_a_pane_leaves_window_settled() {
        let mut h = HostHarness::new();
        let a = h.add_test_pane();
        let tile = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
        h.app.windows[0].focused_pane = Some(tile);
        h.app.windows[0].simplify_and_reconcile();
        h.assert_window_settled(0, "fixture");
        let cwd = h.workspace_root();
        h.app.open_launch_failed_pane(
            0,
            "needs-ai",
            None,
            vec!["ai.query requires [ai] config".to_string()],
            cwd,
        );
        h.assert_window_settled(0, "launch-failed tile beside a pane");
    }

    /// Stint 1511: spawning a terminal beside a pane, with or without taking
    /// focus, leaves the window settled like the other split callers.
    #[test]
    fn spawn_terminal_pane_at_leaves_window_settled() {
        for keep_focus in [false, true] {
            let mut h = HostHarness::new();
            let a = h.add_test_pane();
            let tile = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
            h.app.windows[0].focused_pane = Some(tile);
            h.app.windows[0].simplify_and_reconcile();
            h.assert_window_settled(0, "fixture");
            let tile = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
            let context = h.app.pane_context_env_for_window(0);
            let launch = h.app.desktop_terminal_launch(context, None, None);
            let new_id = h
                .app
                .spawn_terminal_pane_at(
                    SplitPlacement {
                        win_idx: 0,
                        target_tile: tile,
                        vertical: true,
                        new_pane_first: false,
                        keep_focus,
                    },
                    launch,
                )
                .expect("terminal spawned");
            assert!(
                h.app.windows[0].panes.contains_key(&new_id),
                "terminal spawned"
            );
            h.assert_window_settled(
                0,
                &format!("terminal spawned beside a pane, keep_focus={keep_focus}"),
            );
            let focused = h.app.windows[0].focused_pane.and_then(|tile| {
                crate::app::PlexiApp::find_pane_in_tile(&h.app.windows[0].tree, tile)
            });
            let expected = if keep_focus { a } else { new_id };
            assert_eq!(
                focused,
                Some(expected),
                "focus after the settle, keep_focus={keep_focus}"
            );
        }
    }

    fn counter_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/counter.wasm")
    }

    /// A loadable native-WASM component and the state snapshot the pane opens
    /// with. Snapshot is taken before the store moves into the plugin, mirroring the
    /// production launch path.
    fn load_counter() -> (WasmView, StateSnapshot) {
        let store = StateStore::ephemeral();
        let snapshot = store.snapshot();
        let app = WasmView::load_ephemeral_run("counter-pane", &counter_fixture(), store)
            .expect("load counter fixture");
        (app, snapshot)
    }

    fn installed_wasm_timer_case(declares_timer: bool) {
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
        let mut h = HostHarness::new();
        let apps = tempfile::tempdir().expect("apps");
        let plugin_dir = apps.path().join("sysmon");
        std::fs::create_dir(&plugin_dir).expect("plugin dir");
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut manifest =
            std::fs::read_to_string(root.join("plugins/wasm-poc/sysmon/manifest.toml"))
                .expect("sysmon manifest");
        // An entry must be a file inside its plugin folder (stint 1398).
        std::fs::copy(
            root.join("tests/wasm-fixtures/sysmon.wasm"),
            plugin_dir.join("sysmon.wasm"),
        )
        .expect("installed entry");
        manifest = manifest.replace(
            "../../../target/wasm32-wasip1/release/sysmon.wasm",
            "sysmon.wasm",
        );
        if !declares_timer {
            manifest = manifest.replace("capabilities = [\"timer\"]", "capabilities = []");
        }
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).expect("manifest");
        h.trust_plugin_folder(apps.path());
        let registry = PluginRegistry::load_with_global(apps.path(), apps.path());
        let installed = registry
            .get("com.plexi.sysmon")
            .expect("installed sysmon")
            .clone();
        let workspace_root = h.workspace_root();
        let pane_id = h
            .app
            .open_installed_wasm_view_pane(
                0,
                crate::host::plugin_admission::OpenedBy::Id,
                installed,
                workspace_root,
                None,
                Vec::new(),
            )
            .expect("launch without timer consent");
        let view = h.app.windows[0]
            .panes
            .get_mut(&pane_id)
            .unwrap()
            .as_view_mut()
            .unwrap();
        let crate::host::pane::ViewRuntime::Wasm(live) = &mut view.runtime else {
            panic!("expected WASM");
        };
        let colors = crate::ui::theme::Colors::from_config(&crate::config::ThemeConfig::default());
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::ui::theme::setup_fonts(ui.ctx())
        });
        let logs = crate::testing::test_log::capture();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            live.ui(ui, &colors, None, pane_id);
        });
        assert_eq!(
            live.timer_deadline_for_test().is_some(),
            declares_timer,
            "manifest timer declaration must govern the init timer"
        );
        assert!(!live.has_pending_capability_prompt());
        if !declares_timer {
            assert!(logs.records().contains(&(
                log::Level::Info,
                "plugin::com.plexi.sysmon: set_timer denied: missing capability timer".to_string()
            )));
        }
    }

    #[test]
    fn installed_wasm_manifest_timer_arms() {
        installed_wasm_timer_case(true);
    }

    fn restore_wasm_timer_case(fixture: &str, requires_review: bool, manifest_case: &str) {
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
        let h = HostHarness::new();
        let plugin_dir = h.profile_dir().join("plugins/timer-only");
        std::fs::create_dir_all(&plugin_dir).expect("plugin dir");
        let wasm_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/wasm-fixtures/{fixture}.wasm"));
        assert_eq!(
            WasmView::inspect_required_grants(&wasm_path)
                .expect("inspect timer fixture")
                .capability_ids()
                .is_empty(),
            !requires_review,
            "fixture imports must isolate timer from sensitive review"
        );
        let manifest = r#"schema_version = 2
[plugin]
id = "com.plexi.timer-only"
name = "Timer only"
version = "0.1.0"
[plugin.capabilities]
capabilities = ["timer"]
[runtime]
kind = "wasm"
entry = 'component.wasm'
[[contributes.views]]
id = "com.plexi.timer-only.main"
default = true
"#
        .to_string();
        // An entry must live inside its plugin folder (stint 1398).
        let installed_wasm = plugin_dir.join("component.wasm");
        std::fs::copy(&wasm_path, &installed_wasm).expect("installed entry");
        let manifest = match manifest_case {
            "unknown" => manifest.replace("[\"timer\"]", "[\"timer\", \"unknown-timer-cap\"]"),
            "mismatch" => {
                std::fs::copy(&wasm_path, plugin_dir.join("different.wasm")).expect("other entry");
                manifest.replace("component.wasm", "different.wasm")
            }
            _ => manifest,
        };
        // The saved layout names the installed entry when the manifest
        // matches it, and the outside fixture otherwise.
        let wasm_path = if manifest_case == "matching" {
            installed_wasm
        } else {
            wasm_path
        };
        if manifest_case != "missing" {
            std::fs::write(plugin_dir.join("manifest.toml"), manifest).expect("manifest");
        }
        let workspace_root = h.workspace_root();
        assert!(
            PluginRegistry::load_with_global(&workspace_root, &h.profile_dir().join("plugins"))
                .get("com.plexi.timer-only")
                .is_some()
                != matches!(manifest_case, "missing" | "unknown"),
            "restore fixture registry presence"
        );
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir())
            .expect("permission monitor");
        assert!(
            monitor.store().records().is_empty(),
            "no prior launch grants"
        );
        // Restore is admission-checked (stint 1398): allow the installed copy,
        // or trust the fixture folder the saved path names.
        let admission = crate::host::plugin_admission::PluginAdmission::new(
            h.profile_dir(),
            &h.profile_dir().join("plugins"),
        );
        if manifest_case == "matching" {
            admission.allow_copy_for_test("com.plexi.timer-only", &plugin_dir);
        } else {
            admission.trust_folder_for_test(wasm_path.parent().expect("fixture folder"));
        }
        let colors = crate::ui::theme::Colors::from_config(&crate::config::ThemeConfig::default());
        let pane_id = 1596;
        let logs = crate::testing::test_log::capture();
        let restored = restore_view_pane_in_profile(
            "com.plexi.timer-only",
            &crate::workspace::SavedViewLaunch::Wasm {
                wasm_path,
                args: Vec::new(),
            },
            RestoreTarget {
                pane_id,
                context_id: h.app.windows[0].context_id,
                workspace_root: workspace_root.clone(),
                context_root: &workspace_root,
                saved_name: None,
            },
            &colors,
            h.profile_dir(),
        );
        if matches!(manifest_case, "missing" | "mismatch" | "unknown") {
            // The saved id does not own the saved component (stint 1398): the
            // restore fails before any grant, state or timer under that id.
            let error = restored.err().expect("an unowned component is refused");
            assert!(error.contains("belongs to"), "{error}");
            assert!(!logs
                .records()
                .iter()
                .any(|(_, line)| line.contains("restore minted")));
            return;
        }
        if requires_review {
            assert_eq!(
                restored.err().as_deref(),
                Some("restore requires permission review for: state:read-write")
            );
            return;
        }
        let mut restored = restored.expect("restore timer plugin without prior installed launch");
        let view = restored.pane.as_view_mut().expect("view");
        let crate::host::pane::ViewRuntime::Wasm(live) = &mut view.runtime else {
            panic!("expected WASM");
        };
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::ui::theme::setup_fonts(ui.ctx())
        });
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            live.ui(ui, &colors, None, pane_id);
        });
        assert!(
            live.timer_deadline_for_test().is_some() == (manifest_case == "matching"),
            "only a matching manifest may arm the restored timer"
        );
        assert!(!live.has_pending_capability_prompt());
    }

    #[test]
    fn restore_wasm_manifest_timer_arms_in_fresh_workspace() {
        restore_wasm_timer_case("timer-only", false, "matching");
    }

    #[test]
    fn restore_wasm_manifest_timer_does_not_bypass_sensitive_review() {
        restore_wasm_timer_case("sysmon", true, "matching");
    }

    /// Codex 3309 round 1: a Python start that fails after admission leaves
    /// none of the manifest capability rows it planned.
    #[test]
    fn a_python_start_that_fails_keeps_no_capability_rows() {
        let h = HostHarness::new();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("t1733-py");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("manifest.toml"),
            "schema_version = 2\n\n[plugin]\nid = \"t1733-py\"\nname = \"t\"\nversion = \"0.0.1\"\n\n[plugin.capabilities]\ncapabilities = [\"fs.read\"]\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"t1733-py.main\"\ndefault = true\n",
        )
        .unwrap();
        std::fs::write(
            root.join("main.py"),
            "def init(size, args):\n    return []\n",
        )
        .unwrap();
        let resolved = crate::app::registry::resolve_plugin_dir(&root).unwrap();
        let config = python_launch_config(&resolved).unwrap();
        // An entry outside its root fails the runtime launch after planning.
        let elsewhere = dir.path().join("main.py");
        let admitted = crate::host::plugin_admission::AdmittedPlugin::for_test(
            "t1733-py",
            crate::app::registry::ManifestType::Python,
            &resolved.root,
            &elsewhere,
        );
        let result = build_python_runtime(
            admitted,
            config,
            h.profile_dir(),
            h.workspace_root(),
            h.workspace_root(),
            Vec::new(),
            &crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default()),
        );
        assert!(result.is_err(), "the start fails");
        let rows = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir())
            .unwrap()
            .host_records()
            .entries()
            .into_iter()
            .filter(|entry| entry.actor_id.ends_with("t1733-py"))
            .count();
        assert_eq!(rows, 0);
    }

    #[test]
    fn wasm_manifest_permissions_rejects_unknown_before_minting() {
        let h = HostHarness::new();
        let error = wasm_manifest_permissions(
            h.profile_dir(),
            "com.plexi.timer-only",
            &h.workspace_root(),
            &["timer".to_string(), "unknown-timer-cap".to_string()],
            Vec::new(),
        )
        .err()
        .expect("unknown capability must refuse launch and restore");
        assert!(error.starts_with("manifest lists "), "{error}");
        assert!(error.contains("unknown-timer-cap"), "{error}");
        assert!(
            crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir())
                .expect("monitor")
                .store()
                .records()
                .is_empty(),
            "invalid manifest must mint nothing"
        );
    }

    #[test]
    fn restore_wasm_invalid_manifest_with_an_unowned_component_is_refused() {
        restore_wasm_timer_case("timer-only", false, "unknown");
    }

    #[test]
    fn restore_wasm_without_a_manifest_under_a_saved_id_is_refused() {
        restore_wasm_timer_case("timer-only", false, "missing");
    }

    #[test]
    fn restore_wasm_component_the_manifest_does_not_own_is_refused() {
        restore_wasm_timer_case("timer-only", false, "mismatch");
    }

    #[test]
    fn installed_wasm_without_manifest_timer_is_denied() {
        installed_wasm_timer_case(false);
    }

    fn media_permissions() -> PluginPermissions {
        use crate::app::permissions::Capability;
        let mut permissions = PluginPermissions::default();
        permissions
            .capabilities
            .extend(Capability::MEDIA_IO.iter().copied());
        permissions.capabilities.insert(Capability::FsRead);
        permissions
    }

    #[test]
    fn media_gate_withholds_every_media_capability_when_experiment_off() {
        use crate::app::permissions::Capability;
        let _experiments = crate::release::set_test_experiments(&[]);
        let mut permissions = media_permissions();
        apply_media_release_gate("com.example.media", &mut permissions);
        for cap in Capability::MEDIA_IO {
            assert!(!permissions.capabilities.contains(&cap), "{cap:?} granted");
            assert!(permissions.blocked.contains(&cap), "{cap:?} not blocked");
        }
        assert!(permissions.capabilities.contains(&Capability::FsRead));
    }

    #[test]
    fn media_gate_keeps_media_capabilities_when_experiment_on() {
        use crate::app::permissions::Capability;
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::MediaIo]);
        let mut permissions = media_permissions();
        apply_media_release_gate("com.example.media", &mut permissions);
        for cap in Capability::MEDIA_IO {
            assert!(permissions.capabilities.contains(&cap), "{cap:?} lost");
            assert!(!permissions.blocked.contains(&cap));
        }
    }

    #[test]
    fn media_gate_leaves_builtin_apps_alone() {
        let _experiments = crate::release::set_test_experiments(&[]);
        let mut permissions = PluginPermissions::builtin();
        apply_media_release_gate("builtin", &mut permissions);
        assert!(permissions.blocked.is_empty());
    }

    #[test]
    fn raw_wasm_launch_uses_registered_manifest_identity_for_release_gate() {
        let _channel = crate::config::set_test_channel("main");
        // WasmViews on so the launch reaches the manifest-identity gate; DAW stays off.
        let _experiments =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
        let global_apps = tempfile::tempdir().expect("global plugins");
        let plugin_dir = global_apps.path().join("daw");
        std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        let wasm_path = plugin_dir.join("renamed-component.wasm");
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/daw-engine.wasm"),
            &wasm_path,
        )
        .expect("copy DAW fixture");
        std::fs::write(
            plugin_dir.join("manifest.toml"),
            "schema_version = 2\n\n[plugin]\nid = \"com.plexi.daw-engine-poc\"\nname = \"DAW Engine\"\nversion = \"0.1.0\"\n\n[runtime]\nkind = \"wasm\"\nentry = \"renamed-component.wasm\"\n\n[[contributes.views]]\nid = \"com.plexi.daw-engine-poc.main\"\ndefault = true\n",
        )
        .expect("write manifest");
        let bare = tempfile::tempdir().expect("bare cwd");
        let registry = PluginRegistry::load_with_global(bare.path(), global_apps.path());
        assert!(registry.get("com.plexi.daw-engine-poc").is_some());

        let mut h = HostHarness::new();
        let ctx1_root = h.app.router.active().root.clone();
        h.app.registries.set_view_for_test(&ctx1_root, registry);
        h.app
            .trust_plugin_path_for_test(wasm_path.to_str().expect("UTF-8 fixture path"));
        let error = h
            .app
            .launch_view_by_path_with_layout_no_review_modal(
                wasm_path.to_str().expect("UTF-8 fixture path"),
                None,
                None,
                &[],
            )
            .expect_err("stable must reject a raw path to the registered DAW component");

        assert!(
            error.contains("daw is an experiment"),
            "raw launch must use the manifest identity, not the unrelated file name: {error}"
        );
    }

    /// Stint 0431: the native-WASM open path must honor the default `"overlay"`
    /// placement hint (the layout Cmd+P uses) — replace the focused pane in
    /// place, reusing its id and stashing the displaced pane for restore — not
    /// unconditionally split a new pane below. Regression:
    /// `open_wasm_plugin_instance` ignored the hint and always split.
    #[test]
    fn open_wasm_view_instance_overlay_replaces_focused_pane_in_place() {
        let mut h = HostHarness::new();
        let focused = h.add_test_pane();
        h.focus_pane(focused);
        let count_before = h.pane_count();

        let (app, snapshot) = load_counter();
        let pane_id = h
            .app
            .open_wasm_view_instance(super::WasmViewInstanceSpec {
                target_win: 0,
                plugin_id: "counter-pane",
                app,
                snapshot,
                workspace_root: std::env::temp_dir(),
                permissions: PluginPermissions::default(),
                remembered: None,
                layout: Some("overlay"),
                launch_args: Vec::new(),
            })
            .expect("overlay launch returns the reused pane id");

        assert_eq!(pane_id, focused, "overlay reuses the focused pane's id");
        assert_eq!(
            h.pane_count(),
            count_before,
            "overlay replaces in place — no extra pane is split"
        );
        let stashed_replacement = h.app.windows[0]
            .panes
            .get(&focused)
            .and_then(Pane::as_view)
            .map(|app| app.overlay_replaced.is_some())
            .unwrap_or(false);
        assert!(
            stashed_replacement,
            "overlay stashes the displaced pane in overlay_replaced for restore-on-close"
        );
    }

    /// Stint 0431: an overlay launch into an empty context (no focused pane)
    /// must fall through to the root-pane path and install the plugin as the tree
    /// root — never a silent no-op.
    #[test]
    fn open_wasm_view_instance_overlay_no_focused_pane_launches_root() {
        let mut h = HostHarness::new();
        assert!(
            h.app.windows[0].focused_pane.is_none(),
            "empty context has no focused pane"
        );

        let (app, snapshot) = load_counter();
        let pane_id = h
            .app
            .open_wasm_view_instance(super::WasmViewInstanceSpec {
                target_win: 0,
                plugin_id: "counter-pane",
                app,
                snapshot,
                workspace_root: std::env::temp_dir(),
                permissions: PluginPermissions::default(),
                remembered: None,
                layout: Some("overlay"),
                launch_args: Vec::new(),
            })
            .expect("overlay with empty context launches as root pane");

        // The settled tree wraps the root pane in its Tabs container (stint
        // 1230), so the root is that container and focus is the pane inside.
        let win = &h.app.windows[0];
        let root = win.tree.root.expect("root tile installed");
        let pane_tile = win
            .tree
            .tiles
            .find_pane(&pane_id)
            .expect("the launched wasm pane has a tile");
        assert_eq!(
            win.focused_pane,
            Some(pane_tile),
            "new root pane must be focused"
        );
        assert_eq!(
            win.find_first_pane_in(root),
            Some(pane_tile),
            "the root must hold the launched wasm pane"
        );
    }
}
