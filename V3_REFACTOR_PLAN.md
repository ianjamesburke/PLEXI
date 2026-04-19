# Plexi v3 — Full Refactor Plan

**Purpose of this document.** Plexi is mid-cutover from the v2 egui-centric architecture to the v3 pure-state-machine + renderer split. The cutover is visibly half-done: `HostModel` is instantiated and called on every command path, but its effects are read-and-discarded, dual type systems coexist, four whole modules are dead, the PGAP protocol has 17 spec-vs-code mismatches, and CI runs a test step that matches zero tests. This document captures exactly what is in the tree today, what the finished v3 must look like (with the primitives future features will plug into), and a 12-step plan to close the gap.

**How to use this.** Each of the 12 steps is independently testable. Each ends with `cargo test` green, `pytest sdk/python/tests/` green, and `just install-v3` smoke test clean. Do them in order — dependencies are real. When a step ships, check it off; don't move to the next until the ship gate is clean.

**What this replaces.** `CUTOVER_PLAN.md` (the six-hour `launch_app_by_id` slice — now complete, shipped as 3e162f2…8e158d0) and the ad-hoc "Next cutover slices" bullet in `CLAUDE.md`. After this plan lands in its entirety, the v3 ship gate can light green and v3.0 is releasable.

---

## Part 1 — State of the Codebase Today

### 1.1 Headline liabilities

| # | Liability | Where it lives | Why it matters |
|---|---|---|---|
| L-1 | `HostModel` is called on every command path, but its effects are read-and-discarded | `src/pane_ops.rs:58,296,484,584`; `src/app/mod.rs:44` comment | The refactor is visibly half-done; every user action double-mutates state (HostModel + egui_tiles) that can drift |
| L-2 | Dual pane ID counters: `HostModel::next_pane_id` and `PlexiApp::next_pane_id` | `src/host/model.rs:32`, `src/app/mod.rs:26` | Same `OpenPane` command allocates two different IDs; HostModel's pane list does not match the egui tile tree |
| L-3 | `NoopEventSink` is the production default | `src/host/services.rs:43`; wired at `src/app/mod.rs:237` | Every `HostEffect` is silently discarded at runtime; `events.jsonl` event bus doesn't exist |
| L-4 | Only 4 of 13 `HostCommand` variants wired in production | `src/pane_ops.rs` submit sites | `FocusPane`, `NewContext`, `SwitchContext`, `SendKeyToFocusedApp`, `SimulatePathChanged`, `CheckCapability`, `GrantCapability`, `DenyCapability` are harness-only |
| L-5 | 10 of 12 `HostEffect` variants have no production consumer | grep confirmation across `src/` | Only `PaneOpened.share/placement` and `SplitOpened.placement` are read; `FocusChanged`, `PaneClosed`, `ContextCreated`, `ContextSwitched`, `AppKeyDispatched`, `PathBroadcasted`, and all capability effects are observed-only |
| L-6 | `HostServices` missing `fs`, `secrets`, `net` trait objects | `src/host/services.rs:37–39` vs spec `docs/specs/subsystems/host-architecture.md:§3` | Apps' side-effects bypass the host service seam entirely; impossible to mock in harness |
| L-7 | `net.http` broker does not exist in production | `src/process_app/routing.rs`; `http_request` JSON doesn't deserialize as `DrawCommand` | App sends `http_request`, host logs a warning and drops it, app blocks forever. Only `pgap_test_harness.rs` mocks it |
| L-8 | 3 of 9 spec'd capabilities missing from enum | `src/app_permissions.rs:21–34` | `audio.record`, `audio.playback`, `video.playback` are not enforced; silently missing from runtime checks |
| L-9 | `FsRead` / `FsWrite` defined but never checked | `src/app_permissions.rs:116,134`; grep of `check(…, FsRead)` outside tests | Apps read and write the filesystem without declared capability; honor-system is dishonored |
| L-10 | Unknown capability strings silently become `FsRead` | `src/app_permissions.rs:51–64` | `"net.http_"` typo → `FsRead` with a log::warn; no schema validation at manifest load |
| L-11 | Two `PlexiEvent` enums | `src/protocol/event.rs` vs `src/app_protocol.rs:45` | Test harness uses one, production uses the other; `InjectState` and `HttpResponse` exist only in the dead-weight one |
| L-12 | Two `Capability` enums | `src/protocol/effect.rs:58–70` (9 variants) vs `src/app_permissions.rs:21–34` (6 variants) | Zero interop; the typed enum in `protocol/` is unused |
| L-13 | `PipeSend` silently drops payloads | `src/typed_pipes.rs` `send_json`; `src/process_app/routing.rs:289` TODO | Wire is open; peer routing unimplemented |
| L-14 | DrawCommands `Image`, `VideoPlayer`, `AudioMeter`, `AudioPlay`, `AudioCapture` not in `app_protocol.rs` | `docs/specs/releases/plexi-v3.0.md:§3.3` lists them | Any app emitting these falls off the deserialize path |
| L-15 | `DrawCommand::Text.bold` parsed and ignored | `src/process_app/render.rs:43` (`bold: _`) | Rendering is silently incomplete |
| L-16 | Subprocess inherits full host env (`ANTHROPIC_API_KEY` et al.) | `src/process_app/mod.rs:127–133` — no `.env_clear()` | Spec invariant I-1 (isolation) violated at the env layer |
| L-17 | No explicit `close_fds` / `O_CLOEXEC` on non-stdio host FDs | `ProcessApp::launch()` | `fork+exec` leaks any FD not marked CLOEXEC; host's `UnixListener`s and log FDs could leak |
| L-18 | Manifest schema diverges from spec: code uses `[app.capabilities]`, spec says `[launch]` | `src/app_registry.rs:55–84` vs `docs/specs/releases/plexi-v3.0.md:§8` | `layout_hint` is a flat `String` in code; spec calls for structured `{ side, split }` |
| L-19 | `src/cli.rs` scaffolder emits v2 capability names (`terminal_write`, `filesystem = "read_only"`) | `src/cli.rs:288,315` | Every scaffolded app starts out broken |
| L-20 | `AppSpawned` event sent by host; Python SDK has no handler | `src/app/mod.rs:391`; `sdk/python/plexi_sdk/__init__.py` has no `elif t == "app_spawned"` | Spawned-app confirmation silently dropped |
| L-21 | `pgap_test_harness.rs` has zero `#[test]` functions | confirmed via grep | Spec's Layer 1 Rust tests don't exist; CI step is vacuous |
| L-22 | CI workflow runs `cargo test pgap_test_harness` — matches zero tests | `.github/workflows/plexi-v3-test.yml:28` | Green CI means nothing; no real gate |
| L-23 | `scripts/smoke-test.sh` only checks "no panic in log" | `scripts/smoke-test.sh:29` | A host that starts, renders nothing, and idles for 2s passes |
| L-24 | No `pyproject.toml` despite `CLAUDE.md` mandate | grep | Python tests run on bare system interpreter; not reproducible |
| L-25 | `src/protocol/` module mostly dead | `protocol/{effect,event,output,schema}.rs` have no callers outside `protocol/` | 332 LOC of tombstone |
| L-26 | `src/input/` module entirely dead | `src/input/{mod,focus,keymap}.rs` | 98 LOC, no production caller |
| L-27 | `src/error.rs` dead | no callers | 30 LOC |
| L-28 | `src/media/mod.rs` is a 3-line tombstone | `src/media/mod.rs:1–3` | |
| L-29 | 16 `#[allow(dead_code)]` on whole module declarations in `src/main.rs` | `src/main.rs:9,11,13,20,22,27,33,38,40,45,47,52,54,57` | Some are legitimate, some silence now-wired modules |
| L-30 | `src/plexi_iq/{prompt,context,tools}.rs` are stubs | file-level comments mark them "Stage 0/1 TODO" | Agent pane cannot operate tools; prompt is empty |
| L-31 | `src/app/mod.rs` 955 LOC, `src/pane_ops.rs` 932 LOC, `src/file_browser/mod.rs` 983 LOC | wc | Split candidates; hard to navigate |
| L-32 | `lsregister -f` and `pbs -update` documented but not in `justfile` | `CLAUDE.md:67` vs `justfile` grep | macOS Services registration broken after install |

