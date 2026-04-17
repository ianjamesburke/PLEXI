# v3 Progress Tracker

> Session-resumable state for the Plexi v3.0 rewrite.
> Updated at layer boundaries. For per-commit detail, see `git log --oneline`.
> Spec: [`docs/specs/releases/plexi-v3.0.md`](docs/specs/releases/plexi-v3.0.md). Architecture: [`STATE_OF_PLEXI.md`](STATE_OF_PLEXI.md).

## Current state

**Branch:** `v3` (off `main` at `060b729`)
**Worktree:** `.claude/worktrees/v3/`
**Head:** layer 5 complete — Python SDK v3, 6 example apps. Clean compile, ~80 warnings (pre-existing). 18/18 tests passing.

## Layer status

### ✅ Layer 0 — docs foundation (commit `958665b`)
- v3 spec, STATE_OF_PLEXI, VISION, CLAUDE, AGENTS, README, proposals folder (research only).

### ✅ Layer 1 — dead code deleted, alpha modules ported (commit `4671625`)
- Deleted: `audio_app.rs`, `app_api.rs` + all refs.
- Ported: `event_log.rs` (429→253 lines, depth variants stripped), `plexi_iq/` (9 files, dead_code removed, module doc rewritten).
- Deps added to Cargo.toml: `async-anthropic`, `async-trait`, `schemars`, `tokio (rt+macros)`, `tokio-stream`; `chrono` serde feature.
- `mod event_log;` + `mod plexi_iq;` wired in main.rs. Not yet called from anywhere.

### ✅ Layer 2 — protocol + capability + secrets rewire
- [x] Rewrote `src/app_protocol.rs` → PGAP v3: `Init` with `protocol`/`workspace_root`/`capabilities`/`feature_flags`; `AppReply::Ready`; `Render { frame_id, rect }`; `FrameDone { frame_id }`; `CapabilityDecision`, `SecretValue`, `RunUpdate`, `PipeMessage`, `PathChanged`, `Suspend`, `Resume`; new `DrawCommand` variants: `VideoPlayer`, `AudioMeter`, `AudioPlay`, `AudioCapture`, `PipeOpen`, `PipeSend`, `CapabilityRequest`, `SecretGet`, `RunGet`, `RunComplete`, `Notify`, `StatusSummary`; `NotificationAction`. Legacy `RunInTerminal`/`Cd` kept for back-compat.
- [x] Rewrote `src/app_permissions.rs` → `Capability` enum with `From<&str>`/`Display`; `AppPermissions { capabilities: HashSet<Capability>, is_builtin: bool }`; `PermissionsLog` with `permissions.jsonl` append-only persistence keyed by `(app_id, workspace_root, capability)`; `check()` v3 API; `check_command()` v2 shim with TODO(layer-3) markers; v2 types (`TrustLevel`, `FsPermission`) kept as `#[allow(dead_code)]`.
- [x] Surface-edited `src/app_registry.rs` → `AppCapabilities` now holds `capabilities: Vec<String>` (v3 strings); `to_permissions()` delegates to `AppPermissions::from_capability_strings()`.
- [x] Surface-edited `src/secrets.rs` → `SecretEntry` gets `workspace_root: Option<String>`; new `get_secret_scoped()`/`set_secret_scoped()` with hard invariant validation (non-empty absolute path); legacy functions doc-annotated; v1/v2 call sites fixed with `workspace_root: None`.
- [x] Fixed downstream: `process_app.rs` (Init/Render fields, FrameDone struct arm, frame_counter, v3 stub TODO markers), `secrets_app.rs` (SecretEntry workspace_root field).

### ✅ Layer 3a — standalone modules (pane ADT, media, typed pipes)
Three parallel Sonnet agents, merged cleanly.
- [x] `src/pane.rs` → `enum Pane { Terminal(TerminalPane), App(AppPane), Agent(AgentPane) }` with accessor helpers (`id()`, `as_terminal[_mut]()`, `as_app[_mut]()`, `as_agent[_mut]()`, `kind_str()`). `AppPane` holds ProcessApp + workspace_root + permissions + pane_group. `AgentPane` holds `Option<PlexiIqInstance>` + label. ~25 call sites migrated across context.rs, tiling.rs, pane_ops.rs, app.rs, overlays.rs, command_palette.rs via `as_terminal*` helpers.
- [x] `src/media/` (mod.rs, audio.rs, video.rs) — `AudioDevice` + `VideoDecoder` traits. `MockAudioDevice` reads/writes WAV at realtime pace via mpsc. `MockVideoDecoder` generates procedural RGBA at 30fps with full play/pause/seek state machine. Prod impls (`CoreAudioDevice`, `AvfVideoDecoder`) stubbed `todo!()` for Layer 4. Factory reads `PLEXI_AUDIO`/`PLEXI_VIDEO` env vars (`mock://in.wav,out.wav` and `mock://fixture?duration=5000&w=640&h=360`). `hound = "3"` added.
- [x] `src/typed_pipes.rs` (435 lines) — `TypedPipeRegistry` with JSON + binary modes. Binary uses `UnixListener` with `u32` BE length-prefixed frames, lock-free ring (`crossbeam-queue`) for realtime-safe write path, separate drain thread per pipe, drop-oldest backpressure returning `WriteResult::DroppedOldest`. Max frame 1 MiB. 3/3 tests passing.

### ✅ Layer 3b — process_app PGAP v3 + agent wiring

All 7 `TODO(layer-3)` markers resolved (3 in app_permissions.rs re-tagged TODO(layer-4) per spec constraint to keep v2 shims).

