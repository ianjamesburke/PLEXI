//! Focus, navigation, and configuration methods for PlexiApp.

use super::PendingNotification;
use super::PlexiApp;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const FOCUS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Format a Unix timestamp (seconds since epoch) as an ISO-8601 UTC string.
/// Minimal implementation with no external dependencies.
fn unix_secs_to_iso(secs: u64) -> String {
    // Days since epoch → Gregorian date via the Zeller / proleptic algorithm.
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;

    // Algorithm: http://howardhinnant.github.io/date_algorithms.html (civil_from_days)
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let mo = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if mo <= 2 { y + 1 } else { y };

    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FocusLogOutcome {
    Unchanged,
    Started,
    Transition,
    Heartbeat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FocusSegmentReason {
    PaneSwitch,
    Shutdown,
}

impl FocusSegmentReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::PaneSwitch => "pane_switch",
            Self::Shutdown => "shutdown",
        }
    }
}

/// Stable identity for a single overlay on `PlexiApp.focus_stack`.
///
/// This is a **fieldless discriminant used purely for identity** — membership
/// tests (`focus_stack` contains/last), promotion ordering, and log labels.
/// It is NOT a state-bearing dispatch enum (the enum it replaced was matched
/// on 15 arms to route keyboard + render): keyboard handling and rendering now
/// dispatch through the [`FocusOwner`]
/// trait. `FocusKind::owner()` maps a kind to its `&'static dyn FocusOwner`,
/// and the two dispatch sites in `update()` call `handle_key`/`draw` on that
/// token instead of matching 15 arms. Every overlay's working state stays on
/// `PlexiApp`, where the per-frame reconcile-from-flags helpers
/// (`reconcile_focus_layer` / `reconcile_promoted_focus_layer`) read the
/// boolean/Option flag that decides ownership *before* the owner is pushed —
/// so that state cannot move into the owner without duplicating it.
///
/// When any kind is on top, `input_captured_by_overlay()` is `true` for the
/// rest of the frame: `dispatch_app_key_events` does not run, and the top
/// owner's `handle_key` (called before its `draw`) gets first access to the
/// frame's keyboard input. Global keybinds (Cmd+Q, Cmd+W, Cmd+Shift+A) are
/// handled in `keys::poll_actions`, which always runs (see
/// `src/app/input_router.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FocusKind {
    NotificationModal,
    ConfirmClose,
    CommandPalette,
    RenamePane,
    /// Context naming modal shown when a new context is created while the
    /// sidebar is hidden. Mirrors the inline sidebar rename but as a centred
    /// overlay so the terminal is immediately usable after dismissal.
    ContextRename,
    /// Context description editor overlay.
    ContextDescription,
    /// Quick note compose modal (text input phase).
    QuickNote,
    /// First-launch CLI setup prompt. No text input — intercepts keys so they
    /// don't fall through to the active terminal while the modal is visible.
    CliSetupPrompt,
    /// Shared text-input overlay (context root, future: context rename).
    TextInput,
    /// Close-context confirmation dialog with pane inventory and dissolve option.
    ContextCloseConfirm,
    /// Capability consent modal for a focused WASM pane.
    /// Promoted to the focus stack when the focused pane has pending prompts,
    /// so the modal renders in step 2 of `update()` with exclusive keyboard
    /// ownership — before `dispatch_app_key_events` can steal Escape.
    CapabilityModal,
    /// Host event-subscription consent modal. Promoted when a CLI/MCP agent's
    /// subscribe request hit the broker's `Ask` decision and is parked in
    /// `pending_event_consents`, so the Allow/Always/Deny modal owns the
    /// keyboard before `dispatch_app_key_events` can steal Enter/Escape.
    EventConsent,
    /// Pre-launch review for raw `.wasm` path opens. Promoted before the pane is
    /// spawned so link-time host imports are remembered before wasmtime links.
    RawWasmReview,
    /// Notes picker overlay: lists workspace notes sorted by mtime, opens selected in focused text-editor.
    NotesPicker,
    /// Notes inbox triage overlay: shows inbox notes one at a time for keep/trash/action.
    NotesTriage,
}

impl FocusKind {
    /// Stable name for logging and test assertions.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::NotificationModal => "NotificationModal",
            Self::ConfirmClose => "ConfirmClose",
            Self::CommandPalette => "CommandPalette",
            Self::RenamePane => "RenamePane",
            Self::ContextRename => "ContextRename",
            Self::ContextDescription => "ContextDescription",
            Self::QuickNote => "QuickNote",
            Self::CliSetupPrompt => "CliSetupPrompt",
            Self::TextInput => "TextInput",
            Self::ContextCloseConfirm => "ContextCloseConfirm",
            Self::CapabilityModal => "CapabilityModal",
            Self::EventConsent => "EventConsent",
            Self::RawWasmReview => "RawWasmReview",
            Self::NotesPicker => "NotesPicker",
            Self::NotesTriage => "NotesTriage",
        }
    }

    /// Map this identity to its stateless [`FocusOwner`] token. The token
    /// carries no data — it only names which `PlexiApp` overlay methods this
    /// frame's keyboard/render dispatch should call.
    pub(crate) fn owner(self) -> &'static dyn FocusOwner {
        match self {
            Self::NotificationModal => &NotificationModalOwner,
            Self::ConfirmClose => &ConfirmCloseOwner,
            Self::CommandPalette => &CommandPaletteOwner,
            Self::RenamePane => &RenamePaneOwner,
            Self::ContextRename => &ContextRenameOwner,
            Self::ContextDescription => &ContextDescriptionOwner,
            Self::QuickNote => &QuickNoteOwner,
            Self::CliSetupPrompt => &CliSetupPromptOwner,
            Self::TextInput => &TextInputOwner,
            Self::ContextCloseConfirm => &ContextCloseConfirmOwner,
            Self::CapabilityModal => &CapabilityModalOwner,
            Self::EventConsent => &EventConsentOwner,
            Self::RawWasmReview => &RawWasmReviewOwner,
            Self::NotesPicker => &NotesPickerOwner,
            Self::NotesTriage => &NotesTriageOwner,
        }
    }
}