### 1.2 What's actually working

Real, solid, not-touched-in-this-plan:
- `src/host/model.rs`, `src/host/harness.rs` — pure state machine + 26 passing tests; the target API
- `src/headless_renderer.rs` — PGAP → PNG pipeline via tiny-skia; 5 tests
- `src/typed_pipes.rs` — binary/JSON pipe registry with unix sockets and ring buffers
- `src/secrets.rs` — directory-scoped keychain broker (macOS)
- `src/plexi_iq/backend/claude_cli.rs` — `claude -p --output-format stream-json` subprocess streaming
- `sdk/python/plexi_sdk/widgets/` — `ScrollState`, `TextBuffer`, `TextArea`; 81 tests
- `src/event_log.rs` — global + workspace JSONL append infrastructure (wired but not yet the `HostServices::event_sink`)
- `examples/{snake,todo,wikipedia,quick-note}/tests/` — 13 Layer 1 Python tests via `pgap_test_harness` Python harness

### 1.3 Dead / near-dead modules (for deletion)

| Module | LOC | Rationale for deletion |
|---|---|---|
| `src/protocol/effect.rs` | 123 | Parallel to `src/app_protocol.rs::DrawCommand`; no callers |
| `src/protocol/event.rs` | 68 | Parallel `PlexiEvent`; `InjectState` + `HttpResponse` extras folded into `app_protocol.rs` in Step 2 |
| `src/protocol/output.rs` | 52 | No callers |
| `src/protocol/schema.rs` | 59 | `PaneId(u64)` newtype parallels `tiling::PaneId = u64`; no callers |
| `src/protocol/view.rs` | 149 | **KEEP** — used by `HeadlessRenderer` for typed view AST |
| `src/input/mod.rs` | 2 | Re-exports only |
| `src/input/focus.rs` | 12 | No callers |
| `src/input/keymap.rs` | 84 | `keys.rs` is the real keymap |
| `src/error.rs` | 30 | No callers |
| `src/media/mod.rs` | 3 | Tombstone |

Total: **582 LOC to delete.**

---

## Part 2 — End-State (v3.0 ship-ready, future-primitives installed)

### 2.1 The 10 properties the finished v3 must satisfy

