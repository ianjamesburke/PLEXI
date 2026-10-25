pub mod account;
mod agent_tools;
pub(crate) mod agents_app;
mod app_call;
pub mod app_trait;
pub(crate) mod approval_keyboard;
pub(crate) mod assistant_host_tools;
pub mod attention_queue;
pub(crate) mod card_gallery;
pub mod audio_player_app;
pub(crate) mod canvas_bindings;
mod cli_actions;
pub mod command_view_app;
mod conjoined;
mod dispatch;
mod event_stream;
pub mod file_handlers;
mod fleet;
mod focus;
pub(crate) mod focus_journal;
pub mod host_mcp;
pub mod host_version;
pub mod http;
pub mod image_viewer_app;
pub(crate) mod input_owner;
pub(crate) mod input_router;
pub(crate) mod launch_spec;
mod lifecycle;
pub mod marketplace;
pub(crate) mod notification_image;
mod notifications;
pub mod package;
pub mod packs;
pub(crate) mod pane_status;
pub(crate) mod pane_wait;
pub mod permissions;
pub mod plexi_descriptor;
pub(crate) mod python_env;
pub(crate) mod quit;
pub mod registry;
pub(crate) mod registry_views;
pub mod registry_watcher;
pub(crate) mod render;
pub(crate) mod screenshot;
mod secret_get;
pub mod secrets_app;
mod sync;
mod team;
pub mod text_editor_app;
pub(crate) mod ui_mailbox;
pub(crate) mod ui_scale;
pub mod video_player_app;
pub mod zoom;

#[cfg(test)]
pub(crate) use focus::{ContextCloseItem, FocusLogOutcome};
pub(crate) use focus::{
    ContextCloseState, FocusKind, PendingRawWasmLaunch, FOCUS_HEARTBEAT_INTERVAL,
};
pub(crate) use notification_image::NotificationImageState;
#[cfg(test)]
pub(crate) use notifications::save_pending_notifications_to;
pub(crate) use notifications::{load_pending_notifications_from, PendingNotification};

/// A oneshot script pane that already finished must not be respawned as a
/// live `/bin/sh` on restore; it comes back as completed.
pub(crate) fn is_finished_script_session(session: &crate::workspace::SavedAgentSession) -> bool {
    crate::cli::AgentHarness::parse(&session.harness) == Some(crate::cli::AgentHarness::Script)
        && matches!(
            session.last_state,
            crate::protocol::AgentState::Completed | crate::protocol::AgentState::ExitedUnreported
        )
}

/// Build a shell command string from an args list for passing to `zsh -c <cmd>`.
/// A single arg is used as-is (it's already a shell expression — CLI path).
/// Multiple args are joined with shell quoting so word-splitting is preserved.
fn cmd_from_args(args: &[String]) -> Option<String> {
    match args {
        [] => None,
        [single] => Some(single.clone()),
        multiple => Some(crate::host::shell::shell_join(multiple)),
    }
}