/// One overlay's keyboard + render contract. The top of `PlexiApp.focus_stack`
/// names the active owner via its [`FocusKind`]; `FocusKind::owner()` maps it
/// to the corresponding `&'static dyn FocusOwner`, and `update()` dispatches
/// this frame's keyboard input and the overlay render through these methods
/// instead of two 15-arm matches.
///
/// Owners are stateless unit structs: every overlay's working state (buffers,
/// queues, `ContextCloseState`, ...) stays on `PlexiApp`, which the trait
/// methods reach through the `&mut PlexiApp` parameter. This is a deliberate
/// consequence of the reconcile-from-flags model — the flag that decides
/// whether an overlay owns focus is read on `PlexiApp` every frame *before* the
/// owner is pushed, so the state cannot live inside the owner without being
/// duplicated. Extracting it would require inverting that model (owner owns the
/// state, no per-frame reconcile), a larger redesign out of this stint's scope.
pub(crate) trait FocusOwner {
    /// Handle this frame's owned keyboard input, before any render pass. `ctx`
    /// is only read by `NotesTriage` (to surrender egui widget focus before it
    /// reads keys); the other owners ignore it.
    fn handle_key(
        &self,
        app: &mut PlexiApp,
        ctx: &egui::Context,
        input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition;

    /// Render the overlay (visual only — key reads already happened in
    /// `handle_key`). Returns any commands the render produced; only the
    /// notification modal emits any, the rest return an empty vec.
    fn draw(
        &self,
        app: &mut PlexiApp,
        ctx: &egui::Context,
    ) -> Vec<crate::app::app_trait::AppCommand>;
}

/// Define the stateless owner tokens whose `handle_key` returns a
/// `KeyDisposition` and whose `draw` produces no commands — the common shape
/// for 12 of the 15 overlays. The three exceptions (notification modal's
/// command-returning draw; NotesPicker / NotesTriage's forced `Passthrough`,
/// and NotesTriage's extra `ctx`) are written out by hand below.
macro_rules! focus_owner_tokens {
    ($( $kind:ident => $token:ident { key: $key_fn:ident, draw: $draw_fn:ident } ),* $(,)?) => {
        $(
            pub(crate) struct $token;
            impl FocusOwner for $token {
                fn handle_key(
                    &self,
                    app: &mut PlexiApp,
                    _ctx: &egui::Context,
                    input: &mut crate::app::input_router::PlexiInput,
                ) -> crate::app::app_trait::KeyDisposition {
                    app.$key_fn(input)
                }
                fn draw(
                    &self,
                    app: &mut PlexiApp,
                    ctx: &egui::Context,
                ) -> Vec<crate::app::app_trait::AppCommand> {
                    app.$draw_fn(ctx);
                    Vec::new()
                }
            }
        )*
    };
}

focus_owner_tokens! {
    ConfirmClose => ConfirmCloseOwner { key: confirm_close_handle_key, draw: draw_confirm_close },
    CommandPalette => CommandPaletteOwner { key: command_palette_handle_key, draw: draw_command_palette },
    RenamePane => RenamePaneOwner { key: rename_pane_handle_key, draw: draw_rename_pane_overlay },
    ContextRename => ContextRenameOwner { key: context_rename_handle_key, draw: draw_rename_context_overlay },
    ContextDescription => ContextDescriptionOwner { key: context_description_handle_key, draw: draw_edit_description_overlay },
    QuickNote => QuickNoteOwner { key: quick_note_handle_key, draw: draw_quick_note_modal },
    CliSetupPrompt => CliSetupPromptOwner { key: cli_setup_prompt_handle_key, draw: draw_cli_setup_modal },
    TextInput => TextInputOwner { key: text_input_handle_key, draw: draw_text_input_overlay },
    ContextCloseConfirm => ContextCloseConfirmOwner { key: context_close_confirm_handle_key, draw: draw_context_close_confirm },
    CapabilityModal => CapabilityModalOwner { key: capability_modal_handle_key, draw: draw_capability_modal },
    EventConsent => EventConsentOwner { key: event_consent_handle_key, draw: draw_event_consent_modal },
    RawWasmReview => RawWasmReviewOwner { key: raw_wasm_review_handle_key, draw: draw_raw_wasm_review_modal },
}

/// Notification modal — the only overlay whose render produces commands
/// (`DeliverNotifyAction`, routed back to the originating pane).
pub(crate) struct NotificationModalOwner;
impl FocusOwner for NotificationModalOwner {
    fn handle_key(
        &self,
        app: &mut PlexiApp,
        _ctx: &egui::Context,
        input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        app.notification_modal_handle_key(input)
    }
    fn draw(
        &self,
        app: &mut PlexiApp,
        ctx: &egui::Context,
    ) -> Vec<crate::app::app_trait::AppCommand> {
        app.draw_notification_modal(ctx)
    }
}

/// Notes picker — its handler drives selection but never claims the key, so the
/// key still reaches the focused text-editor pane (forced `Passthrough`).
pub(crate) struct NotesPickerOwner;
impl FocusOwner for NotesPickerOwner {
    fn handle_key(
        &self,
        app: &mut PlexiApp,
        _ctx: &egui::Context,
        input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        app.notes_picker_handle_key(input);
        crate::app::app_trait::KeyDisposition::Passthrough
    }
    fn draw(
        &self,
        app: &mut PlexiApp,
        ctx: &egui::Context,
    ) -> Vec<crate::app::app_trait::AppCommand> {
        app.draw_notes_picker(ctx);
        Vec::new()
    }
}

/// Notes triage — like the picker, forces `Passthrough`, and its handler needs
/// `ctx` to surrender egui widget focus before reading keys.
pub(crate) struct NotesTriageOwner;
impl FocusOwner for NotesTriageOwner {
    fn handle_key(
        &self,
        app: &mut PlexiApp,
        ctx: &egui::Context,
        input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        app.notes_triage_handle_key(ctx, input);
        crate::app::app_trait::KeyDisposition::Passthrough
    }
    fn draw(
        &self,
        app: &mut PlexiApp,
        ctx: &egui::Context,
    ) -> Vec<crate::app::app_trait::AppCommand> {
        app.draw_notes_triage(ctx);
        Vec::new()
    }
}

/// A single pane entry shown in the context-close confirmation dialog.
#[derive(Clone, Debug)]
pub(crate) struct ContextCloseItem {
    pub kind: &'static str,
    pub name: String,
}

/// State for the context-close confirmation dialog.
#[derive(Clone, Debug)]
pub(crate) struct ContextCloseState {
    pub context_id: u64,
    pub context_name: String,
    pub items: Vec<ContextCloseItem>,
    /// True when a Portal tile in the active window targets this context —
    /// exactly `dissolve_portal`'s precondition. False for a top-level
    /// context, where dissolving early-returns and does nothing, so the
    /// confirm modal must not offer the action at all.
    pub can_dissolve: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingRawWasmLaunch {
    pub app_id: String,
    pub wasm_path: PathBuf,
    pub workspace_root: PathBuf,
    pub missing_capabilities: Vec<String>,
    pub launch_args: Vec<String>,
}

/// Pane metadata snapshot used for both journal checkpoints and event emission.
pub(crate) struct PaneMetadata {
    pub pane_id: u64,
    pub context_name: String,
    pub context_description: String,
    pub context_root: Option<String>,
    pub cwd: Option<String>,
    pub pty_title: Option<String>,
    pub pane_name: Option<String>,
    pub app_type_id: Option<String>,
}

impl PlexiApp {
    /// Collect metadata for the pane at `tile_id` in the window identified by
    /// stable `window_id`. Returns `None` if the window or tile is missing.
    pub(super) fn collect_pane_metadata(
        &self,
        window_id: u64,
        tile_id: egui_tiles::TileId,
    ) -> Option<PaneMetadata> {
        use egui_tiles::Tile;
        let win = self.windows.iter().find(|w| w.window_id == window_id)?;
        let pane_id = match win.tree.tiles.get(tile_id) {
            Some(Tile::Pane(id)) => *id,
            _ => return None,
        };
        let pane = win.panes.get(&pane_id)?;
        let context_name = self.context_name_for(win.context_id);
        let context_description = self.context_description_for(win.context_id);
        let context_root = self
            .context_root_for(win.context_id)
            .map(|p| p.to_string_lossy().into_owned());

        let (cwd, pty_title, pane_name, app_type_id) = match pane {
            crate::host::pane::Pane::Terminal(t) => {
                let cwd = crate::host::shell::get_pid_cwd(t.backend.child_pid())
                    .map(|p| p.to_string_lossy().into_owned());
                (cwd, t.pty_title.clone(), t.name.clone(), None)
            }
            crate::host::pane::Pane::App(a) => {
                let cwd = Some(a.workspace_root.to_string_lossy().into_owned());
                let type_id = Some(a.manifest_id.clone());
                (cwd, None, None, type_id)
            }
            crate::host::pane::Pane::Portal(_) => (None, None, None, None),
        };

        Some(PaneMetadata {
            pane_id,
            context_name,
            context_description,
            context_root,
            cwd,
            pty_title,
            pane_name,
            app_type_id,
        })
    }