1. **HostModel is the single source of truth.** Every workspace mutation enters via typed `HostCommand`; every observable output leaves as `HostEffect`. No business logic in the renderer. (`docs/specs/subsystems/host-architecture.md:§2`)
2. **Pane ADT frozen at 3 variants** (`Terminal`, `App`, `Agent`). New pane-shaped things are PGAP apps. (`docs/specs/releases/plexi-v3.0.md:§2`)
3. **PGAP is the sole isolation boundary.** NDJSON over piped stdio; binary on unix-socket typed pipes. No shared memory. No inherited FDs. Env clean except whitelist. (`docs/specs/subsystems/host-architecture.md:§5.1`)
4. **Directory-scoped secrets, no escalation path.** Host validates `workspace_root` against actual CWD at spawn. (`docs/specs/releases/plexi-v3.0.md:§5`)
5. **egui is a pure renderer; zero business logic.** Production renderer reads HostModel state, translates input to `HostCommand`. CI renderer (tiny-skia) substitutable. (`docs/specs/releases/plexi-v3.0.md:§10.4`)
6. **All three test layers run under `cargo test` + `pytest`.** Layer 1 (PGAP subprocess), Layer 2 (HostHarness), Layer 3 (headless PNG). Zero GUI, zero real hardware. (`docs/specs/subsystems/testing-infrastructure.md:§1`)
7. **Host-owned media with binary side channel.** Audio device, video decoder host-owned. Mock devices via `PLEXI_AUDIO=mock://`, `PLEXI_VIDEO=mock://`. (`docs/specs/releases/plexi-v3.0.md:§7`)
8. **Plexi IQ wired from commit #1 with no dead code.** `Backend` trait: `ClaudeCli | AnthropicApi | Mock`. `ledger.jsonl` written per turn. (`docs/specs/releases/plexi-v3.0.md:§9`)
9. **First-party apps use no special host access.** `quick-note`, `file-browser`, `secrets-manager` declare capabilities like any third-party app. (`docs/specs/releases/plexi-v3.0.md:§11.1`)
10. **Capability broker is the complete permission surface.** Declared capabilities auto-granted; undeclared ones prompt. Decisions persist to `permissions.json`. (`docs/specs/releases/plexi-v3.0.md:§4`)

### 2.2 Primitive set — the building blocks future features plug into

**(a) State machine**
- `HostModel` — pure Rust, zero egui imports (compile-enforced)
- `HostCommand` — exhaustive mutation vocabulary
- `HostEffect` — exhaustive output vocabulary, every variant has a consumer
- `HostContext` — per-tab state: panes, focus, groups, permissions
- `HostServices` — trait objects for all system boundaries: `event_sink`, `fs`, `secrets`, `net`, `spawn`

**(b) App protocol / PGAP**
- PGAP v3 Init/Ready handshake with 3s deadline
- `PlexiEvent` — single canonical enum with every variant the SDK handles
- `DrawCommand` — single canonical enum covering the full spec §3.3
- `manifest.toml` — schema-validated at load; `[launch]` section, structured `layout_hint`
- `pgap_test_harness` — Rust-side spawn/drive/assert driver with >0 integration tests

**(c) Tree / layout**
- `egui_tiles` tree, host-owned — apps cannot mutate directly
- `OpenPaneRequest` — kind, placement, share, group, capabilities
- Contexts (workspace tabs) — isolated per-tab state
- Pane groups — named; `PathChanged` broadcasts to members

**(d) Capabilities / secrets**
- 9 capabilities in one enum, runtime-enforced
- Directory-scoped secret broker; keychain keys `plexi/{workspace_root}/{key}`
- `MockSecretsService`, `MockFsService`, `MockNetService` for tests

**(e) Event bus / observability**
- `events.jsonl` — `FileEventSink` wired as production `HostServices::event_sink`
- `VecEventSink` — tests only
- `ledger.jsonl` — per-turn agent cost log
- Runs palette, notification log with 3 action types

**(f) Testing**
- Layer 1: Rust `pgap_test_harness` tests with ≥5 `#[test]` functions; Python tests runnable via `uv run pytest`
- Layer 2: `HostHarness` — every `HostCommand` and `HostEffect` touched by ≥1 test
- Layer 3: `HeadlessRenderer` with reference-PNG snapshot comparison
- Real smoke test: launches v3, spawns an app, drives input, asserts render output (not just "no panic")
- CI gate runs: `cargo test --lib`, `cargo test --lib host`, `cargo test --test pgap_integration`, `pytest sdk/python/tests/`, `pytest examples/*/tests/`, `scripts/smoke-test.sh`

### 2.3 Load-bearing invariants

| # | Invariant | Enforced by |
|---|---|---|
| I-1 | `src/host/*` imports zero egui; compiles for `wasm32-unknown-unknown` | Cargo feature gate / custom lint / CI cross-compile |
| I-2 | No `todo!()` / `unimplemented!()` outside `#[cfg(test)]` | `#![deny(clippy::todo, clippy::unimplemented)]` in `main.rs:6` |
| I-3 | No `#[allow(dead_code)]` on module declarations; every live module is called | grep gate in pre-commit or CI |
| I-4 | Every `HostEffect` variant has ≥1 consumer OR is explicitly `#[cfg(test)]`-only | Compile-time match exhaustiveness + CI check |
| I-5 | Pane ADT is exactly 3 variants | Freeze + spec amendment required to change |
| I-6 | Subprocess env is cleaned at spawn (whitelist: `HOME`, `PATH`, `LANG`, `LC_ALL`, `TERM`, `PLEXI_*`) | Explicit `.env_clear()` + whitelist in `ProcessApp::launch` |
| I-7 | Non-stdio FDs marked `O_CLOEXEC`; no inheritance | Explicit `close_fds` or rely on Rust's default CLOEXEC for newly-opened FDs; audit each `UnixListener` open |
| I-8 | Apps use `frame_timestamp` from `Render` event, never `time.time()` / `random` | Determinism requirement in docs; not auto-enforced, but reviewed |
| I-9 | All three test layers green + smoke test pass before reporting done | Ship gate in `just install-v3` |
| I-10 | Capability grants scoped by `(app_id, workspace_root, capability)`; re-checked on each spawn | `permissions.json` persistence + runtime check at `OpenPane` |

