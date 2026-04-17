# v3 Progress Tracker

> Session-resumable state for the Plexi v3.0 rewrite.
> Updated at layer boundaries. For per-commit detail, see `git log --oneline`.
> Spec: [`docs/specs/releases/plexi-v3.0.md`](docs/specs/releases/plexi-v3.0.md). Architecture: [`STATE_OF_PLEXI.md`](STATE_OF_PLEXI.md).

## Current state

**Branch:** `v3` (off `main` at `060b729`)
**Worktree:** `.claude/worktrees/v3/`
**Head:** layer 3b complete — clean compile, ~12 warnings (all pre-existing dead-code/unused-import). 18/18 tests passing (3/3 typed_pipes + 15 others).

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

### 🔄 Layer 4 — event bus emit sites, runs, notifications

**TODO(layer-4) markers added in this layer:**
- `src/process_app.rs` — VideoPlayer: wire rgba frame → egui TextureHandle (texture plumbing nontrivial)
- `src/process_app.rs` — AudioCapture forwarding thread: needs TypedPipeRegistry behind Arc<Mutex<>> for Send
- `src/process_app.rs` — proper Ready handshake: read first stdout line synchronously before draw-command loop
- `src/process_app.rs` — proper capability/secret modal UI with Grant/Deny buttons (currently auto-grant in debug)
- `src/process_app.rs` — Notify action dispatch: `resume_run` → run_registry, `open_intent` → intent palette, `run_command` → PTY write
- `src/process_app.rs` — PipeSend multi-app routing: route PipeMessage to peer apps subscribed on the pipe_id
- `src/pane_ops.rs` — agent pane turn loop: drive `IqSession::send()` from UI thread, stream tokens via DrawCommands
- `src/app_permissions.rs` — remove `check_command` shim; migrate remaining call sites to `check()` + `Capability`
- `src/app_permissions.rs` — remove RunInTerminal; all callers use PipeSend
- `src/app_permissions.rs` — delete TrustLevel, FsPermission, GlobalPermissions, PermissionsConfig

- [ ] Call `event_log::emit()` at every lifecycle point (app spawn/close, permission decision, run update, agent turn, pipe open/close).
- [ ] Wire rich notification action dispatch (`resume_run`, `open_intent`, `run_command`) — skeleton logs today.
- [ ] Run palette + `BlockedOnUser` inline prompts.
- [ ] Proper capability/secret modal UI (replace auto-grant stub).
- [ ] Wire VideoFrame rgba → egui TextureHandle for VideoPlayer rendering.

### ⏳ Layer 5 — example apps (the five + quick-note)
- [ ] `snake` (Rust) — input + draw primitives only.
- [ ] `wikipedia` (Python) — net.http + text render.
- [ ] `todo` (Python) — fs.read/write.
- [ ] `audio-recorder` (Python) — audio.record + binary pipe + mock device proof.
- [ ] `video-player` (Python) — video.playback + `VideoPlayer` command.
- [ ] `quick-note` (first-party, Python) — replaces host-internal backlog scanner.

### ⏳ Layer 6 — CI gate + release
- [ ] Protocol test harness: replay `PlexiEvent` JSON → assert on `DrawCommand` JSON.
- [ ] Headless audio/video tests via mock devices.
- [ ] `v3` → `beta` → `main`. Tag `v3.0.0`.

## Resuming

1. `cd .claude/worktrees/v3 && git log --oneline -10`
2. Read this file's **Layer status** section.
3. Pick up from the first `[ ]` unchecked item in the active `🔄` layer.
4. If no `🔄` layer, advance the next `⏳` to `🔄` and start.