    /// Collect metadata and emit a `FocusChanged` event. Called when the
    /// focused pane changes and on shutdown. Clears the focus journal on clean
    /// transitions so crash-recovery only fires if the process was killed.
    pub(super) fn emit_focus_changed_for_tile(
        &self,
        window_id: u64,
        tile_id: egui_tiles::TileId,
        duration_secs: u64,
        reason: FocusSegmentReason,
    ) {
        let Some(meta) = self.collect_pane_metadata(window_id, tile_id) else {
            return;
        };

        log::info!(
            "focus_changed: pane_id={} reason={} context={:?} context_root={:?} duration_secs={duration_secs} pty_title={:?} pane_name={:?} app_type_id={:?}",
            meta.pane_id,
            reason.as_str(),
            meta.context_name,
            meta.context_root,
            meta.pty_title,
            meta.pane_name,
            meta.app_type_id,
        );
        // On a clean close the journal is no longer needed — delete it so we
        // don't emit a spurious crash_recovery on the next startup.
        crate::app::focus_journal::clear_journal(&self.focus_journal_path);

        crate::host::event_log::emit(crate::host::event_log::HostEvent::FocusChanged {
            pane_id: meta.pane_id,
            context_name: meta.context_name,
            context_description: meta.context_description,
            context_root: meta.context_root,
            cwd: meta.cwd,
            pty_title: meta.pty_title,
            pane_name: meta.pane_name,
            app_type_id: meta.app_type_id,
            reason: Some(reason.as_str().to_string()),
            duration_secs,
            timestamp: crate::host::event_log::now_timestamp(),
        });
    }

    /// Write (or overwrite) the focus journal checkpoint for the pane currently
    /// in focus. The journal records the segment start time plus a `last_checkpoint_at`
    /// so crash recovery can compute duration to now.
    ///
    /// `segment_start` is the `Instant` when this focus segment began. We convert
    /// it to a wall-clock ISO timestamp by subtracting elapsed time from now.
    pub(super) fn write_focus_journal_checkpoint(
        &self,
        window_id: u64,
        tile_id: egui_tiles::TileId,
        segment_start: std::time::Instant,
    ) {
        let Some(meta) = self.collect_pane_metadata(window_id, tile_id) else {
            return;
        };

        // Convert Instant → wall clock by anchoring to SystemTime::now().
        let elapsed_since_start = segment_start.elapsed();
        let started_at_wall = std::time::SystemTime::now()
            .checked_sub(elapsed_since_start)
            .unwrap_or(std::time::SystemTime::now());

        let to_iso = |t: std::time::SystemTime| -> String {
            let secs = t
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            // Minimal ISO-8601 UTC formatter without external deps.
            unix_secs_to_iso(secs)
        };

        let entry = crate::app::focus_journal::FocusJournalEntry {
            pane_id: meta.pane_id,
            context_name: meta.context_name,
            context_description: meta.context_description,
            context_root: meta.context_root,
            cwd: meta.cwd,
            pty_title: meta.pty_title,
            pane_name: meta.pane_name,
            app_type_id: meta.app_type_id,
            started_at: to_iso(started_at_wall),
            last_checkpoint_at: to_iso(std::time::SystemTime::now()),
        };
        crate::app::focus_journal::write_checkpoint(&self.focus_journal_path, &entry);
    }

    pub(crate) fn current_focus_target(&self) -> Option<(u64, egui_tiles::TileId)> {
        self.windows
            .get(self.active_window)
            .and_then(|win| win.focused_pane.map(|tile| (win.window_id, tile)))
    }

    pub(crate) fn reconcile_focus_logging(
        &mut self,
        heartbeat_interval: Duration,
    ) -> FocusLogOutcome {
        let current_focus = self.current_focus_target();
        let now = std::time::Instant::now();

        if current_focus != self.last_logged_focus {
            let had_previous_focus = self.last_logged_focus.is_some();
            if let Some((window_id, tile_id)) = self.last_logged_focus {
                let duration_secs = self
                    .focus_started_at
                    .and_then(|started| now.checked_duration_since(started))
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0);
                self.emit_focus_changed_for_tile(
                    window_id,
                    tile_id,
                    duration_secs,
                    FocusSegmentReason::PaneSwitch,
                );
            }
            self.last_logged_focus = current_focus;
            self.focus_started_at = current_focus.map(|_| now);
            // Start a journal checkpoint for the new focus segment.
            if let Some((window_id, tile_id)) = current_focus {
                self.write_focus_journal_checkpoint(window_id, tile_id, now);
            }
            return if had_previous_focus {
                FocusLogOutcome::Transition
            } else {
                FocusLogOutcome::Started
            };
        }

        let Some((window_id, tile_id)) = current_focus else {
            return FocusLogOutcome::Unchanged;
        };
        let Some(started_at) = self.focus_started_at else {
            self.focus_started_at = Some(now);
            return FocusLogOutcome::Started;
        };
        let Some(elapsed) = now.checked_duration_since(started_at) else {
            self.focus_started_at = Some(now);
            return FocusLogOutcome::Unchanged;
        };
        if elapsed < heartbeat_interval {
            return FocusLogOutcome::Unchanged;
        }

        // Heartbeat: update the journal checkpoint only — do NOT emit to events.jsonl.
        // Do NOT reset focus_started_at so the journal always records the full
        // segment start; crash recovery can then compute the true total duration.
        self.write_focus_journal_checkpoint(window_id, tile_id, started_at);
        FocusLogOutcome::Heartbeat
    }