/// written before windows defaulted to an empty name. Stripped on load so they
/// don't appear as user-given names in the command palette.
fn is_auto_window_name(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("Page ") {
        let mut parts = rest.splitn(2, ',');
        let x = parts.next().unwrap_or("");
        let y = parts.next().unwrap_or("");
        if !x.is_empty()
            && x.chars().all(|c| c.is_ascii_digit())
            && !y.is_empty()
            && y.chars().all(|c| c.is_ascii_digit())
        {
            return true;
        }
    }
    if let Some(rest) = name.strip_prefix("Context ") {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// Provenance captured when the quick note modal opens, recorded into the note's
/// frontmatter as plain text. Where the note is *stored* comes from the active
/// context's tier, not from here.
#[derive(Default, Clone)]
pub(crate) struct QuickNoteCtx {
    pub cwd: std::path::PathBuf,
    pub workspace_root: Option<std::path::PathBuf>,
}

/// What a `TextInputOverlay` commit should do.
#[derive(Clone, Debug)]
pub(crate) enum OverlayTarget {
    /// Set (or clear) the root directory on the context at `idx`.
    ContextRoot(usize),
}

/// Shared text-input overlay state. One instance per open modal.
#[derive(Clone, Debug)]
pub(crate) struct TextInputOverlay {
    pub label: String,
    pub hint: String,
    pub buffer: String,
}

use crate::config;
use crate::host::context::Window;
use crate::host::keys::{self, Action};
use crate::host::pane::{Pane, TerminalPane};
use crate::host::shell;
use crate::spatial::tiling::PaneId;
use crate::ui::theme::{self, Colors};
use crate::workspace::router::ContextMove;
use crate::workspace::WorkspaceFile;
use egui_term::{BackendSettings, PtyEvent, TerminalTheme};
use egui_tiles::{Tile, Tree};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;

struct PaneSwapAnim {
    from: egui::Rect,
    to: egui::Rect,
    started_at: std::time::Instant,
}

struct EdgePulse {
    tile: egui_tiles::TileId,
    dir: crate::host::keys::Direction,
    started_at: std::time::Instant,
}

pub(crate) struct ClickFlash {
    pub(crate) window_id: u64,
    pub(crate) tile: egui_tiles::TileId,
    pub(crate) started_at: std::time::Instant,
}

#[derive(Clone, Debug)]
pub(crate) struct PaneHeartbeat {
    pub every: std::time::Duration,
    pub text: String,
    pub while_idle_only: bool,
    pub next_fire: std::time::Instant,
}

/// A host-chrome approval button, in window points, for the human-intent
/// driver. The floating Allow once control publishes its bounds on
/// `assistant permission list` so a real pointer click can find it.
#[derive(Clone, Debug)]
pub(crate) struct ApprovalButton {
    pub label: String,
    pub bounds: [f32; 4],
    pub pending_request_id: String,
}

pub struct PlexiApp {
    pub(crate) resource_leases: Option<crate::host::resource_leases::LeaseService>,
    pub(crate) pty_event_rx: mpsc::Receiver<(u64, PtyEvent)>,
    pub(crate) pty_event_tx: mpsc::Sender<(u64, PtyEvent)>,
    pub(crate) last_notify_poll: std::time::Instant,
    pub(crate) scheduler: crate::host::scheduler::Scheduler,
    pub(crate) theme: TerminalTheme,
    pub(crate) colors: Colors,
    pub(crate) default_font_size: f32,
    /// App-wide UI scale applied via `ctx.set_zoom_factor`.
    pub(crate) ui_scale: f32,
    /// Deadline for the debounced `ui_scale` write to config.toml.
    pub(crate) ui_scale_persist_at: Option<std::time::Instant>,
    pub(crate) ctx: egui::Context,
    pub(crate) router: crate::workspace::router::WorkspaceRouter,
    pub(crate) windows: Vec<Window>,
    pub(crate) active_window: usize,
    pub(crate) sidebar_visible: bool,
    pub(crate) show_shortcuts: bool,
    pub(crate) show_changelog: bool,
    pub(crate) show_ui_gallery: bool,
    pub(crate) ui_gallery_normal_buf: String,
    pub(crate) ui_gallery_focused_buf: String,
    pub(crate) ui_gallery_show_text_modal: bool,
    pub(crate) ui_gallery_modal_buf: String,
    pub(crate) quitting: bool,
    pub(crate) quit_press_count: u8,
    pub(crate) quit_last_press: Option<std::time::Instant>,
    pub(crate) pending_close: bool,
    pub(crate) pending_quit: bool,
    pub(crate) pending_context_close: Option<ContextCloseState>,
    pub(crate) welcome_delete_press_count: u8,
    pub(crate) welcome_delete_last_press: Option<std::time::Instant>,
    pub(crate) frame_tick: crate::platform::logging::FrameTick,
    pub(crate) ui_phase: crate::platform::logging::UiPhaseTracker,
    /// Repaint-cause diagnostics sample window (#2019): start instant and
    /// frame count. `None` until the first frame opens a window.
    pub(crate) frame_diag_window: Option<(std::time::Instant, u32)>,
    /// Previous frame's `ui_contents_rect`, for detecting a window resize
    /// (stint 0731 drag profiling): compared each frame in `detect_drag_active`.
    pub(crate) prev_screen_rect: Option<egui::Rect>,
    /// Last frame a drag/resize signal (pointer-drag or screen_rect change)
    /// was observed. `detect_drag_active` holds the drag window open for
    /// 300ms past the last signal so it doesn't flap between frames
    /// (stint 0731).
    pub(crate) drag_window_last_seen: Option<std::time::Instant>,
    /// Screenshot requests (`plexi host screenshot`) awaiting the viewport
    /// capture that `AppRequest::Screenshot` triggered (stint 0461).
    pub(crate) pending_screenshots: Vec<crate::app::screenshot::PendingScreenshot>,
    /// Banner buttons drawn last frame. `assistant permission list` publishes them.
    pub(crate) approval_buttons: Vec<ApprovalButton>,
    /// This frame's raw input includes socket-injected pane events.
    pub(crate) synthetic_input_frame: bool,
    /// `plexi pane slot wait` requests parked until the watched slot's
    /// value matches, or their caller-supplied deadline passes (stint 0585).
    pub(crate) pending_slot_waits: Vec<crate::app::pane_wait::PendingSlotWait>,
    /// Per-pane FIFO message inboxes for `plexi pane message send/recv`.
    pub(crate) pane_mailboxes:
        HashMap<PaneId, std::collections::VecDeque<crate::app::pane_wait::PaneMessage>>,
    /// `plexi pane message recv --wait` requests parked until a message lands
    /// or their caller-supplied deadline passes.
    pub(crate) pending_message_recvs: Vec<crate::app::pane_wait::PendingMessageRecv>,
    /// Monotonically increasing host-stamped id for pane inbox messages.
    pub(crate) next_pane_message_id: u64,
    /// Per-pane schema-validated structured results (`plexi pane result`, stint 0666).
    /// Retained across pane close so spawners can read results after an ephemeral worker exits.
    pub(crate) closed_pane_slots: HashMap<PaneId, HashMap<String, std::path::PathBuf>>,
    pub(crate) agent_attention: crate::app::fleet::AttentionState,
    pub(crate) fleet_history: std::cell::RefCell<Vec<crate::workspace::AgentHistoryRecord>>,
    pub(crate) pane_results: HashMap<PaneId, crate::host::pane_result::PaneResultState>,
    /// `plexi pane result --wait` requests parked until a valid result is committed
    /// or their caller-supplied deadline passes (stint 0666).
    pub(crate) conjoined_parent_rect: Option<(u64, egui::Rect)>,
    pub(crate) conjoined_parent_focus: Option<u64>,
    pub(crate) conjoined_contexts: std::collections::HashMap<u64, u64>,
    pub(crate) agent_tool_children: std::collections::HashMap<u64, Option<u64>>,
    pub(crate) agent_tool_waits: Vec<agent_tools::AgentToolWait>,
    pub(crate) pending_result_waits: Vec<crate::app::pane_wait::PendingResultWait>,
    pub(crate) pending_agent_waits: Vec<crate::app::pane_wait::PendingAgentWait>,
    pub(crate) child_result_events: HashMap<PaneId, crate::workspace::QueuedChildResult>,
    pub(crate) child_parent_panes: HashMap<PaneId, PaneId>,
    pub(crate) pulled_child_results: std::collections::HashSet<PaneId>,
    pub(crate) delivered_child_batches: HashMap<PaneId, Vec<String>>,
    pub(crate) team_runs: Vec<team::TeamRun>,
    /// `plexi pane new --agent` spawns whose response is withheld until the
    /// agent in the new pane self-reports idle (stint 0584).
    pub(crate) pending_agent_boots: Vec<crate::app::pane_wait::PendingAgentBoot>,
    /// `plexi pane send --submit` requests whose text is typed and whose reply
    /// is owed once the host has settled, pressed Enter, and confirmed that the
    /// prompt left the pane's input line (stint 0583).
    pub(crate) pending_submits: Vec<crate::app::pane_wait::PendingSubmit>,
    pub(crate) pending_send_approvals: Vec<crate::app::pane_wait::PendingSendApproval>,
    /// Host-owned recurring prompts, keyed by terminal pane id.
    pub(crate) pane_heartbeats: HashMap<PaneId, PaneHeartbeat>,
    /// Worktrees created by `plexi pane new --worktree <branch>`, keyed by the
    /// owning terminal pane id so closing the pane reclaims or retains them (stint 0668).
    pub(crate) pane_worktrees: HashMap<PaneId, crate::host::worktree::ManagedWorktree>,
    /// Most recent close-time worktree disposition per pane id, consumed by
    /// `ClosePane` replies and inspected by tests.
    pub(crate) last_worktree_close_reports:
        HashMap<PaneId, crate::host::worktree::WorktreeCloseReport>,
    /// Last window title sent via `ViewportCommand::Title`. egui's
    /// `send_viewport_cmd` unconditionally requests a repaint, so sending the
    /// title every frame creates a self-sustaining repaint loop (60fps
    /// visible, uncapped when occluded). Only send on change.
    pub(crate) last_sent_window_title: Option<String>,
    /// Directory holding `permissions.toml` for the host-level permission
    /// management handlers (`ListPermissions` / `SetPermission`, stint 0017).
    /// `config_dir()` in production; an isolated temp dir in tests so harness
    /// runs never read or write the developer's real permission store.
    pub(crate) permission_store_dir: std::path::PathBuf,
    /// Cached config so confirmation settings are read through the config
    /// tunnel rather than duplicated as individual bool fields.
    pub(crate) config: crate::config::PlexiConfig,
    pub(crate) key_bindings: crate::host::keys::KeyBindings,
    pub(crate) binding_table: Vec<crate::host::keys::BindingEntry>,
    /// Keys whose press edge fired a host binding and whose matching release
    /// remains host-owned, even if Command is released first.
    pub(crate) claimed_host_key_releases: HashSet<egui::Key>,
    pub(crate) renaming_window: Option<usize>,
    pub(crate) rename_buffer: String,
    pub(crate) editing_description: Option<usize>,
    pub(crate) description_buffer: String,
    pub(crate) drag_context: Option<usize>,
    /// Whether the "Parked (N)" section in the sidebar is expanded.
    pub(crate) parked_section_expanded: bool,
    /// Per-canonical-root `AppRegistry` cache (stint 0724 Phase B). Replaces
    /// the old single active-context-following `AppRegistry` — every context
    /// resolves against its OWN root's view, never whichever context was last
    /// active. See `crate::app::registry_views` for the invalidation rules.
    pub(crate) registries: crate::app::registry_views::RegistryViews,
    pub(crate) show_command_palette: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_selected: usize,
    /// Stable identity of the selected palette result. The numeric index is
    /// only a rendering/navigation position and may change on live updates.
    pub(crate) palette_selected_key: Option<String>,
    /// Cached workspace root for the focused pane at the moment the palette
    /// was opened. Resolved once on open, not per-frame, to avoid repeated
    /// filesystem traversal in the egui draw loop.
    pub(crate) palette_workspace_root: Option<std::path::PathBuf>,
    /// Notes loaded at palette-open time, cleared on close.
    pub(crate) palette_notes: Vec<crate::notes::NotePickerEntry>,
    /// User commands (`commands.toml` + global scripts) resolved at palette-open
    /// time, cleared on close. Re-read on every open so edits hot-reload without
    /// a restart — same pattern as `palette_notes`.
    pub(crate) palette_commands: Vec<crate::cli::ResolvedUserCommand>,
    /// Set to true on palette open; consumed on the first draw frame to reset
    /// the scroll area back to the top before egui can apply the stale offset.
    pub(crate) palette_scroll_reset: bool,
    /// Last agent-pane count logged by the palette. The palette rebuilds its
    /// rows every draw frame, so the collection trace fires only when the
    /// count changes — never once per frame.
    pub(crate) palette_agent_count_logged: Option<usize>,
    pub(crate) context_visit_history: Vec<u64>,
    pub(crate) renaming_pane: Option<PaneId>,
    /// Active text-input overlay and its dispatch target.
    pub(crate) text_overlay: Option<(TextInputOverlay, OverlayTarget)>,
    /// Receiver for async folder-picker results (Browse button).
    pub(crate) text_overlay_browse_rx: Option<mpsc::Receiver<Option<PathBuf>>>,
    /// Notifications queued from apps via ShowNotification.
    pub(crate) pending_notifications: Vec<PendingNotification>,
    /// ID of the notification the modal is currently showing. `None` means
    /// "modal is empty / closed" — at the next render, the next notification
    /// in the queue becomes current.
    ///
    /// The invariant: **the currently-displayed notification is pinned by
    /// id**. New notifications arriving never change this, so the user can
    /// never be yanked to a different notification without their input.
    /// Cmd+] / Cmd+[ move this id across the ordered queue.
    pub(crate) current_notify_id: Option<String>,
    /// Focused option index for `kind = "choice"` notifications (0-based).
    /// Reset to 0 whenever the front of the queue changes.
    #[allow(dead_code)]
    pub(crate) modal_focused_option: usize,
    /// Buffer for `kind = "input"` notifications.
    #[allow(dead_code)]
    pub(crate) modal_input_buffer: String,
    /// Input drafts keyed by notification id. Later and queue cycling are
    /// non-terminal, so they must not discard text the user already entered.
    #[allow(dead_code)]
    pub(crate) notification_input_drafts: HashMap<String, String>,
    /// Text being composed in the quick note modal.
    pub(crate) quick_note_text: String,
    /// Local images staged by the QuickNote modal. They are copied into the
    /// Notes collection only when the user saves, so discard never leaves
    /// unattached assets behind.
    pub(crate) quick_note_attachments: Vec<PathBuf>,
    /// Context captured at the time the quick note modal was opened.
    pub(crate) quick_note_ctx: QuickNoteCtx,
    /// Cleared after the one-shot notes migration off the pre-tier store has
    /// run. Boot-time rather than picker-time so `plexi note list`, which has
    /// no router and therefore cannot migrate, still sees migrated notes without
    /// the GUI ever opening.
    pub(crate) notes_migration_pending: bool,
    /// Notes picker: every visible tier's notes, grouped by tier, newest first.
    pub(crate) notes_picker_entries: Vec<crate::notes::NotePickerEntry>,
    /// Notes picker: currently highlighted index into the filtered view.
    pub(crate) notes_picker_selected: usize,
    /// Notes picker: selection rendered in the previous frame, used to keep
    /// keyboard navigation visible when input is handled before drawing.
    pub(crate) notes_picker_last_drawn_selected: Option<usize>,
    /// Notes picker: fuzzy-filter query.
    pub(crate) notes_picker_query: String,
    /// Notes picker: file content shown in the preview half, keyed by path so
    /// it is read once per selection rather than every frame.
    pub(crate) notes_picker_preview: Option<(std::path::PathBuf, std::sync::Arc<str>)>,
    /// Notes picker: a delete (or cut) awaiting Enter/y confirmation.
    pub(crate) notes_picker_pending_delete: Option<crate::overlays::notes_picker::PendingDelete>,
    /// Notes picker: inline status/error message shown in the fixed status slot.
    pub(crate) notes_picker_status: Option<String>,
    /// Notes picker: last text sent to the clipboard (Cmd+C / Cmd+X).
    pub(crate) notes_picker_last_copied: Option<String>,
    /// Notes picker: `notes_picker_last_copied` still has to be handed to egui.
    pub(crate) notes_picker_copy_unsent: bool,
    /// In-memory notes index. Cmd+O paints this and does not walk.
    pub(crate) notes_index: crate::notes::NotesIndexCache,
    /// Background refresh of `notes_index`. Drained from `App::logic`.
    pub(crate) notes_index_rx: Option<std::sync::mpsc::Receiver<crate::notes::NotesIndexBuild>>,
    /// Anchor the in-flight refresh was started for.
    pub(crate) notes_index_inflight_anchor: Option<std::path::PathBuf>,
    /// A note mutation landed while a refresh was already running.
    pub(crate) notes_index_rerun: bool,
    /// Paths last published into the open picker. A refresh replaces the rows
    /// only while they still match, so a caller that seeded different rows
    /// (screenshot harnesses) is left alone.
    pub(crate) notes_picker_shown_paths: Vec<std::path::PathBuf>,
    /// Persisted index loaded at most once per process.
    pub(crate) notes_index_loaded: bool,
    /// notify_id of the notification the modal currently has state for. Used to
    /// detect a front-of-queue change and reset focus/input buffer.
    #[allow(dead_code)]
    pub(crate) modal_state_notify_id: String,
    /// Lazily-decoded image-render state for notifications that carry an
    /// `image_inline` or `image_pipe_id` attachment (#74). Keyed by
    /// `notify_id`. Populated on first render of the notification; entries
    /// are NOT explicitly evicted on dismiss — egui's TextureHandle drop
    /// cleanup, plus the small upper bound on concurrent visible
    /// notifications, makes this acceptable. A future PR can add eviction
    /// if memory becomes a concern.
    pub(crate) notification_images: HashMap<String, NotificationImageState>,
    /// Cached from `[notifications]` config. See NotificationsConfig for semantics.
    pub(crate) notifications_enabled: bool,
    pub(crate) notifications_focus_mode: bool,
    /// Path to the audible cue played on notification arrival. `None` = silent.
    pub(crate) notifications_sound: Option<String>,
    /// Keeps the most recent cue audible. Dropping a `PlaybackSession` stops
    /// playback, so a local binding inside `enqueue_notification` would cut the
    /// sound off at end of scope. Replacing the field on the next cue — which
    /// drops the previous session — is the intended behaviour.
    pub(crate) notification_cue_playback: Option<crate::media::audio::PlaybackSession>,
    /// Input-focus stack. Top layer receives keyboard input; panes see an
    /// empty event buffer while a non-`Pane` layer is on top. See the
    /// `FocusKind` docs for the invariant.
    pub(crate) focus_stack: Vec<FocusKind>,
    /// Pane focus history for Cmd+[ time-travel. Each entry is (window_id, tile_id)
    /// captured just before a focus change. Capped at 100; oldest evicted from front.
    pub(crate) focus_history_depth: usize,
    pub(crate) pane_focus_history: Vec<(u64, egui_tiles::TileId)>,
    /// Pane focus future — entries undone by back-navigation; cleared on any
    /// organic focus change.
    pub(crate) pane_focus_future: Vec<(u64, egui_tiles::TileId)>,
    /// Set to true during step_focus_history_back/forward to suppress recording
    /// the history-driven focus change as a new history entry.
    pub(crate) navigating_history: bool,
    pub(crate) host: crate::host::model::HostModel,
    pub(crate) host_services: crate::host::services::HostServices,
    /// Parked background WASM app runtimes — kept alive when their pane is closed.
    /// Keyed by app type_id. Value is `(park_context_id, app)` where
    /// `park_context_id` is the context_id the app was running in when its
    /// pane was closed. Used to route notifications to the correct context.
    pub(crate) background_apps: HashMap<String, (u64, Box<dyn crate::app::app_trait::App>)>,
    /// Directed inter-agent / inter-app pipes (#286). Keyed by
    /// `(context_id, pipe_id)` (stint 0724 Phase D 2/2) — `context_id` is the
    /// opening caller's own context, resolved via `origin_for_pane`, never a
    /// client-supplied value. Composite-keying (rather than `pipe_id` alone)
    /// is required, not cosmetic: two different contexts choosing the same
    /// caller-selected `pipe_id` string (e.g. both pick `"agent-chat"`) must
    /// never collide or cross-deliver, and before this phase they shared one
    /// flat namespace. Value is the `(sender_pane_id, target_pane_id)` pair
    /// the host scopes `PipeMessage` deliveries to. This is the sole
    /// JSON-pipe delivery path: `DeliverPipeMessage` routes ONLY to the
    /// non-sender member of the pair; a send on a pipe absent from this map
    /// is dropped. The legacy non-directed peer-broadcast fan-out was removed
    /// in 0327 in favour of event streams. `PipeOpenDirected` is a thin alias
    /// over this map — an exclusive duplex channel primitive; resource-scoped
    /// fan-out belongs on the event bus.
    ///
    /// `OpenDirectedPipe` rejects (never inserts) a request whose target pane
    /// lives in a different context than the caller — see `evaluate_reach` in
    /// `dispatch.rs`'s handler — so every entry here is guaranteed to have
    /// both pane ids live in the SAME context, making `context_id` safe to
    /// key on regardless of which side (sender or target) later calls
    /// `pipe_send`.
    pub(crate) directed_pipes: HashMap<(u64, String), (u64, u64)>,
    /// Hot-reload watcher set (#83). Owns one notify watcher per pane that
    /// opted-in via manifest `[app] watch = true` (workspace-local only).
    /// `hot_reload_rx` is drained each frame; pending requests trigger a
    /// `reload_pane` call which relaunches the WASM runtime inside the
    /// existing `AppPane` envelope.
    pub(crate) hot_reload: crate::host::hot_reload::HotReloadWatcher,
    pub(crate) hot_reload_rx: ui_mailbox::MailboxReceiver<crate::host::hot_reload::ReloadRequest>,
    /// App state file watcher (stint 0644). Watches each file-backed app
    /// pane's state files for external writes; `state_watch_rx` is drained
    /// each `logic` pass (never `ui` — occluded-window trap) and each notice
    /// triggers `apply_external_state` on the matching pane.
    pub(crate) state_watch: crate::host::state_watch::StateWatcher,
    pub(crate) state_watch_rx:
        ui_mailbox::MailboxReceiver<crate::host::state_watch::StateChangedNotice>,
    /// Config file watcher (#1115). Watches `config.toml` for saves and fires
    /// a signal so `reload_config()` runs automatically.
    pub(crate) _config_watcher: Option<crate::config::watcher::ConfigWatcher>,
    pub(crate) config_reload_rx: Option<ui_mailbox::MailboxReceiver<()>>,
    /// App registry filesystem watchers (#1712, re-keyed per-root in stint
    /// 0724 Phase B). One watcher per canonical root currently held by
    /// `registries` — an inactive context's root is watched exactly like the
    /// active one's, so a change lands in `registries` regardless of focus.
    /// Reconciled every `logic` pass by `reconcile_registry_watchers` (starts
    /// watchers for newly-cached roots, drops watchers for roots `registries`
    /// no longer holds).
    pub(crate) registry_watchers: HashMap<
        std::path::PathBuf,
        (
            crate::app::registry_watcher::AppRegistryWatcher,
            ui_mailbox::MailboxReceiver<()>,
        ),
    >,
    /// True when workspace state changed since the last disk save. Set by
    /// `mark_workspace_dirty()`, cleared by the debounce flush in
    /// `update_preamble` (or by a forced `save_workspace_now()` on
    /// shutdown/update-quit). Keeps a burst of workspace-changing calls from
    /// each forcing a synchronous save on the UI thread (stint 0548).
    pub(crate) workspace_dirty: bool,
    /// Last time the workspace was flushed to disk, for the debounce tick in
    /// `update_preamble`.
    pub(crate) last_workspace_save: std::time::Instant,
    /// Watched panes scheduled for crash-restart. Value is the earliest `Instant` at
    /// which the restart fires — giving the developer ~2s to read the crash overlay.
    /// Spatial-grid minimap overlay state. Controls visibility, fade timer,
    /// and the `Cmd+Shift+M` override-visible flag.
    pub(crate) minimap: crate::render::minimap::MinimapState,
    /// Per-row navigation history for the spatial page grid. Maps `grid_y`
    /// to the `grid_x` of the last page visited on that row. Vertical moves
    /// consult this to land on the most recently accessed page in the target
    /// row rather than the spatially closest one.
    pub(crate) last_page_x_per_row: HashMap<u32, u32>,
    /// context_id → last active window_id for that context.
    pub(crate) context_active_window: HashMap<u64, u64>,
    /// Per-context minimap visibility. Saved on context switch so each
    /// context remembers its own minimap state across window changes.
    /// Absent entry = first visit; defaults to `true` iff context has > 1 page.
    pub(crate) minimap_visible_per_context: HashMap<u64, bool>,
    /// Monotonically increasing counter. Assigned to each new `Window` as its
    /// stable `window_id`. Never reused — only increments.
    pub(crate) next_window_id: u64,
    /// Pane count from the last snapshot push. Avoids rebuilding the global
    /// pane context every frame when no panes were opened or closed.
    pane_snapshot_len: usize,
    /// In-flight pane swap animations. Each entry fades out over 160 ms.
    pane_anims: Vec<PaneSwapAnim>,
    /// Boundary edge pulse — shown when a swap is attempted at the wall.
    edge_pulse: Option<EdgePulse>,
    /// Exponential accent flash on focus change (#1141).
    pub(crate) click_flash: Option<ClickFlash>,
    /// The one host-owned wake seam (stint 0507). Every off-UI-thread producer
    /// sends through a `UiMailbox` built on this wake; watcher restarts and
    /// periodic updater re-checks clone it to build fresh mailboxes.
    pub(crate) ui_wake: std::sync::Arc<dyn ui_mailbox::UiWake>,
    /// Channel receiver fed by the background update-check thread. Sends the
    /// latest version string exactly once if a newer release is available.
    update_rx: Option<ui_mailbox::MailboxReceiver<String>>,
    /// Last time the host scheduled an update-check thread. Used to re-check
    /// long-running sessions at the same interval as the updater cache.
    last_update_check: std::time::Instant,
    /// Latest available version string, set after the background check resolves.
    /// `None` means either the check hasn't completed or we're already current.
    pub(crate) display_version: String,
    pub(crate) update_available: Option<String>,
    pub(crate) update_quit_pending: bool,
    pub(crate) update_confirm_prompt: bool,
    /// Set by `AppRequest::Shutdown` (`plexi host stop`'s clean-shutdown
    /// path). Consumed at the top of the next frame in `update_preamble`,
    /// which sends `ViewportCommand::Close`. Mirrors `update_quit_pending`.
    pub(crate) shutdown_requested: bool,
    /// Receiver for AppRequests sent over the PLEXI_SOCKET Unix socket listener.
    /// Drained each frame in `drain_pane_cmd_channel`.
    pane_ipc_rx: ui_mailbox::MailboxReceiver<crate::protocol::AppRequest>,
    /// Host-owned event subscription core shared by the CLI NDJSON transport and
    /// the host MCP server. Resolves subscriber identity, runs the broker check,
    /// and records subscriptions in the global timeline.
    pub(crate) host_subscriptions: crate::host::event_subscriptions::HostSubscriptionService,
    /// Receiver for CLI/MCP subscribe requests routed from socket connection
    /// threads to the UI thread (which owns the grant store). Drained each frame
    /// in `drain_event_subscribe_channel`.
    event_subscribe_rx:
        ui_mailbox::MailboxReceiver<crate::host::event_subscriptions::HostSubscribeRequest>,
    /// Receiver for CLI/MCP publish requests (`events declare`/`emit`) routed
    /// from socket connection threads to the UI thread. Drained each frame in
    /// `drain_event_subscribe_channel` alongside subscribe requests.
    event_publish_rx:
        ui_mailbox::MailboxReceiver<crate::host::event_subscriptions::HostPublishRequest>,
    /// Subscribe or publish requests the broker answered with `Ask`, parked for an explicit
    /// user decision. The front entry is surfaced as the host event-consent
    /// modal; [`FocusKind::EventConsent`] is promoted while this is non-empty.
    pub(crate) pending_event_consents:
        std::collections::VecDeque<crate::host::event_subscriptions::PendingEventConsent>,
    /// App-runtime subscriptions waiting for the shared broker to answer.
    /// Immediate Allow/Deny replies and deferred consent replies use the same
    /// channel so Python and Rust WASM receive an identical result event.
    pending_app_subscription_replies: Vec<PendingAppSubscriptionReply>,
    /// Raw `.wasm` path launches waiting for explicit link-time import review.
    pub(crate) pending_raw_wasm_launches: std::collections::VecDeque<PendingRawWasmLaunch>,
    /// Last (window_id, tile_id) pair that was logged as a FocusChanged event.
    /// Uses stable window_id (u64) not a vector index so removals don't corrupt it.
    /// Compared at end of each frame to detect genuine focus transitions.
    pub(crate) last_logged_focus: Option<(u64, egui_tiles::TileId)>,
    /// When the current focus session started. Reset on each FocusChanged emit.
    pub(crate) focus_started_at: Option<std::time::Instant>,
    /// The terminal pane that currently holds egui's traversal-key focus lock
    /// (stint 0505). Tracked only to emit one info trace per acquisition instead
    /// of every frame the reconciler re-asserts the (idempotent) lock.
    pub(crate) terminal_traversal_lock: Option<PaneId>,
    /// Path to the focus journal file used for crash-recovery checkpoints.
    /// Written on every heartbeat and on clean pane transitions; deleted on
    /// clean shutdown. If present at startup → emit crash_recovery event.
    pub(crate) focus_journal_path: std::path::PathBuf,
    /// App commands held while a modal overlay owns keyboard input. Released and
    /// dispatched on the first frame where `input_captured_by_overlay()` is false.
    /// Only overlay-unsafe side effects are held here; safe commands (ShowNotification,
    /// pipes, queries) are dispatched immediately even during overlay ownership.
    pub(crate) overlay_held_cmds: Vec<crate::app::app_trait::AppCommand>,
    /// Host agent runtime (Phase C, docs/prm/agent-platform.md). Loaded from
    /// the active workspace's `agents/` dir; ticked once per frame in
    /// `update()` to consume agent event deliveries and finished turns.
    pub(crate) agent_host: crate::agent::AgentHost,
    /// Pending `AppRequest::ClickPane` injections (stint 0398), keyed by
    /// target pane. Taken and consumed once per frame by whichever render
    /// path actually paints that pane (tiled or zoomed) — see
    /// `crate::host::pane::PendingPaneClick`.
    pub(crate) pending_pane_clicks:
        HashMap<crate::spatial::tiling::PaneId, crate::host::pane::PendingPaneClick>,
    /// Synthetic key/text input queued by pane IPC until the target pane's next
    /// production render. Egui 0.34 rebuilds `InputState` at `begin_pass`, so
    /// writing directly into `Context::input_mut` from IPC dispatch can be
    /// acknowledged and then discarded before the widget sees it.
    pub(crate) pending_pane_inputs: HashMap<crate::spatial::tiling::PaneId, Vec<egui::RawInput>>,
    /// Per-frame drag samples for canvas/process/WASM panes queued by
    /// `AppRequest::DragPane` (stint 0510). One sample is promoted into
    /// `pending_pane_clicks` per rendered frame so the target pane sees
    /// press → move… → release across consecutive real frames, resolved
    /// against each frame's live canvas rect.
    pub(crate) pending_pane_drags: HashMap<
        crate::spatial::tiling::PaneId,
        std::collections::VecDeque<crate::host::pane::PendingPaneClick>,
    >,
    /// Per-frame pointer batches for builtin (egui-widget) panes queued by
    /// `AppRequest::DragPane`. Unlike `pending_pane_inputs` — whose batches
    /// all merge into one frame — exactly one batch is merged per visible
    /// frame, so egui observes a genuine multi-frame press/move/release
    /// trajectory and resolves real drag state on whatever widget owns the
    /// positions.
    pub(crate) pending_pane_pointer_frames:
        HashMap<crate::spatial::tiling::PaneId, std::collections::VecDeque<egui::RawInput>>,
    pub zoom: zoom::ZoomState,
    pub attention_queue: attention_queue::AttentionQueueState,
    approval_keyboard: approval_keyboard::ApprovalKeyboard,
    // Drop after the host resources that may still refer to these test paths.
    #[cfg(test)]
    _test_scratch: Option<tempfile::TempDir>,
}

/// Keeps painted permission-sheet buttons only for the frame that drew them.
struct SheetButtonFrame;

impl SheetButtonFrame {
    fn begin() -> Self {
        if let Some(monitor) =
            crate::broker::gate::PermissionMonitor::loaded(&crate::config::config_dir())
        {
            monitor.begin_sheet_frame();
        }
        Self
    }
}

impl Drop for SheetButtonFrame {
    fn drop(&mut self) {
        if let Some(monitor) =
            crate::broker::gate::PermissionMonitor::loaded(&crate::config::config_dir())
        {
            monitor.finish_sheet_frame();
        }
    }
}

struct PendingAppSubscriptionReply {
    pane_id: u64,
    request_id: String,
    receiver: std::sync::mpsc::Receiver<crate::host::event_subscriptions::HostSubscribeReply>,
}

#[cfg(test)]
fn configure_egui_ctx(ctx: &egui::Context, colors: &Colors) {
    theme::setup_fonts(ctx);
    ctx.enable_accesskit();
    ctx.set_visuals(egui::Visuals::dark());
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    theme::setup_style(ctx, colors, true);
}

pub(crate) fn configure_native_semantics(ctx: &egui::Context) {
    if crate::release::feature_enabled(crate::release::ReleaseFeature::Accessibility) {
        ctx.enable_accesskit();
    } else {
        crate::release::log_feature_blocked(crate::release::ReleaseFeature::Accessibility);
    }
}

/// Single-byte sentinel written back to the socket peer once the host has
/// captured its ancestry (`capture_peer_ancestry`) for a `Notify`/
/// `DismissNotification` request. `notify_cli`/`dismiss_notify_cli` block
/// reading this before the fire-and-forget path is allowed to exit — see
/// the long comment at the ack-write call site below for why this exists.
const PEER_IDENTITY_ACK: &[u8] = &[0x06]; // ASCII ACK

/// Handle one newline-delimited JSON line from the notify socket: parse the
/// `AppRequest` and queue it on the pane-IPC mailbox (which owns the UI wake).
///
/// `peer_ancestry` is the host-established ancestor-pid chain of the process
/// on the other end of this connection, captured once per connection in
/// `spawn_socket_listener`'s accept loop (`capture_peer_ancestry`, called at
/// accept time — never from the request body, and never re-derived lazily
/// here). It is stamped onto `Notify`/`DismissNotification` requests — the
/// only variants that carry a `peer_pid` field — so `handle_pane_ipc_request`
/// can resolve the sender pane from a value the client never controlled,
/// instead of trusting the wire-supplied `source_context_id`/`source_pane_id`.
pub(crate) trait SocketAckWriter: std::io::Write {
    fn clone_reply_stream(&self) -> Option<crate::platform::ipc::IpcStream> {
        None
    }
}

impl SocketAckWriter for crate::platform::ipc::IpcStream {
    fn clone_reply_stream(&self) -> Option<crate::platform::ipc::IpcStream> {
        self.try_clone().ok()
    }
}

impl SocketAckWriter for Vec<u8> {}
impl SocketAckWriter for std::io::Sink {}

#[cfg(test)]
pub(crate) fn handle_socket_line(
    line: &str,
    mailbox: &ui_mailbox::UiMailbox<crate::protocol::AppRequest>,
    peer_ancestry: Option<&[u32]>,
    ack_writer: &mut impl SocketAckWriter,
) {
    handle_socket_line_in(
        &crate::config::config_dir(),
        line,
        mailbox,
        peer_ancestry,
        ack_writer,
    );
}

pub(crate) fn handle_socket_line_in(
    profile: &std::path::Path,
    line: &str,
    mailbox: &ui_mailbox::UiMailbox<crate::protocol::AppRequest>,
    peer_ancestry: Option<&[u32]>,
    ack_writer: &mut impl SocketAckWriter,
) {
    if let Ok(frame) = serde_json::from_str::<serde_json::Value>(line) {
        if frame["type"] == "cli_trace" {
            match crate::host::flight_recorder::handle_cli_trace(profile, &frame) {
                Ok(()) => {
                    if let Err(error) = ack_writer.write_all(PEER_IDENTITY_ACK) {
                        log::warn!("cli_trace: acknowledgement failed: {error}");
                    }
                }
                Err(error) => log::warn!("{error}"),
            }
            return;
        }
    }
    match serde_json::from_str::<crate::protocol::AppRequest>(line) {
        Ok(mut cmd) => {
            // Fire-and-forget wakes can disconnect before peer lookup. Keep
            // missing-identity diagnostics for requests that use identity.
            if peer_ancestry.is_none() && !matches!(cmd, crate::protocol::AppRequest::Wake) {
                log::warn!("pane_ipc: request arrived without a verified socket peer identity");
            }
            let needs_identity_ack = matches!(
                cmd,
                crate::protocol::AppRequest::Notify { .. }
                    | crate::protocol::AppRequest::DismissNotification { .. }
            );
            match &mut cmd {
                crate::protocol::AppRequest::Notify { peer_pid: p, .. }
                | crate::protocol::AppRequest::DismissNotification { peer_pid: p, .. } => {
                    *p = peer_ancestry.map(<[u32]>::to_vec);
                }
                crate::protocol::AppRequest::AgentCall {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::AgentMessage {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::CallAppTool {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::ListAppTools {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::PaneSecretGet {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::PaneSecretList {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::NeedsYouApi {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::ResourceLease {
                    peer_ancestry: p, ..
                } => {
                    // Overwrite any client-supplied ancestry. A missing capture
                    // leaves the vector empty so the call cannot pretend to be a pane.
                    *p = peer_ancestry.unwrap_or(&[]).to_vec();
                    log::info!("pane_ipc: stamped peer ancestry ({} pid(s))", p.len());
                }
                crate::protocol::AppRequest::SpawnPane {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::SetAgentState {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::AgentDone {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::AgentAsk {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::Permissions {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::CallerContext {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::AgentSpawn {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::AgentWait {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::PaneMessageSend {
                    peer_ancestry: p, ..
                }
                | crate::protocol::AppRequest::SendToPane {
                    peer_ancestry: p, ..
                } => {
                    // Same rule as CallAppTool: the wire value is never the
                    // caller's identity. None when this process has no peer
                    // capture (tests that enqueue the request directly).
                    *p = peer_ancestry.map(<[u32]>::to_vec);
                    log::info!(
                        "pane_ipc: stamped peer ancestry ({} pid(s))",
                        p.as_ref().map(|pids| pids.len()).unwrap_or(0)
                    );
                }
                crate::protocol::AppRequest::AgentsApi { payload, .. } => {
                    if let Some(obj) = payload.as_object_mut() {
                        match peer_ancestry {
                            Some(pids) if !pids.is_empty() => {
                                obj.insert("peer_ancestry".to_string(), serde_json::json!(pids));
                                log::info!(
                                    "pane_ipc: stamped agents_api peer ancestry ({} pid(s))",
                                    pids.len()
                                );
                            }
                            _ => {
                                obj.remove("peer_ancestry");
                            }
                        }
                    }
                }
                _ => {}
            }
            if needs_identity_ack {
                // `capture_peer_ancestry` already ran, synchronously, in the
                // accept loop — strictly before this connection was ever
                // handed to a thread, let alone reached this line-parsing
                // step. Sending this ack now, rather than never (as before
                // this fix), lets a cooperating fire-and-forget caller
                // (`plexi notify`/`plexi notify dismiss`, which otherwise
                // exits the instant its write completes) block on reading
                // it before exiting. Without this, a short-lived CLI process
                // can race the host's own accept()-return latency under
                // scheduler load: it exits before the host code that reads
                // its ancestry has even run, `get_pid_ppid` then fails on an
                // already-gone pid, and the notification silently
                // misattributes to "outside pane" (global scope, no
                // dismiss) even though the sender really was a live, known
                // pane. A write failure here is logged, not fatal — the
                // request is still queued below; an uncooperative or
                // already-gone peer just proceeds without the guarantee,
                // exactly like today's behavior.
                if let Err(e) = ack_writer.write_all(PEER_IDENTITY_ACK) {
                    log::warn!("pane_ipc: peer identity ack write failed: {e}");
                }
            }
            let reply_token = crate::rpc::extract_response_file_token(line);
            if let Some(ref token) = reply_token {
                if let Some(reply_stream) = ack_writer.clone_reply_stream() {
                    crate::rpc::ReplyRegistry::register(token.clone(), reply_stream);
                }
            }
            if mailbox.send(cmd).is_ok() {
                log::debug!("pane_ipc: request queued — mailbox wakes idle UI thread");
            } else if let Some(ref token) = reply_token {
                crate::rpc::ReplyRegistry::remove(token);
            }
        }
        Err(e) => {
            log::warn!("pane_ipc: parse error: {e}  line={line:?}");
        }
    }
}

/// Read one complete newline-delimited request frame. EOF without a newline is
/// an incomplete transport write, even when its bytes happen to form valid
/// JSON, and must never be dispatched.
fn read_socket_frame(reader: &mut impl std::io::BufRead) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line)?;
    if bytes == 0 {
        return Ok(None);
    }
    if !line.ends_with('\n') {
        log::warn!(
            "pane_ipc: discarded EOF-terminated partial frame bytes={bytes}; request was not dispatched"
        );
        return Ok(None);
    }
    line.pop();
    if line.ends_with('\r') {
        line.pop();
    }
    Ok(Some(line))
}

fn spawn_socket_listener(
    mailbox: ui_mailbox::UiMailbox<crate::protocol::AppRequest>,
    subscribe_mailbox: ui_mailbox::UiMailbox<
        crate::host::event_subscriptions::HostSubscribeRequest,
    >,
    publish_mailbox: ui_mailbox::UiMailbox<crate::host::event_subscriptions::HostPublishRequest>,
) {
    use crate::platform::ipc::{self, IpcListener};

    let path = ipc::endpoint_path();
    ipc::remove_stale_endpoint(&path);
    let listener = match IpcListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("pane_ipc: failed to bind {:?}: {e}", path);
            return;
        }
    };
    log::info!("pane_ipc: listening on {:?}", path);
    // Teardown removes exactly this socket, so it has to know which one it is.
    quit::record_notify_socket(&path);
    let listener_path = path.clone();
    let trace_profile = crate::config::config_dir();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    // A Windows named-pipe instance can fail to replenish
                    // transiently. `incoming` remains live and the listener
                    // retains its old spare, so keep retrying rather than
                    // abandoning the host's only CLI endpoint.
                    log::warn!(
                        "pane_ipc: accept failed on {:?}; retaining listener and retrying: {e}",
                        listener_path
                    );
                    continue;
                }
            };
            // Capture the peer's identity HERE, synchronously in the accept
            // loop, before `std::thread::spawn` — not inside the spawned
            // connection thread. A fire-and-forget CLI process (the default
            // for `plexi notify`) can exit essentially immediately after
            // writing its request; a thread-spawn is a real scheduling gap
            // under load (backlog, CPU contention) that a short-lived peer
            // can lose the race against. Resolving and walking the ancestry
            // right here, on the accept thread, removes that gap entirely —
            // this runs before the OS has even scheduled a new thread.
            let peer_pid = resolve_socket_peer_pid(&stream);
            let peer_ancestry = peer_pid.map(capture_peer_ancestry);
            let mailbox = mailbox.clone();
            let subscribe_mailbox = subscribe_mailbox.clone();
            let publish_mailbox = publish_mailbox.clone();
            let trace_profile = trace_profile.clone();
            std::thread::spawn(move || {
                handle_socket_connection(
                    &trace_profile,
                    stream,
                    mailbox,
                    subscribe_mailbox,
                    publish_mailbox,
                    peer_ancestry,
                );
            });
        }
    });
}

/// Per-connection handler. Most connections are one-shot `AppRequest` lines.
/// Event-bus control messages (`events_subscribe`, `events_list`,
/// `events_declare`, `events_emit`) are intercepted before `AppRequest` parsing
/// because they keep the socket open and stream NDJSON back — a transport the
/// normal request/response-file path does not support.
fn handle_socket_connection(
    trace_profile: &std::path::Path,
    stream: crate::platform::ipc::IpcStream,
    mailbox: ui_mailbox::UiMailbox<crate::protocol::AppRequest>,
    subscribe_mailbox: ui_mailbox::UiMailbox<
        crate::host::event_subscriptions::HostSubscribeRequest,
    >,
    publish_mailbox: ui_mailbox::UiMailbox<crate::host::event_subscriptions::HostPublishRequest>,
    // The peer's host-established ancestor-pid chain, captured by the
    // caller (the accept loop in `spawn_socket_listener`) BEFORE this
    // connection was handed to a spawned thread — not resolved in here.
    // `None` means the peer credential lookup itself failed (platform
    // unsupported, or the kernel call errored); a resolved-but-empty or
    // no-match ancestry is logged separately in `resolve_socket_peer_pane`.
    // This is the only trustworthy sender identity for
    // `Notify`/`DismissNotification` — the request body's
    // `source_context_id`/`source_pane_id` are attacker controlled (any
    // process that can write to the socket can claim any pane).
    peer_ancestry: Option<Vec<u32>>,
) {
    use std::io::{BufRead, BufReader};
    // A writable clone so a streaming handler can push NDJSON back while the
    // BufReader owns the read side.
    let mut write_half = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pane_ipc: try_clone failed: {e}");
            return;
        }
    };
    let mut reader = BufReader::new(stream);
    let first = match read_socket_frame(&mut reader) {
        Ok(Some(first)) => first,
        Ok(None) => return,
        Err(error) => {
            log::warn!("pane_ipc: socket read failed: {error}");
            return;
        }
    };
    let first = first.trim().to_owned();
    if first.is_empty() {
        return;
    }
    // Intercept event-stream control messages. These share the `type`-tagged
    // JSON shape but are not `AppRequest` variants, so anything else falls
    // through to the normal one-shot path.
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&first) {
        match val.get("type").and_then(|t| t.as_str()) {
            Some("events_subscribe" | "pane_lifecycle_wait" | "pane_lifecycle_follow") => {
                event_stream::handle_events_subscribe(
                    write_half,
                    reader.lines(),
                    val,
                    &subscribe_mailbox,
                    &mailbox,
                    peer_ancestry.as_deref(),
                );
                return;
            }
            Some("events_list") => {
                handle_events_list(write_half, &val);
                return;
            }
            Some("command_view_follow") => {
                let workspace = val
                    .get("workspace")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string();
                log::info!("command_view: follow connected");
                crate::host::command_view::serve_follow(write_half, workspace);
                return;
            }
            Some("events_declare") | Some("events_emit") => {
                handle_events_publish(write_half, val, &publish_mailbox, peer_ancestry.as_deref());
                return;
            }
            _ => {}
        }
    }
    // Normal path: first line plus any remaining lines as AppRequests.
    handle_socket_line_in(
        trace_profile,
        &first,
        &mailbox,
        peer_ancestry.as_deref(),
        &mut write_half,
    );
    if crate::rpc::extract_response_file_token(&first).is_some() {
        return;
    }
    loop {
        let line = match read_socket_frame(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                log::warn!("pane_ipc: socket read failed: {error}");
                break;
            }
        };
        let line = line.trim().to_owned();
        if line.is_empty() {
            continue;
        }
        handle_socket_line_in(
            trace_profile,
            &line,
            &mailbox,
            peer_ancestry.as_deref(),
            &mut write_half,
        );
        if crate::rpc::extract_response_file_token(&line).is_some() {
            return;
        }
    }
}

/// Walk `peer_pid`'s ancestor chain and collect every visited pid
/// (including `peer_pid` itself), using `get_pid_ppid`. Bounded to 32 hops
/// so a lookup failure or a recycled pid mid-walk cannot spin forever;
/// stops at pid 0/1 or a repeated pid (cycle guard). Called eagerly at
/// socket-accept time — see the call site in `handle_socket_connection` for
/// why this cannot be deferred to the UI thread.
pub(crate) fn capture_peer_ancestry(peer_pid: u32) -> Vec<u32> {
    const MAX_ANCESTOR_HOPS: u32 = 32;

    let mut ancestry = Vec::new();
    let mut candidate = peer_pid;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..MAX_ANCESTOR_HOPS {
        if candidate == 0 || candidate == 1 || !seen.insert(candidate) {
            break;
        }
        ancestry.push(candidate);
        match shell::get_pid_ppid(candidate) {
            Some(ppid) => candidate = ppid,
            None => break,
        }
    }
    ancestry
}

/// Resolve the pid of the process on the other end of a connected IPC stream.
///
/// Every platform needs its own call: macOS has no stable-std API
/// (`getsockopt(SOL_LOCAL, LOCAL_PEERPID)`, verified against `libc` 0.2's
/// apple bindings); Linux exposes it via `UnixStream::peer_cred`; Windows
/// answers it for the named-pipe transport with
/// `GetNamedPipeClientProcessId`.
#[cfg(target_os = "macos")]
fn resolve_socket_peer_pid(stream: &crate::platform::ipc::IpcStream) -> Option<u32> {
    use std::os::unix::io::AsRawFd;
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            &mut pid as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 || pid <= 0 {
        log::debug!("pane_ipc: LOCAL_PEERPID getsockopt failed (rc={rc})");
        return None;
    }
    Some(pid as u32)
}

#[cfg(target_os = "linux")]
fn resolve_socket_peer_pid(stream: &crate::platform::ipc::IpcStream) -> Option<u32> {
    // `UnixStream::peer_cred` is still unstable (`peer_credentials_unix_socket`),
    // so read the same kernel-provided credentials directly. `SO_PEERCRED`
    // yields the pid recorded at `connect()` time — exactly the peer identity
    // `resolve_socket_peer_pane` needs to walk back to the owning shell.
    use std::os::unix::io::AsRawFd;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 || cred.pid <= 0 {
        log::warn!("pane_ipc: SO_PEERCRED getsockopt failed (rc={rc})");
        return None;
    }
    Some(cred.pid as u32)
}

#[cfg(windows)]
fn resolve_socket_peer_pid(stream: &crate::platform::ipc::IpcStream) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    let mut pid: u32 = 0;
    // SAFETY: `stream` owns a live server-side named-pipe handle for the
    // duration of the call, and `pid` is a local out-param.
    let ok = unsafe {
        windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(
            stream.as_raw_handle() as _,
            &mut pid,
        )
    };
    if ok == 0 || pid == 0 {
        log::warn!(
            "pane_ipc: GetNamedPipeClientProcessId failed: {}",
            std::io::Error::last_os_error()
        );
        return None;
    }
    Some(pid)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn resolve_socket_peer_pid(_stream: &crate::platform::ipc::IpcStream) -> Option<u32> {
    None
}

/// Answer `events_list`: write the declared `(context_id, app_id, stream)`
/// triples and close. Pure discovery — no grant check, no identity, no
/// streaming. Context-qualified since stint 0724 Phase D: the same `app_id`
/// may declare different streams/schemas in different contexts.
fn handle_events_list(mut write_half: crate::platform::ipc::IpcStream, val: &serde_json::Value) {
    use std::io::Write;
    let json_mode = val["json"].as_bool().unwrap_or(false);
    let streams = crate::host::app_timeline::global()
        .lock()
        .unwrap()
        .all_declared_streams();
    log::info!(
        "events: list requested ({} stream(s) declared)",
        streams.len()
    );
    if json_mode {
        let arr: Vec<serde_json::Value> = streams
            .iter()
            .map(|(context_id, a, s)| {
                serde_json::json!({"context_id": context_id, "app_id": a, "stream": s})
            })
            .collect();
        let _ = writeln!(
            write_half,
            "{}",
            serde_json::json!({"type": "streams", "streams": arr})
        );
    } else if streams.is_empty() {
        let _ = writeln!(write_half, "No event streams declared by running apps.");
    } else {
        for (context_id, app_id, stream) in &streams {
            let _ = writeln!(write_half, "{context_id}  {app_id}  {stream}");
        }
    }
    let _ = write_half.flush();
}

/// Handle a CLI/agent publish control message (`events_declare` / `events_emit`).
/// Routes the broker-gated declare/emit round-trip through the UI thread (which
/// owns the grant store), then writes exactly one JSON reply line and closes.
/// Like subscribe, a first-time publish under the broker's default `Ask` posture
/// blocks here until the user answers the host consent modal.
fn handle_events_publish(
    mut write_half: crate::platform::ipc::IpcStream,
    val: serde_json::Value,
    publish_mailbox: &ui_mailbox::UiMailbox<crate::host::event_subscriptions::HostPublishRequest>,
    // See `handle_events_subscribe`'s matching parameter.
    peer_ancestry: Option<&[u32]>,
) {
    use crate::host::event_subscriptions::{HostPublishReply, HostPublishRequest, PublishAction};
    use std::io::Write;
    use std::time::Duration;

    let reply_err = |mut w: crate::platform::ipc::IpcStream, message: String| {
        let _ = writeln!(
            w,
            "{}",
            serde_json::json!({"type": "error", "message": message})
        );
        let _ = w.flush();
    };

    let app_id = val["app_id"].as_str().unwrap_or_default().to_string();
    if app_id.trim().is_empty() {
        reply_err(write_half, "missing required field: app_id".to_string());
        return;
    }
    let from_pane_id = val["from_pane_id"].as_u64();

    // Build the publish action from the wire shape. Identity fields are never
    // parsed here — the UI thread host-stamps them.
    let action = match val["type"].as_str() {
        Some("events_declare") => {
            match serde_json::from_value::<Vec<crate::protocol::EventStreamDecl>>(
                val["streams"].clone(),
            ) {
                Ok(decls) => PublishAction::Declare(decls),
                Err(e) => {
                    reply_err(write_half, format!("invalid streams: {e}"));
                    return;
                }
            }
        }
        Some("events_emit") => {
            let emitted = crate::host::app_timeline::EmittedEvent {
                event: val["event"].as_str().unwrap_or_default().to_string(),
                actor: serde_json::from_value(val["actor"].clone())
                    .unwrap_or(crate::protocol::AppEventActor::Agent),
                // Host-stamped by the UI thread from the pane identity.
                actor_id: None,
                caused_by: None,
                summary: val["summary"].as_str().unwrap_or_default().to_string(),
                resource_id: val["resource_id"].as_str().unwrap_or_default().to_string(),
                resource_scope: val["resource_scope"].as_str().map(String::from),
                revision_after: val["revision_after"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                payload: val.get("payload").filter(|v| !v.is_null()).cloned(),
                state_ref: val["state_ref"].as_str().map(String::from),
                revision_before: val["revision_before"].as_str().map(String::from),
                rollback_token: val["rollback_token"].as_str().map(String::from),
                changed_resources: val["changed_resources"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                suggested_trigger: serde_json::from_value(val["suggested_trigger"].clone()).ok(),
            };
            PublishAction::Emit(Box::new(emitted))
        }
        other => {
            reply_err(write_half, format!("unknown publish type: {other:?}"));
            return;
        }
    };

    let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<HostPublishReply>(1);
    let req = HostPublishRequest {
        app_id: app_id.clone(),
        action,
        from_pane_id,
        subscriber_override: None,
        // Resolved on the UI thread — see `drain_event_subscribe_channel`.
        workspace_root_override: None,
        context_id_override: None,
        peer_ancestry: peer_ancestry.map(<[u32]>::to_vec),
        reply: reply_tx,
    };
    if publish_mailbox.send(req).is_err() {
        reply_err(
            write_half,
            "host not accepting publish requests".to_string(),
        );
        return;
    }

    // Generous wait: a first-time publish under the default `Ask` posture blocks
    // until the user answers the host consent modal; a pre-granted publish
    // replies near-instantly.
    let reply = match reply_rx.recv_timeout(Duration::from_secs(120)) {
        Ok(reply) => reply,
        Err(_) => {
            reply_err(write_half, "publish consent timed out".to_string());
            return;
        }
    };
    let line = match reply {
        HostPublishReply::Ok { detail, event_id } => {
            log::info!("events: publish to '{app_id}' ok — {detail}");
            serde_json::json!({"type": "ok", "detail": detail, "event_id": event_id})
        }
        HostPublishReply::Err { message } => {
            log::info!("events: publish to '{app_id}' rejected — {message}");
            serde_json::json!({"type": "error", "message": message})
        }
    };
    let _ = writeln!(write_half, "{line}");
    let _ = write_half.flush();
}

/// Start a registry watcher for every root `registries` currently holds a
/// view for. Used at construction (eagerly watch whatever root(s) were
/// loaded up front) and kept in sync every `logic` pass by
/// `PlexiApp::reconcile_registry_watchers` — a root gains a watcher the
/// first time something resolves a view for it, regardless of which context
/// is active (stint 0724 Phase B).
fn start_registry_watchers_for(
    registries: &crate::app::registry_views::RegistryViews,
    ui_wake: &std::sync::Arc<dyn ui_mailbox::UiWake>,
) -> HashMap<
    std::path::PathBuf,
    (
        crate::app::registry_watcher::AppRegistryWatcher,
        ui_mailbox::MailboxReceiver<()>,
    ),
> {
    let mut watchers = HashMap::new();
    for root in registries.roots() {
        if let Some((watcher, rx)) = crate::app::registry_watcher::start(
            crate::app::registry::registry_watch_dirs(&root),
            std::sync::Arc::clone(ui_wake),
        ) {
            watchers.insert(root, (watcher, rx));
        }
    }
    watchers
}

/// Test-only observability for `PlexiApp::emit_scope_invalidation`. Phase A
/// (stint 0724) has no real consumer of `ScopeInvalidation` — later phases
/// add handling inside that method as they migrate onto the scope model — so
/// this counter is the smoke-test seam proving the emit call sites actually
/// fire, without inventing a log-capture harness for a method that is
/// otherwise side-effect-free logging.
#[cfg(test)]
pub(crate) static SCOPE_INVALIDATION_COUNT_FOR_TEST: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

impl PlexiApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        frame_tick: crate::platform::logging::FrameTick,
        heartbeat_ctx: crate::platform::logging::HeartbeatCtxSlot,
        ui_phase: crate::platform::logging::UiPhaseTracker,
    ) -> Self {
        // Hand the egui context to the heartbeat watchdog so it can tell
        // healthy zero-frame idle apart from a genuine UI freeze.
        if let Ok(mut slot) = heartbeat_ctx.lock() {
            *slot = Some(cc.egui_ctx.clone());
        }

        // Register eframe's shared wgpu device so live WASM surfaces composite
        // zero-copy (no per-frame readback). Absent only if eframe was built
        // without the wgpu backend, in which case WASM GPU apps fall back to a
        // dedicated device + readback path.
        if let Some(render_state) = cc.wgpu_render_state.clone() {
            crate::host::wasm_gpu::register_host_render_state(render_state);
        } else {
            log::warn!("no wgpu render state at startup; WASM surfaces use readback fallback");
        }

        // The one wake seam for every off-UI-thread producer (stint 0507).
        let ui_wake: std::sync::Arc<dyn ui_mailbox::UiWake> =
            std::sync::Arc::new(ui_mailbox::EguiWake::new(cc.egui_ctx.clone()));

        #[cfg(target_os = "macos")]
        crate::platform::macos_menu::customize_app_menu();
        #[cfg(target_os = "macos")]
        crate::platform::finder_service::register(std::sync::Arc::clone(&ui_wake));

        // Repaint-cause diagnostics (#2019): route egui_term's repaint labels
        // into the host counters; `update()` flushes a summary every 10s.
        egui_term::set_repaint_diag_hook(|label| {
            use crate::platform::frame_diag::{note, RepaintCause};
            match label {
                "terminal_cursor_blink" => note(RepaintCause::TerminalCursorBlink),
                "terminal_search_blink" => note(RepaintCause::TerminalSearchBlink),
                "terminal_pty_output" => note(RepaintCause::TerminalPtyOutput),
                "pointer_tracking" => note(RepaintCause::PointerTracking),
                other => log::warn!(
                    target: "plexi::frame_diag",
                    "unknown egui_term repaint cause label: {other}"
                ),
            }
        });
        log::info!(target: "plexi::frame_diag", "frame diagnostics active; summary every 10s");

        // Drag-resize frame profiling (stint 0731): only installed when the
        // env flag is set, so an uninstrumented run pays zero cost.
        if crate::platform::ui_profile::enabled() {
            egui_term::set_span_hook(|label, nanos| {
                crate::platform::ui_profile::note_nanos(label, nanos)
            });
            log::info!(
                target: "plexi::ui_profile",
                "ui_profile: PLEXI_UI_PROFILE=1 — per-frame ui() profiling active"
            );
        }

        theme::setup_fonts(&cc.egui_ctx);
        configure_native_semantics(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        // Hot-reload watcher set (#83). Constructed once per host instance.
        // The receiver lives on `self.hot_reload_rx`; `update()` drains it
        // each frame and reloads the matching pane.
        let (mut hr_watcher, hr_rx) =
            crate::host::hot_reload::HotReloadWatcher::new(std::sync::Arc::clone(&ui_wake));

        // App state file watcher (stint 0644).
        let (mut sw_watcher, sw_rx) =
            crate::host::state_watch::StateWatcher::new(std::sync::Arc::clone(&ui_wake));

        // Config file watcher (#1115). Watches config.toml for saves so the
        // host can hot-reload theme/font/notification settings automatically.
        let (mut cfg_watcher, mut cfg_reload_rx) = match crate::config::watcher::start(
            crate::config::config_path(),
            std::sync::Arc::clone(&ui_wake),
        ) {
            Some((w, rx)) => (Some(w), Some(rx)),
            None => (None, None),
        };

        // Resolve the active workspace (explicit `plexi <path>` arg, then
        // CWD-walk fallback) and overlay its channel-scoped config on top of
        // the global config. Project values win on a per-field basis; unset
        // project fields preserve the global value.
        let active_workspace = config::active_workspace_root();
        let config = config::PlexiConfig::load_with_workspace(active_workspace.as_deref());
        let model_modes = config.resolved_model_modes();
        let key_bindings = crate::host::keys::build_key_bindings_with_user_keys(
            config.keybindings.as_ref(),
            config.keys.as_ref(),
        );
        let notifications_enabled = config
            .notifications
            .as_ref()
            .and_then(|n| n.enabled)
            .unwrap_or(true);
        let notifications_focus_mode = config
            .notifications
            .as_ref()
            .and_then(|n| n.focus_mode)
            .unwrap_or(false);
        let notifications_sound = config.notifications.as_ref().and_then(|n| n.cue_sound());
        let focus_history_depth = config.focus_history_depth.unwrap_or(100);
        log::info!("config: focus_history_depth={focus_history_depth}");
        let default_font_size = config.font_size.unwrap_or(theme::FONT_SIZE);
        let theme_cfg = Self::resolve_theme_config(&config);
        let colors = Colors::from_config(&theme_cfg);
        let dark_mode = !theme::is_light_preset(
            config
                .theme
                .as_ref()
                .and_then(|t| t.preset.as_deref())
                .unwrap_or(""),
        );
        theme::setup_style(&cc.egui_ctx, &colors, dark_mode);
        let window_theme = if dark_mode {
            egui::SystemTheme::Dark
        } else {
            egui::SystemTheme::Light
        };
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::SetTheme(window_theme));
        log::info!("theme: set_window_theme dark_mode={dark_mode}");

        let (tx, rx) = mpsc::channel();

        let cwd = std::env::current_dir().unwrap_or_default();
        let mut registries = crate::app::registry_views::RegistryViews::new();
        registries.view_for_root(&cwd);
        let registry_watchers = start_registry_watchers_for(&registries, &ui_wake);

        let profile_dir = crate::config::config_dir();
        crate::host::process_cleanup::record_session_marker(&profile_dir);

        // Initialize the event log. Global log goes to ~/.plexi-*/events.jsonl.
        // Per-context routing (stint 0724 Phase E) is decided per-record at
        // each emit call site via `emit_scoped`, not once here from cwd.
        {
            let global_path = profile_dir.join("events.jsonl");
            crate::host::event_log::init_global(global_path);
        }

        // Check for an unclean shutdown from the previous session.
        // If the focus journal has an unclosed segment, emit a crash_recovery event.
        {
            let journal_path = profile_dir.join("focus-journal.jsonl");
            crate::app::focus_journal::recover_from_focus_journal(&journal_path);
        }

        // Spawn background update check. Sends the latest version once if newer.
        let (update_mailbox, update_rx) =
            ui_mailbox::UiMailbox::channel(std::sync::Arc::clone(&ui_wake), "update_check");
        crate::cli::updater::spawn_update_check(profile_dir.clone(), update_mailbox);
        let last_update_check = std::time::Instant::now();

        let (pane_ipc_mailbox, pane_ipc_rx) =
            ui_mailbox::UiMailbox::channel(std::sync::Arc::clone(&ui_wake), "pane_ipc");
        let (event_subscribe_mailbox, event_subscribe_rx) =
            ui_mailbox::UiMailbox::channel(std::sync::Arc::clone(&ui_wake), "events_subscribe");
        let (event_publish_mailbox, event_publish_rx) =
            ui_mailbox::UiMailbox::channel(std::sync::Arc::clone(&ui_wake), "events_publish");
        // The mailbox wake only reaches an idle host if macOS never naps
        // the process; an App-Napped host defers cross-thread event-loop
        // wakeups and drains queued IPC in one late burst (stint 0479).
        #[cfg(target_os = "macos")]
        crate::platform::app_nap::disable_app_nap();
        // File a death/tamper item and leave a running stamp before any client
        // can connect. A kill skips `exit_host`, so the next start can see it.
        crate::broker::gate::PermissionMonitor::for_profile(&profile_dir);
        spawn_socket_listener(
            pane_ipc_mailbox,
            event_subscribe_mailbox.clone(),
            event_publish_mailbox,
        );
        let relay_start = crate::cli::relay::start_host_relay();
        log::info!("relay: host start outcome={relay_start:?}");
        let host_subscriptions = crate::host::event_subscriptions::HostSubscriptionService::new(
            &profile_dir,
            crate::host::app_timeline::global(),
        );
        if crate::release::feature_enabled(crate::release::ReleaseFeature::McpClient) {
            if let Err(e) = host_mcp::start_host_mcp_server(
                event_subscribe_mailbox.with_source("host_mcp_subscribe"),
                &profile_dir,
            ) {
                log::warn!("host_mcp: failed to start host MCP server: {e}");
            }
        } else {
            // MCP is outside the stable v1 host. Do not bind a listener or
            // create pane credentials merely because the terminal host boots.
            crate::release::log_feature_blocked(crate::release::ReleaseFeature::McpClient);
        }

        // One-time migration: remove the legacy file-queue directory if it
        // still exists from a previous install. Notify commands now travel
        // over the PLEXI_SOCKET, so the directory is dead weight.
        let _ = std::fs::remove_dir_all(profile_dir.join("notify-queue"));

        // Try to load saved workspace
        if let Some(ws) = WorkspaceFile::load() {
            if let Some(restored) = restore_workspace(
                ws,
                &config,
                WorkspaceRestoreCollaborators {
                    profile_dir: &profile_dir,
                    colors: &colors,
                    hot_reload: &mut hr_watcher,
                    state_watch: &mut sw_watcher,
                    terminal_spawner: Some((&cc.egui_ctx, &tx)),
                },
            ) {
                let mut host = crate::host::model::HostModel::new();
                host.seed_next_pane_id(restored.next_pane_id);
                let mut app = Self {
                    pty_event_rx: rx,
                    pty_event_tx: tx,
                    theme: theme::terminal_theme(&theme_cfg),
                    colors,
                    default_font_size,
                    ui_scale: config.ui_scale.map_or(ui_scale::UI_SCALE_DEFAULT, ui_scale::clamp_scale),
                    ui_scale_persist_at: None,
                    ctx: cc.egui_ctx.clone(),
                    router: restored.router,
                    windows: restored.windows,
                    active_window: restored.active_window,
                    sidebar_visible: restored.sidebar_visible,
                    show_shortcuts: false,
                    show_changelog: false,
                    show_ui_gallery: false,
                    ui_gallery_normal_buf: "workspace/search".to_string(),
                    ui_gallery_focused_buf: "Focused field".to_string(),
                    ui_gallery_show_text_modal: false,
                    ui_gallery_modal_buf: String::new(),
                    quitting: false,
                    quit_press_count: 0,
                    quit_last_press: None,
                    welcome_delete_press_count: 0,
                    welcome_delete_last_press: None,
                    config: config.clone(),
                    key_bindings: key_bindings.clone(),
                    binding_table: crate::host::keys::build_binding_table(&key_bindings),
                    claimed_host_key_releases: HashSet::new(),
                    pending_close: false,
                    pending_quit: false,
                    pending_context_close: None,
                    frame_tick,
                    ui_phase: ui_phase.clone(),
                    frame_diag_window: None,
                    prev_screen_rect: None,
                    drag_window_last_seen: None,
                    pending_screenshots: Vec::new(),
                    approval_buttons: Vec::new(),
                    synthetic_input_frame: false,
                    pending_slot_waits: Vec::new(),
                    pane_mailboxes: HashMap::new(),
                    pending_message_recvs: Vec::new(),
                    next_pane_message_id: 1,
                    closed_pane_slots: HashMap::new(),
                    agent_attention: Default::default(),
                    fleet_history: std::cell::RefCell::new(crate::workspace::load_agent_history()),
                    pane_results: HashMap::new(),
                    pending_result_waits: Vec::new(),
                    pending_agent_waits: Vec::new(),
                    child_result_events: HashMap::new(),
                    child_parent_panes: HashMap::new(),
                    pulled_child_results: std::collections::HashSet::new(),
                    delivered_child_batches: HashMap::new(),
                    agent_tool_waits: Vec::new(),
                    agent_tool_children: std::collections::HashMap::new(),
                    conjoined_contexts: std::collections::HashMap::new(),
                    conjoined_parent_focus: None,
                    conjoined_parent_rect: None,
                    team_runs: Vec::new(),
                    pending_agent_boots: Vec::new(),
                    pending_submits: Vec::new(),
                    pending_send_approvals: Vec::new(),
                    pane_heartbeats: restored.pane_heartbeats,
                    pane_worktrees: HashMap::new(),
                    last_worktree_close_reports: HashMap::new(),
                    last_sent_window_title: None,
                    permission_store_dir: profile_dir.clone(),
                    #[cfg(test)]
                    _test_scratch: None,
                    renaming_window: None,
                    rename_buffer: String::new(),
                    editing_description: None,
                    description_buffer: String::new(),
                    drag_context: None,
                    parked_section_expanded: false,
                    show_command_palette: false,
                    palette_query: String::new(),
                    palette_selected: 0,
                    palette_selected_key: None,
                    palette_workspace_root: None,
                    palette_notes: Vec::new(),
                    palette_commands: Vec::new(),
                    palette_scroll_reset: false,
                    palette_agent_count_logged: None,
                    context_visit_history: Vec::new(),
                    renaming_pane: None,
                    text_overlay: None,
                    text_overlay_browse_rx: None,
                    registries,
                    pending_notifications: load_pending_notifications_from(
                        &profile_dir.join("notifications.json"),
                    ),
                    current_notify_id: None,
                    modal_focused_option: 0,
                    modal_input_buffer: String::new(),
                    notification_input_drafts: HashMap::new(),
                    quick_note_text: String::new(),
                    quick_note_attachments: Vec::new(),
                    quick_note_ctx: QuickNoteCtx::default(),
                    notes_migration_pending: true,
                    notes_picker_entries: Vec::new(),
                    notes_picker_selected: 0,
                    notes_picker_last_drawn_selected: None,
                    notes_picker_query: String::new(),
                    notes_picker_preview: None,
                    notes_picker_pending_delete: None,
                    notes_picker_status: None,
                    notes_picker_last_copied: None,
                    notes_picker_copy_unsent: false,
                    notes_index: crate::notes::NotesIndexCache::default(),
                    notes_index_rx: None,
                    notes_index_inflight_anchor: None,
                    notes_index_rerun: false,
                    notes_picker_shown_paths: Vec::new(),
                    notes_index_loaded: false,
                    modal_state_notify_id: String::new(),
                    notification_images: HashMap::new(),
                    notifications_enabled,
                    notifications_focus_mode,
                    notifications_sound,
                    notification_cue_playback: None,
                    focus_stack: Vec::new(),
                    focus_history_depth,
                    pane_focus_history: Vec::new(),
                    pane_focus_future: Vec::new(),
                    navigating_history: false,
                    last_notify_poll: std::time::Instant::now(),
                    scheduler: crate::host::scheduler::Scheduler::new(),
                    host,
                    host_services: crate::host::services::HostServices::new(),
                    background_apps: HashMap::new(),
                    directed_pipes: HashMap::new(),
                    hot_reload: hr_watcher,
                    hot_reload_rx: hr_rx,
                    state_watch: sw_watcher,
                    state_watch_rx: sw_rx,
                    _config_watcher: cfg_watcher.take(),
                    config_reload_rx: cfg_reload_rx.take(),
                    registry_watchers,
                    workspace_dirty: false,
                    last_workspace_save: std::time::Instant::now(),
                    minimap: crate::render::minimap::MinimapState::with_visible(
                        restored.minimap_visible,
                    ),
                    last_page_x_per_row: HashMap::new(),
                    context_active_window: restored.context_active_window,
                    minimap_visible_per_context: HashMap::new(),
                    next_window_id: restored.next_window_id,
                    pane_snapshot_len: 0,
                    pane_anims: Vec::new(),
                    edge_pulse: None,
                    click_flash: None,
                    ui_wake: std::sync::Arc::clone(&ui_wake),
                    update_rx: Some(update_rx),
                    last_update_check,
                    display_version: read_display_version(),
                    update_available: None,
                    update_quit_pending: false,
                    update_confirm_prompt: false,
                    shutdown_requested: false,
                    pane_ipc_rx,
                    host_subscriptions,
                    resource_leases: None,
                    event_subscribe_rx,
                    event_publish_rx,
                    pending_event_consents: std::collections::VecDeque::new(),
                    pending_app_subscription_replies: Vec::new(),
                    pending_raw_wasm_launches: std::collections::VecDeque::new(),
                    last_logged_focus: None,
                    focus_started_at: None,
                    terminal_traversal_lock: None,
                    focus_journal_path: profile_dir.join("focus-journal.jsonl"),
                    overlay_held_cmds: Vec::new(),
                    agent_host: crate::agent::AgentHost::production(config.ai, model_modes),
                    pending_pane_clicks: HashMap::new(),
                    pending_pane_inputs: HashMap::new(),
                    pending_pane_drags: HashMap::new(),
                    pending_pane_pointer_frames: HashMap::new(),
                    zoom: zoom::ZoomState::default(),
                    attention_queue: attention_queue::AttentionQueueState::default(),
                    approval_keyboard: Default::default(),
                };
                app.apply_context_transition_effects();
                return app;
            }
        }

        // Default: empty context — welcome screen is shown until the user creates a pane.
        // If CWD has a .plexi/ anchor, use its context defaults for name/description
        // and set root to the anchor path.
        let panes: HashMap<u64, Pane> = HashMap::new();
        let tree = Tree::empty("plexi");

        let path = {
            let cwd = std::env::current_dir()
                .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
            // macOS GUI apps launch with CWD = /. Use home_dir instead so
            // the initial context and all derived CWDs start at ~.
            if cwd == std::path::Path::new("/") {
                dirs::home_dir().unwrap_or(cwd)
            } else {
                cwd
            }
        };

        let anchor = crate::host::anchor::Anchor::detect(&path);
        let (default_name, default_description, default_root) = match anchor.as_ref() {
            Some(a) => {
                let defaults = a.context_defaults.as_ref();
                let name = defaults.and_then(|d| d.name.clone()).unwrap_or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Default".to_string())
                });
                let desc = defaults.and_then(|d| d.description.clone());
                log::info!(
                    "anchor detected at CWD: name={:?} description={:?} root={}",
                    name,
                    desc,
                    a.root.display()
                );
                (name, desc, a.root.clone())
            }
            None => (
                "Default".to_string(),
                None,
                dirs::home_dir().unwrap_or_else(|| path.clone()),
            ),
        };

        let default_cwd = std::env::current_dir().unwrap_or_default();
        let mut default_registries = crate::app::registry_views::RegistryViews::new();
        default_registries.view_for_root(&default_cwd);
        let default_registry_watchers = start_registry_watchers_for(&default_registries, &ui_wake);

        let agent_host =
            crate::agent::AgentHost::production(config.ai.clone(), config.resolved_model_modes());
        // Standing ruling: a context root must gitignore its app_states dir
        // (personal local data, never committed). Guarded so constructing a
        // Context never creates directories as a side effect.
        if default_root.is_dir() {
            if let Err(error) = crate::workspace::secrets::ensure_app_state_gitignore(&default_root)
            {
                log::warn!(
                    "could not ensure {}/.plexi/.gitignore covers app_states/: {error}",
                    default_root.display()
                );
            }
        }
        let mut app = Self {
            pty_event_rx: rx,
            pty_event_tx: tx,
            theme: theme::terminal_theme(&theme_cfg),
            colors,
            default_font_size,
            ui_scale: config.ui_scale.map_or(ui_scale::UI_SCALE_DEFAULT, ui_scale::clamp_scale),
            ui_scale_persist_at: None,
            ctx: cc.egui_ctx.clone(),
            router: crate::workspace::router::WorkspaceRouter::new(
                vec![crate::host::context::Context {
                    name: crate::host::context::ContextName::custom(default_name),
                    root: default_root,
                    description: default_description,
                    context_id: 1,
                    parent_id: None,
                    depth: 0,
                    parked: false,
                }],
                0,
            ),
            windows: vec![Window {
                name: String::new(),
                path,
                tree,
                panes,
                focused_pane: None,
                zoomed_pane: None,
                grid_x: 0,
                grid_y: 0,
                window_id: 1,
                context_id: 1,
            }],
            active_window: 0,
            sidebar_visible: true,
            show_shortcuts: false,
            show_changelog: false,
            show_ui_gallery: false,
            ui_gallery_normal_buf: "workspace/search".to_string(),
            ui_gallery_focused_buf: "Focused field".to_string(),
            ui_gallery_show_text_modal: false,
            ui_gallery_modal_buf: String::new(),
            quitting: false,
            quit_press_count: 0,
            quit_last_press: None,
            welcome_delete_press_count: 0,
            welcome_delete_last_press: None,
            config,
            binding_table: crate::host::keys::build_binding_table(&key_bindings),
            claimed_host_key_releases: HashSet::new(),
            key_bindings,
            pending_close: false,
            pending_quit: false,
            pending_context_close: None,
            frame_tick,
            ui_phase,
            frame_diag_window: None,
            prev_screen_rect: None,
            drag_window_last_seen: None,
            pending_screenshots: Vec::new(),
            approval_buttons: Vec::new(),
            synthetic_input_frame: false,
            pending_slot_waits: Vec::new(),
            pane_mailboxes: HashMap::new(),
            pending_message_recvs: Vec::new(),
            next_pane_message_id: 1,
            closed_pane_slots: HashMap::new(),
            agent_attention: Default::default(),
            fleet_history: std::cell::RefCell::new(crate::workspace::load_agent_history()),
            pane_results: HashMap::new(),
            pending_result_waits: Vec::new(),
            pending_agent_waits: Vec::new(),
            child_result_events: HashMap::new(),
            child_parent_panes: HashMap::new(),
            pulled_child_results: std::collections::HashSet::new(),
            delivered_child_batches: HashMap::new(),
            agent_tool_waits: Vec::new(),
            agent_tool_children: std::collections::HashMap::new(),
            conjoined_contexts: std::collections::HashMap::new(),
            conjoined_parent_focus: None,
            conjoined_parent_rect: None,
            team_runs: Vec::new(),
            pending_agent_boots: Vec::new(),
            pending_submits: Vec::new(),
            pending_send_approvals: Vec::new(),
            pane_heartbeats: HashMap::new(),
            pane_worktrees: HashMap::new(),
            last_worktree_close_reports: HashMap::new(),
            last_sent_window_title: None,
            permission_store_dir: crate::config::config_dir(),
            #[cfg(test)]
            _test_scratch: None,
            renaming_window: None,
            rename_buffer: String::new(),
            editing_description: None,
            description_buffer: String::new(),
            drag_context: None,
            parked_section_expanded: false,
            show_command_palette: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_selected_key: None,
            palette_workspace_root: None,
            palette_notes: Vec::new(),
            palette_commands: Vec::new(),
            palette_scroll_reset: false,
            palette_agent_count_logged: None,
            context_visit_history: Vec::new(),
            renaming_pane: None,
            text_overlay: None,
            text_overlay_browse_rx: None,
            registries: default_registries,
            pending_notifications: load_pending_notifications_from(
                &crate::config::config_dir().join("notifications.json"),
            ),
            current_notify_id: None,
            modal_focused_option: 0,
            modal_input_buffer: String::new(),
            notification_input_drafts: HashMap::new(),
            quick_note_text: String::new(),
            quick_note_attachments: Vec::new(),
            quick_note_ctx: QuickNoteCtx::default(),
            notes_migration_pending: true,
            notes_picker_entries: Vec::new(),
            notes_picker_selected: 0,
            notes_picker_last_drawn_selected: None,
            notes_picker_query: String::new(),
            notes_picker_preview: None,
            notes_picker_pending_delete: None,
            notes_picker_status: None,
            notes_picker_last_copied: None,
            notes_picker_copy_unsent: false,
            notes_index: crate::notes::NotesIndexCache::default(),
            notes_index_rx: None,
            notes_index_inflight_anchor: None,
            notes_index_rerun: false,
            notes_picker_shown_paths: Vec::new(),
            notes_index_loaded: false,
            modal_state_notify_id: String::new(),
            notification_images: HashMap::new(),
            notifications_enabled,
            notifications_focus_mode,
            notifications_sound,
            notification_cue_playback: None,
            focus_stack: Vec::new(),
            focus_history_depth,
            pane_focus_history: Vec::new(),
            pane_focus_future: Vec::new(),
            navigating_history: false,
            last_notify_poll: std::time::Instant::now(),
            scheduler: crate::host::scheduler::Scheduler::new(),
            host: crate::host::model::HostModel::new(),
            host_services: crate::host::services::HostServices::new(),
            background_apps: HashMap::new(),
            directed_pipes: HashMap::new(),
            hot_reload: hr_watcher,
            hot_reload_rx: hr_rx,
            state_watch: sw_watcher,
            state_watch_rx: sw_rx,
            _config_watcher: cfg_watcher.take(),
            config_reload_rx: cfg_reload_rx.take(),
            registry_watchers: default_registry_watchers,
            workspace_dirty: false,
            last_workspace_save: std::time::Instant::now(),
            minimap: crate::render::minimap::MinimapState::new(),
            last_page_x_per_row: HashMap::new(),
            context_active_window: HashMap::new(),
            minimap_visible_per_context: HashMap::new(),
            next_window_id: 2,
            pane_snapshot_len: 0,
            pane_anims: Vec::new(),
            edge_pulse: None,
            click_flash: None,
            ui_wake: std::sync::Arc::clone(&ui_wake),
            update_rx: Some(update_rx),
            last_update_check,
            display_version: read_display_version(),
            update_available: None,
            update_quit_pending: false,
            update_confirm_prompt: false,
            shutdown_requested: false,
            pane_ipc_rx,
            host_subscriptions,
            resource_leases: None,
            event_subscribe_rx,
            event_publish_rx,
            pending_event_consents: std::collections::VecDeque::new(),
            pending_app_subscription_replies: Vec::new(),
            pending_raw_wasm_launches: std::collections::VecDeque::new(),
            last_logged_focus: None,
            focus_started_at: None,
            terminal_traversal_lock: None,
            focus_journal_path: profile_dir.join("focus-journal.jsonl"),
            overlay_held_cmds: Vec::new(),
            agent_host,
            pending_pane_clicks: HashMap::new(),
            pending_pane_inputs: HashMap::new(),
            pending_pane_drags: HashMap::new(),
            pending_pane_pointer_frames: HashMap::new(),
            zoom: zoom::ZoomState::default(),
            attention_queue: attention_queue::AttentionQueueState::default(),
            approval_keyboard: Default::default(),
        };
        // Seed the base root terminal so a fresh profile boots straight into a
        // live pane — identical in shape to a freshly created context (root pane
        // present, focused_pane set) rather than the empty welcome screen. Only
        // the fresh-profile Default branch reaches here; the restore-from-saved
        // workspace path returns earlier and is untouched. The welcome screen
        // still renders for any genuinely-empty window elsewhere.
        let seed_cwd = app.windows[0].path.clone();
        let seed_context = app.pane_context_env_for_window(0);
        if app
            .seed_window_root_pane(
                0,
                &seed_context,
                seed_cwd,
                crate::pane_ops::create::TerminalSpawnOptions::default(),
            )
            .is_some()
        {
            log::info!("first boot: seeded base root pane in context 1");
        } else {
            log::warn!(
                "first boot: base root pane seeding failed — falling back to welcome screen"
            );
        }
        app.apply_context_transition_effects();
        app
    }

    /// Install a [`RestoredWorkspace`] into an already-constructed `PlexiApp`,
    /// replacing its router, windows, focus, heartbeats, and pane/window id
    /// counters, then applying context transition effects.
    #[cfg(test)]
    pub(crate) fn install_restored_workspace(&mut self, restored: RestoredWorkspace) {
        self.fleet_history = std::cell::RefCell::new(crate::workspace::load_agent_history());
        self.router = restored.router;
        self.windows = restored.windows;
        self.active_window = restored.active_window;
        self.sidebar_visible = restored.sidebar_visible;
        self.pane_heartbeats = restored.pane_heartbeats;
        self.context_active_window = restored.context_active_window;
        self.minimap = crate::render::minimap::MinimapState::with_visible(restored.minimap_visible);
        self.next_window_id = restored.next_window_id;
        self.host.seed_next_pane_id(restored.next_pane_id);
        self.apply_context_transition_effects();
    }

    /// Resume a Standby agent pane by ID, marking the workspace dirty when
    /// a Standby pane wakes up.
    pub(crate) fn resume_standby_pane(&mut self, pane_id: crate::spatial::tiling::PaneId) -> bool {
        let mut resumed = false;
        for win in &mut self.windows {
            if let Some(pane) = win.panes.get_mut(&pane_id) {
                if pane.resume_from_standby() {
                    resumed = true;
                }
                break;
            }
        }
        if resumed {
            self.mark_workspace_dirty();
        }
        resumed
    }

    /// Search ALL windows (not just the active one) for a pane by ID.
    /// Returns (window_index, tile_id). O(n*m) but n is typically 1-3.
    pub(crate) fn find_pane_in_any_window(
        &self,
        pane_id: crate::spatial::tiling::PaneId,
    ) -> Option<(usize, egui_tiles::TileId)> {
        self.windows
            .iter()
            .enumerate()
            .find_map(|(idx, win)| win.tree.tiles.find_pane(&pane_id).map(|tile| (idx, tile)))
    }

    /// True when `pane_id` is a live human-owned terminal pane with no bound
    /// team member. App panes, agent panes, and panes spawned by an agent
    /// (including via `plexi pane new`) return `false`.
    pub(crate) fn is_human_owned_pane(&self, pane_id: crate::spatial::tiling::PaneId) -> bool {
        if self.team_member_for_pane(pane_id).is_some() {
            return false;
        }
        let Some((win_idx, _)) = self.find_pane_in_any_window(pane_id) else {
            return false;
        };
        self.windows
            .get(win_idx)
            .and_then(|win| win.panes.get(&pane_id))
            .and_then(|pane| pane.as_terminal())
            .is_some_and(|term| term.is_human_owned())
    }

    /// The posture belongs to the pane, not the mutable agent registry. It is
    /// therefore safe to use at admission time after the worker has changed
    /// directory or its settings.toml has been edited.
    pub(crate) fn pane_permission_posture(
        &self,
        pane_id: crate::spatial::tiling::PaneId,
    ) -> Option<crate::broker::PermissionPosture> {
        let (window, _) = self.find_pane_in_any_window(pane_id)?;
        let pane = self.windows[window].panes.get(&pane_id)?.as_terminal()?;
        if let Some(posture) = pane.agent_permission_posture.clone() {
            return Some(posture);
        }
        if let Some(posture) = pane
            .standby_session
            .as_ref()
            .and_then(|s| s.permission_posture.clone())
        {
            return Some(posture);
        }
        let role = pane.agent_role.as_deref().or_else(|| {
            pane.standby_session
                .as_ref()
                .and_then(|s| s.config_name.as_deref())
        })?;
        let lookup_root = self.pane_context_authority_root(pane_id);
        let profile_dir = crate::config::config_dir();
        let posture = crate::cli::agent::resolve_agent_definition_in(
            role,
            &profile_dir,
            lookup_root.as_deref(),
        )
        .ok()
        .map(|(def, _)| def.posture)?;
        if let Some(parent_id) = pane.parent_pane_id.filter(|&pid| pid != pane_id) {
            if let Some(parent_posture) = self.pane_permission_posture(parent_id) {
                return Some(posture.restricted_by(&parent_posture));
            }
        }
        Some(posture)
    }

    /// Canonical context root that bounds a pane's authority.
    pub(crate) fn pane_context_authority_root(
        &self,
        pane_id: crate::spatial::tiling::PaneId,
    ) -> Option<std::path::PathBuf> {
        let origin = self.origin_for_pane(pane_id)?;
        let raw_root = crate::platform::path::canonical_or_self(&origin.context_root);
        let ws_root = crate::app::registry::resolve_workspace_root(&raw_root)
            .map(|p| crate::platform::path::canonical_or_self(&p));
        // If `raw_root` is deeper than `ws_root` (a subdirectory context inside a
        // parent workspace), the context's own `raw_root` is the narrower authority
        // root. Otherwise use the resolved workspace root when present.
        Some(match ws_root {
            Some(ws) if raw_root.starts_with(&ws) && raw_root != ws => raw_root,
            Some(ws) => ws,
            None => raw_root,
        })
    }

    /// True when `pane_id` has context authority over `target_path`.
    ///
    /// Authority is limited to `pane_id`'s context root. Subdirectory contexts
    /// are isolated units: a pane in context A has no authority over context B,
    /// including when B is a nested context or nested workspace inside A's directory.
    pub(crate) fn pane_may_access_context_root(
        &self,
        pane_id: crate::spatial::tiling::PaneId,
        target_path: &std::path::Path,
    ) -> bool {
        let Some(pane_root) = self.pane_context_authority_root(pane_id) else {
            return false;
        };
        let target_canon = crate::platform::path::canonical_or_self(target_path);
        if !target_canon.starts_with(&pane_root) {
            return false;
        }
        // A person may organize panes into nested contexts without losing
        // access to folders they already controlled. Agent panes still run
        // every narrowing check below, so cd and context creation never widen
        // their delegated reach.
        if self.is_human_owned_pane(pane_id) {
            return true;
        }
        if target_canon != pane_root {
            // Check 1: another context in the router is rooted at a deeper
            // directory under `pane_root` that contains `target_canon`.
            let shadowed_by_nested_context = self.router.iter().any(|ctx| {
                let ctx_root = crate::platform::path::canonical_or_self(&ctx.root);
                ctx_root != pane_root
                    && ctx_root.starts_with(&pane_root)
                    && target_canon.starts_with(&ctx_root)
            });
            if shadowed_by_nested_context {
                return false;
            }
            // Check 2: workspace config resolution on disk resolves `target_canon`
            // to a nested workspace root different from `pane_root`.
            if let Some(ws) = crate::app::registry::resolve_workspace_root(&target_canon) {
                if crate::platform::path::canonical_or_self(&ws) != pane_root {
                    return false;
                }
            }
            // Check 3: agent lookup root or folder secrets scope at a nested path.
            if let Some(agent_ws) =
                crate::app::registry::resolve_agent_lookup_workspace_root(&target_canon)
            {
                if crate::platform::path::canonical_or_self(&agent_ws) != pane_root {
                    return false;
                }
            }
            let nested_folder_secret = crate::workspace::secrets::list_folder_secrets(
                crate::workspace::secrets::folder_store(),
            )
            .iter()
            .any(|meta| {
                let f = crate::platform::path::canonical_or_self(&meta.folder);
                f != pane_root && f.starts_with(&pane_root) && target_canon.starts_with(&f)
            });
            if nested_folder_secret {
                return false;
            }
        }
        true
    }

    /// Resolve a notify-socket peer's pre-captured ancestor chain
    /// (`capture_peer_ancestry`) to the pane that owns it, by matching each
    /// candidate pid against every live terminal pane's shell pid
    /// (`TerminalBackend::child_pid()`). This is the trustworthy sender
    /// identity for CLI commands whose provenance controls scope/ownership
    /// (`Notify`, `DismissNotification`) — the ancestry is host-established
    /// (captured from the socket's kernel credential at accept time, never
    /// from the wire), so a match here cannot be forged by the request body.
    ///
    /// Deliberately does no OS lookups: the ancestry was already walked
    /// eagerly, at accept time, while the peer process was still guaranteed
    /// connected. A short-lived CLI process (the common case — `plexi
    /// notify` does not wait around) can have already exited by the time
    /// this runs on the UI thread after the mailbox drains; walking
    /// `get_pid_ppid` lazily here would then fail on the very first hop and
    /// misattribute every fire-and-forget notification to "unresolvable".
    ///
    /// Known limitation: this does not cross-check process start time, so a
    /// pid reused between the CLI's fork and the ancestry capture could in
    /// principle misattribute — `TerminalBackend` exposes no spawn
    /// timestamp to guard against this, and adding one is out of scope for
    /// this stint.
    ///
    /// Returns `(pane_id, context_id, window_id)` — the same identity shape
    /// `Notify`/`DismissNotification` need, sourced from the owning window's
    /// live `context_id`/`window_id` rather than the caller's claim.
    pub(crate) fn resolve_socket_peer_pane(
        &self,
        peer_ancestry: &[u32],
    ) -> Option<(crate::spatial::tiling::PaneId, u64, u64)> {
        for &candidate in peer_ancestry {
            for window in self.windows.iter() {
                for (pane_id, pane) in window.panes.iter() {
                    let Some(terminal) = pane.as_terminal() else {
                        continue;
                    };
                    if terminal.backend.child_pid() == candidate {
                        return Some((*pane_id, window.context_id, window.window_id));
                    }
                }
            }
        }
        log::info!(
            "pane_ipc: peer identity: ancestry {peer_ancestry:?} resolved to no live pane — treating as outside-pane caller"
        );
        None
    }

    /// Resolve the host-established `ScopeOrigin` for a live pane: the
    /// window's `context_id`, the router's canonical `Context.root` for that
    /// context, the window's `window_id`, `pane_id` itself, and — for an app
    /// pane — its manifest id. Never reads `router.active()` or
    /// `active_window`; every field comes from the pane's owning window,
    /// found via `find_pane_in_any_window`, so a caller in one context can
    /// never be resolved against another context's active state.
    pub(crate) fn origin_for_pane(
        &self,
        pane_id: crate::spatial::tiling::PaneId,
    ) -> Option<crate::host::scope::ScopeOrigin> {
        let (window_index, _tile_id) = self.find_pane_in_any_window(pane_id)?;
        let window = &self.windows[window_index];
        let context_id = window.context_id;
        let context_root = self
            .router
            .position(|c| c.context_id == context_id)
            .map(|idx| self.router.get(idx).root.clone())?;
        let app_id = window
            .panes
            .get(&pane_id)
            .and_then(crate::host::pane::Pane::as_app)
            .map(|app_pane| app_pane.manifest_id.clone());
        Some(crate::host::scope::ScopeOrigin {
            context_id,
            context_root,
            window_id: window.window_id,
            pane_id,
            app_id,
        })
    }

    /// Resolve the host-established origin for a raw event-bus socket request
    /// (`events_subscribe`/`events_declare`/`events_emit`) — stint 0724 Phase
    /// D. Returns `(verified_pane_id, context_id, workspace_root)`, all
    /// `None` together when neither ancestry nor the claimed pane id resolve
    /// to a live pane (an outside-pane connection).
    ///
    /// `peer_ancestry` (host-captured kernel ancestor-pid chain, present only
    /// for the raw CLI socket transport) is tried FIRST and, when it resolves
    /// to a live pane, wins outright — this is the same trust upgrade
    /// `Notify`/`DismissNotification` already get from stint 0636's
    /// `resolve_socket_peer_pane`. `claimed_pane_id` (the client-forwarded
    /// `PLEXI_PANE_ID`) is only a fallback for when ancestry resolution fails
    /// (platform unsupported, or the peer already exited) — never trusted
    /// over a successful ancestry resolution, since any local process can set
    /// that environment variable to name an arbitrary pane.
    pub(crate) fn resolve_event_bus_caller(
        &self,
        claimed_pane_id: Option<u64>,
        peer_ancestry: Option<&[u32]>,
    ) -> (Option<u64>, Option<u64>, Option<std::path::PathBuf>) {
        let verified_pane_id = peer_ancestry
            .and_then(|ancestry| self.resolve_socket_peer_pane(ancestry))
            .map(|(pane_id, _context_id, _window_id)| pane_id);
        let effective_pane_id = verified_pane_id.or(claimed_pane_id);
        let Some(effective_pane_id) = effective_pane_id else {
            return (None, None, None);
        };
        match self.origin_for_pane(effective_pane_id) {
            Some(origin) => (
                Some(effective_pane_id),
                Some(origin.context_id),
                Some(origin.context_root),
            ),
            None => (Some(effective_pane_id), None, None),
        }
    }

    /// Single choke point for every scope-affecting lifecycle event: a
    /// context's root moving, a context or pane being removed, a package
    /// being installed/replaced, or a watched source reporting a new
    /// generation. Phase A (stint 0724) wired the plumbing; Phase B gives it
    /// its first two real consumers — `registries` (the per-root `AppRegistry`
    /// cache) and, for `ContextRootChanged`, every live app pane in the
    /// affected context, not just the ones in the currently-rendered window.
    /// Later phases (connector tools, the event bus, notifications) add
    /// handling here too, so every future consumer reacts to one shared
    /// vocabulary of "what changed" instead of re-deriving it from ad hoc call
    /// sites. Never logs secrets, notification bodies, or event payloads —
    /// `ScopeInvalidation` carries only ids and paths.
    pub(crate) fn emit_scope_invalidation(&mut self, ev: crate::host::scope::ScopeInvalidation) {
        match &ev {
            crate::host::scope::ScopeInvalidation::ContextRootChanged {
                context_id,
                old_root,
                new_root,
            } => {
                log::info!(
                    target: "plexi::scope",
                    "invalidation: context_root_changed context_id={context_id} old_root={} new_root={}",
                    old_root.display(),
                    new_root.display()
                );
            }
            crate::host::scope::ScopeInvalidation::ContextRemoved { context_id } => {
                log::info!(
                    target: "plexi::scope",
                    "invalidation: context_removed context_id={context_id}"
                );
            }
            crate::host::scope::ScopeInvalidation::PaneClosed {
                pane_id,
                context_id,
            } => {
                log::info!(
                    target: "plexi::scope",
                    "invalidation: pane_closed pane_id={pane_id} context_id={context_id}"
                );
            }
            crate::host::scope::ScopeInvalidation::SourceGenerationChanged { source_path } => {
                log::info!(
                    target: "plexi::scope",
                    "invalidation: source_generation_changed source_path={}",
                    source_path.display()
                );
            }
        }
        #[cfg(test)]
        SCOPE_INVALIDATION_COUNT_FOR_TEST.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if let crate::host::scope::ScopeInvalidation::PaneClosed { pane_id, .. } = &ev {
            match crate::host::app_timeline::global().lock() {
                Ok(mut timeline) => timeline.close_lifecycle_pane(*pane_id),
                Err(error) => log::error!("pane_lifecycle: closing pane ownership failed: {error}"),
            }
        }
        self.registries.invalidate(&ev, &self.router);

        // A context's root moved: every live app pane in that context must
        // see the new root immediately, not just whichever pane happens to
        // be in the currently-rendered (active) window. `render::app_pane`
        // already refreshes the active window's app panes every frame
        // (stint 0652), which covers the common case with zero staleness —
        // but a pane in a background/inactive context never gets a render
        // pass at all (see the "not rendering has two independent axes" trap
        // in the repo root AGENTS.md), so without this it would keep
        // persisting state against the OLD root until the user happened to
        // navigate back to it.
        if let crate::host::scope::ScopeInvalidation::ContextRootChanged {
            context_id,
            new_root,
            ..
        } = &ev
        {
            for window in self
                .windows
                .iter_mut()
                .filter(|w| w.context_id == *context_id)
            {
                for pane in window.panes.values_mut() {
                    if let Some(app_pane) = pane.as_app_mut() {
                        app_pane.runtime.refresh_context_root(new_root);
                    }
                }
            }
        }
    }

    /// Keep `registry_watchers` in sync with whatever roots `registries`
    /// currently holds a view for: start a watcher for any newly-cached root
    /// (regardless of which context is active) and drop watchers for roots
    /// `registries` no longer holds (dropped as orphaned by an invalidation).
    /// Called once per `logic` pass — cheap when nothing changed, since an
    /// already-watched root is just a `HashMap` membership check.
    pub(crate) fn reconcile_registry_watchers(&mut self) {
        let live_roots: std::collections::HashSet<std::path::PathBuf> =
            self.registries.roots().into_iter().collect();
        for root in &live_roots {
            if !self.registry_watchers.contains_key(root) {
                if let Some((watcher, rx)) = crate::app::registry_watcher::start(
                    crate::app::registry::registry_watch_dirs(root),
                    std::sync::Arc::clone(&self.ui_wake),
                ) {
                    self.registry_watchers.insert(root.clone(), (watcher, rx));
                }
            }
        }
        self.registry_watchers
            .retain(|root, _| live_roots.contains(root));
    }

    fn complete_wasm_host_effect(
        &mut self,
        pane_id: crate::spatial::tiling::PaneId,
        event: crate::host::wasm_app::InputEvent,
    ) {
        let Some((window_index, _)) = self.find_pane_in_any_window(pane_id) else {
            log::warn!("wasm effect: source pane {pane_id} closed before result delivery");
            return;
        };
        let Some(app) = self.windows[window_index]
            .panes
            .get_mut(&pane_id)
            .and_then(crate::host::pane::Pane::as_app_mut)
        else {
            log::warn!("wasm effect: source pane {pane_id} is no longer an app");
            return;
        };
        if let crate::host::pane::AppRuntime::Wasm(wasm) = &mut app.runtime {
            wasm.complete_host_effect(event);
        } else {
            log::warn!("wasm effect: source pane {pane_id} is no longer a WASM app");
        }
    }

    fn queue_event_to_app_pane_anywhere(
        &mut self,
        pane_id: crate::spatial::tiling::PaneId,
        event: crate::protocol::PlexiEvent,
    ) {
        let Some((window_index, _)) = self.find_pane_in_any_window(pane_id) else {
            log::warn!("app event: target pane {pane_id} closed before delivery");
            crate::host::app_timeline::global()
                .lock()
                .unwrap()
                .clear_subscriber(
                    crate::broker::ActorType::App,
                    &crate::host::event_subscriptions::app_subscriber_id(pane_id),
                );
            return;
        };
        let Some(app) = self.windows[window_index]
            .panes
            .get_mut(&pane_id)
            .and_then(crate::host::pane::Pane::as_app_mut)
        else {
            log::warn!("app event: target pane {pane_id} is no longer an app");
            return;
        };
        app.runtime.queue_outbound_event(event);
    }

    fn deliver_app_event_subscriptions(&mut self) {
        let pane_ids: Vec<u64> = self
            .windows
            .iter()
            .flat_map(|window| {
                window
                    .panes
                    .iter()
                    .filter_map(|(pane_id, pane)| pane.as_app().is_some().then_some(*pane_id))
            })
            .collect();
        let timeline = crate::host::app_timeline::global();
        for pane_id in pane_ids {
            let deliveries = timeline.lock().unwrap().take_deliveries_for(
                crate::broker::ActorType::App,
                &crate::host::event_subscriptions::app_subscriber_id(pane_id),
            );
            for delivery in deliveries {
                log::info!(
                    "app events: deliver subscription={} publisher={} stream={} to pane={pane_id}",
                    delivery.subscription_id,
                    delivery.app_id,
                    delivery.event
                );
                self.queue_event_to_app_pane_anywhere(
                    pane_id,
                    crate::protocol::PlexiEvent::AppEvent {
                        subscription_id: delivery.subscription_id,
                        app_id: delivery.app_id,
                        event: delivery.event,
                        event_id: delivery.event_id,
                        resource_id: delivery.resource_id,
                        trigger_mode: delivery.trigger_mode,
                        summary: delivery.summary,
                        payload: delivery.payload,
                        state_ref: delivery.state_ref,
                        created_at: delivery.created_at,
                    },
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_app_event_subscription(
        &mut self,
        pane_id: u64,
        request_id: String,
        publisher_app_id: String,
        event_names: Vec<String>,
        payload_mode: crate::protocol::PayloadMode,
        trigger_mode: crate::protocol::TriggerMode,
        resource_id: Option<String>,
    ) {
        let Some((subscriber_app_id, workspace_root)) = self.windows.iter().find_map(|window| {
            window.panes.get(&pane_id).and_then(|pane| {
                pane.as_app()
                    .map(|app| (app.manifest_id.clone(), app.workspace_root.clone()))
            })
        }) else {
            log::warn!("app events: subscriber pane {pane_id} is closed");
            return;
        };
        // Host-established context this subscriber pane lives in (stint 0724
        // Phase D) — the viewer side of `evaluate_reach` for every stream
        // this subscription will ever match against. Resolved fresh here,
        // never cached: `origin_for_pane` reads the pane's owning window's
        // live `context_id`, not whichever context was last active.
        let context_id = self
            .origin_for_pane(pane_id)
            .map(|origin| origin.context_id);
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        let request = crate::host::event_subscriptions::HostSubscribeRequest {
            publisher_app_id,
            event_names,
            payload_mode,
            trigger_mode,
            resource_id,
            from_pane_id: Some(pane_id),
            subscriber_override: Some(crate::host::event_subscriptions::app_subscriber_id(pane_id)),
            subscriber_type_override: Some(crate::broker::ActorType::App),
            broker_actor_override: Some(subscriber_app_id),
            workspace_root_override: Some(workspace_root),
            context_id_override: context_id,
            // This request never travels over the raw event-bus socket — the
            // pane id is already host-known (the WASM/Python app dispatch
            // path resolved it, not a client claim), so there is no peer
            // ancestry to verify against.
            peer_ancestry: None,
            cancelled: None,
            reply,
        };
        self.host_subscriptions.reload(&crate::config::config_dir());
        if let Some(consent) = self.host_subscriptions.classify_subscribe_request(request) {
            self.pending_event_consents.push_back(consent);
        }
        self.pending_app_subscription_replies
            .push(PendingAppSubscriptionReply {
                pane_id,
                request_id,
                receiver,
            });
        log::info!("app events: brokered subscription queued for pane={pane_id}");
    }

    fn drain_app_subscription_replies(&mut self) {
        let mut completed = Vec::new();
        let mut deliveries = Vec::new();
        for (index, pending) in self.pending_app_subscription_replies.iter().enumerate() {
            let response = match pending.receiver.try_recv() {
                Ok(crate::host::event_subscriptions::HostSubscribeReply::Ok {
                    subscription_id,
                    ..
                }) => Some(crate::protocol::PlexiEvent::AppEventsSubscribed {
                    request_id: pending.request_id.clone(),
                    subscription_id: Some(subscription_id),
                    error: None,
                }),
                Ok(crate::host::event_subscriptions::HostSubscribeReply::Err { message }) => {
                    Some(crate::protocol::PlexiEvent::AppEventsSubscribed {
                        request_id: pending.request_id.clone(),
                        subscription_id: None,
                        error: Some(message),
                    })
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::warn!(
                        "app events: broker reply disconnected for pane={}",
                        pending.pane_id
                    );
                    completed.push(index);
                    None
                }
            };
            if let Some(response) = response {
                completed.push(index);
                deliveries.push((pending.pane_id, response));
            }
        }
        for index in completed.into_iter().rev() {
            self.pending_app_subscription_replies.swap_remove(index);
        }
        for (pane_id, response) in deliveries {
            self.queue_event_to_app_pane_anywhere(pane_id, response);
        }
    }

    /// Set focused pane in a specific window.
    /// Use this everywhere instead of `self.windows[i].focused_pane = Some(...)` directly
    /// so that the grep pattern `windows[active].focused_pane = ` has zero matches outside tests.
    pub(crate) fn set_window_focused_pane(&mut self, win_idx: usize, tile: egui_tiles::TileId) {
        self.windows[win_idx].navigate_to(tile);
        let window_id = self.windows[win_idx].window_id;
        log::info!("focus: flash set — win={window_id} tile={tile:?}");
        self.click_flash = Some(ClickFlash {
            window_id,
            tile,
            started_at: std::time::Instant::now(),
        });
    }

    /// Restore focused pane in a specific window from a saved `Option<TileId>`.
    /// Use instead of `self.windows[i].focused_pane = saved_opt` so the grep
    /// pattern `windows[active].focused_pane = ` has zero matches outside tests.
    pub(crate) fn restore_window_focused_pane(
        &mut self,
        win_idx: usize,
        saved: Option<egui_tiles::TileId>,
    ) {
        self.windows[win_idx].focused_pane = saved;
    }

    /// Move the empty window `new_for_test` builds off context 1 onto
    /// `context_id`. Harnesses call this before any pane exists so the
    /// process-global tool registry keeps each test in its own namespace.
    #[cfg(test)]
    pub(crate) fn set_initial_context_id_for_test(&mut self, context_id: u64) {
        assert!(
            self.windows.len() == 1 && self.windows[0].panes.is_empty(),
            "set_initial_context_id_for_test runs before panes exist"
        );
        self.windows[0].context_id = context_id;
        self.router.get_mut(0).context_id = context_id;
        log::info!("test harness: isolated tool namespace context_id={context_id}");
    }

    /// Create a `PlexiApp` for headless tests. No workspace restore, no macOS
    /// menu setup, no PTY or audio hardware. Initialises a single empty window
    /// so `state().open_panes` is empty and the harness can add panes via
    /// `inject_app_pane`.
    #[cfg(test)]
    pub fn new_for_test(
        ctx: egui::Context,
        frame_tick: crate::platform::logging::FrameTick,
    ) -> (Self, ui_mailbox::UiMailbox<crate::protocol::AppRequest>) {
        Self::new_for_test_with_config(ctx, frame_tick, config::PlexiConfig::default())
    }

    /// Construct the isolated headless host with an explicit already-loaded
    /// configuration. Tests that exercise config plumbing load it through an
    /// isolated profile before calling this helper.
    #[cfg(test)]
    pub fn new_for_test_with_config(
        ctx: egui::Context,
        frame_tick: crate::platform::logging::FrameTick,
        config: config::PlexiConfig,
    ) -> (Self, ui_mailbox::UiMailbox<crate::protocol::AppRequest>) {
        let key_bindings = crate::host::keys::build_key_bindings_with_user_keys(
            config.keybindings.as_ref(),
            config.keys.as_ref(),
        );
        let theme_cfg = Self::resolve_theme_config(&config);
        let colors = Colors::from_config(&theme_cfg);
        configure_egui_ctx(&ctx, &colors);
        let ui_wake: std::sync::Arc<dyn ui_mailbox::UiWake> =
            std::sync::Arc::new(ui_mailbox::EguiWake::new(ctx.clone()));
        let (tx, rx) = mpsc::channel();
        let (hr_watcher, hr_rx) =
            crate::host::hot_reload::HotReloadWatcher::new(std::sync::Arc::clone(&ui_wake));
        let (sw_watcher, sw_rx) =
            crate::host::state_watch::StateWatcher::new(std::sync::Arc::clone(&ui_wake));
        let scratch = tempfile::Builder::new()
            .prefix(&format!("plexi-test-host-{}-", std::process::id()))
            .tempdir()
            .expect("create owned test scratch");
        let path = scratch.path().join("workspace");
        let permission_store_dir = scratch.path().join("permissions");
        std::fs::create_dir_all(&path).expect("create isolated test workspace");
        std::fs::create_dir_all(&permission_store_dir).expect("create test permission store");
        // Standing ruling: a context root must gitignore its app_states dir.
        if path.is_dir() {
            if let Err(error) = crate::workspace::secrets::ensure_app_state_gitignore(&path) {
                log::warn!(
                    "could not ensure {}/.plexi/.gitignore covers app_states/: {error}",
                    path.display()
                );
            }
        }
        let (pane_ipc_tx, pane_ipc_rx) =
            ui_mailbox::UiMailbox::<crate::protocol::AppRequest>::channel(
                std::sync::Arc::clone(&ui_wake),
                "pane_ipc",
            );
        let (_event_subscribe_tx, event_subscribe_rx) = ui_mailbox::UiMailbox::<
            crate::host::event_subscriptions::HostSubscribeRequest,
        >::channel(
            std::sync::Arc::clone(&ui_wake),
            "events_subscribe",
        );
        let (_event_publish_tx, event_publish_rx) =
            ui_mailbox::UiMailbox::<crate::host::event_subscriptions::HostPublishRequest>::channel(
                std::sync::Arc::clone(&ui_wake),
                "events_publish",
            );
        let host_subscriptions = crate::host::event_subscriptions::HostSubscriptionService::new(
            &crate::config::config_dir(),
            crate::host::app_timeline::global(),
        );
        let mut app = Self {
            pty_event_rx: rx,
            pty_event_tx: tx,
            last_notify_poll: std::time::Instant::now(),
            scheduler: crate::host::scheduler::Scheduler::new(),
            theme: theme::terminal_theme(&theme_cfg),
            colors,
            default_font_size: config.font_size.unwrap_or(theme::FONT_SIZE),
            ui_scale: config.ui_scale.map_or(ui_scale::UI_SCALE_DEFAULT, ui_scale::clamp_scale),
            ui_scale_persist_at: None,
            ctx,
            router: crate::workspace::router::WorkspaceRouter::new(
                vec![crate::host::context::Context {
                    name: "Test".into(),
                    root: path.clone(),
                    description: None,
                    context_id: 1,
                    parent_id: None,
                    depth: 0,
                    parked: false,
                }],
                0,
            ),
            windows: vec![Window {
                name: "Test".into(),
                path: path.clone(),
                tree: egui_tiles::Tree::empty("test_tree"),
                panes: HashMap::new(),
                focused_pane: None,
                zoomed_pane: None,
                grid_x: 0,
                grid_y: 0,
                window_id: 1,
                context_id: 1,
            }],
            active_window: 0,
            sidebar_visible: false,
            show_shortcuts: false,
            show_changelog: false,
            show_ui_gallery: false,
            ui_gallery_normal_buf: "workspace/search".to_string(),
            ui_gallery_focused_buf: "Focused field".to_string(),
            ui_gallery_show_text_modal: false,
            ui_gallery_modal_buf: String::new(),
            quitting: false,
            quit_press_count: 0,
            quit_last_press: None,
            welcome_delete_press_count: 0,
            welcome_delete_last_press: None,
            config,
            binding_table: crate::host::keys::build_binding_table(&key_bindings),
            claimed_host_key_releases: HashSet::new(),
            key_bindings,
            pending_close: false,
            pending_quit: false,
            pending_context_close: None,
            frame_tick,
            ui_phase: crate::platform::logging::new_ui_phase_tracker(),
            frame_diag_window: None,
            prev_screen_rect: None,
            drag_window_last_seen: None,
            pending_screenshots: Vec::new(),
            approval_buttons: Vec::new(),
            synthetic_input_frame: false,
            pending_slot_waits: Vec::new(),
            pending_agent_boots: Vec::new(),
            pending_submits: Vec::new(),
            pending_send_approvals: Vec::new(),
            pane_heartbeats: HashMap::new(),
            pane_worktrees: HashMap::new(),
            last_worktree_close_reports: HashMap::new(),
            pane_mailboxes: HashMap::new(),
            pending_message_recvs: Vec::new(),
            next_pane_message_id: 1,
            closed_pane_slots: HashMap::new(),
            agent_attention: Default::default(),
            fleet_history: std::cell::RefCell::new(Vec::new()),
            pane_results: HashMap::new(),
            pending_result_waits: Vec::new(),
            pending_agent_waits: Vec::new(),
            child_result_events: HashMap::new(),
            child_parent_panes: HashMap::new(),
            pulled_child_results: std::collections::HashSet::new(),
            delivered_child_batches: HashMap::new(),
            agent_tool_waits: Vec::new(),
            agent_tool_children: std::collections::HashMap::new(),
            conjoined_contexts: std::collections::HashMap::new(),
            conjoined_parent_focus: None,
            conjoined_parent_rect: None,
            team_runs: Vec::new(),
            last_sent_window_title: None,
            permission_store_dir,
            _test_scratch: Some(scratch),
            renaming_window: None,
            rename_buffer: String::new(),
            editing_description: None,
            description_buffer: String::new(),
            drag_context: None,
            parked_section_expanded: false,
            show_command_palette: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_selected_key: None,
            palette_workspace_root: None,
            palette_notes: Vec::new(),
            palette_commands: Vec::new(),
            palette_scroll_reset: false,
            palette_agent_count_logged: None,
            context_visit_history: Vec::new(),
            renaming_pane: None,
            text_overlay: None,
            text_overlay_browse_rx: None,
            registries: {
                let mut views = crate::app::registry_views::RegistryViews::new_with_global(
                    &path.join("nonexistent-apps-dir"),
                );
                views.view_for_root(&path);
                views
            },
            pending_notifications: Vec::new(),
            current_notify_id: None,
            modal_focused_option: 0,
            modal_input_buffer: String::new(),
            notification_input_drafts: HashMap::new(),
            quick_note_text: String::new(),
            quick_note_attachments: Vec::new(),
            quick_note_ctx: QuickNoteCtx::default(),
            notes_migration_pending: true,
            notes_picker_entries: Vec::new(),
            notes_picker_selected: 0,
            notes_picker_last_drawn_selected: None,
            notes_picker_query: String::new(),
            notes_picker_preview: None,
            notes_picker_pending_delete: None,
            notes_picker_status: None,
            notes_picker_last_copied: None,
            notes_picker_copy_unsent: false,
            notes_index: crate::notes::NotesIndexCache::default(),
            notes_index_rx: None,
            notes_index_inflight_anchor: None,
            notes_index_rerun: false,
            notes_picker_shown_paths: Vec::new(),
            notes_index_loaded: false,
            modal_state_notify_id: String::new(),
            notification_images: HashMap::new(),
            notifications_enabled: false,
            notifications_focus_mode: false,
            notifications_sound: None,
            notification_cue_playback: None,
            focus_stack: Vec::new(),
            focus_history_depth: 100,
            pane_focus_history: Vec::new(),
            pane_focus_future: Vec::new(),
            navigating_history: false,
            host: crate::host::model::HostModel::new(),
            host_services: crate::host::services::HostServices::new(),
            background_apps: HashMap::new(),
            directed_pipes: HashMap::new(),
            hot_reload: hr_watcher,
            hot_reload_rx: hr_rx,
            state_watch: sw_watcher,
            state_watch_rx: sw_rx,
            _config_watcher: None,
            config_reload_rx: None,
            registry_watchers: HashMap::new(),
            workspace_dirty: false,
            last_workspace_save: std::time::Instant::now(),
            minimap: crate::render::minimap::MinimapState::new(),
            last_page_x_per_row: HashMap::new(),
            context_active_window: HashMap::new(),
            minimap_visible_per_context: HashMap::new(),
            next_window_id: 2,
            pane_snapshot_len: 0,
            pane_anims: Vec::new(),
            edge_pulse: None,
            click_flash: None,
            ui_wake,
            update_rx: None,
            last_update_check: std::time::Instant::now(),
            display_version: read_display_version(),
            update_available: None,
            update_quit_pending: false,
            update_confirm_prompt: false,
            shutdown_requested: false,
            pane_ipc_rx,
            host_subscriptions,
            resource_leases: None,
            event_subscribe_rx,
            event_publish_rx,
            pending_event_consents: std::collections::VecDeque::new(),
            pending_app_subscription_replies: Vec::new(),
            pending_raw_wasm_launches: std::collections::VecDeque::new(),
            last_logged_focus: None,
            focus_started_at: None,
            terminal_traversal_lock: None,
            focus_journal_path: crate::config::config_dir().join("focus-journal.jsonl"),
            overlay_held_cmds: Vec::new(),
            agent_host: crate::agent::AgentHost::inert(),
            pending_pane_clicks: HashMap::new(),
            pending_pane_inputs: HashMap::new(),
            pending_pane_drags: HashMap::new(),
            pending_pane_pointer_frames: HashMap::new(),
            zoom: zoom::ZoomState::default(),
            attention_queue: attention_queue::AttentionQueueState::default(),
            approval_keyboard: Default::default(),
        };
        // Launch paths allocate through HostModel, which otherwise starts at 1
        // in every test process. Seed a private block so a pane drop in one
        // test cannot unregister another test's tools.
        app.host
            .seed_next_pane_id(crate::testing::reserve_pane_id_block());
        log::info!("test host: pane ids start at {}", app.host.next_pane_id());
        (app, pane_ipc_tx)
    }

    /// Add a minimal builtin pane directly to window 0 for unit tests.
    /// Returns `(tile_id, pane_id)` — `tile_id` is suitable for `focused_pane` assignments.
    #[cfg(test)]
    pub(crate) fn add_test_pane(&mut self) -> (egui_tiles::TileId, u64) {
        let pane_id = self.host.alloc_pane_id();
        let cwd = std::path::PathBuf::from("/tmp");
        let pane = crate::host::pane::Pane::App(Box::new(crate::host::pane::AppPane {
            pip_status: None,
            id: pane_id,
            permissions: crate::app::permissions::AppPermissions::builtin(),
            runtime: crate::host::pane::AppRuntime::Builtin(Box::new(
                crate::file_browser::FileBrowserApp::new(cwd.clone()),
            )),
            workspace_root: cwd,
            manifest_id: "test".to_string(),
            name: "test".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        }));
        self.windows[0].panes.insert(pane_id, pane);
        let tile_id = self.windows[0].tree.tiles.insert_pane(pane_id);
        if self.windows[0].tree.root().is_none() {
            self.windows[0].tree.root = Some(tile_id);
        }
        (tile_id, pane_id)
    }

    /// Inject an app pane running `type_id` into window `win_idx` (test-only).
    /// Backs the `[launch] on_launch` dedup tests (#0336), which need instances
    /// with a chosen `type_id` planted in specific windows/contexts.
    #[cfg(test)]
    pub(crate) fn add_app_pane_in_window(
        &mut self,
        win_idx: usize,
        type_id: &str,
    ) -> (egui_tiles::TileId, u64) {
        self.add_app_pane_in_window_with_runtime_id(win_idx, type_id, type_id)
    }

    /// Like [`Self::add_app_pane_in_window`] but with the pane's runtime
    /// `type_id` set independently of its `manifest_id`. Simulates a WASM pane,
    /// whose runtime reports `type_id() == "wasm"` while its manifest identity
    /// is the app id — the case the `on_launch` resolver must match by
    /// `manifest_id`, not `type_id` (#0336).
    #[cfg(test)]
    pub(crate) fn add_app_pane_in_window_with_runtime_id(
        &mut self,
        win_idx: usize,
        manifest_id: &str,
        _runtime_type_id: &str,
    ) -> (egui_tiles::TileId, u64) {
        let pane_id = self.host.alloc_pane_id();
        let cwd = std::path::PathBuf::from("/tmp");
        let pane = crate::host::pane::Pane::App(Box::new(crate::host::pane::AppPane {
            pip_status: None,
            id: pane_id,
            permissions: crate::app::permissions::AppPermissions::builtin(),
            runtime: crate::host::pane::AppRuntime::Builtin(Box::new(
                crate::file_browser::FileBrowserApp::new(cwd.clone()),
            )),
            workspace_root: cwd,
            manifest_id: manifest_id.to_string(),
            name: manifest_id.to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        }));
        self.windows[win_idx].panes.insert(pane_id, pane);
        let tile_id = self.windows[win_idx].tree.tiles.insert_pane(pane_id);
        if self.windows[win_idx].tree.root().is_none() {
            self.windows[win_idx].tree.root = Some(tile_id);
        }
        (tile_id, pane_id)
    }

    fn resolve_theme_config(config: &config::PlexiConfig) -> config::ThemeConfig {
        let user_theme = config.theme.clone().unwrap_or_default();
        // Prefer [theme] preset; fall back to legacy top-level theme_preset so
        // existing configs survive the migration without silently losing their theme.
        let preset_name = user_theme
            .preset
            .as_deref()
            .or(config.theme_preset.as_deref());
        if let Some(preset_name) = preset_name {
            if let Some(preset) = theme::preset_colors(preset_name) {
                log::info!("Applying theme preset: {}", preset_name.trim());
                return theme::apply_preset(&preset, &user_theme);
            }
            log::warn!("Unknown theme preset: {preset_name}");
        }
        user_theme
    }

    pub(crate) fn make_backend_settings(
        pane_id: u64,
        working_directory: Option<PathBuf>,
        colors: &Colors,
        context: &crate::pane_ops::create::PaneContextEnv,
        agent_pane: bool,
    ) -> (BackendSettings, host_mcp::PendingPaneCredential) {
        Self::make_backend_settings_in_profile(
            &crate::config::config_dir(),
            pane_id,
            working_directory,
            colors,
            context,
            agent_pane,
        )
    }

    pub(crate) fn make_backend_settings_in_profile(
        profile_dir: &std::path::Path,
        pane_id: u64,
        working_directory: Option<PathBuf>,
        colors: &Colors,
        context: &crate::pane_ops::create::PaneContextEnv,
        agent_pane: bool,
    ) -> (BackendSettings, host_mcp::PendingPaneCredential) {
        let crate::pane_ops::create::PaneContextEnv {
            context_id,
            name: context_name,
            description: context_description,
            root: context_root,
            depth: context_depth,
        } = context;
        let context_id = *context_id;
        let context_depth = *context_depth;
        log::info!(
            "make_backend_settings: pane_id={pane_id} context_id={context_id} \
             context_name={context_name:?} context_root={context_root:?} context_depth={context_depth} \
             agent_pane={agent_pane}"
        );
        let mut env = shell::build_env_with_secrets(working_directory.as_deref(), !agent_pane);
        env.remove("PLEXI_PARENT_PANE");
        env.remove("PLEXI_EPHEMERAL");
        env.remove("PLEXI_AGENT_HARNESS");
        env.remove("PLEXI_AGENT_ROLE");
        env.remove("PLEXI_AGENT_ID");
        env.remove("PLEXI_AGENT_SPAWNED");
        env.insert("PLEXI_PANE_ID".into(), pane_id.to_string());
        env.insert("PLEXI_SOCKET".into(), crate::platform::ipc::endpoint());
        // Host MCP discovery. The bearer is registered against this trusted
        // pane/context identity; requests cannot supply their own workspace.
        let workspace_root = context_root
            .clone()
            .or_else(|| working_directory.clone())
            .unwrap_or_default();
        let call_credential = crate::broker::gate::PermissionMonitor::for_profile(profile_dir)
            .issue_credential(
                Some(pane_id),
                context_id,
                &workspace_root,
                &format!("pane:{pane_id}"),
            );
        env.insert("PLEXI_CALL_CREDENTIAL".into(), call_credential);
        if let Some((port, token)) =
            host_mcp::discovery_for_pane(pane_id, context_id, workspace_root)
        {
            env.insert("PLEXI_HOST_MCP_PORT".into(), port.to_string());
            env.insert("PLEXI_HOST_MCP_TOKEN".into(), token);
        }
        env.insert("PLEXI_CONTEXT_ID".into(), context_id.to_string());
        env.insert("PLEXI_CONTEXT_NAME".into(), context_name.to_string());
        env.insert(
            "PLEXI_CONTEXT_DESCRIPTION".into(),
            context_description.to_string(),
        );
        if let Some(root) = context_root {
            env.insert(
                "PLEXI_CONTEXT_ROOT".into(),
                root.to_string_lossy().into_owned(),
            );
        }
        env.insert("PLEXI_CONTEXT_DEPTH".into(), context_depth.to_string());
        if agent_pane {
            env.insert("PLEXI_AGENT_SPAWNED".into(), "1".into());
            shell::withhold_terminal_secrets(&mut env, working_directory.as_deref());
        }
        crate::broker::seal::scrub_pane_env(&mut env, agent_pane);
        let shell_path = shell::detect_shell();
        let args = shell::default_shell_args(&shell_path, agent_pane);
        (
            BackendSettings {
                shell: shell_path,
                args,
                env,
                dynamic_colors: theme::terminal_dynamic_colors(colors),
                working_directory,
                // `TerminalPane::new` installs the program-status tap.
                byte_tap: None,
            },
            host_mcp::PendingPaneCredential::new(pane_id),
        )
    }

    /// Single entry point for Cmd+R. Opens the pane rename modal for the
    /// focused pane — except portals, which have no name of their own:
    /// there the rename falls through to the portal's target subcontext,
    /// same as Cmd+Shift+R from inside it.
    pub(crate) fn open_rename_for_focused(&mut self) {
        let active_ctx = &self.windows[self.active_window];
        let Some(focused_tile) = active_ctx.focused_pane else {
            return;
        };
        let Some(Tile::Pane(pane_id)) = active_ctx.tree.tiles.get(focused_tile) else {
            return;
        };
        let pane_id = *pane_id;
        let portal_target = active_ctx
            .panes
            .get(&pane_id)
            .and_then(|p| p.portal_target());
        if let Some(target) = portal_target {
            if let Some(ctx_idx) = self.router.position(|c| c.context_id == target) {
                self.open_context_rename(ctx_idx);
            } else {
                log::warn!("rename_pane: portal {pane_id} targets unknown context {target}");
            }
            return;
        }
        self.rename_buffer = active_ctx
            .panes
            .get(&pane_id)
            .map(|p| match p {
                crate::host::pane::Pane::Terminal(t) => t.name.clone().unwrap_or_default(),
                // Note editors seed the frontmatter title so renaming edits
                // the title, not the filename.
                crate::host::pane::Pane::App(a) => {
                    a.runtime.rename_seed().unwrap_or_else(|| a.name.clone())
                }
                crate::host::pane::Pane::Portal(_) => String::new(),
            })
            .unwrap_or_default();
        self.renaming_pane = Some(pane_id);
        // Sync the focus layer immediately so `input_captured_by_overlay()`
        // is accurate for the rest of this frame — without this, there is a
        // one-frame window where `renaming_pane` is Some but the focus layer
        // has not been pushed yet.
        self.sync_rename_pane_focus();
        log::info!("rename_pane: opened for pane {pane_id:?}");
    }

    /// Single entry point for opening a context rename. Used by Cmd+Shift+R
    /// (active context) and by Cmd+R falling through from a focused portal
    /// pane (the portal's target subcontext). Renders inline in the sidebar
    /// when visible, otherwise as the centered overlay.
    pub(crate) fn open_context_rename(&mut self, ctx_idx: usize) {
        self.rename_buffer = self.router.get(ctx_idx).name.to_string();
        self.renaming_window = Some(ctx_idx);
        log::info!(
            "context_rename: opened for context {:?} (idx {ctx_idx})",
            self.router.get(ctx_idx).name
        );
    }

    pub(crate) fn context_name_for(&self, context_id: u64) -> String {
        self.router
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.name.to_string())
            .unwrap_or_default()
    }

    pub(crate) fn context_description_for(&self, context_id: u64) -> String {
        self.router
            .iter()
            .find(|c| c.context_id == context_id)
            .and_then(|c| c.description.clone())
            .unwrap_or_default()
    }

    pub(crate) fn context_root_for(&self, context_id: u64) -> Option<PathBuf> {
        self.router
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.root.clone())
    }

    pub(crate) fn context_depth_for(&self, context_id: u64) -> u32 {
        self.router
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.depth)
            .unwrap_or(0)
    }
}

/// Returns true for app commands that must NOT execute while a modal overlay
/// owns keyboard input. Safe commands (ShowNotification, pipes, queries) always
/// dispatch immediately. Unsafe commands (layout/terminal mutations, focus changes)
/// are held in `overlay_held_cmds` and released on the next modal-free frame.
fn is_overlay_unsafe_cmd(cmd: &crate::app::app_trait::AppCommand) -> bool {
    use crate::app::app_trait::AppCommand;
    match cmd {
        AppCommand::WasmHostEffect { effect, .. } => {
            matches!(effect, crate::host::wasm_pane::WasmHostEffect::Spawn { .. })
        }
        AppCommand::SpawnApp { .. }
        | AppCommand::SpawnPane { .. }
        | AppCommand::RequestLinkedTerminal { .. }
        | AppCommand::RunInLinkedTerminal { .. }
        | AppCommand::InsertPathToken { .. }
        | AppCommand::OpenArtifact { .. } => true,
        AppCommand::AssistantHostTool { name, .. } => !matches!(
            name.as_str(),
            "host.panes.list" | "host.panes.state" | "host.introspect"
        ),
        AppCommand::DeliverNotifyAction { host_action, .. } => host_action
            .as_deref()
            .map(|a| a.starts_with("pane_focus:"))
            .unwrap_or(false),
        _ => false,
    }
}

fn overlay_unsafe_cmd_name(cmd: &crate::app::app_trait::AppCommand) -> &'static str {
    use crate::app::app_trait::AppCommand;
    match cmd {
        AppCommand::WasmHostEffect { .. } => "WasmHostEffect",
        AppCommand::SpawnApp { .. } => "SpawnApp",
        AppCommand::SpawnPane { .. } => "SpawnPane",
        AppCommand::RequestLinkedTerminal { .. } => "RequestLinkedTerminal",
        AppCommand::RunInLinkedTerminal { .. } => "RunInLinkedTerminal",
        AppCommand::InsertPathToken { .. } => "InsertPathToken",
        AppCommand::OpenArtifact { .. } => "OpenArtifact",
        AppCommand::AssistantHostTool { .. } => "AssistantHostTool",
        AppCommand::DeliverNotifyAction { .. } => "DeliverNotifyAction(pane_focus)",
        _ => "unknown",
    }
}

impl PlexiApp {
    /// Open the assistant sheet for a phone that redeemed a pairing code.
    /// Returns true when a sheet was newly opened and the window should paint.
    fn present_relay_pairing_sheet(&mut self) -> bool {
        let Some((code, fingerprint)) = crate::cli::relay::pairing_prompt() else {
            for window in &mut self.windows {
                for pane in window.panes.values_mut() {
                    if let Some(app) = pane.as_app_mut() {
                        app.runtime.clear_relay_pairing_sheet();
                    }
                }
            }
            return false;
        };
        let mut opened = false;
        for window in &mut self.windows {
            let visible: Vec<u64> = window
                .panes
                .iter()
                .filter(|(id, pane)| {
                    !pane.is_hidden()
                        && window.tree.tiles.iter().any(
                            |(_, tile)| matches!(tile, egui_tiles::Tile::Pane(pid) if pid == *id),
                        )
                })
                .map(|(id, _)| *id)
                .collect();
            let targets: Vec<u64> = if visible.is_empty() {
                window.panes.keys().copied().collect()
            } else {
                visible
            };
            for id in targets {
                let Some(pane) = window.panes.get_mut(&id) else {
                    continue;
                };
                let Some(app) = pane.as_app_mut() else {
                    continue;
                };
                if app.runtime.offer_relay_pairing(&code, &fingerprint) {
                    log::info!("relay: pairing sheet offered pane_id={id}");
                    opened = true;
                    break;
                }
            }
            if opened {
                break;
            }
        }
        opened
    }
}

impl eframe::App for PlexiApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.synthetic_input_frame = false;
        self.fulfill_screenshot_events(ctx, raw_input);
        // A hidden window (minimized/occluded) gets logic-only passes with no
        // widget pass to receive events — leave queued pane inputs in place so
        // the next visible frame delivers them instead of silently eating them.
        if raw_input.viewport().visible() == Some(false) {
            return;
        }
        let focused_pane = self.windows.get(self.active_window).and_then(|window| {
            window
                .focused_pane
                .and_then(|tile_id| window.tree.tiles.get(tile_id))
                .and_then(|tile| match tile {
                    egui_tiles::Tile::Pane(pane_id) => Some(*pane_id),
                    egui_tiles::Tile::Container(_) => None,
                })
        });
        if let Some(pane_id) = focused_pane {
            let defer_text_until_focused =
                self.pending_pane_inputs
                    .get(&pane_id)
                    .is_some_and(|batches| {
                        batches.iter().any(|batch| {
                            batch
                                .events
                                .iter()
                                .any(|event| matches!(event, egui::Event::Text(_)))
                        })
                    })
                    && !crate::app::input_owner::pane_receives_ipc_text(ctx, pane_id);
            if defer_text_until_focused {
                // A pane can be created while eframe runs hidden logic-only
                // passes. Its first visible pass has not rendered a text
                // surface yet, so feeding queued text into that pass loses it
                // before the post-frame reconciler grants default focus.
                // Preserve the ordered batch until that focus exists.
                log::info!(
                    "pane_ipc: deferring text input for pane_id={pane_id} until its text surface receives focus"
                );
            } else if let Some(batches) = self.pending_pane_inputs.remove(&pane_id) {
                if !batches.is_empty() {
                    self.synthetic_input_frame = true;
                }
                for batch in batches {
                    raw_input.modifiers = batch.modifiers;
                    raw_input.events.extend(batch.events);
                }
            }
            // Drag pacing (stint 0510): exactly ONE pointer batch per visible
            // frame, so egui sees press → move… → release as a genuine
            // multi-frame trajectory instead of a same-frame click.
            if let Some(frames) = self.pending_pane_pointer_frames.get_mut(&pane_id) {
                if let Some(batch) = frames.pop_front() {
                    self.synthetic_input_frame = true;
                    raw_input.events.extend(batch.events);
                }
                if frames.is_empty() {
                    self.pending_pane_pointer_frames.remove(&pane_id);
                } else {
                    // Keep frames coming until the schedule drains.
                    ctx.request_repaint();
                }
            }
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::Logic,
        );
        // Everything that services external clients — pane IPC, spawn queue,
        // PTY events, shutdown, event-bus subscriptions, agent turns — must
        // run here, not in `ui`. eframe skips `ui` entirely while the window
        // is hidden (minimized or fully occluded by other windows) but still
        // runs `logic` passes for every repaint request. With these drains in
        // `ui`, a fully covered host serviced no pane IPC at all: wakes fired,
        // logic-only passes ran, and queued commands sat until the window was
        // next uncovered (stint 0505 fix round 3).
        self.update_preamble(ctx);
        self.apply_ui_scale(ctx);
        self.flush_ui_scale_persist();
        if self.present_relay_pairing_sheet() {
            ctx.request_repaint();
        }

        // File watching/reload is core app servicing, including for an
        // occluded host. Keep it independent of change-set preview.
        if self.service_pane_external_io() {
            ctx.request_repaint();
        }

        // Change-set preview is host state the editor and `plexi changes`
        // both read. It has to run off the paint path so an occluded window
        // still publishes open buffers and a pending diff.
        if self.sync_editor_change_sets() {
            ctx.request_repaint();
        }

        // Hot reload (#83): drain any pending file-watcher reload requests.
        // Each `ReloadRequest` causes the matching pane's WASM runtime to be
        // dropped (sending Shutdown + reaping the child) and replaced with
        // a fresh subprocess. Idempotent if the pane was closed since. Lives
        // in `logic`, never `ui` — this drained per frame from `ui` until
        // stint 0759, so a watcher save under a fully occluded host sat
        // unserviced (`ui_mailbox: source=hot_reload has N unserviced
        // message(s)`) until the window was next uncovered.
        self.drain_hot_reload_requests();

        // App state files (stint 0644): drain external-change notices and
        // re-read the affected scope on the matching pane. Lives in `logic`,
        // never `ui` — external CLI/agent edits must land even when the host
        // window is hidden or occluded.
        self.drain_state_change_requests();

        // Crash-resilient dev reload (#1055): auto-restart watched panes that
        // have crashed, after a 2s delay so the developer can read the traceback.
        self.drain_crash_restarts();

        // Reload configuration from disk when the user clicks
        // "Reload Configuration" in the app menu. macOS-only: the hook is an
        // NSMenu item. Elsewhere the command palette and the config file
        // watcher below cover the same ground.
        #[cfg(target_os = "macos")]
        {
            crate::platform::macos_menu::apply_version_title_once();
            if crate::platform::macos_menu::take_reload_config_flag() {
                self.reload_config();
            }
            if crate::platform::macos_menu::take_open_config_flag() {
                let path = crate::config::ensure_config_file();
                self.open_config_in_editor(path);
            }
        }

        // Config hot-reload (#1115): drain filesystem watcher signals.
        let config_changed = self.config_reload_rx.as_ref().is_some_and(|rx| {
            let hit = rx.try_recv().is_ok();
            if hit {
                while rx.try_recv().is_ok() {}
            }
            hit
        });
        if config_changed {
            self.reload_config();
        }

        // App registry hot-reload (#1712, re-keyed per-root in stint 0724
        // Phase B): drain filesystem watcher signals for EVERY live root, not
        // just the active context's — an inactive context's registry must
        // react to on-disk changes exactly like the active one's.
        let changed_roots: Vec<std::path::PathBuf> = self
            .registry_watchers
            .iter()
            .filter_map(|(root, (_watcher, rx))| {
                let hit = rx.try_recv().is_ok();
                if hit {
                    while rx.try_recv().is_ok() {}
                }
                hit.then(|| root.clone())
            })
            .collect();
        for root in changed_roots {
            self.emit_scope_invalidation(
                crate::host::scope::ScopeInvalidation::SourceGenerationChanged {
                    source_path: root,
                },
            );
        }

        // Registries are a synchronous per-root cache (stint 0724 Phase B) —
        // resolving or rescanning a view happens at the point of use, so
        // there is no background-load result to drain here anymore. Keep the
        // watcher set in sync with whatever roots are now cached.
        self.reconcile_registry_watchers();

        // One-shot notes migration off the pre-tier store. Here rather than in
        // `PlexiApp::new` because it needs the restored router to resolve the old
        // one-way-hashed directory names, and in `logic` rather than `ui` so an
        // occluded host still runs it (see the drain rules above).
        if self.notes_migration_pending {
            self.notes_migration_pending = false;
            let report = crate::notes::migrate_notes_storage(self.router.as_slice());
            if report.moved > 0 || report.failed > 0 || !report.left_behind.is_empty() {
                log::info!("notes_migration: {report:?}");
            }
        }

        // Notes picker index: load the persisted snapshot once, then apply any
        // background refresh. Both are filesystem work that must not live in
        // `ui` — an occluded host still has to finish a refresh it already
        // started, and the open path only reads the in-memory cache.
        self.ensure_notes_index_loaded();
        self.drain_notes_index_refresh();

        // Expired needs-you items auto-deny even while the window is hidden.
        // A queue file left by the previous process is opened once. A frame
        // with no file does not create a profile store.
        let profile = crate::config::config_dir();
        if crate::broker::gate::PermissionMonitor::loaded(&profile).is_none()
            && crate::broker::gate::PermissionMonitor::has_persisted_queue(&profile)
        {
            let monitor = crate::broker::gate::PermissionMonitor::for_profile(&profile);
            log::info!(
                "needs_you: host opened restored queue count={}",
                monitor.open_needs_you().len()
            );
        }
        if let Some(monitor) = crate::broker::gate::PermissionMonitor::loaded(&profile) {
            monitor.expire_needs_you();
        }

        // Screenshot capture is an external-client request path, not UI: the
        // viewport request, the readback poll, and the bounded-deadline reply
        // all live here so an occluded or minimized host — which never enters
        // `ui` at all — still captures, and still answers (stints 0495, 0504).
        // It follows `update_preamble` so a request arriving on this pass is
        // issued on this pass, not one frame later.
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::Screenshots,
        );
        crate::platform::logging::time_drain("service_pending_screenshots", || {
            self.service_pending_screenshots(ctx, frame.wgpu_render_state())
        });

        // Same reasoning for parked `pane slot wait` requests: an occluded
        // host must still expire them on schedule, so the caller sees the
        // host's typed timeout instead of its own (stint 0585).
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::SlotWaits,
        );
        crate::platform::logging::time_drain("service_pending_slot_waits", || {
            self.service_pending_slot_waits(ctx);
            self.service_pending_message_recvs(ctx);
            self.service_pending_result_waits(ctx);
            self.service_pending_agent_waits(ctx);
            self.service_agent_tool_waits(ctx);
            self.service_team_runs(ctx);
        });

        // And for `pane new --agent`, whose reply is owed either an idle
        // report, an expired deadline, or a pane that went away (stint 0584).
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::AgentBoots,
        );
        crate::platform::logging::time_drain("service_pending_agent_boots", || {
            self.service_pending_agent_boots(ctx)
        });

        // And for `pane send --submit`, which drives its settle → Enter →
        // confirm sequence from here: an occluded host must still press the
        // Enter it promised and still read the grid to confirm it (stint 0583).
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::Submits,
        );
        crate::platform::logging::time_drain("service_pending_submits", || {
            self.service_pending_submits(ctx);
            self.service_pending_send_approvals(ctx);
        });
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::PaneHeartbeats,
        );
        crate::platform::logging::time_drain("service_pane_heartbeats", || {
            self.service_pane_heartbeats(ctx)
        });

        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::SubscriptionReplies,
        );
        crate::platform::logging::time_drain("drain_app_subscription_replies", || {
            self.drain_app_subscription_replies()
        });
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::EventDeliveries,
        );
        crate::platform::logging::time_drain("deliver_app_event_subscriptions", || {
            self.deliver_app_event_subscriptions()
        });

        // WASM Python panes are fed by external clients too — MCP server
        // processes, HTTP workers — and their guest runtimes run on their own
        // threads regardless of paint. Service their I/O here so an occluded
        // host still moves server data into the guest and guest replies out
        // (stint 0382 fix round; guard:
        // `mcp_reply_reaches_guest_while_window_hidden`).
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::PythonRuntimes,
        );
        crate::platform::logging::time_drain("service_python_pane_runtimes", || {
            self.service_python_pane_runtimes(ctx)
        });

        // Host agent runtime (Phase C): consume queued event deliveries and
        // finished agent turns. Cheap no-op when nothing is pending. While a
        // turn runs on a worker thread, keep frames coming so its outcome is
        // collected promptly even when the UI is otherwise idle.
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::AgentTick,
        );
        crate::platform::logging::time_drain("agent_host.tick", || self.agent_host.tick());
        if self.agent_host.turns_in_flight() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // App panes are external clients too: they answer assistant tool calls
        // and raise notifications on their own schedule, with no dependence on
        // being painted. This drain lived in `ui` until stint 0688, so an
        // occluded host executed no app command at all (see
        // `service_app_commands`). It runs last in `logic` because the services
        // above may queue work an app command then consumes on this same pass.
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::AppCommands,
        );
        crate::platform::logging::time_drain("service_app_commands", || {
            self.service_app_commands(ctx)
        });
        self.resolve_auto_context_names();
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::LogicComplete,
        );
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let _sheet_frame = SheetButtonFrame::begin();
        let ctx = &ui.ctx().clone();
        let ui_profile_start = crate::platform::ui_profile::enabled().then(std::time::Instant::now);
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::UiRender,
        );
        if crate::platform::ui_profile::enabled() {
            let drag_active = self.detect_drag_active(ctx);
            crate::platform::ui_profile::set_drag_active(drag_active);
        }
        self.frame_tick
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Repaint-cause diagnostics (#2019): count this frame, attribute
        // input-carrying frames to UserInput, and flush one summary per
        // 10s sample window.
        {
            let (started, frames) = self
                .frame_diag_window
                .get_or_insert_with(|| (std::time::Instant::now(), 0));
            *frames += 1;
            let frames = *frames;
            let elapsed = started.elapsed();
            if ctx.input(|i| !i.raw.events.is_empty()) {
                crate::platform::frame_diag::note(
                    crate::platform::frame_diag::RepaintCause::UserInput,
                );
            }
            // Ground truth (#2209): egui records the file:line of every
            // request_repaint() from the previous pass — including sites the
            // self-reported counters above never instrumented.
            crate::platform::frame_diag::note_egui_causes(&ctx.repaint_causes());
            if elapsed >= std::time::Duration::from_secs(10) {
                let counts = crate::platform::frame_diag::snapshot_and_reset();
                log::debug!(
                    target: "plexi::frame_diag",
                    "{}",
                    crate::platform::frame_diag::summary_line(
                        frames,
                        elapsed.as_secs_f32(),
                        &counts
                    )
                );
                let egui_counts = crate::platform::frame_diag::egui_snapshot_and_reset();
                log::debug!(
                    target: "plexi::frame_diag",
                    "{}",
                    crate::platform::frame_diag::egui_summary_line(&egui_counts, 8)
                );
                self.frame_diag_window = Some((std::time::Instant::now(), 0));
            }
        }
        if self.approval_keyboard.id.is_some()
            && (ctx.input(|input| input.pointer.any_pressed() || !input.focused)
                || self.focus_stack.last() != Some(&FocusKind::ApprovalBanner))
        {
            self.leave_approval();
        }
        // Unified overlay dispatch: each overlay owns its complete keyboard
        // contract via a `*_handle_key` method that returns `Consumed`, preventing
        // `dispatch_app_key_events` and `poll_actions` from seeing those events.
        // Global shortcuts (Cmd+Q, Cmd+W, Cmd+P) remain active via `poll_actions`
        // even when an overlay holds focus (see early-return guard in `keys::poll_actions`).
        // A close chord belongs to the top modal, never the pane beneath it.
        // Reuse that modal's Escape/cancel contract, including unsaved-work vetoes.
        let modal_focus_open = !self.focus_stack.is_empty()
            || (self.sidebar_visible && self.renaming_window.is_some());
        if modal_focus_open {
            let binding = self.key_bindings.close_pane;
            let mut claimed = false;
            ctx.input_mut(|input| {
                for event in &mut input.events {
                    if let egui::Event::Key {
                        key,
                        modifiers,
                        physical_key,
                        ..
                    } = event
                    {
                        if *key == binding.1 && modifiers.matches_logically(binding.0) {
                            claimed = true;
                            *key = egui::Key::Escape;
                            *physical_key = None;
                            *modifiers = egui::Modifiers::NONE;
                        }
                    }
                }
            });
            if claimed {
                self.claimed_host_key_releases.insert(binding.1);
            }
        }
        let mut early_modal_cmds: Vec<crate::app::app_trait::AppCommand> = Vec::new();
        let mut overlay_key_disposition = if modal_focus_open {
            // Ownership-transfer (stint 0240): take this frame's input-intent
            // events out of `ctx` once, hand them to the top `FocusKind`
            // below, then give back whatever it didn't claim before rendering
            // — see `src/app/input_router.rs`.
            let mut router_input = crate::app::input_router::PlexiInput::take_from(ctx);
            // Step 0: QuickNote preemption (#1626).
            // Cmd+0 is a high-priority action that must fire even when a non-critical
            // modal is open. Consume the key here — before the overlay draw phase can
            // absorb it via a focused TextEdit widget — then dismiss the non-critical
            // modal and open QuickNote. Critical modals (ConfirmClose, CapabilityModal,
            // ContextCloseConfirm) are not preemptable and fall through to handle_key.
            let qn_binding = self.key_bindings.open_quick_note;
            let qn_pressed = router_input.consume_key_exact(qn_binding.0, qn_binding.1);
            if qn_pressed && self.is_quick_note_preemptable() {
                self.dismiss_preemptable_modal();
                self.open_quick_note_modal();
                self.sync_command_palette_focus();
                log::info!("quick_note: opened by preempting non-critical modal");
                router_input.give_back(ctx);
                crate::app::app_trait::KeyDisposition::Consumed
            } else {
                // Step 1: run the top overlay's handle_key through the
                // `FocusOwner` trait (consumes its owned key events from this
                // frame's ownership-transfer buffer, before any render).
                let disposition = match self.focus_stack.last().copied() {
                    Some(kind) => kind.owner().handle_key(self, ctx, &mut router_input),
                    None => crate::app::app_trait::KeyDisposition::Passthrough,
                };
                // Give back whatever this overlay didn't claim so the render
                // pass below (a focused `TextEdit`'s own Text/Key consumption)
                // still sees it — this refactor transfers ownership for the
                // *first* consumer each frame, not for the whole frame.
                router_input.give_back(ctx);
                // Step 2: render the top overlay (visual only — key reads
                // already done above). `handle_key` may have popped or replaced
                // the top layer (NotesPicker/QuickNote/ContextDescription
                // self-close), so re-read the current top rather than reusing
                // the kind dispatched above.
                if let Some(kind) = self.focus_stack.last().copied() {
                    early_modal_cmds = kind.owner().draw(self, ctx);
                }
                // The overlay may have self-closed (confirm-close confirmed/cancelled,
                // palette picked an entry, rename committed). Re-sync so the layer is
                // accurate for the rest of this frame.
                self.sync_confirm_close_focus();
                self.sync_context_close_focus();
                self.sync_command_palette_focus();
                self.sync_rename_pane_focus();
                self.sync_context_rename_focus();
                self.sync_text_input_focus();
                self.sync_capability_modal_focus();
                self.sync_event_consent_focus();
                self.sync_raw_wasm_review_focus();
                disposition
            } // end else (non-preempted path)
        } else {
            crate::app::app_trait::KeyDisposition::Passthrough
        };

        // Host bindings own their chords before an app can classify raw key
        // events. This is the structural guard against one physical Cmd+D
        // both splitting the pane and reaching a PGAP app as its bare `d`
        // shortcut. Overlay owners still run first because their local key
        // contracts determine which unclaimed events remain in the router.
        let (app_active, keyboard_capture_active) = self.focused_app_capture_state();
        let overlay_open = self.input_captured_by_overlay();
        let skip_toggle_shortcuts = !overlay_open
            && !self.show_shortcuts
            && self.focused_builtin_editor_captures_shortcuts();
        let filtered_bindings: Option<Vec<keys::BindingEntry>> = skip_toggle_shortcuts.then(|| {
            self.binding_table
                .iter()
                .filter(|b| b.action != Action::ToggleShortcuts)
                .cloned()
                .collect()
        });
        let mut router_input = crate::app::input_router::PlexiInput::take_from(ctx);
        self.suppress_approval_decision_repeats(&mut router_input);
        let mut poll_actions_result = keys::poll_actions(
            &mut router_input,
            filtered_bindings.as_deref().unwrap_or(&self.binding_table),
            &mut self.claimed_host_key_releases,
            /* app_active */ false,
            keyboard_capture_active,
            overlay_open,
            self.show_shortcuts,
        );
        // Cmd+Down belongs to editors unless the focused pane has tool children.
        // Use the normal binding poller so both edges and overlay capture retain
        // the same ownership rules as other host chords.
        if self.visual_owner_pane().is_some_and(|parent| {
            self.conjoined_contexts.iter().any(|(context, id)| {
                *id == parent
                    && self
                        .windows
                        .iter()
                        .any(|window| window.context_id == *context && !window.panes.is_empty())
            })
        }) {
            poll_actions_result.extend(keys::poll_actions(
                &mut router_input,
                &[keys::BindingEntry {
                    modifiers: egui::Modifiers::COMMAND,
                    key: egui::Key::ArrowDown,
                    exact: true,
                    no_repeat: true,
                    context: keys::BindingContext::Normal,
                    action: Action::ToggleZoom,
                }],
                &mut self.claimed_host_key_releases,
                false,
                keyboard_capture_active,
                overlay_open,
                self.show_shortcuts,
            ));
        }
        // Route bare-key contracts on Plexi-owned surfaces (Decision Overlay,
        // Attention Queue list, Sibling Cards, Tree Atlas) after Global chords
        // (`Cmd+Esc`, `Cmd+Enter`, `Cmd+I`, `Cmd+Alt+B`) have been claimed by
        // `poll_actions` above, swallowing all remaining bare keys and `Cmd+V`
        // so nothing leaks to any underlying pane (R40).
        if !modal_focus_open {
            if self.attention_queue.decision_overlay.is_some() {
                overlay_key_disposition = self.handle_decision_overlay_input(&mut router_input);
            } else if self.attention_queue.open {
                overlay_key_disposition = self.handle_attention_queue_list_input(&mut router_input);
            } else if self.zoom.level != zoom::ZoomLevel::Columns {
                overlay_key_disposition = self.handle_zoom_surface_input(&mut router_input);
            }
        }
        router_input.give_back(ctx);

        // Apps only receive key input when no overlay holds focus. Double-check
        // input_captured_by_overlay() after the re-syncs above — an overlay may
        // have returned Passthrough for keys it doesn't own (e.g. a Choice/Input
        // notification) while still being open. Re-syncs may also have dismissed
        // the overlay, in which case it's safe to forward keys to the app pane.
        if overlay_key_disposition == crate::app::app_trait::KeyDisposition::Passthrough
            && !self.input_captured_by_overlay()
        {
            // Ownership-transfer (stint 0387): take this frame's input-intent
            // events out of `ctx`, hand them to the focused app pane's
            // `handle_key`, then give back whatever it didn't claim so the
            // pane's own render pass (a focused `TextEdit`) still sees typed
            // text. Mirrors the overlay and `poll_actions` router pairs.
            let mut router_input = crate::app::input_router::PlexiInput::take_from(ctx);
            self.dispatch_app_key_events(ctx, &mut router_input);
            router_input.give_back(ctx);
        }
        // Stint 0688: app-command servicing moved to `App::logic` — see
        // `service_app_commands`. Only the modal-render follow-up stays here,
        // because `early_modal_cmds` is produced by this pass's overlay draw.
        self.sync_app_cwd();

        // Dispatch any DeliverNotifyAction commands the early modal render
        // produced. Routes back to the originating pane as NotifyAction events.
        self.dispatch_notify_action_cmds(early_modal_cmds);

        // Check if the focused app wants to close itself (e.g. after saving).
        {
            let ctx_ref = &self.windows[self.active_window];
            let should_close = ctx_ref
                .focused_pane
                .and_then(|tile| ctx_ref.tree.tiles.get(tile))
                .and_then(|tile| {
                    if let egui_tiles::Tile::Pane(pid) = tile {
                        Some(*pid)
                    } else {
                        None
                    }
                })
                .and_then(|pid| ctx_ref.panes.get(&pid))
                .map(|pane| {
                    pane.as_app()
                        .map(|a| a.runtime.wants_close())
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if should_close {
                // The keystroke that triggered the close (e.g. `t` in the file
                // browser) is still in this frame's input queue. The restored
                // terminal renders later this same frame and would forward it
                // to its PTY — swallow all key/text events before closing.
                ctx.input_mut(|i| {
                    i.events
                        .retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
                });
                self.close_focused_app();
            }
        }

        // Update window title to reflect active pane — readable by AppleScript / OS scripts
        {
            let context = &self.windows[self.active_window];
            let pane_name = context
                .focused_pane
                .and_then(|tile_id| context.tree.tiles.get(tile_id))
                .and_then(|tile| {
                    if let egui_tiles::Tile::Pane(pane_id) = tile {
                        context.panes.get(pane_id)
                    } else {
                        None
                    }
                })
                .and_then(|pane| {
                    if let Some(t) = pane.as_terminal() {
                        t.name.clone()
                    } else {
                        pane.as_app().map(|a| a.name.clone())
                    }
                });
            let ws = self.router.active();
            let ws_id = ws.context_id;
            let window_count = self
                .windows
                .iter()
                .filter(|c| c.context_id == ws_id)
                .count();
            let context_label = if window_count > 1 {
                format!("{} ({},{})", ws.name, context.grid_x, context.grid_y)
            } else {
                ws.name.to_string()
            };
            let title = match pane_name {
                Some(name) => format!("{} — {}", context_label, name),
                None => context_label,
            };
            // Only send on change: egui's `send_viewport_cmd` unconditionally
            // requests a repaint, so sending every frame pins the render loop
            // at 60fps (uncapped when occluded).
            if self.last_sent_window_title.as_deref() != Some(title.as_str()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
                self.last_sent_window_title = Some(title);
            }
        }

        // Handle keyboard shortcuts. Global shortcuts (Cmd+Q, Cmd+W, Cmd+P) always
        // fire; all other shortcuts are suppressed when an overlay holds focus via
        // the early-return guard in `keys::poll_actions`.
        //
        // AppActive bindings are intentionally polled after app dispatch.
        // Focused TextInputs and apps own bare Escape first; the dispatch gate
        // removes it when it exits a field so CloseApp cannot fire in the same
        // frame. Global and Normal host chords were already removed above.
        let mut router_input = crate::app::input_router::PlexiInput::take_from(ctx);
        poll_actions_result.extend(keys::poll_actions(
            &mut router_input,
            filtered_bindings.as_deref().unwrap_or(&self.binding_table),
            &mut self.claimed_host_key_releases,
            app_active,
            keyboard_capture_active,
            overlay_open,
            self.show_shortcuts,
        ));
        router_input.give_back(ctx);
        for action in poll_actions_result {
            match action {
                Action::SplitHorizontal => {
                    self.windows[self.active_window].clear_zoom();
                    self.split_focused(false, None, false, false, None);
                    self.mark_workspace_dirty();
                }
                Action::SplitVertical => {
                    self.windows[self.active_window].clear_zoom();
                    self.split_focused(true, None, false, false, None);
                    self.mark_workspace_dirty();
                }
                Action::SplitRight => {
                    self.windows[self.active_window].clear_zoom();
                    self.split_focused_mirror(crate::host::command::Placement::Right);
                    self.mark_workspace_dirty();
                }
                Action::SplitDown => {
                    self.windows[self.active_window].clear_zoom();
                    self.split_focused_mirror(crate::host::command::Placement::Below);
                    self.mark_workspace_dirty();
                }
                Action::Navigate(dir) => {
                    let old_active_window = self.active_window;
                    let old_focus = self.windows[self.active_window].focused_pane;
                    let old_window_id = self.windows[self.active_window].window_id;
                    let navigated = self.navigate_at_current_zoom_level(dir);
                    let new_window_id = self.windows[self.active_window].window_id;
                    let new_focus = self.windows[self.active_window].focused_pane;
                    if navigated && self.zoom.level == zoom::ZoomLevel::Columns {
                        self.windows[old_active_window].clear_zoom();
                        self.windows[self.active_window].clear_zoom();
                        log::info!(
                            "zoom: directional navigate exited fullscreen source_window={} destination_window={} focused_pane={new_focus:?}",
                            old_window_id,
                            new_window_id,
                        );
                    }
                    if new_window_id != old_window_id || new_focus != old_focus {
                        self.push_focus_history(old_window_id, old_focus);
                    }
                }
                Action::SwapPane(dir) => self.swap_pane_action(dir),
                Action::SendPane(dir) => {
                    match self.send_pane(dir) {
                        crate::pane_ops::SwapResult::Swapped { rect_a, rect_b, .. } => {
                            let now = std::time::Instant::now();
                            self.pane_anims = vec![
                                PaneSwapAnim {
                                    from: rect_a,
                                    to: rect_b,
                                    started_at: now,
                                },
                                PaneSwapAnim {
                                    from: rect_b,
                                    to: rect_a,
                                    started_at: now,
                                },
                            ];
                            self.ctx.request_repaint();
                        }
                        crate::pane_ops::SwapResult::AtBoundary => {
                            // For U/D at boundary: edge-pulse only (no row-boundary move).
                            // For L/R at boundary: try cross-window send (focus stays on source).
                            let moved = match dir {
                                crate::host::keys::Direction::Down
                                | crate::host::keys::Direction::Up => false,
                                _ => self.send_pane_to_adjacent_window(dir),
                            };
                            if moved {
                                self.ctx.request_repaint();
                            } else if let Some(focused) =
                                self.windows[self.active_window].focused_pane
                            {
                                self.edge_pulse = Some(EdgePulse {
                                    tile: focused,
                                    dir,
                                    started_at: std::time::Instant::now(),
                                });
                                self.ctx.request_repaint();
                            }
                        }
                        crate::pane_ops::SwapResult::NoFocus => {}
                    }
                }
                Action::ClosePane => {
                    if self.show_ui_gallery {
                        self.show_ui_gallery = false;
                        continue;
                    }
                    if self.show_changelog {
                        self.show_changelog = false;
                        continue;
                    }
                    if self.show_shortcuts {
                        self.show_shortcuts = false;
                        continue;
                    }
                    if self.attention_queue.decision_overlay.is_some()
                        || self.attention_queue.open
                        || self.zoom.level != zoom::ZoomLevel::Columns
                    {
                        self.zoom_out_or_close_surface();
                        continue;
                    }
                    // If the focused app pane has a non-empty nav stack
                    // (via PushNav), Escape routes NavBack to the app
                    // instead of closing the pane.
                    if !self.try_nav_back_focused() && !self.focused_app_vetoes_close() {
                        if let Some(child_ctx_id) = self.get_focused_portal_context_id() {
                            let state = self.build_context_close_state(child_ctx_id);
                            if state.items.is_empty() {
                                // Empty context — close immediately, no dialog needed.
                                let idx = self
                                    .router
                                    .iter()
                                    .position(|c| c.context_id == child_ctx_id);
                                if let Some(i) = idx {
                                    log::info!("context_close: empty ctx={child_ctx_id} — closing immediately");
                                    self.delete_context(i);
                                    self.mark_workspace_dirty();
                                }
                            } else {
                                log::info!("context_close: ctx={child_ctx_id} has {} panes — showing dialog", state.items.len());
                                self.pending_context_close = Some(state);
                            }
                        } else if self.confirm_close() {
                            self.pending_close = true;
                        } else {
                            self.execute_close_pane();
                            self.mark_workspace_dirty();
                        }
                    }
                }
                Action::NavBackApp => {
                    if !self.try_nav_back_focused() {
                        self.step_focus_history_back();
                    }
                    log::info!("nav: back-app — window={}", self.active_window);
                }
                Action::FocusHistoryForward => {
                    self.step_focus_history_forward();
                }
                Action::NewTab => {
                    self.new_tab(self.active_window, None, false, None, false, true);
                    self.mark_workspace_dirty();
                }
                Action::ToggleZoom => self.toggle_pane_zoom(),
                Action::Quit => {
                    if !self.prepare_user_quit() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    } else if !self.confirm_quit() {
                        self.quitting = true;
                        // Closing the viewport is the last frame this host
                        // will run — a debounced `mark_workspace_dirty()`
                        // never gets a later tick to flush, so quit paths
                        // save synchronously (stint 0548).
                        self.save_workspace_now();
                        self.workspace_dirty = false;
                        log::info!("workspace: synchronous save (quit)");
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        let now = std::time::Instant::now();
                        let elapsed = self
                            .quit_last_press
                            .map(|t| now.duration_since(t))
                            .unwrap_or(std::time::Duration::MAX);
                        if elapsed > std::time::Duration::from_millis(1500) {
                            self.quit_press_count = 0;
                        }
                        self.quit_press_count += 1;
                        self.quit_last_press = Some(now);
                        if self.quit_press_count >= 3 {
                            self.quit_press_count = 0;
                            self.quit_last_press = None;
                            self.quitting = true;
                            self.save_workspace_now();
                            self.workspace_dirty = false;
                            log::info!("workspace: synchronous save (quit)");
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
                Action::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
                Action::ToggleShortcuts => self.show_shortcuts = !self.show_shortcuts,
                Action::ToggleCommandPalette => {
                    self.show_command_palette = !self.show_command_palette;
                    if self.show_command_palette {
                        self.palette_query.clear();
                        self.palette_selected = 0;
                        self.palette_selected_key = None;
                        self.palette_scroll_reset = true;
                        // Resolve focused pane workspace once at open-time — not per draw-frame —
                        // to avoid filesystem traversal in the egui hot path.
                        self.palette_workspace_root = None;
                        self.palette_workspace_root = self.effective_commands_workspace_root();
                        log::info!(
                            "palette: opened, focused workspace = {:?}",
                            self.palette_workspace_root
                        );
                        // The palette is a switcher for what you are working
                        // on, so it lists only notes open in an editor pane —
                        // never the whole notes dir. Cmd+O's picker remains
                        // the browse-everything surface. No disk scan here.
                        let palette_notes = self.open_note_entries();
                        log::info!("palette: loaded {} open note(s)", palette_notes.len());
                        self.palette_notes = palette_notes;
                        // Resolve user commands once at open-time (re-read each open
                        // → hot reload). Mirrors `plexi run` precedence exactly.
                        self.palette_commands = crate::cli::resolve_user_commands(
                            self.palette_workspace_root.as_deref(),
                        );
                        log::info!(
                            "palette: loaded {} user command(s)",
                            self.palette_commands.len()
                        );
                    } else {
                        self.palette_workspace_root = None;
                        self.palette_notes.clear();
                        self.palette_commands.clear();
                    }
                }
                Action::RenamePane => {
                    self.open_rename_for_focused();
                }
                Action::HidePane => {
                    let win = &self.windows[self.active_window];
                    let target = win
                        .focused_pane
                        .and_then(|tile| Self::find_pane_in_tile(&win.tree, tile))
                        .and_then(|id| win.panes.get(&id).map(|pane| (id, !pane.is_hidden())));
                    if let Some((pane_id, hidden)) = target {
                        if let Err(error) = self.set_pane_hidden(pane_id, hidden) {
                            log::warn!("pane_hide: {error}");
                        }
                    }
                }
                Action::SwitchContext(n) => {
                    // Map display position n (0-indexed) to the actual router
                    // index through the same top-level order the sidebar
                    // renders, so Cmd+1..9 stays pinned to the numbered rows
                    // regardless of how many subcontexts exist.
                    let active_order: Vec<usize> = self
                        .router
                        .top_level_order()
                        .into_iter()
                        .filter(|&i| !self.router.get(i).parked)
                        .collect();
                    if let Some(&router_idx) = active_order.get(n) {
                        let target_ctx_id = self.router.get(router_idx).context_id;
                        log::info!(
                            "SwitchContext: display_pos={} → router_idx={} context_id={target_ctx_id}",
                            n + 1,
                            router_idx
                        );
                        self.switch_workspace(router_idx);
                    } else {
                        log::info!(
                            "SwitchContext: display_pos={} unmapped (active top-level count={})",
                            n + 1,
                            active_order.len()
                        );
                    }
                }
                Action::NextTab => {
                    self.cycle_tab(true);
                    log::info!("tab: next — window={}", self.active_window);
                }
                Action::PrevTab => {
                    self.cycle_tab(false);
                    log::info!("tab: prev — window={}", self.active_window);
                }
                Action::NextContext => {
                    // Cycle the sidebar's own enumeration, not raw router
                    // indices: masters only, unparked, wrapping at both ends.
                    if let Some(idx) = self.router.cycle_top_level(true) {
                        log::info!("context: next — idx={}", idx);
                        self.switch_workspace(idx);
                    }
                }
                Action::PrevContext => {
                    if let Some(idx) = self.router.cycle_top_level(false) {
                        log::info!("context: prev — idx={}", idx);
                        self.switch_workspace(idx);
                    }
                }
                Action::MoveContextUp => {
                    let active = self.router.active_idx();
                    let ctx_id = self.router.active().context_id;
                    if self.router.reorder_top_level(active, ContextMove::Up) {
                        log::info!("context: moved up — id={ctx_id}");
                        self.mark_workspace_dirty();
                    }
                }
                Action::MoveContextDown => {
                    let active = self.router.active_idx();
                    let ctx_id = self.router.active().context_id;
                    if self.router.reorder_top_level(active, ContextMove::Down) {
                        log::info!("context: moved down — id={ctx_id}");
                        self.mark_workspace_dirty();
                    }
                }
                Action::IncreasePaneFontSize => {
                    self.adjust_active_or_remembered_pane_font_size(1.0);
                }
                Action::DecreasePaneFontSize => {
                    self.adjust_active_or_remembered_pane_font_size(-1.0);
                }
                Action::ResetPaneFontSize => {
                    self.reset_active_or_remembered_pane_font_size();
                }
                Action::UiScaleUp => self.set_ui_scale(ui_scale::step_up(self.ui_scale)),
                Action::UiScaleDown => self.set_ui_scale(ui_scale::step_down(self.ui_scale)),
                Action::UiScaleReset => self.set_ui_scale(ui_scale::reset()),
                Action::ScrollUp => {
                    self.scroll_focused_pane(3);
                }
                Action::ScrollDown => {
                    self.scroll_focused_pane(-3);
                }
                Action::CloseApp => {
                    self.close_focused_app();
                }
                Action::NextBlockedAgent => {
                    self.summon_decision_overlay();
                }
                Action::FocusApproval => {
                    self.focus_approval();
                }
                Action::OpenAgents => self.open_agents_pane(),
                Action::OpenFileBrowser => {
                    self.open_file_browser();
                }
                Action::OpenQuickNote => {
                    self.open_quick_note_modal();
                }
                Action::OpenConfig => {
                    let path = crate::config::ensure_config_file();
                    self.open_config_in_editor(path);
                }
                Action::ReloadConfig => {
                    self.reload_config();
                }
                Action::OpenSecretsManager => {
                    self.open_secrets_manager();
                }
                Action::OpenAssistant => {
                    self.open_assistant_pane();
                }
                Action::RenameContext => {
                    self.open_context_rename(self.router.active_idx());
                }
                Action::ToggleAttentionQueue => {
                    self.toggle_attention_queue_list();
                }
                Action::ZoomIn => {
                    self.zoom_in_level();
                }
                Action::ForceReloadApp => {
                    self.force_reload_focused_app();
                }
                Action::NewPageRight => {
                    if self.windows[self.active_window].panes.is_empty()
                        || self.windows[self.active_window].tree.root.is_none()
                    {
                        self.reset_active_context();
                    } else {
                        self.new_page_right();
                    }
                    self.mark_workspace_dirty();
                }
                Action::NewContext => {
                    self.new_context();
                    self.mark_workspace_dirty();
                }
                Action::NewChildContext => {
                    self.new_child_context_from_keyboard();
                }
                Action::PushPaneToSubcontext => {
                    self.push_pane_to_subcontext(None, None);
                }
                Action::SetContextRootFromCwd => {
                    let active = self.active_window;
                    if let Some(tile_id) = self.windows[active].focused_pane {
                        if let Some(cwd) = self.windows[active].get_focused_pane_cwd(tile_id) {
                            log::info!("SetContextRootFromCwd: setting root to {}", cwd.display());
                            self.set_context_root(cwd, None);
                        } else {
                            log::warn!("SetContextRootFromCwd: no CWD available for focused pane");
                        }
                    }
                }
                Action::ParkContext => {
                    self.toggle_park_active_context();
                }
                Action::ToggleMinimap => {
                    if crate::release::feature_enabled(crate::release::ReleaseFeature::PageGrid) {
                        self.minimap.toggle();
                    } else {
                        crate::release::log_feature_blocked(
                            crate::release::ReleaseFeature::PageGrid,
                        );
                    }
                }
                Action::OpenScratchpad => {
                    log::info!("scratchpad: Cmd+Shift+Space — opening");
                    self.open_scratchpad();
                }
                Action::OpenNotesPicker => {
                    log::info!("notes_picker: Cmd+O — opening picker");
                    self.open_notes_picker();
                }
                Action::CloseContext => {
                    let _ = self.close_context_action(self.router.active().context_id);
                }
                Action::ContextZoomOut => {
                    self.zoom_out_or_close_surface();
                }
                Action::RunUserCommand(ref name) => {
                    log::info!("keys: executing user command binding 'run:{name}'");
                    let _ = self.run_user_command_by_name(name);
                }
            }
        }

        // Handle window close request (X button or macOS Cmd+Q OS event).
        //
        // On macOS, Cmd+Q fires BOTH a keyboard event (consumed by keys.rs →
        // Action::Quit → triple-tap) AND a close_requested viewport event in the
        // same frame. Without CancelClose here, the OS close wins the race,
        // quitting immediately and bypassing the triple-tap entirely.
        //
        // We distinguish the two sources by whether a keyboard quit flow is in
        // progress (quit_press_count > 0):
        //   - quit_press_count > 0 → Cmd+Q initiated; cancel the OS close and let
        //     the triple-tap flow own the quit path.
        //   - quit_press_count == 0 → X button or system quit; save and allow close
        //     immediately (no triple-tap for deliberate window close gestures).
        //   - quitting == true → triple-tap completed; save and allow close.
        if ctx.input(|i| i.viewport().close_requested()) {
            crate::platform::logging::mark_ui_phase(
                &self.ui_phase,
                crate::platform::logging::UiPhase::CloseRequest,
            );
            log::info!(
                "quit_phase: close_request quitting={} quit_press_count={}",
                self.quitting,
                self.quit_press_count
            );
            if !self.prepare_user_quit() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if self.quitting {
                // Triple-tap already completed and the window is closing this
                // frame — no later tick will run the debounce flush, so save
                // synchronously (stint 0548).
                self.save_workspace_now();
                self.workspace_dirty = false;
                log::info!("workspace: synchronous save (quit)");
            } else if self.confirm_quit() && self.quit_press_count > 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                // X button / system close with no triple-tap in progress —
                // the window closes immediately, same reasoning as above.
                self.save_workspace_now();
                self.workspace_dirty = false;
                log::info!("workspace: synchronous save (quit)");
            }
        }

        // Ownership-transfer (stint 0387): if the focused pane is a terminal
        // and no overlay owns input, take this frame's keyboard events out of
        // `ctx` *before* the render pass and hand them to that terminal. Doing
        // this before `render_panels` is what prevents egui's own render-time
        // widget machinery (allocate_painter's built-in Cmd+A select-all) from
        // swallowing a key before the terminal processes it. `poll_actions`
        // above already claimed global hotkeys from the same buffer, so the
        // terminal only ever sees what the global allowlist left behind.
        // Bare Escape while zoomed goes to the pane with no layout change
        // (stint 0898: Cmd+Enter is the only zoom toggle).
        let focused_terminal_input = self.take_focused_terminal_input(ctx);
        // Host focus (keyboard nav, a click handled before this pass) is
        // already current. egui focus still belongs to last frame until
        // `reconcile_egui_focus` at the end of `ui`. Publish the owner now so
        // an editor that just lost the pane cannot insert this frame's text.
        crate::app::input_owner::publish_frame_input_owner(ctx, self.host_input_owner());
        // Every surface that paints an approval-capable button (sidebar
        // Needs-you row, approval banner) registers its bounds this frame.
        self.approval_buttons.clear();
        self.render_panels(ui, focused_terminal_input);
        self.draw_attention_queue_and_decision_overlay(ctx);
        self.draw_keychain_recovery_banner(ctx);

        // Detect genuine pane focus transitions, and periodically bank long
        // same-pane sessions so Stats has live data without keystroke tracking.
        self.reconcile_focus_logging(FOCUS_HEARTBEAT_INTERVAL);

        // Last step of every frame: project the derived input owner onto egui
        // widget focus (stint 0429). Everything above may change host focus
        // state; nothing after this may touch egui focus.
        self.reconcile_egui_focus(ctx);
        if let Some(start) = ui_profile_start {
            crate::platform::ui_profile::end_frame(start.elapsed());
        }
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::RendererPresent,
        );
    }

    fn on_exit(&mut self) {
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::Exit,
        );
        log::info!("quit_phase: on_exit begin");
        self.flush_ui_scale_persist_now();
        self.bank_final_focus_segment();
        log::info!("quit_phase: on_exit complete");
        // eframe stops here: it has destroyed the window but will only end the
        // process on a later winit event that a chrome close need never send
        // (see `quit`). Finish the quit ourselves.
        quit::exit_host(quit::ExitReason::WindowClosed);
    }
}

