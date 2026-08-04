//! HostHarness — headless egui test harness for host self-validation (issue #552).
//!
//! Layers:
//!   1. `HostHarness` — runs `PlexiApp` in a headless `egui::Context`. Input
//!      events are injected via `RawInput`; observable state is extracted into
//!      `HostSnapshot` after each frame.
//!   2. Pane/IPC injection for host-model behavior without a display server.

/// Prompt-equals-infrastructure-fail, as a mechanism rather than a rule
/// (2026-07-28 keychain incident): a pre-main constructor disables keychain
/// user interaction for the whole test process before `main`/libtest run —
/// so it covers code outside test bodies too (ordering relative to other
/// pre-main initializers is unspecified; none in this binary touch the
/// keychain). A keychain call that would have raised a macOS credential
/// dialog fails loudly instead of prompting: observed on the ACL value-read
/// path as `errSecAuthFailed` (-25293), not the keychain-level
/// `errSecInteractionNotAllowed` (-25308) the API name suggests — same
/// error-not-dialog class, recorded errno from the live falsification.
/// `__mod_init_func` is the macOS constructor section (what the `ctor`
/// crate emits); no dependency needed. The returned lock is leaked
/// deliberately — its `Drop` would re-enable interaction.
///
/// Honest scope: this suppresses UI, not access. Mechanism A (the
/// `system_store()` seam in `src/workspace/secrets.rs`) removes every
/// keychain ROUTE in our own code from test builds, and this guard removes
/// PROMPTS — but the `security_framework` dependency itself is callable
/// from any test in this crate (a same-crate test sees all dependencies;
/// no cfg can hide one), and a deliberate direct call performs a real,
/// silent, unprompted keychain operation. That is banned by contract
/// (`src/workspace/AGENTS.md`) and is genuinely closed only by pointing the
/// process's keychain search list at a throwaway keychain — stint 0603.
/// This module is itself the one sanctioned `security_framework` call site
/// in test builds.
#[cfg(all(test, target_os = "macos"))]
mod keychain_prompt_guard {
    extern "C" fn disable_keychain_prompts() {
        use security_framework::os::macos::keychain::SecKeychain;
        match SecKeychain::disable_user_interaction() {
            Ok(lock) => std::mem::forget(lock),
            Err(e) => {
                eprintln!(
                    "FATAL: could not disable keychain user interaction for the test process \
                     ({e}); refusing to run — a test that can prompt is an infrastructure failure"
                );
                std::process::abort();
            }
        }
    }

    #[used]
    #[link_section = "__DATA,__mod_init_func"]
    static GUARD: extern "C" fn() = disable_keychain_prompts;
}

use crate::app::permissions::AppPermissions;
use crate::app::PlexiApp;
use crate::app_protocol::AppRequest;
use crate::config::set_test_profile_dir;
use crate::host::pane::{AppPane, AppRuntime, Pane};
use crate::spatial::tiling::PaneId;
use egui::RawInput;
use std::collections::HashMap;
use std::sync::{atomic::AtomicU64, Arc};
use std::time::Duration;

/// Returns a correctness timeout scaled for the machine's current one-minute
/// load average. Wall-clock budgets prove that an asynchronous operation
/// eventually completes; they are not performance claims, so a busy fleet
/// must not turn them into false failures. The multiplier is capped to keep a
/// genuine hang bounded even when the host is severely overloaded.
pub(crate) fn load_aware_timeout(base: Duration) -> Duration {
    let cores = std::thread::available_parallelism()
        .map(|count| count.get() as f64)
        .unwrap_or(1.0);
    let mut load = [0.0; 3];
    let samples = unsafe { libc::getloadavg(load.as_mut_ptr(), load.len() as i32) };
    let multiplier = if samples > 0 {
        load_multiplier(load[0], cores)
    } else {
        1
    };
    base.saturating_mul(multiplier)
}

fn load_multiplier(load: f64, cores: f64) -> u32 {
    const MAX_MULTIPLIER: u32 = 4;
    (load / cores).ceil().clamp(1.0, MAX_MULTIPLIER as f64) as u32
}

#[cfg(test)]
mod load_aware_timeout_tests {
    use super::*;