    /// Returns `(app_active, keyboard_capture_active)` for the pane focused in
    /// the active window. `app_active` is true whenever the focused pane is
    /// running an app surface at all; `keyboard_capture_active` mirrors that
    /// pane's own declared `App::keyboard_capture()` (e.g. file-browser
    /// rename/quick-look mode, a CLI-backed app's Form view) — an advisory
    /// policy flag `keys::poll_actions` reads to suppress global shortcuts
    /// while the app owns text input. This is intentionally separate from
    /// `focus_stack`/`FocusKind`: overlay layers are exclusive (they block
    /// `dispatch_app_key_events` entirely), while app-declared capture must
    /// NOT block the app from still receiving its own keys — folding the two
    /// into one stack would make an app go deaf on the frame it starts
    /// capturing. Deduplicates what was previously an inline block
    /// re-computed at the `poll_actions` call site.
    pub(crate) fn focused_app_capture_state(&self) -> (bool, bool) {
        let context = &self.windows[self.active_window];
        let focused_pane = context.focused_pane.and_then(|tile_id| {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = context.tree.tiles.get(tile_id) {
                context.panes.get(pane_id)
            } else {
                None
            }
        });
        let active = focused_pane
            .map(|pane| pane.as_app().is_some())
            .unwrap_or(false);
        let capture = if active {
            focused_pane
                .and_then(|p| p.as_app())
                .map(|a| a.runtime.keyboard_capture())
                .unwrap_or(false)
        } else {
            false
        };
        (active, capture)
    }

    /// Returns `true` when the current top focus layer is a non-critical modal
    /// that QuickNote (Cmd+0) is allowed to dismiss and replace.
    ///
    /// Critical modals (`ConfirmClose`, `CapabilityModal`, `ContextCloseConfirm`)
    /// require explicit user acknowledgement and must NOT be preempted.
    /// Non-critical modals (`NotificationModal`, `CommandPalette`) can be safely
    /// dismissed so QuickNote can open on top.
    pub(crate) fn is_quick_note_preemptable(&self) -> bool {
        matches!(
            self.focus_stack.last(),
            Some(FocusKind::NotificationModal) | Some(FocusKind::CommandPalette)
        )
    }

    /// Dismiss the current non-critical modal so QuickNote can open on top.
    /// Only call after `is_quick_note_preemptable()` returns `true`.
    pub(crate) fn dismiss_preemptable_modal(&mut self) {
        match self.focus_stack.last().cloned() {
            Some(FocusKind::NotificationModal) => {
                log::info!("quick_note: dismissing NotificationModal to open QuickNote");
                self.show_notification_modal = false;
                self.focus_stack
                    .retain(|l| *l != FocusKind::NotificationModal);
            }
            Some(FocusKind::CommandPalette) => {
                log::info!("quick_note: dismissing CommandPalette to open QuickNote");
                self.show_command_palette = false;
                self.focus_stack.retain(|l| *l != FocusKind::CommandPalette);
            }
            _ => {}
        }
    }

    /// True when a host overlay surface owns keyboard input: any modal on the
    /// focus stack, or the inline sidebar rename editor. Thin wrapper over
    /// [`Self::overlay_surface`] (stint 0429) — the one derivation of overlay
    /// ownership. Used by `update()` to keep key events away from panes.
    pub(crate) fn input_captured_by_overlay(&self) -> bool {
        self.overlay_surface().is_some()
    }

    /// Push a focus layer. Idempotent — if the same layer is already on top,
    /// it's a no-op. Callers should pair with `pop_focus_layer`.
    pub(crate) fn push_focus_layer(&mut self, layer: FocusKind) {
        if self.focus_stack.last() != Some(&layer) {
            self.focus_stack.push(layer);
        }
    }

    /// Pop the given layer if it's currently on top. No-op otherwise; this
    /// prevents out-of-order pops from corrupting the stack.
    pub(crate) fn pop_focus_layer(&mut self, layer: &FocusKind) {
        if self.focus_stack.last() == Some(layer) {
            self.focus_stack.pop();
        }
    }

    /// Shared reconciliation for a plain boolean-backed focus layer: push it
    /// when `should_own` becomes true and it isn't already in the stack, pop
    /// it (via `retain`, so a stale entry buried under something else is still
    /// removed) when `should_own` goes false. Every non-promoting `sync_*`
    /// function is a one-line call to this — see `FocusKind` doc comment for
    /// why this replaces 11 near-identical bodies instead of dyn dispatch.
    fn reconcile_focus_layer(&mut self, layer: FocusKind, should_own: bool) {
        let has_layer = self.focus_stack.contains(&layer);
        if should_own && !has_layer {
            self.push_focus_layer(layer);
        } else if !should_own && has_layer {
            log::info!("focus: {} layer removed by sync (retain)", layer.name());
            self.focus_stack.retain(|l| *l != layer);
        }
    }

    /// Shared reconciliation for a "promoted" focus layer: a queue-backed
    /// overlay (capability prompts, event consents, raw-wasm review) that
    /// must jump back to the top of the stack even if another layer pushed on
    /// top of it while it was buried, not just toggle membership.
    fn reconcile_promoted_focus_layer(&mut self, layer: FocusKind, should_own: bool) {
        let has_layer = self.focus_stack.contains(&layer);
        let is_top = self.focus_stack.last() == Some(&layer);
        if should_own && !is_top {
            self.focus_stack.retain(|l| *l != layer);
            log::info!("focus: {} promoted to top", layer.name());
            self.push_focus_layer(layer);
        } else if !should_own && has_layer {
            log::info!("focus: {} layer released", layer.name());
            self.focus_stack.retain(|l| *l != layer);
        }
    }

    /// Reconcile the focus stack with the notification modal visibility. Called
    /// once per frame — the modal can open/close from many paths (arrival,
    /// Cmd+Shift+A, queue drains to empty mid-frame), so the source of truth is
    /// `show_notification_modal && !pending_notifications.is_empty()`.
    /// Read `confirm_quit` from the config tunnel. Defaults to `true` so
    /// users get the safer triple-tap behavior unless they explicitly opt out.
    pub(crate) fn confirm_quit(&self) -> bool {
        self.config.confirm_quit.unwrap_or(true)
    }

    /// Read `confirm_close` from the config tunnel. Defaults to `false` so
    /// pane close is instant unless the user explicitly enables the dialog.
    pub(crate) fn confirm_close(&self) -> bool {
        self.config.confirm_close.unwrap_or(false)
    }