fn read_display_version() -> String {
    crate::distribution::build_tag()
        .trim_start_matches('v')
        .to_string()
}

impl PlexiApp {
    /// Adjust the font size of the active pane in Columns, or the remembered active pane
    /// of the selected context while Sibling Cards or Tree Atlas is visible.
    pub(crate) fn adjust_active_or_remembered_pane_font_size(&mut self, delta: f32) {
        let pane_id = self.resolve_active_or_remembered_pane_id();
        let Some(pane_id) = pane_id else {
            return;
        };
        self.apply_pane_font_size_delta(pane_id, delta);
    }

    /// Reset the font size of the active pane in Columns, or the remembered active pane
    /// of the selected context while Sibling Cards or Tree Atlas is visible.
    pub(crate) fn reset_active_or_remembered_pane_font_size(&mut self) {
        let pane_id = self.resolve_active_or_remembered_pane_id();
        let Some(pane_id) = pane_id else {
            return;
        };
        self.apply_pane_font_size_reset(pane_id);
    }

    pub(crate) fn resolve_active_or_remembered_pane_id(
        &mut self,
    ) -> Option<crate::spatial::tiling::PaneId> {
        match self.zoom.level {
            crate::app::zoom::ZoomLevel::Columns => {
                let ctx = &self.windows[self.active_window];
                let focused_tile = ctx.focused_pane?;
                match ctx.tree.tiles.get(focused_tile) {
                    Some(egui_tiles::Tile::Pane(pid)) => Some(*pid),
                    Some(egui_tiles::Tile::Container(_)) => {
                        let tile = ctx.find_first_pane_in(focused_tile)?;
                        if let Some(egui_tiles::Tile::Pane(pid)) = ctx.tree.tiles.get(tile) {
                            Some(*pid)
                        } else {
                            None
                        }
                    }
                    None => None,
                }
            }
            crate::app::zoom::ZoomLevel::SiblingCards => {
                let ctx_id = self
                    .zoom
                    .cards_focused_context
                    .unwrap_or_else(|| self.router.active().context_id);
                self.resolve_remembered_pane_for_context(ctx_id)
            }
            crate::app::zoom::ZoomLevel::TreeAtlas => {
                let ctx_id = self
                    .zoom
                    .atlas_focused_context
                    .unwrap_or_else(|| self.router.active().context_id);
                self.resolve_remembered_pane_for_context(ctx_id)
            }
        }
    }