    #[test]
    fn correctness_budget_scales_with_load_and_stops_at_the_cap() {
        let base = Duration::from_secs(10);
        assert_eq!(base.saturating_mul(load_multiplier(0.5, 4.0)), base);
        assert_eq!(
            base.saturating_mul(load_multiplier(7.0, 4.0)),
            Duration::from_secs(20)
        );
        assert_eq!(
            base.saturating_mul(load_multiplier(100.0, 4.0)),
            Duration::from_secs(40)
        );
    }
}

// ─── HostSnapshot ────────────────────────────────────────────────────────────

/// Observable state extracted from `PlexiApp` after a frame. Returned by
/// `HostHarness::state()` — tests assert against this instead of reaching
/// into `PlexiApp` internals.
#[derive(Debug, Clone)]
pub struct HostSnapshot {
    pub open_panes: Vec<PaneId>,
    pub pane_titles: HashMap<PaneId, String>,
}

impl HostSnapshot {
    fn from_app(app: &PlexiApp) -> Self {
        let win = &app.windows[app.active_window];
        let open_panes: Vec<PaneId> = win.panes.keys().copied().collect();
        let pane_titles: HashMap<PaneId, String> = win
            .panes
            .iter()
            .map(|(id, pane)| {
                let title = match pane {
                    Pane::App(p) => p.name.clone(),
                    Pane::Terminal(_) => "Terminal".to_string(),
                    Pane::Portal(p) => format!("Portal({})", p.target_context_id),
                };
                (*id, title)
            })
            .collect();
        Self {
            open_panes,
            pane_titles,
        }
    }
}

// ─── HostHarness ─────────────────────────────────────────────────────────────

/// Process-wide pane-id block allocator for `HostHarness` instances.
///
/// `cargo test` runs every `#[test]` in one process across a thread pool, and
/// production singletons that are correctly process-global in a real host
/// (e.g. `plexi_ai::tool_dispatch::GLOBAL_REGISTRY`, keyed by `pane_id`) stay
/// exactly that global in test builds too — there is no per-test process to
/// isolate them. Before this, every `HostHarness::new()` started its own
/// `next_pane_id` at the same fixed `100`, so two tests that both register
/// into such a global registry could race on the identical key: one test's
/// entry silently overwrites the other's, and the loser observes missing or
/// foreign data with no error (`connector_tool_visible_only_to_assistant_in_the_owning_context`,
/// stint 0724 Phase F — flaky only under full-suite parallel execution,
/// always green in isolation). Each harness now reserves its own disjoint
/// block from one atomic counter, so no two concurrently-running harnesses
/// can ever assign the same `pane_id`, regardless of test execution order.
static NEXT_TEST_PANE_ID_BLOCK: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(100);

/// Reserve a fresh block of pane ids for one `HostHarness` instance. `10_000`
/// is far more than any single test creates, so blocks never collide; `u64`
/// has no realistic risk of exhausting across even a very large test run.
fn reserve_pane_id_block() -> u64 {
    NEXT_TEST_PANE_ID_BLOCK.fetch_add(10_000, std::sync::atomic::Ordering::SeqCst)
}

/// Headless egui test harness wrapping `PlexiApp`.
///
/// ```rust,no_run
/// let mut h = HostHarness::new();
/// let pane = h.add_test_pane();
/// h.inject_ipc(AppRequest::SetPipStatus {
///     pane_id: pane,
///     status: crate::app_protocol::PipStatus::Green,
/// });
/// h.run_frames(1);
/// ```
pub struct HostHarness {
    /// The app under test.
    pub app: PlexiApp,
    /// Shared egui context — same instance that `app.ctx` holds.
    ctx: egui::Context,
    /// Next pane id to assign.
    next_pane_id: u64,
    /// IPC mailbox for injecting AppRequests into the pane_ipc channel.
    pub ipc_tx: crate::app::ui_mailbox::UiMailbox<AppRequest>,
    /// Platform output from the most recently completed frame.
    /// Contains clipboard writes, open URLs, etc.
    pub last_platform_output: egui::PlatformOutput,
    /// Keeps the per-test tempdir alive for the harness lifetime; auto-deletes on drop.
    _profile_dir: tempfile::TempDir,
    /// Isolated workspace root for panes the harness creates. Held alive for the
    /// harness lifetime and auto-deleted on drop, so any channel dir a pane
    /// writes (e.g. the assistant's `AssistantStore`) lands here — never in the
    /// shared `std::env::temp_dir()` root, where it would leak into the walk-up
    /// path of every other test's tempdir.
    _workspace_dir: tempfile::TempDir,
    /// Keeps the thread-local profile dir override active for the harness lifetime.
    _profile_guard: crate::config::TestProfileDirGuard,
}