    /// If the currently focused pane is an app with a non-empty nav stack,
    /// emit `PlexiEvent::NavBack` to it and return `true`. Returns `false`
    /// when there is no nav-active focused pane (caller may fall back to
    /// default behaviour such as closing the pane or cycling tabs).
    pub(crate) fn try_nav_back_focused(&mut self) -> bool {
        let active = self.active_window;
        let active_ctx = &self.windows[active];

        // Read nav state under shared borrow first.
        let nav_result = active_ctx.focused_pane.and_then(|tile_id| {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = active_ctx.tree.tiles.get(tile_id) {
                active_ctx.panes.get(pane_id).and_then(|pane| {
                    pane.as_app().and_then(|app| {
                        if app.runtime.nav_stack_depth() > 0 {
                            Some((*pane_id, app.runtime.nav_back_view_id()))
                        } else {
                            None
                        }
                    })
                })
            } else {
                None
            }
        });

        if let Some((pane_id, view_id)) = nav_result {
            if let Some(pane) = self.windows[active].panes.get_mut(&pane_id) {
                if let Some(app) = pane.as_app_mut() {
                    app.runtime
                        .queue_outbound_event(crate::app_protocol::PlexiEvent::NavBack { view_id });
                }
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn push_focus_history(
        &mut self,
        window_id: u64,
        old_focus: Option<egui_tiles::TileId>,
    ) {
        if self.navigating_history {
            return;
        }
        let Some(tile_id) = old_focus else { return };
        self.pane_focus_history.push((window_id, tile_id));
        if self.pane_focus_history.len() > self.focus_history_depth {
            self.pane_focus_history.remove(0);
        }
        self.pane_focus_future.clear();
        log::info!(
            "focus_history: recorded window={window_id} tile={tile_id:?} history_len={}",
            self.pane_focus_history.len()
        );
    }

    /// Step backward through pane focus history (Cmd+[).
    /// Skips stale entries where the window or tile no longer exists.
    pub(crate) fn step_focus_history_back(&mut self) {
        self.navigating_history = true;
        loop {
            let Some((window_id, tile_id)) = self.pane_focus_history.pop() else {
                log::info!("focus_history: back exhausted");
                self.navigating_history = false;
                return;
            };
            let window_idx = self.windows.iter().position(|w| w.window_id == window_id);
            let Some(idx) = window_idx else {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (window gone)");
                continue;
            };
            if self.windows[idx].tree.tiles.get(tile_id).is_none() {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (tile gone)");
                continue;
            }
            let ctx_id = self.windows[idx].context_id;
            self.save_minimap_before_context_navigation(ctx_id);

            // Save current focus to future stack before navigating.
            let current_window_id = self.windows[self.active_window].window_id;
            if let Some(current_tile) = self.windows[self.active_window].focused_pane {
                self.pane_focus_future
                    .push((current_window_id, current_tile));
                if self.pane_focus_future.len() > self.focus_history_depth {
                    self.pane_focus_future.remove(0);
                }
            }
            self.windows[idx].navigate_to(tile_id);
            self.active_window = idx;
            // Sync sidebar: router active must match the context of the window we navigated to.
            if let Some(ctx_idx) = self.router.position(|c| c.context_id == ctx_id) {
                self.router.set_active(ctx_idx);
                self.reload_config_for_active_context();
            }
            self.context_active_window.insert(ctx_id, window_id);
            self.restore_minimap_for_context(ctx_id);
            log::info!(
                "focus_history: back — to window={window_id} tile={tile_id:?} ctx={ctx_id} minimap_visible={} history_len={}",
                self.minimap.visible,
                self.pane_focus_history.len()
            );
            break;
        }
        self.navigating_history = false;
    }

    /// Step forward through pane focus future (Cmd+]).
    /// Skips stale entries where the window or tile no longer exists.
    pub(crate) fn step_focus_history_forward(&mut self) {
        self.navigating_history = true;
        loop {
            let Some((window_id, tile_id)) = self.pane_focus_future.pop() else {
                log::info!("focus_history: forward exhausted");
                self.navigating_history = false;
                return;
            };
            let window_idx = self.windows.iter().position(|w| w.window_id == window_id);
            let Some(idx) = window_idx else {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (window gone)");
                continue;
            };
            if self.windows[idx].tree.tiles.get(tile_id).is_none() {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (tile gone)");
                continue;
            }
            let ctx_id = self.windows[idx].context_id;
            self.save_minimap_before_context_navigation(ctx_id);

            // Save current focus to history stack before navigating.
            let current_window_id = self.windows[self.active_window].window_id;
            if let Some(current_tile) = self.windows[self.active_window].focused_pane {
                self.pane_focus_history
                    .push((current_window_id, current_tile));
                if self.pane_focus_history.len() > self.focus_history_depth {
                    self.pane_focus_history.remove(0);
                }
            }
            self.windows[idx].navigate_to(tile_id);
            self.active_window = idx;
            // Sync sidebar: router active must match the context of the window we navigated to.
            if let Some(ctx_idx) = self.router.position(|c| c.context_id == ctx_id) {
                self.router.set_active(ctx_idx);
                self.reload_config_for_active_context();
            }
            self.context_active_window.insert(ctx_id, window_id);
            self.restore_minimap_for_context(ctx_id);
            log::info!(
                "focus_history: forward — to window={window_id} tile={tile_id:?} ctx={ctx_id} minimap_visible={} future_len={}",
                self.minimap.visible,
                self.pane_focus_future.len()
            );
            break;
        }
        self.navigating_history = false;
    }

    fn save_minimap_before_context_navigation(&mut self, target_ctx_id: u64) {
        let old_ctx_id = self.windows[self.active_window].context_id;
        if old_ctx_id != target_ctx_id {
            self.context_active_window
                .insert(old_ctx_id, self.windows[self.active_window].window_id);
        }
        self.minimap_visible_per_context
            .insert(old_ctx_id, self.minimap.visible);
    }

    fn restore_minimap_for_context(&mut self, ctx_id: u64) {
        let page_count = self
            .windows
            .iter()
            .filter(|w| w.context_id == ctx_id)
            .count();
        self.minimap.visible = self
            .minimap_visible_per_context
            .get(&ctx_id)
            .copied()
            .unwrap_or(page_count > 1);
    }

    /// Re-read configuration from disk and apply changes that can take
    /// effect without a restart (theme, font size, notification settings,
    /// confirmation toggles). Logs the reload so the user knows it worked.
    pub(crate) fn reload_config(&mut self) {
        self.reload_config_for_active_context();
    }

    pub(crate) fn reload_config_for_active_context(&mut self) {
        let active_workspace = Some(self.router.active().root.clone());
        self.sync_app_registry_for_active_context(active_workspace.as_deref());
        self.reload_config_for_workspace(active_workspace.as_deref());
    }

    pub(crate) fn reload_app_registry_for_root(&mut self, root: &Path) {
        log::info!(
            "app_registry: rescanning registry for root={}",
            root.display()
        );
        self.registry = crate::app::registry::AppRegistry::load(root);
        match crate::app::registry_watcher::start(
            crate::app::registry::registry_watch_dirs(root),
            std::sync::Arc::clone(&self.ui_wake),
        ) {
            Some((watcher, rx)) => {
                self._registry_watcher = Some(watcher);
                self.registry_reload_rx = Some(rx);
            }
            None => {
                self._registry_watcher = None;
                self.registry_reload_rx = None;
            }
        }
    }

    fn sync_app_registry_for_active_context(&mut self, active_workspace: Option<&Path>) {
        let fallback;
        let root = match active_workspace {
            Some(root) => root,
            None => {
                fallback = std::env::current_dir()
                    .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
                fallback.as_path()
            }
        };
        let expected_workspace = crate::app::registry::resolve_workspace_root(root);
        if self.registry.loaded_workspace.as_ref() != expected_workspace.as_ref() {
            self.reload_app_registry_for_root(root);
        }
    }

    pub(crate) fn reload_config_for_workspace(&mut self, active_workspace: Option<&Path>) {
        let mut all_diags = crate::config::validate_from_path(&crate::config::config_path());
        if let Some(root) = active_workspace {
            let project_path = root
                .join(crate::config::workspace_channel_dir())
                .join("config.toml");
            all_diags.extend(crate::config::validate_from_path(&project_path));
        }

        let has_errors = all_diags.iter().any(|d| d.is_error());

        let warnings: Vec<_> = all_diags.iter().filter(|d| !d.is_error()).collect();
        if !warnings.is_empty() {
            let body = warnings
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            log::warn!("config: unknown keys found:\n{body}");
        }

        if has_errors {
            let error_msg = all_diags
                .iter()
                .filter(|d| d.is_error())
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            log::warn!("config: parse error, keeping current config:\n{error_msg}");
            let notify_id = format!(
                "config-error-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            );
            self.enqueue_notification(
                crate::app::notifications::NotifySource::HostInternal,
                PendingNotification {
                    notify_id,
                    sender_pane_id: 0,
                    source_context_id: 0,
                    source_window_id: 0,
                    title: "Config Error".to_string(),
                    body: error_msg,
                    kind: crate::app_protocol::NotifyKind::Message,
                    options: vec![],
                    input_prompt: None,
                    required: false,
                    // A broken config is not a property of one context — it
                    // affects the whole workspace, so this stays the explicit
                    // global case rather than taking the shared default.
                    scope: crate::app_protocol::NotifyScope::Global,
                    image_inline: None,
                    image_pipe_id: None,
                    response_file: None,
                    timeout_secs: None,
                    on_dismiss: None,
                    enqueued_at: std::time::Instant::now(),
                    tombstoned: false,
                    deliver_after: None,
                },
            );
            return;
        }

        let fresh = crate::config::PlexiConfig::load_with_workspace(active_workspace);

        // Log level — applies live; the fern filter reads it atomically per record.
        let new_level = fresh
            .log
            .as_ref()
            .and_then(|l| l.level_filter())
            .unwrap_or(log::LevelFilter::Info);
        if crate::platform::logging::set_level(new_level) {
            log::warn!("log: level changed to {new_level} (config reload)");
        }

        // Theme
        let theme_cfg = Self::resolve_theme_config(&fresh);
        let new_colors = crate::ui::theme::Colors::from_config(&theme_cfg);
        if self.colors != new_colors {
            self.colors = new_colors;
            let dark_mode = !crate::ui::theme::is_light_preset(
                fresh
                    .theme
                    .as_ref()
                    .and_then(|t| t.preset.as_deref())
                    .unwrap_or(""),
            );
            crate::ui::theme::setup_style(&self.ctx, &new_colors, dark_mode);
            let window_theme = if dark_mode {
                egui::SystemTheme::Dark
            } else {
                egui::SystemTheme::Light
            };
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::SetTheme(window_theme));
            log::info!("theme: set_window_theme dark_mode={dark_mode} (config reload)");
            self.broadcast_theme_event();
        }

        // Terminal theme
        self.theme = crate::ui::theme::terminal_theme(&theme_cfg);

        // Font size
        if let Some(size) = fresh.font_size {
            if (size - self.default_font_size).abs() > 0.01 {
                self.default_font_size = size;
            }
        }

        // Notifications
        self.notifications_enabled = fresh
            .notifications
            .as_ref()
            .and_then(|n| n.enabled)
            .unwrap_or(true);
        self.notifications_focus_mode = fresh
            .notifications
            .as_ref()
            .and_then(|n| n.focus_mode)
            .unwrap_or(false);
        self.notifications_sound = fresh.notifications.as_ref().and_then(|n| n.cue_sound());

        self.focus_history_depth = fresh.focus_history_depth.unwrap_or(100);

        // Feature flags
        self.features = crate::features::FeatureFlags::from_config(&fresh);

        // Replace the cached config
        self.config = fresh;
        self.key_bindings = crate::host::keys::build_key_bindings(self.config.keybindings.as_ref());
        self.binding_table = crate::host::keys::build_binding_table(&self.key_bindings);
        log::info!("keybindings: rebuilt after config reload");

        log::info!(
            "Configuration reloaded from disk. active_workspace={}",
            active_workspace
                .map(|root| root.display().to_string())
                .unwrap_or_else(|| "<none>".to_string())
        );
    }

    /// Push the current host `Colors` to every running app as a `Theme` event.
    /// Called after `self.colors` is updated on config hot-reload.
    pub(crate) fn broadcast_theme_event(&mut self) {
        let event = crate::app_protocol::PlexiEvent::Theme {
            colors: self.colors.to_theme_map(),
        };
        let mut delivered = 0;
        for window in &mut self.windows {
            for pane in window.panes.values_mut() {
                if let Some(app) = pane.as_app_mut() {
                    app.runtime.queue_outbound_event(event.clone());
                    delivered += 1;
                }
            }
        }
        for (_, app) in self.background_apps.values_mut() {
            app.queue_outbound_event(event.clone());
            delivered += 1;
        }
        log::info!("theme: broadcast Theme event to {delivered} running apps");
    }

    /// Reconcile the confirm-close focus layer with `pending_close`. Mirrors
    /// `sync_notification_modal_focus` — the source of truth is a boolean
    /// toggled from multiple paths, and the focus stack must follow it
    /// deterministically each frame.
    pub(crate) fn sync_confirm_close_focus(&mut self) {
        self.reconcile_focus_layer(FocusKind::ConfirmClose, self.pending_close);
    }

    pub(crate) fn sync_context_close_focus(&mut self) {
        self.reconcile_focus_layer(
            FocusKind::ContextCloseConfirm,
            self.pending_context_close.is_some(),
        );
    }

    /// Returns the `context_id` of the child context if the focused pane is a Portal tile.
    pub(crate) fn get_focused_portal_context_id(&self) -> Option<u64> {
        let win = &self.windows[self.active_window];
        let focused_tile = win.focused_pane?;
        let pane_id = match win.tree.tiles.get(focused_tile) {
            Some(egui_tiles::Tile::Pane(id)) => *id,
            _ => return None,
        };
        win.panes.get(&pane_id)?.portal_target()
    }

    /// Collect the pane inventory for a child context close dialog.
    pub(crate) fn build_context_close_state(&self, context_id: u64) -> ContextCloseState {
        let context_name = self
            .router
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        let mut items = Vec::new();
        for win in &self.windows {
            if win.context_id != context_id {
                continue;
            }
            let mut pane_entries: Vec<_> = win.panes.iter().collect();
            pane_entries.sort_by_key(|(id, _)| *id);
            for (_, pane) in pane_entries {
                match pane {
                    crate::host::pane::Pane::Terminal(t) => {
                        let name = t
                            .name
                            .clone()
                            .or_else(|| t.pty_title.clone())
                            .unwrap_or_else(|| "Terminal".to_string());
                        items.push(ContextCloseItem {
                            kind: "Terminal",
                            name,
                        });
                    }
                    crate::host::pane::Pane::App(a) => {
                        items.push(ContextCloseItem {
                            kind: "App",
                            name: a.name.clone(),
                        });
                    }
                    crate::host::pane::Pane::Portal(p) => {
                        let name = self
                            .router
                            .iter()
                            .find(|c| c.context_id == p.target_context_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|| "Portal".to_string());
                        items.push(ContextCloseItem {
                            kind: "Context",
                            name,
                        });
                    }
                }
            }
        }

        let can_dissolve = self.context_has_portal(context_id);
        log::info!(
            "context_close: prompt ctx={context_id} name={context_name:?} panes={} can_dissolve={can_dissolve}",
            items.len(),
        );

        ContextCloseState {
            context_id,
            context_name,
            items,
            can_dissolve,
        }
    }

    pub(crate) fn sync_notification_modal_focus(&mut self) {
        self.reconcile_focus_layer(FocusKind::NotificationModal, self.show_notification_modal);
    }

    pub(crate) fn sync_cli_setup_prompt_focus(&mut self) {
        self.reconcile_focus_layer(FocusKind::CliSetupPrompt, self.show_cli_setup_prompt);
    }

    /// Reconcile the command-palette focus layer with `show_command_palette`.
    /// Same pattern as the notification modal: boolean visibility flag is the
    /// source of truth, focus stack follows it deterministically each frame.
    pub(crate) fn sync_command_palette_focus(&mut self) {
        self.reconcile_focus_layer(FocusKind::CommandPalette, self.show_command_palette);
    }

    /// Every note currently open in an editor pane, across all windows, newest
    /// modification first. This is the command palette's note corpus — the
    /// palette is a switcher for what is already open, while the Cmd+O picker
    /// stays the browse-everything surface that scans inbox and workspace.
    ///
    /// The same note open in two panes yields one entry.
    pub(crate) fn open_note_entries(&self) -> Vec<crate::notes::NotePickerEntry> {
        let notes_base = crate::config::config_dir().join("notes");
        let inbox_dir = notes_base.join("inbox");

        let mut seen = std::collections::HashSet::new();
        let mut paths: Vec<PathBuf> = Vec::new();
        for win in &self.windows {
            let mut pane_entries: Vec<_> = win.panes.iter().collect();
            // Pane id order keeps the pre-sort deterministic when two notes
            // share an mtime.
            pane_entries.sort_by_key(|(id, _)| *id);
            for (_, pane) in pane_entries {
                let Some(app) = pane.as_app() else { continue };
                let crate::host::pane::AppRuntime::Builtin(builtin) = &app.runtime else {
                    continue;
                };
                let Some(path) = builtin.open_note_path() else {
                    continue;
                };
                let identity = crate::app::text_editor_app::note_path_identity(path);
                if seen.insert(identity) {
                    paths.push(path.to_path_buf());
                }
            }
        }

        // Preserve the picker's newest-first ordering. One stat per open note,
        // not a directory walk.
        let mut with_mtime: Vec<(Option<std::time::SystemTime>, PathBuf)> = paths
            .into_iter()
            .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).ok(), p))
            .collect();
        with_mtime.sort_by_key(|e| std::cmp::Reverse(e.0));

        with_mtime
            .into_iter()
            .filter_map(|(_, path)| {
                let inbox = path.parent() == Some(inbox_dir.as_path());
                crate::notes::NotePickerEntry::load(&path, inbox)
            })
            .collect()
    }