    pub(crate) fn resolve_remembered_pane_for_context(
        &mut self,
        ctx_id: u64,
    ) -> Option<crate::spatial::tiling::PaneId> {
        // 1. Ghost marker if it matches this context and pane is alive.
        if let Some(ghost) = &self.zoom.ghost {
            if ghost.context_id == ctx_id
                && self.find_pane_in_any_window(ghost.pane_id).is_some()
            {
                self.zoom.context_last_pane.insert(ctx_id, ghost.pane_id);
                return Some(ghost.pane_id);
            }
        }
        // 2. Focus memory for this context.
        if let Some(&last_pid) = self.zoom.context_last_pane.get(&ctx_id) {
            if self.find_pane_in_any_window(last_pid).is_some() {
                return Some(last_pid);
            }
        }
        // 3. Fallback: pane in the window corresponding to this context.
        if let Some(win) = self.windows.iter().find(|w| w.context_id == ctx_id) {
            if let Some(focused) = win.focused_pane {
                let pane_tile = match win.tree.tiles.get(focused) {
                    Some(egui_tiles::Tile::Pane(_)) => Some(focused),
                    Some(egui_tiles::Tile::Container(_)) => win.find_first_pane_in(focused),
                    None => None,
                };
                if let Some(tile) = pane_tile {
                    if let Some(egui_tiles::Tile::Pane(pid)) = win.tree.tiles.get(tile) {
                        self.zoom.context_last_pane.insert(ctx_id, *pid);
                        return Some(*pid);
                    }
                }
            }
            if let Some(&first_pid) = win.panes.keys().next() {
                self.zoom.context_last_pane.insert(ctx_id, first_pid);
                return Some(first_pid);
            }
        }
        None
    }