### 2.4 Future-feature landing pads

| Future feature | Primitive it plugs into | Ready after step |
|---|---|---|
| Multi-agent / sub-agent panes | `spawn.app` capability + typed pipes (duplex JSON) + per-agent capability sets | 7, 9 |
| WASM app runtime | `src/host/*` compiles for wasm32; PGAP Init/Render/Key/Shutdown map 1:1 to WASM component exports | 12 (I-1 enforced) |
| Spatial zoom / recursive nesting | Separate proposal; v3.0 rejects it. Primitive NOT in v3.0 scope; plan leaves HostModel clean enough to add a `Canvas` concept later without rewriting | n/a (intentional non-goal) |
| SpacetimeDB sync (Plexi Teams) | `events.jsonl` event bus + directory-scoped secrets invariant (I-2) + `permissions.json` | 6, 9 |
| Claude Code resume as Layer 2 LLM | `plexi_iq::Backend` trait; `ClaudeCliBackend` already wired | 3 |
| OSC 7 terminal cwd sync | `Pane::Terminal` + `HostCommand::SimulatePathChanged` (real source, not simulated) | 8 |
| Live-edge preview terminals | UNCITED in v3 spec; would need terminal-cell-grid exposure as `DrawCommand` stream. Primitive doesn't exist; plan leaves room | future |
| Excalidraw pane / browser pane | `Pane::App` (is already the right boundary — I-5 freeze); may need richer `DrawCommand` primitives (bezier/SVG) | 9 (Image baseline); richer draws = future |
| Community app distribution | Capability broker (I-10) + manifest schema (step 8) is the complete trust surface | 7, 8 |

### 2.5 Anti-goals (prevent wandering)

Explicitly deferred or rejected in v3.0: fractal PGAP, `Pane::Embedded`, depth tree, `plexi --embedded` / overlay mode, gRPC, live in-process DSP plugin surface, MIDI subsystem, in-place v2→v3 upgrade tooling, spatial canvas, rich-text/IME/multiline-input primitives, WASM sandbox for v3.0 (deferred to v3.1+), `#[allow(dead_code)]` on IQ module, example apps beyond the five. (`docs/specs/releases/plexi-v3.0.md:§1,§12`)

---

## Part 3 — The 12-Step Refactor Plan

Each step: goal, exact files touched, acceptance test, `Breaks if:` gate (the observable symptom if the step regresses), rough effort. Steps 1–3 (clean up) can run in parallel. Steps 4–6 are sequential (foundation). Steps 7–9 can run in parallel after 4–6. Steps 10–12 are sequential (lockdown).

### Phase A — Clean up the half-refactor (steps 1–3, parallel)

---

#### Step 1 — Dead code sweep

**Goal.** Remove every orphan module, parallel enum, stale comment, and broken scaffolder so the tree stops lying about what's live.