    /// Navigate to a pane by id, updating both `focused_pane` on its window and
    /// `active_window`. Returns `true` if the pane was found.
    pub(crate) fn pane_navigate(&mut self, pane_id: u64) -> bool {
        // Read-only pass: find the window index, tile_id, and context_id before mutating.
        // Using iter() instead of iter_mut() so self.windows borrow ends before we call
        // push_focus_history (which needs &mut self).
        let found_read = self.windows.iter().enumerate().find_map(|(idx, win)| {
            win.tree
                .tiles
                .find_pane(&pane_id)
                .map(|tile_id| (idx, tile_id, win.context_id))
        });
        let Some((idx, tile_id, ctx_id)) = found_read else {
            log::warn!("notify:action: pane_navigate pane_id={pane_id} not found");
            return false;
        };
        let old_focus = self.windows[self.active_window].focused_pane;
        let old_window_id = self.windows[self.active_window].window_id;
        // Clear any stale zoom on the destination window — a programmatic focus
        // redirect must not leave zoomed_pane pointing at a pane that is no longer focused.
        if self.windows[idx].zoomed_pane.is_some() {
            self.windows[idx].clear_zoom();
            log::info!("notify:action: pane_navigate cleared stale zoom on window={idx}");
        }
        self.save_minimap_before_context_navigation(ctx_id);
        // navigate_to sets focused_pane and activates the ancestor Tabs container.
        self.windows[idx].navigate_to(tile_id);
        self.push_focus_history(old_window_id, old_focus);
        let prev = self.active_window;
        self.active_window = idx;
        // Sync the router so the sidebar context switcher reflects the
        // new active context immediately (router.active_idx() drives the highlight).
        if let Some(ctx_idx) = self.router.position(|ctx| ctx.context_id == ctx_id) {
            self.router.set_active(ctx_idx);
            self.reload_config_for_active_context();
            self.context_active_window
                .insert(ctx_id, self.windows[idx].window_id);
            self.restore_minimap_for_context(ctx_id);
            log::info!(
                "notify:action: pane_navigate active_window {prev}→{idx} ctx_idx={ctx_idx} pane_id={pane_id} minimap_visible={}",
                self.minimap.visible
            );
        } else {
            log::warn!(
                "notify:action: pane_navigate active_window {prev}→{idx} pane_id={pane_id} ctx_id={ctx_id} not found in router"
            );
        }
        true
    }