    fn apply_pane_font_size_delta(
        &mut self,
        pane_id: crate::spatial::tiling::PaneId,
        delta: f32,
    ) {
        if let Some((win_idx, _)) = self.find_pane_in_any_window(pane_id) {
            if let Some(pane) = self.windows[win_idx].panes.get_mut(&pane_id) {
                if let Some(t) = pane.as_terminal_mut() {
                    let old = t.font_size;
                    t.font_size = (t.font_size + delta).clamp(8.0, 32.0);
                    log::info!(
                        "pane font size: pane_id={pane_id} terminal {old:.1} -> {:.1}",
                        t.font_size
                    );
                } else if let Some(a) = pane.as_app_mut() {
                    if let crate::host::pane::AppRuntime::Builtin(app) = &mut a.runtime {
                        app.adjust_font_size(delta);
                        log::info!(
                            "pane font size: pane_id={pane_id} builtin app delta={delta}"
                        );
                    }
                }
            }
        }
    }

    fn apply_pane_font_size_reset(&mut self, pane_id: crate::spatial::tiling::PaneId) {
        if let Some((win_idx, _)) = self.find_pane_in_any_window(pane_id) {
            if let Some(pane) = self.windows[win_idx].panes.get_mut(&pane_id) {
                if let Some(t) = pane.as_terminal_mut() {
                    let old = t.font_size;
                    t.font_size = self.config.font_size.unwrap_or(14.0).clamp(8.0, 32.0);
                    log::info!(
                        "pane font size reset: pane_id={pane_id} terminal {old:.1} -> {:.1}",
                        t.font_size
                    );
                } else if let Some(a) = pane.as_app_mut() {
                    if let crate::host::pane::AppRuntime::Builtin(app) = &mut a.runtime {
                        app.reset_font_size();
                        log::info!("pane font size reset: pane_id={pane_id} builtin app");
                    }
                }
            }
        }
    }