**Touches.**
- Delete: `src/protocol/effect.rs`, `src/protocol/event.rs`, `src/protocol/output.rs`, `src/protocol/schema.rs`, `src/input/` (entire), `src/error.rs`, `src/media/mod.rs`. Update `src/protocol/mod.rs` to re-export only `view`. Update `src/main.rs` to drop `mod input;`, `mod error;`, `mod media;`.
- Delete all `#[allow(dead_code)]` on module declarations in `src/main.rs` where the module is now consumed. Keep only genuinely-pending ones with an inline comment naming the step they unlock.
- Fix `src/cli.rs` scaffolder (lines 288, 315) to emit v3 manifest: `[app.capabilities]\ncapabilities = ["fs.read"]\n`.
- Remove stale TODO comments: `src/app_permissions.rs:7` (`check_command` doesn't exist), `src/plexi_iq/backend/mod.rs:7` (`agent_llm.rs` reference), `src/app/mod.rs:44` (`// TODO Phase B: consume host model`).

**Acceptance.** `cargo build --release` warning count drops to zero on dead-code warnings. `rg "allow\(dead_code\)" src/main.rs | wc -l` drops to ≤4 (the legitimate test-only stubs). `rg "TODO Phase" src/` is empty.

**Breaks if:** the v3 binary fails to scaffold a new app via `plexi-v3 app new`, or any example app fails to build because of a dropped import.

**Effort.** ~1 hour.

---

#### Step 2 — Unify dual types

**Goal.** Pick one canonical representation for every concept and delete the alternates.

**Touches.**
- **`Direction`:** canonical is `keys::Direction`. Delete `host::command::Direction`. Replace `use crate::host::command::Direction` sites with `keys::Direction`. Update `src/pane_ops.rs:578–583` to drop the manual mapping.
- **`PaneId`:** canonical is `tiling::PaneId = u64` (type alias). Delete `protocol::schema::PaneId(u64)` newtype. (Step 1 already deletes `schema.rs`.)
- **`Capability`:** canonical is `app_permissions::Capability`. Extend it to cover all 9 spec capabilities (add `AudioRecord`, `AudioPlayback`, `VideoPlayback` — wired from spec §4). `host::command::Capability = String` stays as the spec'd serialization form, but add a `TryFrom<&str>` that errors on unknown strings (stop the silent `→ FsRead` fallback at `src/app_permissions.rs:51–64`).
- **`PlexiEvent`:** canonical is `app_protocol::PlexiEvent`. Add `InjectState { payload }` and `HttpResponse { request_id, status, body, error }` variants (currently only in deleted `protocol/event.rs`). Update `pgap_test_harness.rs` to construct them via the typed enum instead of raw JSON.
- **`DrawCommand`:** canonical is `app_protocol::DrawCommand`. Add the variants currently only in deleted `protocol/effect.rs`: `Image`, and placeholders for `VideoPlayer`, `AudioMeter`, `AudioPlay`, `AudioCapture` (routing lands in step 9).

**Acceptance.** Each of `Direction`, `PaneId`, `Capability`, `PlexiEvent`, `DrawCommand` is defined in exactly one file. `cargo test` green. Manifest-loading test rejects unknown capability strings with a clear error.

**Breaks if:** launching any example app errors with "unknown capability", or `cargo check` reports a missing variant in a match-exhaustive arm for these enums.

**Effort.** ~2 hours.

---

#### Step 3 — Finish or delete stubs

**Goal.** Every module in the tree is either LIVE or test-only — nothing STUB.

**Touches.**
- `src/plexi_iq/prompt.rs` — either implement `build_system_prompt(...)` or delete the file. Recommendation: delete, plexi_iq can operate without a templated system prompt for v3.0.
- `src/plexi_iq/context.rs` — either add the `session` / `app_bus` / `plexi_ctx` fields the TODO mentions, or delete the stub fields and simplify. Recommendation: simplify to the 2 fields actually used.
- `src/plexi_iq/tools/mod.rs` — either register ≥1 tool or delete the ToolRegistry. Recommendation: delete for v3.0; re-add when a real tool needs registering.
- `examples/video-player/video_player.py` — currently a STUB (no decoding). Either wire it to the real binary pipe in step 9 or remove it from the ship set.

**Acceptance.** Grep of `TODO (Stage 0|Stage 1)` returns zero. No module is a pure tombstone.

**Breaks if:** Plexi IQ pane fails to open or crashes on spawn, or `video-player` app fails to install.

**Effort.** ~1 hour (mostly deletion).

---

### Phase B — Foundation: HostModel becomes authoritative (steps 4–6, sequential)

---

#### Step 4 — Pane ID reconciliation

**Goal.** `HostModel` owns pane ID allocation. `PlexiApp::next_pane_id` is deleted. Every pane allocation site (8+ in `pane_ops.rs` + 1 in `app/mod.rs`) routes through HostModel.

**Touches.**
- Make `HostModel::alloc_pane_id()` public.
- In `src/pane_ops.rs`: at every `new_id = self.next_pane_id; self.next_pane_id += 1` site, replace with `new_id = self.host.alloc_pane_id()`. Order matters — must allocate BEFORE submitting the command if the command also allocates.
- **Simpler:** have the submit helper return the allocated ID from the effect. `HostModel::open_pane` / `split` already return `PaneOpened { pane_id }`. Change call sites to use the returned ID directly instead of allocating separately.
- Delete `PlexiApp::next_pane_id` field at `src/app/mod.rs:26`. Update `WorkspaceFile` persistence (`src/workspace.rs:12`) — `next_pane_id` moves under HostModel's persisted state.
- Add `HostCommand::HydrateContext { panes: Vec<HostPane>, focused: Option<PaneId> }` for workspace restore. Update restore path in `src/app/mod.rs` to submit HydrateContext for each saved context instead of inserting panes directly.

**Acceptance.** A new Layer 2 test `ids_synchronize_across_commands` asserts: after 3 `OpenPane` + 2 `SplitVertical`, the IDs returned by HostModel effects match the IDs in the `ctx.panes` map.

**Breaks if:** a new app launch produces `PaneOpened { pane_id: 5 }` but the egui tile is inserted with `pane_id: 3` (or any ID mismatch visible in `plexi-v3.log` debug output). Workspace restore loads the saved file but IDs renumber from scratch.

**Effort.** ~3 hours.

---

#### Step 5 — `HostServices` gets `fs`, `secrets`, `net`, `spawn` trait objects

**Goal.** Every side-effect (filesystem read/write, secrets read, HTTP request, subprocess spawn) flows through a `HostServices` trait object. Tests can inject mocks.

**Touches.**
- Extend `src/host/services.rs`:
  ```
  pub struct HostServices {
      pub event_sink: Box<dyn EventSink>,
      pub fs: Box<dyn FsService>,
      pub secrets: Box<dyn SecretsService>,
      pub net: Box<dyn NetService>,
      pub spawn: Box<dyn SpawnService>,
  }
  ```
- Define each trait with a minimal surface (`FsService::read(&Path) -> Result<Vec<u8>>`, `SecretsService::get(&str, &Path) -> Result<Option<String>>`, `NetService::http_get(&str) -> Result<HttpResponse>`, `SpawnService::launch(&InstalledApp, &Path, &[String]) -> Result<ProcessApp>`).
- Production impls: wrap `src/secrets.rs`, `reqwest::blocking::Client`, `std::fs`, `src/app_registry.rs`.
- Test impls: `MockFsService`, `MockSecretsService` (inject values), `MockNetService` (URL → body map — replaces pgap_test_harness's current mock_http kludge), `MockSpawnService`.
- Migrate `src/process_app/routing.rs::SecretGet` handling to call `services.secrets.get(...)` instead of `crate::secrets::get_secret_scoped(...)` directly.

**Acceptance.** A new Layer 2 test `mock_secrets_unblocks_app` asserts: harness → `OpenPane(app)` → `SecretGet` → `MockSecretsService` returns `Some("v")` → next render shows the value.

**Breaks if:** `just install-v3` launches but SecretGet for any real secret fails silently (production secrets service wiring regression).

**Effort.** ~4 hours.

---

#### Step 6 — Event sink + effect consumption

**Goal.** Every `HostEffect` has a production consumer. Slices 3 and 4 (close, navigate) stop being observation-only. `events.jsonl` becomes the real event bus.

**Touches.**
- Replace `NoopEventSink` in production with `FileEventSink` that appends to `~/.plexi-v3/events.jsonl` (wire via `src/event_log.rs`, which already has the append infrastructure).
- In `src/app/mod.rs`, after each `self.submit(...)`, dispatch the returned effects:
  - `FocusChanged { pane_id }` → `contexts[active].focused_pane = find_tile_for(pane_id)` (Step 4 made this well-defined).
  - `PaneClosed { pane_id }` → `close_tile(ctx_idx, find_tile_for(pane_id))`. Delete the redundant `close_tile` call in `pane_ops::close_focused`.
  - `ContextCreated { index }` / `ContextSwitched { index }` → update `active_context`.
  - `PathBroadcasted { group, cwd, recipient_pane_ids }` → iterate recipients, send `PlexiEvent::PathChanged` to each app pane's runtime.
- Delete the parallel find-pane-in-direction code in `pane_ops::navigate` (Step 4 + 6 replaces it). The renderer no longer does directional geometry — HostModel owns it.
- Wait: HostModel's current `Navigate` is linear-index, not directional. Step 6 needs HostModel to know tile geometry OR PlexiApp keeps driving nav and HostModel observes. Simpler: keep PlexiApp's geometric nav, but after PlexiApp computes the target, it submits `FocusPane(target)` and HostModel applies. PlexiApp then consumes `FocusChanged` to drive the egui focus update.

**Acceptance.** `events.jsonl` is written after every command (verify with `tail -f ~/.plexi-v3/events.jsonl` during manual test). A new Layer 2 test asserts a consumer for every `HostEffect` variant — if a variant has no match arm in PlexiApp and no `#[cfg(test)]` marker, the test fails.

**Breaks if:** `~/.plexi-v3/events.jsonl` stops growing when the user takes an action. Close-pane or navigate regresses to "works but logs nothing" or "doesn't work." A `HostEffect` variant lands without a consumer.

**Effort.** ~4 hours.

---

### Phase C — PGAP spec compliance (steps 7–9, parallel after Phase B)

---

#### Step 7 — Capability enforcement complete

**Goal.** All 9 spec capabilities exist in the enum. All are enforced at runtime. Manifest validation fails loudly on unknown strings.

**Touches.**
- Step 2 already added `AudioRecord`, `AudioPlayback`, `VideoPlayback` to `app_permissions::Capability`. Step 7 enforces them.
- `src/process_app/routing.rs`: add capability check at the top of every DrawCommand handler that maps to a spec capability. `FsRead` checked before any `fs.read`-emitting DrawCommand; `FsWrite` before any writer; `NetHttp` before the HTTP broker (step 9) dispatches; `AudioRecord` before any audio.record route; etc.
- Delete `From<&str> for Capability` silent-fallback-to-FsRead. Replace with `TryFrom<&str>` that errors.
- Manifest loader (`src/app_registry.rs::scan_apps_dir`) fails to install apps whose `capabilities` array contains an unknown string. Log the offending app + bad capability; skip it.
- `check_cd()` — currently never called. Either wire it into `DrawCommand::cd` routing or delete it. Recommendation: delete; `cd` isn't a spec DrawCommand.

**Acceptance.** New Layer 2 test `capability_enforcement_matrix` iterates all 9 capabilities × {declared+granted, declared+denied, undeclared} = 27 cases, asserts correct effect for each. Manifest-loading test asserts: app with `capabilities = ["bogus"]` fails to install with a named error.

**Breaks if:** an installed app can read/write files without `fs.read` / `fs.write` declared, or an app with a typo in `capabilities` launches silently.

**Effort.** ~3 hours.

---

#### Step 8 — Manifest schema freeze

**Goal.** One canonical manifest schema, matching the spec. Validated at load. Scaffolder emits it. All examples updated.

**Touches.**
- Migrate `AppManifest` in `src/app_registry.rs:55–84`:
  ```
  [app]
  id = "todo"
  name = "Todo"
  version = "0.1.0"
  entry = "todo.py"

  [app.capabilities]
  capabilities = ["fs.read", "fs.write"]

  [launch]
  join_group = "cwd"
  layout_hint = { side = "right", split = 0.4 }
  keyboard_capture = false
  ```
- Drop flat `layout_hint: Option<String>` and `initial_share: Option<f32>`. Replace with structured `LaunchSection { join_group, layout_hint: LayoutHint { side, split } }`. `side` ∈ {`"right"`, `"below"`, `"overlay"`}.
- Update every manifest under `examples/*/manifest.toml` to new schema.
- Update `src/cli.rs` scaffolder templates.
- Schema validator runs at load; apps with missing required fields fail.
- `keybinding` field: decide — either wire it into a global shortcut registrar or delete. Recommendation: delete for v3.0; re-add when an app actually uses it.

**Acceptance.** `manifest_schema_test` parses every example manifest and reports exact fields found. `manifest_rejection_test` asserts: missing `entry`, bad `layout_hint.side`, unknown `join_group` (if validated) each produce a named error.

**Breaks if:** any example app fails to install after schema migration, or `plexi-v3 app new foo` produces a manifest that fails validation.

**Effort.** ~2 hours.

---

#### Step 9 — PGAP protocol surface completion

**Goal.** Close every PGAP gap so the v3 spec §3 matches the code exactly.

**Touches.**
- **Host HTTP broker.** Route `DrawCommand::HttpRequest { request_id, url, method, headers, body }` through `services.net.http_*`. Reply with `PlexiEvent::HttpResponse { request_id, status, body, error }`. Deletes the pgap_test_harness's custom mock_http machinery — mocks now flow through `MockNetService` (step 5).
- **Missing DrawCommands.** Add variants (step 2 added shapes; step 9 routes them):
  - `Image { src, x, y, w, h, fit }` → `render.rs` paints via egui texture.
  - `VideoPlayer { source, rect, state }` → host-owned video decoder emits frames on a binary pipe.
  - `AudioMeter { rect, pipe_id }` → render amplitude from a pipe.
  - `AudioPlay { source | pipe_id, volume, state }` → host-owned `rodio` playback.
  - `AudioCapture { pipe_id, sample_rate, buffer_size }` → host mic → PCM → binary pipe.
- **`PipeSend` peer routing.** `src/typed_pipes.rs::send_json` currently drops payloads. Implement: look up pipe_id's registered peers, enqueue `PlexiEvent::PipeMessage` to each. Removes TODO at `routing.rs:289`.
- **`AppSpawned` SDK handler.** Add `elif t == "app_spawned"` branch to `sdk/python/plexi_sdk/__init__.py`; call new `on_app_spawned(pane_id, type_id)` hook.
- **`RunUpdate { completed }` round-trip.** When host receives `RunComplete`, emit `PlexiEvent::RunUpdate { run_id, status: "completed", payload }` back to the originating app.
- **`bold` text rendering.** `src/process_app/render.rs:43` — stop destructuring `bold: _`; read the field and pass to painter.
- **Env isolation.** `src/process_app/mod.rs`: add `.env_clear()` + whitelist. Preserve `HOME`, `PATH`, `LANG`, `LC_ALL`, `TERM`, `USER`, `SHELL`, and any `PLEXI_*` vars. Explicitly strip `ANTHROPIC_API_KEY` and similar credentials.
- **FD isolation.** Audit every `UnixListener::bind` and other FD open call in the host. Ensure Rust's default `SOCK_CLOEXEC` / `O_CLOEXEC` is on; explicit `fcntl(FD_CLOEXEC)` where not. No inherited FDs to subprocess except stdio.

**Acceptance.**
- `http_broker_test` (Layer 1 or 2): app sends http_request, `MockNetService` resolves, app receives http_response. Without pgap_test_harness's custom mock.
- `env_clean_test`: spawn app with `ANTHROPIC_API_KEY=xyz` in host env; app logs its env; assert `ANTHROPIC_API_KEY` not present.
- `fd_inheritance_test`: open a `UnixListener` in host, spawn app, verify app's `/proc/self/fd` or equivalent does not include the listener.
- Every `DrawCommand` variant in `app_protocol.rs` has a routing match arm.

**Breaks if:** `wikipedia` app hangs forever (HTTP broker regression), app subprocess can read host's `ANTHROPIC_API_KEY`, an app using `Image` or `AudioPlay` silently renders nothing, or `PipeSend` from app A fails to deliver to app B on the same pipe_id.

**Effort.** ~6 hours.

---

### Phase D — Testing & CI (steps 10–11, sequential)

---

#### Step 10 — Real Rust Layer 1 tests + `uv` Python runner

**Goal.** `src/pgap_test_harness.rs` actually has tests. Python tests are reproducible under `uv`.

**Touches.**
- Move `pgap_test_harness` from `#[cfg(test)]`-only to a `tests/` integration dir OR add at least 5 `#[test]` functions inside the existing module. Cover:
  - `init_ready_handshake`: app sends Ready within 3s of Init.
  - `render_then_inject_state_then_render`: state persists across renders.
  - `http_broker_happy_path`: with `MockNetService`.
  - `secret_broker_happy_path`: with `MockSecretsService`.
  - `app_crash_at_startup`: app panics immediately; host reports error, doesn't hang.
  - `capability_prompt_flow`: app requests undeclared capability; host emits `CapabilityPromptRequired`; test grants; app unblocks.
- Create `pyproject.toml` at repo root or at `sdk/python/` per `CLAUDE.md:91`: `requires-python = ">=3.11"`, declare test deps (`pytest`), define `tool.pytest.ini_options` with `testpaths = ["sdk/python/tests", "examples/*/tests"]`.
- `just test-py` recipe: `uv sync && uv run pytest`.
- Update `.github/workflows/plexi-v3-test.yml` to match (step 11 lands that).

**Acceptance.** `cargo test --test pgap_integration` runs ≥5 tests, all green. `uv run pytest` from repo root runs all 13 example tests + 81 widget tests = 94+ tests.

**Breaks if:** `cargo test pgap_test_harness` continues to match zero tests, or `uv run pytest` fails to resolve imports.

**Effort.** ~4 hours.

---

#### Step 11 — CI gate that actually enforces

**Goal.** CI runs the full test matrix. Smoke test verifies real render output. `just install-v3` runs `lsregister` + `pbs`.

**Touches.**
- `.github/workflows/plexi-v3-test.yml` — replace current workflow with:
  1. `cargo test --lib` (all lib tests including `host::`)
  2. `cargo test --test pgap_integration`
  3. `uv sync && uv run pytest`
  4. `scripts/smoke-test.sh`
- `scripts/smoke-test.sh` — beyond "no panic", add: spawn `snake` app, send a Render event, assert host log shows `frame_done` within 3s. Exit non-zero on failure.
- `justfile:install-v3` — append `lsregister -f "/Applications/Plexi v3.app"` and `/System/Library/CoreServices/pbs -update` (macOS Services registration, per `CLAUDE.md:67` which currently lies about these running).
- Pre-commit hook (installed via `.githooks/` + `core.hooksPath`) that rejects `#[allow(dead_code)]` on module declarations in `src/main.rs` unless the line has an inline comment `// STEP-N` explaining why.

**Acceptance.** CI for a deliberate regression in each category fails loudly: a broken host test, a broken integration test, a broken Python test, a host that starts but renders nothing. `just install-v3` registers the app with macOS Services (right-click → Services shows Plexi entries).

**Breaks if:** CI green while a real test fails. Smoke test green for a host that renders nothing. Right-click Services menu doesn't show Plexi v3 entries after install.

**Effort.** ~3 hours.

---

### Phase E — Invariant hardening (step 12)

---

#### Step 12 — Invariant enforcement

**Goal.** The invariants from Part 2.3 become mechanically enforced, not just documented.

**Touches.**
- **I-1 (HostModel zero egui):** add a CI job that runs `cargo check --manifest-path src/host/Cargo.toml --target wasm32-unknown-unknown` once `src/host/` is extracted into its own crate (split from the main crate for this check; or add a build.rs check that greps `src/host/*.rs` for `use egui::` / `use eframe::` and fails).
- **I-3 (no module-level `#[allow(dead_code)]`):** pre-commit hook from step 11 covers this.
- **I-4 (every `HostEffect` has a consumer):** exhaustive-match test: the consumer dispatch in `src/app/mod.rs` must match-exhaustively on `HostEffect`; compile error if a variant is added without a consumer.
- **I-5 (Pane ADT frozen):** add `#[non_exhaustive]` or a spec-amendment comment. Document in `CLAUDE.md`.
- **I-6 (env isolation):** `env_clean_test` from step 9 locks this in; extend to verify whitelist is exact.
- **I-7 (FD isolation):** `fd_inheritance_test` from step 9.
- **I-10 (capability scoping):** test: grant `fs.read` to app `foo` at `/ws/a`; spawn same app at `/ws/b`; assert `CheckCapability` emits `CapabilityPromptRequired`.
- Ship gate: `just install-v3` green = v3.0 releasable. Tag `v3.0.0` and trigger release workflow.

**Acceptance.** Every invariant in Part 2.3 has at least one test or CI step that fails if the invariant is broken. `just install-v3` green; tag pushed; release artifact uploaded.

**Breaks if:** a future commit adds `use egui` to `src/host/`, adds a `HostEffect` variant without a consumer, or re-introduces the `AudioRecord` capability without adding it to the enum.

**Effort.** ~3 hours.

---

## Part 4 — Execution Discipline

### Order of operations

```
Parallel:  [Step 1] [Step 2] [Step 3]           ← Phase A cleanup
                   ↓
Sequential: Step 4 → Step 5 → Step 6            ← Phase B foundation
                   ↓
Parallel:  [Step 7] [Step 8] [Step 9]           ← Phase C spec compliance
                   ↓
Sequential: Step 10 → Step 11                    ← Phase D testing
                   ↓
            Step 12                              ← Phase E lockdown
```

Parallel = dispatch 3 Sonnet sub-agents simultaneously; orchestrator reviews, runs ship gate, commits each. Sequential = one after the other; ship gate between each.

### Ship gate (runs after every step)

```
cargo test --release                # all Rust tests green
cargo test --lib host --release    # host tests specifically
uv run pytest                       # Python tests
just install-v3                     # build + bundle + install + smoke test
```

All four green = step done. Anything red = revert or fix; do not advance to the next step.

### Commit discipline per step

- One commit per logical unit within a step (clean diffs, reversible).
- Each step ends with a `DEV_LOG.md` entry tagged `[CHANGED]` with `Breaks if:`.
- Update `CLAUDE.md` Current Queue on step completion — check off the step, surface any follow-ups discovered.

### Total estimated effort

| Phase | Steps | Estimate |
|---|---|---|
| A — Cleanup | 1, 2, 3 | 4 hours |
| B — Foundation | 4, 5, 6 | 11 hours |
| C — Spec compliance | 7, 8, 9 | 11 hours |
| D — Testing | 10, 11 | 7 hours |
| E — Lockdown | 12 | 3 hours |
| **Total** | | **~36 hours** |

Realistic wall-clock: 4–6 focused sessions, with parallel sub-agents on Phase A and Phase C steps. All code is AI-generated; the bottleneck is specification clarity (this document closes that) and verification between phases (ship gate is the guard).

---

## Part 5 — What This Plan Does NOT Do

Explicit non-goals to prevent scope creep during execution:

- Does not migrate `file_browser` from builtin to a PGAP app. (Step 3 flags it; future session.)
- Does not split `src/app/mod.rs` or `src/pane_ops.rs` into smaller modules. After step 6, those files will naturally shrink because HostModel absorbs their state logic. Further splits can follow organically.
- Does not add the spatial canvas, fractal PGAP, or `Pane::Embedded`. Those are v3.1+ proposals and explicit v3.0 non-goals.
- Does not ship the WASM app runtime. Step 12 enforces I-1 (HostModel zero egui), which unblocks v3.1+ WASM, but WASM itself is out of scope.
- Does not implement SpacetimeDB sync. Step 6 ships the `events.jsonl` bus that Plexi Teams will consume; the sync layer is a separate future proposal.
- Does not rewrite the Python SDK. Widgets and harness are in good shape; only protocol surface additions (step 9) and the new `on_app_spawned` hook (step 9) touch the SDK.

When these items come up mid-execution, file them under `.plexi/backlog/` and return to the step.

---

**End of plan.** Start with Step 1.