    /// Reconcile the rename-pane focus layer with `renaming_pane`.
    pub(crate) fn sync_rename_pane_focus(&mut self) {
        self.reconcile_focus_layer(FocusKind::RenamePane, self.renaming_pane.is_some());
    }

    /// Reconcile the context-rename focus layer. Active when `renaming_window`
    /// is set AND the sidebar is hidden -- in that case the inline sidebar row
    /// never renders, so we promote the rename to a modal overlay instead.
    pub(crate) fn sync_context_rename_focus(&mut self) {
        let should_own = self.renaming_window.is_some() && !self.sidebar_visible;
        self.reconcile_focus_layer(FocusKind::ContextRename, should_own);
    }

    /// Reconcile the text-input overlay focus layer with `text_overlay`.
    pub(crate) fn sync_text_input_focus(&mut self) {
        self.reconcile_focus_layer(FocusKind::TextInput, self.text_overlay.is_some());
    }

    /// Push/pop `FocusKind::CapabilityModal` based on whether the focused app
    /// pane has pending prompts. Called every frame (both before and after the
    /// overlay render block) so the layer tracks prompt state without polling
    /// lag.
    pub(crate) fn sync_capability_modal_focus(&mut self) {
        let should_own = self.focused_pane_has_pending_prompts();
        self.reconcile_promoted_focus_layer(FocusKind::CapabilityModal, should_own);
    }