    /// True while a drag/resize gesture appears active: the pointer is down
    /// and moving, the screen rect changed since last frame (window resize),
    /// or either signal fired within the last 300ms (stint 0731). The
    /// trailing hold prevents the drag window from flapping open/closed
    /// between frames when a gesture pauses briefly mid-drag.
    fn detect_drag_active(&mut self, ctx: &egui::Context) -> bool {
        let screen_rect = ctx.content_rect();
        let rect_changed = self.prev_screen_rect.replace(screen_rect) != Some(screen_rect);
        let pointer_dragging = ctx.input(|i| i.pointer.any_down() && i.pointer.is_moving());

        let now = std::time::Instant::now();
        if rect_changed || pointer_dragging {
            self.drag_window_last_seen = Some(now);
            return true;
        }
        matches!(
            self.drag_window_last_seen,
            Some(last_seen) if now.duration_since(last_seen) < std::time::Duration::from_millis(300)
        )
    }

    /// Open the Cmd+O picker from the in-memory index and refresh it off the
    /// UI thread.
    ///
    /// Tier discovery shares `discover_notes_scopes` with `plexi note list`
    /// (`notes_scopes_for_root`). The picker also passes context roots it
    /// already knows so a home-rooted context does not have to walk `~/Library`
    /// to find them. That walk never runs on this thread: a cache hit paints
    /// immediately, and a miss paints an empty list until the refresh lands.
    pub(crate) fn open_notes_picker(&mut self) {
        let started = std::time::Instant::now();
        log::info!("notes_picker: Cmd+O — opening picker");
        self.ensure_notes_index_loaded();

        let active_root = self.router.active().root.clone();
        let cached = self.notes_index.cached_entries(&active_root);
        let tier_count = self.notes_index.cached_tier_count(&active_root);
        let tier_dirs = self.notes_index.cached_tier_dirs(&active_root);
        let note_count = cached.as_ref().map(|entries| entries.len()).unwrap_or(0);
        if let Some(entries) = cached {
            log::info!(
                "notes_picker: {note_count} note(s) across {tier_count} tier(s) ({tier_dirs:?}) cache=hit"
            );
            self.publish_notes_picker_entries(entries, true);
        } else {
            log::info!(
                "notes_picker: 0 note(s) across 0 tier(s) cache=miss — refresh in background"
            );
            self.publish_notes_picker_entries(Vec::new(), true);
        }

        // Aggregates notes across every visible tier — no single context owns
        // this event, so it stays global-only.
        crate::host::event_log::emit_scoped(
            crate::host::event_log::HostEvent::NotesPickerOpened {
                tier_count,
                note_count,
                timestamp: crate::host::event_log::now_timestamp(),
            },
            None,
        );
        self.push_focus_layer(FocusKind::NotesPicker);
        self.kick_notes_index_refresh();
        log::info!(
            "notes_picker: picker_open_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }

    /// Mark the index stale and refresh. Note create, delete, and rename call
    /// this so the next paint is not stuck on the pre-mutation snapshot.
    pub(crate) fn invalidate_notes_index(&mut self) {
        self.notes_index.invalidate();
        if self.notes_index_rx.is_some() {
            self.notes_index_rerun = true;
            return;
        }
        self.kick_notes_index_refresh();
    }

    fn ensure_notes_index_loaded(&mut self) {
        if self.notes_index_loaded {
            return;
        }
        self.notes_index_loaded = true;
        let Some(path) = crate::notes::notes_index_cache_path() else {
            return;
        };
        if let Some(loaded) = crate::notes::NotesIndexCache::load(&path) {
            self.notes_index = loaded;
        }
    }

    fn notes_discovery_query(&self) -> crate::notes::NotesDiscovery {
        let active = self.router.active().root.clone();
        let mut known_roots: Vec<std::path::PathBuf> = self
            .router
            .as_slice()
            .iter()
            .map(|ctx| ctx.root.clone())
            .collect();
        if let Some(adopted) = crate::config::adopted_workspace_root() {
            known_roots.push(adopted);
        }
        crate::notes::NotesDiscovery {
            active_root: Some(active.clone()),
            known_roots,
            global_notes: crate::notes::global_notes_dir(),
            home: dirs::home_dir(),
            cached_tiers: self.notes_index.cached_tier_dirs(&active),
        }
    }

    fn kick_notes_index_refresh(&mut self) {
        let active = self.router.active().root.clone();
        if self.notes_index_rx.is_some() {
            if self
                .notes_index_inflight_anchor
                .as_ref()
                .is_some_and(|anchor| anchor != &active)
            {
                self.notes_index_rerun = true;
            }
            return;
        }
        let generation = self.notes_index.generation();
        let query = self.notes_discovery_query();
        let (tx, rx) = std::sync::mpsc::channel();
        self.notes_index_rx = Some(rx);
        self.notes_index_inflight_anchor = Some(active.clone());
        let wake = std::sync::Arc::clone(&self.ui_wake);
        let spawned = std::thread::Builder::new()
            .name("notes-index".to_string())
            .spawn(move || {
                let started = std::time::Instant::now();
                let snapshot = crate::notes::build_notes_index(&query);
                let (walk_dirs, budget_hit) = crate::notes::notes_walk_result();
                let stats_elapsed = started.elapsed();
                let _ = tx.send(crate::notes::NotesIndexBuild {
                    anchor: active,
                    snapshot,
                    generation,
                    walk_dirs,
                    budget_hit,
                    elapsed: stats_elapsed,
                });
                wake.wake("notes_index");
            });
        if let Err(err) = spawned {
            log::warn!("notes_index: failed to spawn refresh: {err}");
            self.notes_index_rx = None;
            self.notes_index_inflight_anchor = None;
        }
    }

    fn drain_notes_index_refresh(&mut self) {
        // `try_recv` borrows the receiver. End that borrow before clearing the
        // slot — assigning `notes_index_rx = None` while it is borrowed does
        // not compile.
        let recv_result = {
            let Some(rx) = self.notes_index_rx.as_ref() else {
                return;
            };
            rx.try_recv()
        };
        let built = match recv_result {
            Ok(built) => built,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.notes_index_rx = None;
                self.notes_index_inflight_anchor = None;
                if self.notes_index_rerun {
                    self.notes_index_rerun = false;
                    self.kick_notes_index_refresh();
                }
                return;
            }
        };
        self.notes_index_rx = None;
        self.notes_index_inflight_anchor = None;

        let anchor = built.anchor.clone();
        let elapsed_ms = built.elapsed.as_secs_f64() * 1000.0;
        let note_count = built.snapshot.entries.len();
        let tier_count = built.snapshot.scopes.len();
        let tier_dirs: Vec<&std::path::Path> = built
            .snapshot
            .scopes
            .iter()
            .map(|scope| scope.dir.as_path())
            .collect();
        let walk_dirs = built.walk_dirs;
        let budget_hit = built.budget_hit;
        let generation = built.generation;
        log::info!(
            "notes_index: refresh notes={note_count} tiers={tier_count} ({tier_dirs:?}) \
             walk_dirs={walk_dirs} budget_hit={budget_hit} refresh_ms={elapsed_ms:.1} \
             generation={generation}"
        );

        let stored = self
            .notes_index
            .store(&anchor, built.snapshot.clone(), built.generation);
        if stored {
            if let Some(path) = crate::notes::notes_index_cache_path() {
                if let Err(err) = self.notes_index.save(&path) {
                    log::warn!("notes_index: failed to persist cache: {err}");
                }
            }
        }

        let picker_open = self.focus_stack.contains(&FocusKind::NotesPicker);
        let same_anchor = self.router.active().root == anchor;
        if stored && picker_open && same_anchor && self.notes_picker_rows_match_shown() {
            let entries = built.snapshot.entries;
            log::info!(
                "notes_picker: refresh applied {note_count} note(s) across {tier_count} tier(s)"
            );
            self.publish_notes_picker_entries(entries, false);
        }

        // Store fails when `invalidate` bumped the generation after this build
        // started. Also rebuild when the active context moved mid-walk.
        let anchor_moved = self.router.active().root != anchor;
        let needs_refresh = !stored || anchor_moved;
        if needs_refresh || self.notes_index_rerun {
            self.notes_index_rerun = false;
            if needs_refresh {
                self.kick_notes_index_refresh();
            }
        }
    }

    fn notes_picker_rows_match_shown(&self) -> bool {
        self.notes_picker_entries
            .iter()
            .map(|entry| entry.path.as_path())
            .eq(self
                .notes_picker_shown_paths
                .iter()
                .map(std::path::PathBuf::as_path))
    }

    fn publish_notes_picker_entries(
        &mut self,
        entries: Vec<crate::notes::NotePickerEntry>,
        reset_query: bool,
    ) {
        let selected_path = if reset_query {
            None
        } else {
            self.notes_picker_filtered()
                .get(self.notes_picker_selected)
                .and_then(|&idx| self.notes_picker_entries.get(idx))
                .map(|entry| entry.path.clone())
        };
        self.notes_picker_shown_paths = entries.iter().map(|entry| entry.path.clone()).collect();
        self.notes_picker_entries = entries;
        if reset_query {
            self.notes_picker_selected = 0;
            self.notes_picker_query.clear();
            self.notes_picker_pending_delete = None;
            self.notes_picker_status = None;
            return;
        }
        let filtered = self.notes_picker_filtered();
        if filtered.is_empty() {
            self.notes_picker_selected = 0;
            return;
        }
        if let Some(path) = selected_path {
            if let Some(pos) = filtered
                .iter()
                .position(|&idx| self.notes_picker_entries[idx].path == path)
            {
                self.notes_picker_selected = pos;
                return;
            }
        }
        if self.notes_picker_selected >= filtered.len() {
            self.notes_picker_selected = filtered.len() - 1;
        }
    }

    #[cfg(test)]
    pub(crate) fn finish_notes_index_refresh_for_test(&mut self) {
        let Some(rx) = self.notes_index_rx.take() else {
            return;
        };
        self.notes_index_inflight_anchor = None;
        let built = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("notes index refresh");
        let anchor = built.anchor.clone();
        let stored = self
            .notes_index
            .store(&anchor, built.snapshot.clone(), built.generation);
        if stored && self.router.active().root == anchor {
            self.publish_notes_picker_entries(built.snapshot.entries, false);
        }
    }
}

// ── Directed pipe helpers (#286) ─────────────────────────────────────────────

/// Translate a key string (e.g. "enter", "ctrl+c", "h") to PTY bytes.
fn key_str_to_pty_bytes(key: &str) -> Vec<u8> {
    let key_lower = key.to_lowercase();
    // Handle ctrl+X chords using bit-mask to support any ASCII char ([, ], \, /, @, etc.)
    if let Some(rest) = key_lower.strip_prefix("ctrl+") {
        if let Some(ch) = rest.chars().next() {
            if ch.is_ascii() && !ch.is_ascii_control() {
                return vec![(ch as u8) & 0x1F];
            }
        }
    }
    match key_lower.as_str() {
        "enter" => b"\r".to_vec(),
        "escape" | "esc" => b"\x1b".to_vec(),
        "space" => b" ".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "tab" => b"\t".to_vec(),
        "up" | "arrowup" => b"\x1b[A".to_vec(),
        "down" | "arrowdown" => b"\x1b[B".to_vec(),
        "right" | "arrowright" => b"\x1b[C".to_vec(),
        "left" | "arrowleft" => b"\x1b[D".to_vec(),
        _ => {
            // single printable char
            let mut chars = key.chars();
            if let Some(ch) = chars.next() {
                if chars.next().is_none() {
                    let mut buf = [0u8; 4];
                    return ch.encode_utf8(&mut buf).as_bytes().to_vec();
                }
            }
            log::warn!(
                "pane_ipc: key_pane: unrecognized key string ({} chars), sending raw bytes",
                key.chars().count()
            );
            key.as_bytes().to_vec()
        }
    }
}

/// Parse a key string into a (key_name, Modifiers) pair for PGAP app panes.
#[cfg(test)]
fn parse_key_str_to_event(key: &str) -> (String, crate::protocol::Modifiers) {
    let mut parts: Vec<&str> = key.split('+').collect();
    let key_part = parts.pop().unwrap_or(key);
    let mut modifiers = crate::protocol::Modifiers::default();
    for m in &parts {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            "cmd" | "command" | "meta" => modifiers.cmd = true,
            _ => {}
        }
    }
    let key_str = match key_part.to_lowercase().as_str() {
        "enter" | "return" => "return".to_string(),
        "escape" | "esc" => "escape".to_string(),
        "space" => "space".to_string(),
        "backspace" => "backspace".to_string(),
        "tab" => "tab".to_string(),
        "up" | "arrowup" => "up".to_string(),
        "down" | "arrowdown" => "down".to_string(),
        "right" | "arrowright" => "right".to_string(),
        "left" | "arrowleft" => "left".to_string(),
        "plus" => "plus".to_string(),
        "equals" => "equals".to_string(),
        "minus" => "minus".to_string(),
        "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12" => {
            key_part.to_lowercase()
        }
        _ => {
            // SDK KeyEvent.key is lowercase for app panes.
            if key_part.chars().count() == 1 {
                key_part.to_lowercase()
            } else {
                key_part.to_lowercase()
            }
        }
    };
    (key_str, modifiers)
}

