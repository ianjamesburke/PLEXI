# v3 Progress Tracker

> Session-resumable state for the Plexi v3.0 rewrite.
> Updated at layer boundaries. For per-commit detail, see `git log --oneline`.
> Spec: [`docs/specs/releases/plexi-v3.0.md`](docs/specs/releases/plexi-v3.0.md). Architecture: [`STATE_OF_PLEXI.md`](STATE_OF_PLEXI.md).

## Current state

**Branch:** `v3` (off `main` at `060b729`)
**Worktree:** `.claude/worktrees/v3/`
**Head:** layer 2 complete — clean compile, 82 warnings (all dead-code on unwired items, no errors).

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

### 🔄 Layer 3 — Pane ADT + process_app PGAP v3 + media subsystem

**TODO(layer-3) markers to resolve:**
- `src/process_app.rs:285` — populate `workspace_root` in Init from actual pane CWD at spawn
- `src/process_app.rs:286` — populate `capabilities` in Init from AppPermissions
- `src/process_app.rs:38` — drive frame_counter from host frame clock
- `src/process_app.rs:259` — route v3 draw commands (VideoPlayer, AudioMeter, etc.) to subsystems
- `src/app_permissions.rs:142` — remove `check_command` shim; all call sites use `check()` directly
- `src/app_permissions.rs:152` — RunInTerminal → PipeSend migration
- `src/app_permissions.rs:241` — delete TrustLevel, FsPermission, GlobalPermissions, PermissionsConfig

- [ ] Rewrite `src/pane.rs` → `enum Pane { Terminal, App, Agent }`.
- [ ] Update `src/process_app.rs` → PGAP v3 handshake, binary pipe support.
- [ ] Create `src/media/` → `AudioDevice` + `VideoDecoder` traits, CoreAudio/AVFoundation prod impls, mock impls (`PLEXI_AUDIO=mock://`, `PLEXI_VIDEO=mock://`).
- [ ] Binary-mode typed pipes (unix domain sockets, length-prefixed frames).
- [ ] Wire `Pane::Agent` to `plexi_iq`.

### ⏳ Layer 4 — event bus emit sites, runs, notifications
- [ ] Call `event_log::emit()` at every lifecycle point (app spawn/close, permission decision, run update, agent turn, pipe open/close).
- [ ] Wire rich notification action dispatch (`resume_run`, `open_intent`, `run_command`) — no TODOs.
- [ ] Run palette + `BlockedOnUser` inline prompts.

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