    /// Promote/release the host event-consent modal layer to mirror
    /// `pending_event_consents`. A parked CLI/MCP subscribe consent must own the
    /// keyboard so Enter/Esc resolve it instead of leaking to the focused pane.
    pub(crate) fn sync_event_consent_focus(&mut self) {
        let should_own = !self.pending_event_consents.is_empty();
        self.reconcile_promoted_focus_layer(FocusKind::EventConsent, should_own);
    }

    pub(crate) fn sync_raw_wasm_review_focus(&mut self) {
        let should_own = !self.pending_raw_wasm_launches.is_empty();
        self.reconcile_promoted_focus_layer(FocusKind::RawWasmReview, should_own);
    }

    /// Returns true when the focused app pane has at least one pending prompt.
    ///
    /// `win.focused_pane` holds a `TileId`. After egui_tiles renders a bare-pane
    /// root for the first time it wraps that tile in a Container, so the stored
    /// TileId may now refer to a Container instead of a Pane. `find_pane_in_tile`
    /// descends through any Container layer to reach the actual pane.
    fn focused_pane_has_pending_prompts(&self) -> bool {
        let win = &self.windows[self.active_window];
        let focused_tile = match win.focused_pane {
            Some(t) => t,
            None => return false,
        };
        let pane_id = match Self::find_pane_in_tile(&win.tree, focused_tile) {
            Some(id) => id,
            None => return false,
        };
        match win.panes.get(&pane_id) {
            Some(crate::host::pane::Pane::App(app_pane)) => match &app_pane.runtime {
                crate::host::pane::AppRuntime::Builtin(_) => false,
                crate::host::pane::AppRuntime::Python(_) => false,
                crate::host::pane::AppRuntime::Wasm(wasm) => wasm.has_pending_capability_prompt(),
            },
            _ => false,
        }
    }

    /// Walk a tile tree node and return the first `PaneId` found within it.
    /// Handles the case where `tile_id` is a Container wrapping the actual pane
    /// (egui_tiles normalises bare-pane roots into containers on first render).
    pub(crate) fn find_pane_in_tile(
        tree: &egui_tiles::Tree<crate::spatial::tiling::PaneId>,
        tile_id: egui_tiles::TileId,
    ) -> Option<crate::spatial::tiling::PaneId> {
        match tree.tiles.get(tile_id)? {
            egui_tiles::Tile::Pane(id) => Some(*id),
            egui_tiles::Tile::Container(c) => c
                .children()
                .copied()
                .find_map(|child| Self::find_pane_in_tile(tree, child)),
        }
    }

    /// Route `DeliverNotifyAction` commands back to the originating app pane as
    /// `NotifyAction` events. Shared by the modal and the sidebar panel so both
    /// surfaces dispatch identically.
    pub(crate) fn dispatch_notify_action_cmds(
        &mut self,
        cmds: Vec<crate::app::app_trait::AppCommand>,
    ) {
        use crate::app::app_trait::AppCommand;
        for cmd in cmds {
            if let AppCommand::DeliverNotifyAction {
                pane_id,
                notify_id,
                action_label,
                value,
                response_file,
                host_action,
            } = cmd
            {
                log::info!(
                    "notify:action: pane_id={pane_id} notify_id={notify_id:?} value={value:?} host_action={host_action:?}"
                );
                crate::host::event_log::emit(
                    crate::host::event_log::HostEvent::NotificationActionInvoked {
                        id: notify_id.clone(),
                        action: action_label.clone(),
                        timestamp: crate::host::event_log::now_timestamp(),
                    },
                );
                // Execute host-side action synchronously before writing the response
                // file so the navigation is complete before the shell unblocks.
                if let Some(ref action) = host_action {
                    if let Some(id_str) = action.strip_prefix("pane_focus:") {
                        if let Ok(pane_id_target) = id_str.parse::<u64>() {
                            self.pane_navigate(pane_id_target);
                        } else {
                            log::warn!("notify:action: pane_focus: invalid pane_id {:?}", id_str);
                        }
                    } else {
                        log::warn!("notify:action: unknown host_action {:?}", action);
                    }
                }
                if let Some(rf) = &response_file {
                    let content = value.as_deref().unwrap_or("");
                    if crate::rpc::write_response(rf, content.as_bytes()) {
                        log::info!("notify:action: wrote {:?} to {:?}", content, rf);
                    }
                }
                // Search all windows for the sender pane — it may not be in the
                // active context (cross-context notification path).
                let window_idx = self
                    .windows
                    .iter()
                    .position(|w| w.panes.contains_key(&pane_id));
                if let Some(win_idx) = window_idx {
                    if let Some(pane) = self.windows[win_idx].panes.get_mut(&pane_id) {
                        if let Some(app) = pane.as_app_mut() {
                            app.runtime.queue_outbound_event(
                                crate::app_protocol::PlexiEvent::NotifyAction {
                                    notify_id,
                                    action_label,
                                    value,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn record_context_visit(&mut self, context_id: u64) {
        self.context_visit_history.retain(|&id| id != context_id);
        self.context_visit_history.insert(0, context_id);
        self.context_visit_history.truncate(50);
    }

    pub(super) fn draw_feature_effects(&self, ctx: &egui::Context) {
        use egui::{Color32, Stroke};

        // CRT effect — scanlines + green phosphor tint
        if self.features.is_enabled("crt") {
            egui::Area::new(egui::Id::new("crt_overlay"))
                .fixed_pos(egui::pos2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    let screen = ctx.content_rect();
                    let painter = ui.painter();

                    // Green phosphor tint
                    painter.rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(0, 40, 0, 18));

                    // Scanlines every 3 pixels
                    let mut y = screen.top();
                    while y < screen.bottom() {
                        painter.line_segment(
                            [egui::pos2(screen.left(), y), egui::pos2(screen.right(), y)],
                            Stroke::new(0.5_f32, Color32::from_black_alpha(38)),
                        );
                        y += 3.0;
                    }

                    crate::platform::frame_diag::note(
                        crate::platform::frame_diag::RepaintCause::CrtEffect,
                    );
                    ctx.request_repaint_after(std::time::Duration::from_millis(16));
                });
        }
    }
}