/// Translate a key string (same vocabulary as `key_str_to_pty_bytes`) into a
/// synthesized `egui::RawInput` for native app panes. Native (builtin/WASM)
/// apps read keys from `egui::InputState` via `App::handle_key`, not the PGAP
/// event queue, so CLI key injection must produce egui events. Returns `None`
/// for key strings that don't map to an `egui::Key`.
/// Clipboard operations carried by a keyboard chord (matching egui-winit's
/// `is_cut_command`/`is_copy_command`/`is_paste_command` vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipboardChord {
    Cut,
    Copy,
    Paste,
}

/// Detects the platform clipboard chords (`cmd+x`/`cmd+c`/`cmd+v`; ctrl on
/// non-mac, where `key_str_to_egui_raw_input` already maps ctrl→command).
pub(crate) fn clipboard_chord(
    modifiers: egui::Modifiers,
    key: egui::Key,
) -> Option<ClipboardChord> {
    if !modifiers.command {
        return None;
    }
    match key {
        egui::Key::X => Some(ClipboardChord::Cut),
        egui::Key::C => Some(ClipboardChord::Copy),
        egui::Key::V => Some(ClipboardChord::Paste),
        _ => None,
    }
}

pub(crate) fn key_str_to_egui_raw_input(key: &str) -> Option<egui::RawInput> {
    let (parts, key_part): (Vec<&str>, &str) = if key == "+" {
        (Vec::new(), "+")
    } else if let Some(prefix) = key.strip_suffix("++") {
        (prefix.split('+').filter(|s| !s.is_empty()).collect(), "+")
    } else {
        let mut parts: Vec<&str> = key.split('+').collect();
        let key_part = parts.pop().unwrap_or(key);
        (parts, key_part)
    };
    let mut modifiers = egui::Modifiers::default();
    for m in &parts {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" => {
                modifiers.ctrl = true;
                if !cfg!(target_os = "macos") {
                    modifiers.command = true;
                }
            }
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            "cmd" | "command" | "meta" => {
                modifiers.mac_cmd = cfg!(target_os = "macos");
                modifiers.command = true;
            }
            _ => {}
        }
    }
    let lower = key_part.to_lowercase();
    if matches!(lower.as_str(), "*" | "asterisk") {
        modifiers.shift = true;
    }
    // Named keys use egui's `Key::from_name` vocabulary; single printable
    // chars ("a", "/", "1") and F-keys ("f5") pass through — `from_name`
    // accepts lowercase letters and punctuation chars directly.
    let name: String = match lower.as_str() {
        "enter" | "return" => "Enter".into(),
        "escape" | "esc" => "Escape".into(),
        "space" => "Space".into(),
        "backspace" => "Backspace".into(),
        "tab" => "Tab".into(),
        "delete" => "Delete".into(),
        "insert" => "Insert".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "up" | "arrowup" => "ArrowUp".into(),
        "down" | "arrowdown" => "ArrowDown".into(),
        "left" | "arrowleft" => "ArrowLeft".into(),
        "right" | "arrowright" => "ArrowRight".into(),
        "+" | "plus" => "Plus".into(),
        "-" | "minus" => "Minus".into(),
        "=" | "equals" => "Equals".into(),
        "/" | "slash" => "Slash".into(),
        "*" | "asterisk" => "8".into(),
        f if f.starts_with('f') && f.len() > 1 && f[1..].chars().all(|c| c.is_ascii_digit()) => {
            f.to_uppercase()
        }
        other => other.into(),
    };
    let egui_key = egui::Key::from_name(&name)?;
    // Mirror egui-winit's chord translation: a physical clipboard chord never
    // reaches widgets as a raw `Key` event — it becomes `Event::Cut`/`Copy`/
    // `Paste` before egui sees it, and text surfaces consume only those
    // translated events. Replaying the raw chord would be silently dropped by
    // every text widget, so the synthetic path must translate identically.
    if let Some(chord) = clipboard_chord(modifiers, egui_key) {
        let events = match chord {
            ClipboardChord::Cut => vec![egui::Event::Cut],
            ClipboardChord::Copy => vec![egui::Event::Copy],
            ClipboardChord::Paste => {
                match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                    Ok(contents) => {
                        let contents = contents.replace("\r\n", "\n");
                        if contents.is_empty() {
                            vec![]
                        } else {
                            vec![egui::Event::Paste(contents)]
                        }
                    }
                    Err(error) => {
                        log::warn!(
                            "pane_ipc: paste chord: clipboard read failed, delivering no event: {error}"
                        );
                        vec![]
                    }
                }
            }
        };
        log::info!(
            "pane_ipc: clipboard chord {chord:?} translated to {} egui event(s)",
            events.len()
        );
        return Some(egui::RawInput {
            events,
            modifiers,
            ..Default::default()
        });
    }
    let mut events = vec![egui::Event::Key {
        key: egui_key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }];
    // Real keyboard input also emits an `Event::Text` for printable chars
    // (search fields and text inputs read Text, not Key). Mirror that unless
    // a chord modifier is held.
    if !modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd && !modifiers.alt {
        let text = match lower.as_str() {
            "space" => Some(" ".to_string()),
            "*" | "asterisk" => Some("*".to_string()),
            "plus" => Some("+".to_string()),
            "minus" => Some("-".to_string()),
            "equals" => Some("=".to_string()),
            "slash" => Some("/".to_string()),
            _ => {
                let mut chars = key_part.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) if !ch.is_control() => Some(if modifiers.shift {
                        ch.to_uppercase().to_string()
                    } else {
                        ch.to_string()
                    }),
                    _ => None,
                }
            }
        };
        if let Some(t) = text {
            events.push(egui::Event::Text(t));
        }
    }
    events.push(egui::Event::Key {
        key: egui_key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers,
    });
    Some(egui::RawInput {
        events,
        modifiers,
        ..Default::default()
    })
}