- [x] `src/process_app.rs` fully rewritten: `ProcessApp::launch` takes `workspace_root` + `capabilities`; Init populates both; frame_counter drives Render; all 11 v3 DrawCommands routed (`VideoPlayer`, `AudioPlay`, `AudioCapture`, `AudioMeter`, `CapabilityRequest`, `SecretGet`, `RunGet`, `RunComplete`, `Notify`, `PipeOpen`, `PipeSend`, `StatusSummary`).
- [x] `src/app_protocol.rs` — added `PlexiEvent::PipeOpened` and `PipeOverrun` variants.
- [x] `src/runs.rs` — new 80-line in-memory `RunRegistry` (`allocate`, `complete`, `list_runs`).
- [x] `src/keys.rs` — added `Action::SpawnAgentPane` + `Cmd+Shift+I` binding.
- [x] `src/pane_ops.rs` — added `spawn_agent_pane()` that creates `Pane::Agent(AgentPane { instance: Some(PlexiIqInstance), label })`.
- [x] `src/tiling.rs` — Agent panes render placeholder title bar + transcript area.
- [x] `src/app.rs` — `Action::SpawnAgentPane` wired to `self.spawn_agent_pane()`.
- [x] `src/app_registry.rs` — updated launch call to pass `workspace_root` + `capabilities`.

### ✅ Layer 4 — event bus emit sites, runs, notifications

- [x] `event_log::emit()` at every lifecycle point: AppSpawned (launch), AppClosed (Drop), PermissionPrompted (CapabilityRequest), RunCreated/RunCompleted (runs.rs), NotificationEmitted/NotificationActioned (Notify handler), AgentTurn (tiling.rs turn drain), PipeWrite (audio capture thread).
- [x] Notification action dispatch wired: `resume_run` → `RunRegistry::resume`, `open_intent` → pending_commands, `run_command` → `AppCommand::RunInTerminal`. Emits `NotificationActioned` for each.
- [x] Capability/secret modal UI: real egui `Window::new("Plexi needs permission")` with Grant/Deny buttons and secret text input.
- [x] VideoFrame: frames pulled from decoder, queued in `pending_video_frames`, uploaded via `ctx.load_texture()` + `painter().image()` in `ui()`.
- [x] AudioCapture forwarding thread: `TypedPipeRegistry` wrapped in `Arc<Mutex<>>`. Thread holds Arc clone, writes PCM, emits `PipeWrite`.
- [x] Agent turn loop: transcript + input bar UI, background thread with `ClaudeCliBackend`, streams tokens via `TurnMsg` mpsc, emits `AgentTurn` on Done.
- [x] Run palette: `Cmd+R` → `ToggleRunPalette` → `draw_run_palette` overlay. `RunRegistry::set_blocked/unblock/resume` all wired.
- [x] v2 cleanup: deleted `check_command`, `TrustLevel`, `FsPermission`. Added `check_cd()`. Migrated `app.rs` call site.

**TODO(layer-5) carried forward:**
- `src/process_app.rs` — PipeSend multi-app routing: requires global pipe broker at host level.
- `src/process_app.rs` — AudioMeter peak read: requires TypedPipeRegistry peak API.
- `src/process_app.rs` — Ready handshake: split stdout reader into handshake + draw-command phases.
- `src/overlays.rs` — Run palette run aggregation: requires `list_runs()` on the `App` trait.

### ✅ Layer 5 — example apps (the five + quick-note)

- [x] `sdk/python/plexi_sdk.py` rewritten for PGAP v3: `App` subclass model, `RenderContext`, `Emitter`, `Pipe` (binary + JSON), blocking `capability_request`/`secret_get`, `audio_capture`. 290 lines. Covers all PlexiEvent variants and all DrawCommand variants.
- [x] `snake` (Python) — input + draw primitives. Tick loop via background thread. Arrow/hjkl navigation, game-over screen. 100 lines. Cap: none.
- [x] `wikipedia` (Python) — net.http + text render + List primitive. Background urllib fetch thread. Search + results + article extract modes. 100 lines. Cap: `net.http`.
- [x] `todo` (Python) — fs.read + fs.write + persistence to `.plexi/todos.json`. Up/down/space/a/d keys. 80 lines. Cap: `fs.read`, `fs.write`.
- [x] `audio-recorder` (Python) — audio.record + binary pipe + WAV write via stdlib `wave`. AudioMeter draw command. R/S keys. 105 lines. Cap: `audio.record`, `fs.write`.
- [x] `video-player` (Python) — video.playback + VideoPlayer draw command. Space play/pause, arrow seek ±5s. 75 lines. Cap: `video.playback`, `fs.read`.
- [x] `quick-note` (first-party, Python) — compose + browse modes, `.plexi/notes/<timestamp>.md`, Cmd+Enter save, Cmd+K browse, `Notify` on save. 125 lines. Cap: `fs.read`, `fs.write`.
- [x] SDK copied into each example dir (`plexi_sdk.py`).
- [x] All 6 apps tested: `echo '{"type":"init",...}' | python app.py` returns `{"type":"ready","sdk":"plexi-sdk-py/0.4.0","features_used":[]}`.

**TODO(layer-6) — protocol test harness needed:**
- JSON replay tests: feed PlexiEvent sequence → assert DrawCommand sequence for each app.
- Audio mock device end-to-end: feed WAV fixture through binary pipe → assert output WAV.

### 🔄 Layer 6 — CI gate + release
- [ ] Protocol test harness: replay `PlexiEvent` JSON → assert on `DrawCommand` JSON.
- [ ] Headless audio/video tests via mock devices.
- [ ] `v3` → `beta` → `main`. Tag `v3.0.0`.

## Resuming

1. `cd .claude/worktrees/v3 && git log --oneline -10`
2. Read this file's **Layer status** section.
3. Pick up from the first `[ ]` unchecked item in the active `🔄` layer.
4. If no `🔄` layer, advance the next `⏳` to `🔄` and start.