impl HostHarness {
    /// Create a harness with an empty `PlexiApp` and a 1280×800 viewport.
    pub fn new() -> Self {
        let profile_dir = tempfile::TempDir::new().expect("failed to create test profile tempdir");
        let workspace_dir =
            tempfile::TempDir::new().expect("failed to create test workspace tempdir");
        let profile_guard = set_test_profile_dir(profile_dir.path().to_path_buf());
        let ctx = egui::Context::default();
        let frame_tick = Arc::new(AtomicU64::new(0));
        let (app, ipc_tx) = PlexiApp::new_for_test(ctx.clone(), frame_tick);
        Self {
            app,
            ctx,
            next_pane_id: reserve_pane_id_block(),
            ipc_tx,
            last_platform_output: egui::PlatformOutput::default(),
            _profile_dir: profile_dir,
            _workspace_dir: workspace_dir,
            _profile_guard: profile_guard,
        }
    }

    // ── Pane management ──────────────────────────────────────────────────────

    /// Add a builtin app pane (not a Terminal) for host testing.
    /// The pane has builtin permissions (all capability checks pass).
    pub fn add_test_pane(&mut self) -> PaneId {
        self.add_test_pane_with_permissions(AppPermissions::builtin())
    }

    /// Add a test app pane with the given permissions.
    pub fn add_test_pane_with_permissions(&mut self, permissions: AppPermissions) -> PaneId {
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        // Root the file browser in the harness's empty workspace dir — NEVER
        // the real temp_dir(), which on a dev machine can hold ~50k entries
        // and balloons any test that renders the pane by gigabytes.
        let browser_root = self._workspace_dir.path().to_path_buf();
        let app_pane = AppPane {
            pip_status: None,
            id: pane_id,
            runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
                browser_root.clone(),
            ))),
            workspace_root: browser_root,
            permissions,
            manifest_id: "test".to_string(),
            name: "Test App".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: None,
            slots: HashMap::new(),
            semantic_state: Default::default(),
        };

        let win = &mut self.app.windows[0];
        win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }

        pane_id
    }

    /// Add a builtin Assistant app pane backed by an inert AI broker. Used by
    /// input-ownership tests that assert on the composer's observable state.
    /// Always lands in `windows[0]` — for a pane in a specific (e.g.
    /// secondary-context) window, use
    /// [`Self::add_assistant_pane_in_window`].
    pub fn add_assistant_pane(&mut self) -> PaneId {
        self.add_assistant_pane_in_window(0)
    }

    /// Like [`Self::add_assistant_pane`] but lands the pane in `windows[win_idx]`
    /// — lets a multi-context test put an Assistant pane in a specific
    /// (non-active) context's window (stint 0724 Phase C connector-tool
    /// reachability regression coverage).
    pub fn add_assistant_pane_in_window(&mut self, win_idx: usize) -> PaneId {
        struct InertBroker;
        impl crate::plexi_ai::broker::AiBroker for InertBroker {
            fn dispatch(
                &self,
                request: crate::plexi_ai::broker::AiBrokerRequest,
                _on_delta: &mut dyn FnMut(crate::plexi_ai::turn_loop::TurnDelta<'_>),
            ) -> crate::plexi_ai::broker::AiBrokerResponse {
                log::warn!(
                    "testing: InertBroker dispatch for '{}' — no AI in harness",
                    request.app_id
                );
                crate::plexi_ai::broker::AiBrokerResponse {
                    content: None,
                    tokens_in: 0,
                    tokens_out: 0,
                    error: Some("no AI broker in HostHarness".to_string()),
                }
            }
        }

        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        let workspace_root = self._workspace_dir.path().to_path_buf();
        // Scope the store to the target window's own context — never
        // `windows[0]`'s, so a pane placed in a secondary context gets that
        // context's identity.
        let context_id = self.app.windows[win_idx].context_id;
        let assistant = crate::assistant::AssistantApp::new(
            workspace_root.clone(),
            Arc::new(InertBroker),
            &crate::config::config_dir(),
            context_id,
        );
        let app_pane = AppPane {
            pip_status: None,
            id: pane_id,
            runtime: AppRuntime::Builtin(Box::new(assistant)),
            workspace_root,
            permissions: AppPermissions::builtin(),
            manifest_id: "assistant".to_string(),
            name: "Assistant".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: None,
            slots: HashMap::new(),
            semantic_state: Default::default(),
        };
        let win = &mut self.app.windows[win_idx];
        win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        // Join the layout so this pane actually renders each frame — a tile
        // outside the root container would never reach the tiling behavior.
        match win.tree.root {
            None => win.tree.root = Some(tile_id),
            Some(old_root) => {
                let new_root = win
                    .tree
                    .tiles
                    .insert_horizontal_tile(vec![old_root, tile_id]);
                win.tree.root = Some(new_root);
            }
        }
        pane_id
    }

    /// Mutable access to an assistant pane's concrete app for state assertions.
    /// Searches every window, not just `windows[0]` — a pane placed via
    /// [`Self::add_assistant_pane_in_window`] with a non-zero index must
    /// still be found.
    pub fn assistant_mut(&mut self, pane_id: PaneId) -> &mut crate::assistant::AssistantApp {
        let pane = self
            .app
            .windows
            .iter_mut()
            .find_map(|window| window.panes.get_mut(&pane_id))
            .expect("assistant pane not found");
        let AppRuntime::Builtin(app) = &mut pane.as_app_mut().expect("not an app pane").runtime
        else {
            panic!("pane {pane_id} is not a builtin app");
        };
        app.as_any_mut()
            .downcast_mut::<crate::assistant::AssistantApp>()
            .expect("pane is not an AssistantApp")
    }

    // ── Frame pump ───────────────────────────────────────────────────────────

    /// Run one egui frame with the given raw input.
    ///
    /// Mirrors eframe's visible-window pass: `raw_input_hook`, then `logic`,
    /// then `ui` inside one egui pass — the same order
    /// `epi_integration::update` uses.
    pub fn frame(&mut self, mut input: RawInput) -> &mut Self {
        let app = &mut self.app;
        use eframe::App;
        app.raw_input_hook(&self.ctx, &mut input);
        let full_output = self.ctx.run_ui(input, |ui| {
            app.logic(ui.ctx(), &mut eframe::Frame::_new_kittest());
            app.ui(ui, &mut eframe::Frame::_new_kittest());
        });
        self.last_platform_output = full_output.platform_output;
        self
    }

    /// Run one egui pass the way eframe runs it for a *hidden* window
    /// (minimized, or fully occluded by other windows): `raw_input_hook` and
    /// `App::logic` only — `App::ui` is never called. External servicing
    /// (pane IPC, spawn queue, PTY events, shutdown) must complete on these
    /// passes alone; anything only drained in `ui` wedges until the window is
    /// next uncovered.
    pub fn hidden_frame(&mut self) -> &mut Self {
        let app = &mut self.app;
        use eframe::App;
        let viewport_id = egui::ViewportId::ROOT;
        let mut input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        input.viewports.entry(viewport_id).or_default().occluded = Some(true);
        app.raw_input_hook(&self.ctx, &mut input);
        let full_output = self.ctx.run_ui(input, |ui| {
            app.logic(ui.ctx(), &mut eframe::Frame::_new_kittest());
        });
        self.last_platform_output = full_output.platform_output;
        self
    }

    /// Run `n` hidden-window passes. See [`Self::hidden_frame`].
    pub fn run_hidden_frames(&mut self, n: u32) -> &mut Self {
        for _ in 0..n {
            self.hidden_frame();
        }
        self
    }

    /// Run `n` idle frames (no input events).
    pub fn run_frames(&mut self, n: u32) -> &mut Self {
        for _ in 0..n {
            self.frame(RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            });
        }
        self
    }

    // ── State inspection ─────────────────────────────────────────────────────

    /// Snapshot observable state from the app after the last frame.
    pub fn state(&self) -> HostSnapshot {
        HostSnapshot::from_app(&self.app)
    }

    /// Number of panes in the active window.
    pub fn pane_count(&self) -> usize {
        self.app.windows[self.app.active_window].panes.len()
    }

    /// Number of windows in the active window list.
    pub fn window_count(&self) -> usize {
        self.app.windows.len()
    }

    /// Focus a pane by PaneId, looking up its TileId in the active window's
    /// tile tree. No-op if the pane has no corresponding tile.
    pub fn focus_pane(&mut self, pane_id: PaneId) -> &mut Self {
        let win = &mut self.app.windows[self.app.active_window];
        let tile_id = win.tree.tiles.iter().find_map(|(tid, tile)| {
            if let egui_tiles::Tile::Pane(pid) = tile {
                if *pid == pane_id {
                    Some(tid)
                } else {
                    None
                }
            } else {
                None
            }
        });
        win.focused_pane = tile_id.copied();
        self
    }

    /// Split off a real terminal pane, focus it, and settle enough frames for
    /// the reconciler to establish the terminal's egui focus *and* its stint
    /// 0505 traversal-key lock. The lock only applies once the terminal has
    /// held egui focus for a prior frame (`set_focus_lock_filter`'s
    /// `had_focus_last_frame` gate), so a single frame is not enough — tests
    /// asserting that Tab/Escape reach the PTY must start from a settled
    /// terminal. Returns the terminal's `PaneId`.
    pub fn add_focused_terminal(&mut self) -> PaneId {
        let app_pane = self.add_test_pane();
        self.focus_pane(app_pane);
        self.run_frames(1);
        self.app.split_focused(true, None, false, false, None);
        let term = self.app.windows[0]
            .panes
            .iter()
            .find_map(|(id, pane)| pane.as_terminal().map(|_| *id))
            .expect("split_focused should create a terminal pane");
        self.focus_pane(term);
        self.run_frames(3);
        term
    }

    /// Mutable access to a terminal pane's backend, for input-tap assertions
    /// (`enable_input_tap` / `take_input_tap`) that prove a keystroke reached
    /// the PTY writer.
    pub fn terminal_backend(&mut self, pane_id: PaneId) -> &mut egui_term::TerminalBackend {
        &mut self.app.windows[0]
            .panes
            .get_mut(&pane_id)
            .and_then(|pane| pane.as_terminal_mut())
            .expect("pane is not a terminal")
            .backend
    }

    /// Run one frame delivering a single key press (with modifiers) through the
    /// production `RawInput` path — the same route a physical keystroke takes.
    pub fn press_key(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> &mut Self {
        self.frame(RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            modifiers,
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        })
    }

    /// Inject a `AppRequest` directly into the pane_ipc channel.
    pub fn inject_ipc(&self, cmd: AppRequest) -> &Self {
        let _ = self.ipc_tx.send(cmd);
        self
    }

    /// Poll the recorded event bus — the global `AppTimeline`, the same store
    /// production `EmitEvent` requests record into — until an event on
    /// `stream` whose serialized JSON payload contains `payload_contains`
    /// (when set) appears, pumping one harness frame per poll (stint 0511).
    /// Returns the matched record, or a timeout error naming what was
    /// expected. The timeline is process-global, so use a distinct stream
    /// name (or payload marker) per test to stay isolated under parallel
    /// test runs.
    pub fn wait_for_app_event(
        &mut self,
        stream: &str,
        payload_contains: Option<&str>,
        timeout: std::time::Duration,
    ) -> Result<crate::host::app_timeline::AppEventRecord, String> {
        let started = std::time::Instant::now();
        loop {
            {
                let timeline = crate::host::app_timeline::global();
                let timeline = timeline.lock().expect("app timeline lock");
                let matched = timeline.events().iter().rev().find(|record| {
                    record.event == stream
                        && payload_contains.is_none_or(|needle| {
                            record
                                .payload
                                .as_ref()
                                .is_some_and(|payload| payload.to_string().contains(needle))
                        })
                });
                if let Some(record) = matched {
                    return Ok(record.clone());
                }
            }
            if started.elapsed() > timeout {
                let payload_note = payload_contains
                    .map(|needle| format!(" with payload containing {needle:?}"))
                    .unwrap_or_default();
                return Err(format!(
                    "no event on stream '{stream}'{payload_note} within {timeout:?}"
                ));
            }
            self.run_frames(1);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Inject a synthetic pointer click at pane-pixel coordinates (origin at
    /// the pane's top-left), through the real `AppRequest::ClickPane`
    /// dispatch path — the same host code `plexi pane click` drives. A real
    /// pointer move + press + release is delivered into the live production
    /// egui pass, so it exercises the pane's own `canvas_transform`
    /// inversion, never a parallel resolver. Call `run_frames` afterward to
    /// let the dispatch (and, for process apps, the async IPC round trip to
    /// the guest) take effect. `button` is `"left"`, `"right"`, or `"middle"`.
    pub fn inject_click(
        &self,
        pane_id: PaneId,
        x: f32,
        y: f32,
        button: &str,
        response_file: Option<String>,
    ) -> &Self {
        self.inject_ipc(AppRequest::ClickPane {
            pane_id,
            x,
            y,
            button: Some(button.to_string()),
            response_file,
        })
    }

    /// Inject a sanctioned pointer drag through the real
    /// `AppRequest::DragPane` dispatch path — the same host code
    /// `plexi pane drag` drives (stint 0510). `from`/`to` are pane-pixel
    /// coordinates (origin at the pane's top-left). The host queues a
    /// press → `steps` moves → release schedule delivered one frame at a
    /// time through production input paths (raw-input merge for builtin
    /// panes, per-frame render-pass samples for canvas/process/WASM panes),
    /// so call `run_frames(steps + 2)` or more afterward to let the whole
    /// trajectory land. `button` is `"left"`, `"right"`, or `"middle"`.
    pub fn inject_drag(
        &self,
        pane_id: PaneId,
        from: (f32, f32),
        to: (f32, f32),
        steps: u32,
        button: &str,
        response_file: Option<String>,
    ) -> &Self {
        self.inject_ipc(AppRequest::DragPane {
            pane_id,
            from: Some([from.0, from.1]),
            from_node: None,
            to: Some([to.0, to.1]),
            to_node: None,
            steps: Some(steps),
            button: Some(button.to_string()),
            response_file,
        })
    }

    /// Inject a node-addressed pointer drag: each endpoint names a
    /// `SemanticPaneNode.id` (as reported by `plexi pane state`) whose cached
    /// bounds center becomes that endpoint. Nodes without recorded bounds
    /// fail loudly in the response file — currently that means builtin
    /// (accesskit) panes; canvas/WASM trees record no per-node bounds yet.
    pub fn inject_node_drag(
        &self,
        pane_id: PaneId,
        from_node: &str,
        to_node: &str,
        steps: u32,
        button: &str,
        response_file: Option<String>,
    ) -> &Self {
        self.inject_ipc(AppRequest::DragPane {
            pane_id,
            from: None,
            from_node: Some(from_node.to_string()),
            to: None,
            to_node: Some(to_node.to_string()),
            steps: Some(steps),
            button: Some(button.to_string()),
            response_file,
        })
    }

    /// Inject a synthetic node-targeted click, through the real
    /// `AppRequest::ClickPaneNode` dispatch path — the same host code
    /// `plexi pane click <pane_id> --node <node_id>` drives. `node_id`
    /// matches `SemanticPaneNode.id`, the id `plexi pane state` reports for
    /// every node in the pane's tree. The host resolves the node's on-screen
    /// rect during the next render pass and delivers the same
    /// `PendingPaneClick` honest hit-test the pixel path uses. Call
    /// `run_frames` afterward to let the dispatch (and, for process apps,
    /// the async IPC round trip to the guest) take effect. `button` is
    /// `"left"`, `"right"`, or `"middle"`.
    pub fn inject_node_click(
        &self,
        pane_id: PaneId,
        node_id: &str,
        button: &str,
        response_file: Option<String>,
    ) -> &Self {
        self.inject_ipc(AppRequest::ClickPaneNode {
            pane_id,
            node_id: node_id.to_string(),
            button: Some(button.to_string()),
            response_file,
        })
    }
}

#[cfg(test)]
mod cargo_lease;
#[cfg(test)]
mod daw_gate;
#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod harness_tests;

#[cfg(test)]
mod profile_isolation_tests {
    use super::HostHarness;

    #[test]
    fn host_harness_profile_dir_is_not_in_home() {
        let _h = HostHarness::new();
        let dir = crate::config::config_dir();
        let home = dirs::home_dir().expect("no home dir");
        assert!(
            !dir.starts_with(&home),
            "config_dir() must not point into $HOME inside HostHarness, got: {}",
            dir.display()
        );
    }
}