/// Drive a native (builtin or WASM) app pane's `App::handle_key` with a
/// synthesized key — the same handler a real keystroke reaches through
/// `dispatch_app_key_events`. Never bypass this into app internals: `pane key`
/// must exercise the app's real keyboard path so live validation observes
/// production behavior.
fn drive_native_pane_key(
    runtime: &mut crate::host::pane::AppRuntime,
    key: &str,
) -> Result<app_trait::KeyDisposition, String> {
    let raw = key_str_to_egui_raw_input(key)
        .ok_or_else(|| format!("key {key:?} does not map to a native app key event"))?;
    // Throwaway context: the cheapest way to build a well-formed input buffer.
    // handle_key only reads input — it never paints — so no fonts/render
    // setup is needed. Route through the same `PlexiInput` ownership buffer the
    // live frame uses so `pane key` exercises the production keyboard path.
    let ctx = egui::Context::default();
    let mut disposition = app_trait::KeyDisposition::Passthrough;
    let _ = ctx.run_ui(raw, |ui| {
        let input = crate::app::input_router::PlexiInput::take_from(ui.ctx());
        disposition = runtime.handle_key(&input);
    });
    Ok(disposition)
}

/// Register the directed pipe on the target's own local pipe registry so a
/// runtime that maintains one (`AppRuntime::Wasm` and `AppRuntime::Python`)
/// reports `has_pipe_reader` true for it on the reverse-send path (stint 0742).
///
/// Builtin apps (`AppRuntime::Builtin`) have no per-pane `TypedPipeRegistry`
/// and return `false`; `OpenDirectedPipe` still proceeds for them because the
/// duplex data plane is host-mediated via `directed_pipes` +
/// `queue_outbound_event`.
fn register_directed_pipe_on_target(pane: &mut crate::host::pane::Pane, pipe_id: &str) -> bool {
    let crate::host::pane::Pane::App(app_pane) = pane else {
        return false;
    };
    let runtime_kind = app_pane.runtime.runtime_kind();
    let registered = app_pane.runtime.register_directed_pipe(pipe_id)
        && app_pane.runtime.has_pipe_reader(pipe_id);
    if registered {
        log::info!(
            "register_directed_pipe_on_target: registered directed pipe '{pipe_id}' on target pane {} (app_id={}, runtime={runtime_kind})",
            app_pane.id,
            app_pane.manifest_id
        );
    }
    registered
}

/// Explicit collaborators passed to [`restore_workspace`] so workspace
/// reconstruction reads no ambient profile directory, starts no servers, and
/// spawns no terminal processes unless `terminal_spawner` is provided.
pub(crate) struct WorkspaceRestoreCollaborators<'a> {
    pub profile_dir: &'a std::path::Path,
    pub colors: &'a Colors,
    pub hot_reload: &'a mut crate::host::hot_reload::HotReloadWatcher,
    pub state_watch: &'a mut crate::host::state_watch::StateWatcher,
    pub terminal_spawner: Option<(&'a egui::Context, &'a mpsc::Sender<(u64, PtyEvent)>)>,
}

/// Reconstructed workspace state returned by [`restore_workspace`].
pub(crate) struct RestoredWorkspace {
    pub router: crate::workspace::router::WorkspaceRouter,
    pub windows: Vec<Window>,
    pub active_window: usize,
    pub sidebar_visible: bool,
    pub pane_heartbeats: HashMap<crate::spatial::tiling::PaneId, PaneHeartbeat>,
    pub context_active_window: HashMap<u64, u64>,
    pub minimap_visible: bool,
    pub next_pane_id: u64,
    pub next_window_id: u64,
}

/// Reconstruct windows, contexts, heartbeats, and focus state from a loaded
/// [`WorkspaceFile`]. Returns `None` when the workspace has no contexts or
/// no windows survive pane restoration.
pub(crate) fn restore_workspace(
    ws: WorkspaceFile,
    config: &config::PlexiConfig,
    collaborators: WorkspaceRestoreCollaborators<'_>,
) -> Option<RestoredWorkspace> {
    if ws.contexts.is_empty() {
        return None;
    }
    let default_font_size = config.font_size.unwrap_or(theme::FONT_SIZE);
    let restore_mode = config.agent_restore_mode();
    let initial_active_ctx = ws.active_context.min(ws.contexts.len().saturating_sub(1));
    let initial_active_ctx_id = ws.contexts[initial_active_ctx].context_id;
    let mut windows = Vec::new();
    let mut restored_heartbeats = HashMap::new();
    let ctx_name_map: std::collections::HashMap<u64, String> = ws
        .contexts
        .iter()
        .map(|c| (c.context_id, c.name.to_string()))
        .collect();
    let ctx_desc_map: std::collections::HashMap<u64, String> = ws
        .contexts
        .iter()
        .filter_map(|c| c.description.as_ref().map(|d| (c.context_id, d.clone())))
        .collect();
    let ctx_root_map: std::collections::HashMap<u64, PathBuf> = ws
        .contexts
        .iter()
        .map(|c| (c.context_id, c.root.clone()))
        .collect();
    let ctx_depth_map: std::collections::HashMap<u64, u32> = ws
        .contexts
        .iter()
        .map(|c| (c.context_id, c.depth))
        .collect();

    for mut saved_win in ws.windows {
        let mut panes = HashMap::new();
        let mut non_skipped_pane_count = 0usize;
        let ctx_depth = ctx_depth_map
            .get(&saved_win.context_id)
            .copied()
            .unwrap_or(0);
        for saved_pane in &saved_win.panes {
            // Do not respawn completed ephemeral agent panes on startup.
            let is_completed_ephemeral = saved_pane.agent_session.as_ref().is_some_and(|s| {
                s.ephemeral
                    && (s.last_state == crate::protocol::AgentState::Completed
                        || saved_pane.slots.contains_key("completed"))
            });
            if is_completed_ephemeral {
                crate::workspace::archive_saved_completion(
                    collaborators.profile_dir,
                    saved_pane,
                    saved_win.context_id,
                    ctx_name_map
                        .get(&saved_win.context_id)
                        .map(String::as_str)
                        .unwrap_or("unknown"),
                );
                log::info!(
                    "workspace_restore: skipping completed ephemeral agent pane {}",
                    saved_pane.id
                );
                if let Some(tile_id) = saved_win.tree.tiles.find_pane(&saved_pane.id) {
                    saved_win.tree.remove_recursively(tile_id);
                }
                continue;
            }
            non_skipped_pane_count += 1;

            if let Some(heartbeat) = &saved_pane.heartbeat {
                restored_heartbeats.insert(
                    saved_pane.id,
                    PaneHeartbeat {
                        every: std::time::Duration::from_millis(heartbeat.every_ms),
                        text: heartbeat.text.clone(),
                        while_idle_only: heartbeat.while_idle_only,
                        next_fire: std::time::Instant::now()
                            + std::time::Duration::from_millis(heartbeat.every_ms),
                    },
                );
            }
            let cwd = if saved_pane.cwd.is_dir() {
                Some(saved_pane.cwd.clone())
            } else if saved_win.path.is_dir() {
                Some(saved_win.path.clone())
            } else {
                dirs::home_dir()
            };
            let mut pane_entry: Option<Pane> = None;
            if matches!(saved_pane.kind, crate::workspace::SavedPaneKind::App) {
                let Some(app_type) = &saved_pane.app_id else {
                    continue;
                };
                let app_cwd = saved_pane.cwd.clone();
                if let Some(launch) = &saved_pane.launch {
                    // Real manifest identity + launch context (stint 0680):
                    // relaunch through the shared runtime construction path,
                    // keeping the saved pane id.
                    let ctx_root = ctx_root_map
                        .get(&saved_win.context_id)
                        .cloned()
                        .unwrap_or_else(|| app_cwd.clone());
                    match crate::pane_ops::restore_app_pane_in_profile(
                        app_type,
                        launch,
                        crate::pane_ops::RestoreTarget {
                            pane_id: saved_pane.id,
                            context_id: saved_win.context_id,
                            workspace_root: app_cwd.clone(),
                            context_root: &ctx_root,
                            saved_name: saved_pane.name.as_deref(),
                        },
                        collaborators.colors,
                        collaborators.profile_dir,
                    ) {
                        Ok(crate::pane_ops::RestoredAppPane {
                            pane,
                            state_paths,
                            hot_reload_dir,
                        }) => {
                            if !hot_reload_dir.as_os_str().is_empty() {
                                collaborators
                                    .hot_reload
                                    .watch(saved_pane.id, &hot_reload_dir);
                            }
                            if !state_paths.is_empty() {
                                collaborators.state_watch.watch(saved_pane.id, &state_paths);
                            }
                            log::info!(
                                "workspace_restore: pane {} (app_id={app_type}) relaunched as its own app",
                                saved_pane.id
                            );
                            pane_entry = Some(pane);
                        }
                        Err(reason) => {
                            log::info!(
                                "workspace_restore: pane {} (app_id={app_type}) failed to relaunch: {reason} — rendering launch-failed pane",
                                saved_pane.id
                            );
                            pane_entry = Some(crate::pane_ops::restore_launch_failed_pane(
                                app_type,
                                saved_pane.id,
                                app_cwd.clone(),
                                reason,
                            ));
                        }
                    }
                } else if !saved_pane.is_legacy_runtime_kind_id() {
                    pane_entry = crate::pane_ops::restore_builtin_app_pane(
                        app_type,
                        saved_pane.id,
                        app_cwd.clone(),
                        saved_pane.app_state.as_ref(),
                    );
                    // The Assistant is a builtin needing a broker + profile
                    // dir, so the broker-free factory can't build it. Build
                    // it here where the loaded `config.ai` is available; the
                    // conversation auto-resumes from disk.
                    if pane_entry.is_none()
                        && app_type == "assistant"
                        && crate::release::feature_enabled(
                            crate::release::ReleaseFeature::Assistant,
                        )
                    {
                        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
                            std::sync::Arc::new(
                                crate::plexi_ai::broker::LiveAiBroker::with_model_modes(
                                    config.ai.clone(),
                                    config.resolved_model_modes(),
                                ),
                            );
                        pane_entry = Some(crate::pane_ops::restore_assistant_pane(
                            saved_pane.id,
                            app_cwd.clone(),
                            broker,
                            collaborators.profile_dir,
                            saved_win.context_id,
                        ));
                    }
                    if pane_entry.is_none() && app_type == "assistant" {
                        crate::release::log_feature_blocked(
                            crate::release::ReleaseFeature::Assistant,
                        );
                    }
                } else {
                    // Pre-0680 save of a Python/WASM app pane: `app_id`
                    // holds the runtime kind, not a manifest id, and
                    // there is no launch context to relaunch from.
                    log::info!(
                        "workspace_restore: legacy app pane pane_id={} runtime_kind={app_type} restored as launch-failed",
                        saved_pane.id
                    );
                    pane_entry = Some(crate::pane_ops::restore_launch_failed_pane(
                        app_type,
                        saved_pane.id,
                        app_cwd.clone(),
                        format!(
                            "app pane saved before Plexi recorded app identity (runtime kind only: {app_type}); reopen the app with `plexi app open <id>`"
                        ),
                    ));
                }
            }

            // Portal panes — restore the tile reference, no process to start.
            if matches!(
                saved_pane.kind,
                crate::workspace::SavedPaneKind::Portal { .. }
            ) {
                if let crate::workspace::SavedPaneKind::Portal { context_id } = &saved_pane.kind {
                    pane_entry = Some(Pane::Portal(Box::new(crate::host::pane::PortalPane {
                        pane_id: saved_pane.id,
                        target_context_id: *context_id,
                        context_state: None,
                        hidden: false,
                    })));
                }
            }

            if pane_entry.is_none() {
                if let Some((egui_ctx, pty_event_tx)) = collaborators.terminal_spawner {
                    let ctx_name = ctx_name_map
                        .get(&saved_win.context_id)
                        .cloned()
                        .unwrap_or_default();
                    let ctx_desc = ctx_desc_map
                        .get(&saved_win.context_id)
                        .cloned()
                        .unwrap_or_default();
                    let ctx_root = ctx_root_map.get(&saved_win.context_id);
                    let effective_session = saved_pane
                        .agent_session
                        .as_ref()
                        .filter(|_| restore_mode != config::AgentRestoreMode::Off)
                        .map(|s| {
                            let mut s = s.clone();
                            if s.last_state == crate::protocol::AgentState::Working {
                                s.last_state = crate::protocol::AgentState::Interrupted;
                            }
                            s
                        });
                    let is_active_top_context =
                        saved_win.context_id == initial_active_ctx_id && ctx_depth == 0;
                    let finished_script = effective_session
                        .as_ref()
                        .is_some_and(is_finished_script_session);
                    let should_auto_resume = !finished_script
                        && (matches!(restore_mode, config::AgentRestoreMode::Eager)
                            || (matches!(restore_mode, config::AgentRestoreMode::Lazy)
                                && is_active_top_context));

                    let context_env = crate::pane_ops::create::PaneContextEnv {
                        context_id: saved_win.context_id,
                        name: ctx_name,
                        description: ctx_desc,
                        root: ctx_root.cloned(),
                        depth: ctx_depth,
                    };
                    let (mut settings, pending_credential) =
                        PlexiApp::make_backend_settings_in_profile(
                            collaborators.profile_dir,
                            saved_pane.id,
                            cwd.clone(),
                            collaborators.colors,
                            &context_env,
                            effective_session.is_some(),
                        );
                    let mut effective_session = effective_session;
                    if let Some(ref mut session) = effective_session {
                        settings
                            .env
                            .insert("PLEXI_AGENT_HARNESS".to_string(), session.harness.clone());
                        if let Some(ref role) = session.config_name {
                            settings
                                .env
                                .insert("PLEXI_AGENT_ROLE".to_string(), role.clone());
                        }
                        if let Some(parent_id) = session.parent_pane_id {
                            settings
                                .env
                                .insert("PLEXI_PARENT_PANE".to_string(), parent_id.to_string());
                        }
                        let resolved_spawned_by = session
                            .spawned_by
                            .clone()
                            .filter(|s| {
                                !s.is_empty() && !(s == "human" && session.parent_pane_id.is_some())
                            })
                            .or_else(|| {
                                let pid = session.parent_pane_id?;
                                let parent = saved_win.panes.iter().find(|p| p.id == pid)?;
                                if let Some(ref ps) = parent.agent_session {
                                    let label = ps
                                        .config_name
                                        .as_deref()
                                        .or(parent.name.as_deref())
                                        .unwrap_or(&ps.harness);
                                    Some(format!("agent:{label}"))
                                } else {
                                    Some("human".to_string())
                                }
                            })
                            .unwrap_or_else(|| "human".to_string());
                        session.spawned_by = Some(resolved_spawned_by.clone());
                        settings
                            .env
                            .insert("PLEXI_SPAWNED_BY".to_string(), resolved_spawned_by);
                        if session.ephemeral {
                            settings
                                .env
                                .insert("PLEXI_EPHEMERAL".to_string(), "1".to_string());
                        }
                        if should_auto_resume {
                            if let Some(harness) = crate::cli::AgentHarness::parse(&session.harness)
                            {
                                let flags = crate::cli::agent::resolved_session_harness_flags_in(
                                    session.config_name.as_deref(),
                                    harness,
                                    collaborators.profile_dir,
                                    cwd.as_deref(),
                                );
                                let resume_cmd = harness.restore_command_with_flags(
                                    session.session_id.as_deref(),
                                    &flags,
                                );
                                if harness == crate::cli::AgentHarness::Pi
                                    && resume_cmd == harness.adapter().binary
                                    && session.session_id.is_some()
                                {
                                    session.session_id = None;
                                }
                                log::info!(
                                    "workspace_restore: auto-resuming agent pane {} harness={} session_id={:?} state={} cmd={:?}",
                                    saved_pane.id,
                                    session.harness,
                                    session.session_id,
                                    session.last_state.as_str(),
                                    resume_cmd
                                );
                                settings
                                    .env
                                    .insert("PLEXI_RESUME_COMMAND".to_string(), resume_cmd.clone());
                                crate::pane_ops::apply_initial_cmd(
                                    &mut settings,
                                    &resume_cmd,
                                    session.ephemeral,
                                );
                            }
                        }
                    }
                    let standby_resume_command = (!should_auto_resume && !finished_script)
                        .then(|| {
                            effective_session.as_ref().and_then(|session| {
                                crate::cli::AgentHarness::parse(&session.harness).map(|harness| {
                                    let flags = crate::cli::agent::resolved_session_harness_flags_in(
                                        session.config_name.as_deref(),
                                        harness,
                                        collaborators.profile_dir,
                                        cwd.as_deref(),
                                    );
                                    harness.restore_command_with_flags(
                                        session.session_id.as_deref(),
                                        &flags,
                                    )
                                })
                            })
                        })
                        .flatten();
                    if let Some(mut pane) = TerminalPane::new(
                        saved_pane.id,
                        egui_ctx.clone(),
                        pty_event_tx.clone(),
                        settings,
                        default_font_size,
                    ) {
                        pane.name = saved_pane.name.clone().or_else(|| {
                            effective_session
                                .as_ref()
                                .and_then(|s| s.config_name.clone())
                        });
                        pane.name_locked = saved_pane
                            .name
                            .as_deref()
                            .is_some_and(|n| !n.is_empty() && n != "terminal")
                            || effective_session.is_some();
                        pane.slots = saved_pane.slots.clone();
                        if let Some(session) = effective_session {
                            pane.fleet_result = session.fleet_result.clone();
                            pane.fleet_seen = session.fleet_seen;
                            pane.fleet_dismissed = session.fleet_dismissed;
                            pane.fleet_done_unseen = session.fleet_done_unseen;
                            pane.fleet_since = session.fleet_since;
                            pane.ephemeral = session.ephemeral;
                            pane.parent_pane_id = session.parent_pane_id;
                            pane.spawned_by = session.spawned_by.clone().or(pane.spawned_by);
                            pane.pending_child_results = session.pending_child_results.clone();
                            pane.agent_harness = Some(session.harness.clone());
                            pane.agent_session_id = session.session_id.clone();
                            pane.agent_role = session.config_name.clone();
                            pane.agent_permission_posture = session
                                .permission_posture
                                .clone()
                                .or_else(|| {
                                    let role = session.config_name.as_deref()?;
                                    let lookup_root = cwd
                                        .as_deref()
                                        .and_then(crate::app::registry::resolve_agent_lookup_workspace_root)
                                        .or_else(|| ctx_root.cloned());
                                    crate::cli::agent::resolve_agent_definition_in(
                                        role,
                                        collaborators.profile_dir,
                                        lookup_root.as_deref(),
                                    )
                                    .ok()
                                    .map(|(def, _)| def.posture)
                                });
                            pane.last_blocked_reason = session.last_blocked_reason;
                            pane.tail_preview = session.tail_preview.clone();
                            pane.agent_completed = session.last_state
                                == crate::protocol::AgentState::Completed
                                || saved_pane.slots.contains_key("completed");
                            pane.agent = Some(crate::protocol::PaneAgentState {
                                pane_id: saved_pane.id,
                                state: session.last_state.clone(),
                                agent: session.harness.clone(),
                                detail: session.last_detail.clone(),
                                session_id: session.session_id.clone(),
                            });
                            if !should_auto_resume {
                                pane.standby_session = Some(session);
                                pane.standby_resume_command = standby_resume_command;
                                pane.standby_cwd = cwd.clone();
                                log::info!(
                                    "workspace_restore: restored agent pane {} into standby ({})",
                                    saved_pane.id,
                                    pane.standby_banner().unwrap_or_default()
                                );
                            }
                        }
                        pane_entry = Some(Pane::Terminal(Box::new(pane)));
                        pending_credential.mark_live();
                    }
                } else {
                    log::info!(
                        "workspace_restore: skipping terminal pane {} in headless restore",
                        saved_pane.id
                    );
                }
            }

            if let Some(mut pane) = pane_entry {
                if let Some(slots) = pane.slots_mut() {
                    if slots.is_empty() && !saved_pane.slots.is_empty() {
                        *slots = saved_pane.slots.clone();
                    }
                }
                if saved_pane.hidden {
                    pane.set_hidden(true);
                }
                panes.insert(saved_pane.id, pane);
            }
        }
        // Skip if there were non-skipped saved panes that all failed to restore
        // (corrupted state or headless restore without terminal spawner), or if
        // a sub-context window had only completed ephemeral panes that were all
        // skipped. Top-level windows whose panes were all completed ephemeral
        // panes survive as an empty welcome window.
        if non_skipped_pane_count > 0 && panes.is_empty() {
            continue;
        }
        if non_skipped_pane_count == 0 && !saved_win.panes.is_empty() && ctx_depth > 0 {
            continue;
        }
        let mut win = Window {
            // Strip old auto-generated "Page X,Y" names — treat them as
            // unnamed so they don't pollute the command palette.
            name: if is_auto_window_name(&saved_win.name) {
                String::new()
            } else {
                saved_win.name
            },
            path: saved_win.path,
            tree: saved_win.tree,
            panes,
            focused_pane: saved_win.focused_pane,
            zoomed_pane: None,
            grid_x: saved_win.grid_x,
            grid_y: saved_win.grid_y,
            window_id: saved_win.window_id,
            context_id: saved_win.context_id,
        };
        if win.panes.is_empty() {
            win.tree = egui_tiles::Tree::empty("plexi");
            win.focused_pane = None;
        } else {
            win.reconcile_stale_tiles();
            if win
                .focused_pane
                .is_none_or(|fp| win.tree.tiles.get(fp).is_none())
            {
                win.focused_pane = win.tree.root.and_then(|root| win.find_first_pane_in(root));
            }
        }
        windows.push(win);
    }

    if windows.is_empty() {
        return None;
    }

    // Prune any sub-contexts whose windows were all dropped (e.g. all panes were
    // completed ephemeral workers), along with their Portal tiles in parent windows.
    let surviving_ctx_ids: std::collections::HashSet<u64> =
        windows.iter().map(|w| w.context_id).collect();
    let pruned_subcontexts: std::collections::HashSet<u64> = ws
        .contexts
        .iter()
        .filter(|c| c.parent_id.is_some() && !surviving_ctx_ids.contains(&c.context_id))
        .map(|c| c.context_id)
        .collect();
    if !pruned_subcontexts.is_empty() {
        for win in &mut windows {
            let portal_ids: Vec<u64> = win
                .panes
                .iter()
                .filter_map(|(id, p)| {
                    p.portal_target()
                        .filter(|target| pruned_subcontexts.contains(target))
                        .map(|_| *id)
                })
                .collect();
            for pid in portal_ids {
                win.panes.remove(&pid);
                if let Some(tile_id) = win.tree.tiles.find_pane(&pid) {
                    win.tree.remove_recursively(tile_id);
                }
            }
            if !win.panes.is_empty() {
                win.reconcile_stale_tiles();
                if win
                    .focused_pane
                    .is_none_or(|fp| win.tree.tiles.get(fp).is_none())
                {
                    win.focused_pane = win.tree.root.and_then(|root| win.find_first_pane_in(root));
                }
            } else {
                win.tree = egui_tiles::Tree::empty("plexi");
                win.focused_pane = None;
            }
        }
    }

    // Repair window_ids: old workspace files have window_id=0.
    // Assign sequential IDs so every window has a stable unique ID.
    let mut next_id: u64 = 1;
    for win in &mut windows {
        if win.window_id == 0 {
            win.window_id = next_id;
            next_id += 1;
        } else {
            next_id = next_id.max(win.window_id + 1);
        }
    }
    let mut target_active_ctx_id = initial_active_ctx_id;
    while pruned_subcontexts.contains(&target_active_ctx_id) {
        if let Some(parent_id) = ws
            .contexts
            .iter()
            .find(|c| c.context_id == target_active_ctx_id)
            .and_then(|c| c.parent_id)
        {
            target_active_ctx_id = parent_id;
        } else {
            break;
        }
    }
    let contexts: Vec<crate::host::context::Context> = ws
        .contexts
        .into_iter()
        .filter(|c| !pruned_subcontexts.contains(&c.context_id))
        .collect();
    if contexts.is_empty() {
        return None;
    }

    // Populate initial `context_state` on restored Portal panes so parent portals
    // immediately reflect child Blocked / NeedsYou / Idle states on startup.
    let subcontext_states: std::collections::HashMap<
        u64,
        crate::host::context_state::ContextState,
    > = contexts
        .iter()
        .filter(|c| c.parent_id.is_some())
        .map(|c| {
            let state = crate::host::context_state::ContextState::compute(
                c.context_id,
                &contexts,
                &windows,
            );
            (c.context_id, state)
        })
        .collect();
    for win in &mut windows {
        for pane in win.panes.values_mut() {
            if let Pane::Portal(portal) = pane {
                if let Some(state) = subcontext_states.get(&portal.target_context_id) {
                    portal.context_state = Some(state.clone());
                }
            }
        }
    }
    let active_ctx = contexts
        .iter()
        .position(|c| c.context_id == target_active_ctx_id)
        .unwrap_or(0);
    let active_ctx_id = contexts[active_ctx].context_id;
    let active = ws
        .context_active_window
        .get(&active_ctx_id)
        .and_then(|win_id| windows.iter().position(|w| w.window_id == *win_id))
        .or_else(|| windows.iter().position(|w| w.context_id == active_ctx_id))
        .unwrap_or(0);
    let window_count = windows
        .iter()
        .filter(|w| w.context_id == active_ctx_id)
        .count();

    let mut router = crate::workspace::router::WorkspaceRouter::new(contexts, active_ctx);
    // Reconstruct depth_stack so Cmd+Escape works immediately when
    // the workspace was saved while viewing a subcontext. The stack
    // is in-memory only and starts empty; without this, zoom_out
    // silently no-ops until the user manually navigates away and
    // back in (which calls push_depth).
    {
        let active_ctx_id = router.active().context_id;
        let mut ancestors: Vec<(u64, u64, Option<egui_tiles::TileId>)> = Vec::new();
        let mut cur_id = active_ctx_id;
        loop {
            let parent_id = router
                .iter()
                .find(|c| c.context_id == cur_id)
                .and_then(|c| c.parent_id);
            let Some(pid) = parent_id else { break };
            let win_id = ws
                .context_active_window
                .get(&pid)
                .and_then(|wid| {
                    windows
                        .iter()
                        .find(|w| w.window_id == *wid && w.context_id == pid)
                        .map(|w| w.window_id)
                })
                .or_else(|| {
                    windows
                        .iter()
                        .find(|w| w.context_id == pid)
                        .map(|w| w.window_id)
                })
                .unwrap_or(0);
            ancestors.push((pid, win_id, None));
            cur_id = pid;
        }
        ancestors.reverse();
        if !ancestors.is_empty() {
            log::info!(
                "workspace restore: rebuilding depth_stack ({} entries) for subcontext ctx={}",
                ancestors.len(),
                active_ctx_id
            );
            router.depth_stack = ancestors;
        }
    }

    log::info!(
        "workspace_restore: restored contexts={} windows={} active_window={} next_pane_id={}",
        router.len(),
        windows.len(),
        active,
        ws.next_pane_id
    );

    Some(RestoredWorkspace {
        router,
        windows,
        active_window: active,
        sidebar_visible: ws.sidebar_visible,
        pane_heartbeats: restored_heartbeats,
        context_active_window: ws.context_active_window,
        minimap_visible: window_count >= 2,
        next_pane_id: ws.next_pane_id,
        next_window_id: next_id,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod hidden_drain_tests {
    use super::*;

    #[test]
    fn state_change_and_config_reload_watchers_drain_while_window_hidden() {
        let mut h = crate::testing::HostHarness::new();
        let wake = std::sync::Arc::clone(&h.app.ui_wake);

        let (state_tx, state_rx) =
            ui_mailbox::UiMailbox::channel(std::sync::Arc::clone(&wake), "test_state_watch_hidden");
        h.app.state_watch_rx = state_rx;
        let _ = state_tx.send(crate::host::state_watch::StateChangedNotice {
            pane_id: 999,
            scope: crate::host::state_scope::StateScope::Global,
        });

        let (cfg_tx, cfg_rx) = ui_mailbox::UiMailbox::channel(wake, "test_config_reload_hidden");
        h.app.config_reload_rx = Some(cfg_rx);
        let _ = cfg_tx.send(());

        // Pump a logic-only hidden frame (no App::ui pass).
        h.hidden_frame();

        assert!(
            h.app.state_watch_rx.try_recv().is_err(),
            "state_watch_rx must be drained by App::logic during hidden_frame"
        );
        assert!(
            h.app.config_reload_rx.as_ref().unwrap().try_recv().is_err(),
            "config_reload_rx must be drained by App::logic during hidden_frame"
        );
    }
}
