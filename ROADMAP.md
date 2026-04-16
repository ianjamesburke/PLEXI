# Plexi Roadmap

> **V2 is the only active target.** Everything after this moment is v2 work.
>
> - **Vision (compass):** [docs/VISION.md](docs/VISION.md) — read first, always.
> - **Spec index (single source of truth for where every spec lives):** [docs/specs/README.md](docs/specs/README.md) — start here for any spec question.
>
> This roadmap is the **weekly operational view**. The scope doc (linked via the spec index) is the contract. When they disagree, the spec wins.

**Last updated:** 2026-04-16

---

## Where We Are

### On `main` (v1.1.2) — Stable

A polished spatial terminal multiplexer. Installable via `brew install --cask ianjamesburke/plexi/plexi`.

Working: multi-context tiling workspace, 36+ apps, external app protocol (JSON/stdio), Python + Rust SDK, file browser, audio player, secrets manager, agent mode (Ctrl+/), notification log + palette, Homebrew tap with auto-update on release.

### On `alpha` — In Flight

Everything on `main` plus:

| Feature | Status |
|---|---|
| Typed pipes Phase 0 — `PipeWrite`/`PipeData` bidirectional IPC | ✅ merged |
| `spawn_app` draw command — parent/child lifecycle | ✅ merged (host dispatcher pending) |
| SDK 0.3.0 — breakpoints decorator, `spawn_app` helper | ✅ merged |
| Mermaid diagram viewer + `--filter=mermaid` file browser | ✅ merged |
| Notification system — urgency, Unix socket ingestion, action types, focus-pane | ✅ merged |
| `docs/VISION.md` — foundational vision as source of truth | ✅ merged |
| `docs/specs/releases/plexi-v2.0.md` — full V2 technical spec + ship order | ✅ merged |
| Fractal PGAP POC (#260) — `.plexi` depth discovery, depth tree pane, v2 lifecycle/render summary wire types, embedded stdio smoke | 🚧 in PR |
| Homebrew release automation (SHA256 + cask update on tag) | ✅ merged |

---

## V2 — The Agent-Native Release

**All v2 specs:** [docs/specs/README.md](docs/specs/README.md) — index with scope, contract, subsystems, and proposals.
**Goal:** Apps can be skills. Agents can be apps. The host orchestrates. One install, three interfaces.

### Month 1 — Plumbing
| Item | Unlocks |
|---|---|
| Protocol version negotiation | Every subsequent change is additive |
| Host event bus (`events.jsonl`) | Cross-app observation; required by everything downstream |
| Fractal depth tree POC (#260) | `.plexi` boundaries are discoverable and visible before embedded rendering |
| `OpenIntent` payload on `Init` | Apps know *why* they were opened (file, prompt, caller) |
| `Run` primitive — dumb store, draw commands | Stateful multi-step tasks with blocked/running/done lifecycle |

### Month 2 — Surface
| Item | Unlocks |
|---|---|
| Rich notification actions (extends #219) | Notifications can wrap and resume a Run |
| Render summary protocol | Parent depths can request cheap status without full embedded rendering |
| Capability enforcement + `permissions.json` ✅ | Runtime Yes once / Yes always / No; persistent grants |
| Typed pipes Phase 1 — manifest `[app.io]`, auto-wire | Apps compose without code changes |

### Month 3 — Intelligence
| Item | Unlocks |
|---|---|
| `[app.skill]` manifest section | Plexi IQ skill registry — apps are invokable capabilities |
| `[app.agent]` manifest section | Installable agent apps: system prompt + tool allowlist |
| Embedded Plexi spike (`--embedded`) | PGAP stdin/stdout path for recursive instances; renderer internals still later |
| Plexi IQ Stage 1 — in-host orchestrator | Agent delegation, Run lifecycle, `/approve` workflow, `@agent` syntax |
| SDK 0.4.0 | `OpenIntent` + `Run` convenience methods; all examples migrated |

### V2 Product (ships with or after v2)
| Feature | Description |
|---|---|
| App registry + `plexi install` | Remote registry, user-global + `--local` project-scoped installs (#233) |
| App store | Discoverable catalog, developer publishing, one-click install |
| Plexi Intelligence (PGAP) | Hosted LLM gateway: audit, budget, model routing — all LLM calls route through here |
| Billing — Plexi Credits or BYOK | Users buy credits (Anthropic wrapper) or bring their own API key |
| `@agent` syntax | In agent mode, `@agentname` invokes installed `[app.agent]` apps; finds open instance first (#232) |

### Deferred to V2.1+
PGAP as protocol-level gateway, trust/risk float learning, agent replay testing, WASM/PWA, SpacetimeDB collaborative workspaces, notification undo (#223), spatial canvas Option B/C.

---

## Layer Status Snapshot

| Layer | Status | What's blocking |
|---|---|---|
| Layer 0 — Unblocked Now | **6/6 done** | — |
| Layer 1 — App Protocol Testing | **Done** (24 tests, 4 apps) | — |
| Layer 2 — Agent Mode in Terminal | **Shipped** (`claude -p --resume` backend + streaming done; slash commands + context loading remain) | — |
| Layer 3 — Parallax Refactor | **Done** (manifest-first, validator, Senior-only routing, Parallax viewer app) | — |
| Layer 4 — Apps That Prove the Protocol | **Done** (~36 apps on alpha) | App Store update management (in flight); test coverage gap (~22 apps) |
| Layer 4.5 — SDK Packaging & Protocol Stability | **Done** (Python + Rust SDK 0.3.0, protocol spec v1) | — |
| Layer 4.6 — App Composition Primitive | **Done** (host dispatcher, file browser wiring, photo-viewer + text-editor composition) | — |
| Layer 5 — WASM/PWA | **Back-burner** | Revisit when multiplayer is needed |

---

## Layer 0: Unblocked Now

**Status: ALL 6 DONE**

| Task | Type | Status | Reference |
|------|------|--------|-----------|
| Hot reload for app development | Code | **Done** | [#83](https://github.com/ianjamesburke/PLEXI/issues/83) |
| Theme: surface-specific hover tokens | Code | **Done** | [#70](https://github.com/ianjamesburke/PLEXI/issues/70) |
| Self-closing panes via OSC title | Code | **Done** | [#90](https://github.com/ianjamesburke/PLEXI/issues/90) |
| Finder Service "Open in Plexi" | Code | **Done** | — |
| App protocol test harness (`plexi_test.py`) | Code | **Done** | [#100](https://github.com/ianjamesburke/PLEXI/issues/100) |
| Issue triage cleanup | Ops | **Done** | DEV_LOG 2026-04-11 |

### Verification steps for Layer 0

1. **Hot reload** — edit `~/.plexi-alpha/apps/wikipedia/wikipedia.py`, save → app restarts within ~250ms
2. **Hover tokens** — hover a row in the file browser → row background visibly distinct
3. **Self-closing panes** — `printf '\e]0;plexi:close\x07'` in a pane → pane closes
4. **Finder Service** — right-click a folder → Services → "Open in Plexi" → Plexi opens that folder
5. **Test harness** — `python3 examples/wikipedia/tests/test_wikipedia.py` → 7 tests pass

---

## Layer 1: App Protocol Testing

**Status: DONE**

| Task | Status | Reference |
|------|--------|-----------|
| `plexi_test.py` test harness | Done | PR #101, merged |
| Test cases for existing Python apps | Done | 24 tests across hello-app, git-log, plexi-browser, wikipedia |
| CI integration | Deferred | Local testing sufficient for now |
| Rust universal test harness | Filed | [#102](https://github.com/ianjamesburke/PLEXI/issues/102) — future |

### Verification steps for Layer 1
```bash
python3 examples/wikipedia/tests/test_wikipedia.py          # 7 tests
python3 examples/git-log/tests/test_git_log.py              # 5 tests
python3 examples/plexi-browser/tests/test_plexi_browser.py  # 6 tests
python3 examples/hello-app/tests/test_hello_app.py          # 6 tests
```
All 24 should pass. Apps must be installed at `~/.plexi-alpha/apps/` first.

**Unlocks:** All app development iteration, Parallax app, app store

---

## Layer 2: Agent Mode in Terminal

**Status: SHIPPED. LLM backend + streaming done. Slash commands + context loading remain.**

The Warp-style inline agent UI is live. `claude -p --resume` backend is wired with per-pane session IDs, streaming tokens, and system-prompt-on-first-turn-only fix.

| Task | Status | Reference |
|------|--------|-----------|
| `Ctrl+/` mode switching, agent UI shell | **Done** | `src/agent_mode.rs`, `agent_ui.rs`, `agent_context.rs` |
| LLM backend: `claude -p --resume` subprocess wrapper | **Done** | `src/agent_llm.rs` — per-pane session ID, cancel, kill |
| Streaming responses | **Done** | `LlmResponse::Token` emitted per delta, rendered inline (#214) |
| Bare `/` at empty prompt | Open | [#104](https://github.com/ianjamesburke/PLEXI/issues/104) |
| Slash commands (/status, /cost, /jobs) | Not started | Agent Mode spec §4 |
| Agent context loading (lazy index) | Not started | Agent Mode spec §5 |
| Background job tracker | Not started | Agent Mode spec §7 |

### Verification steps for Layer 2
1. Open Plexi, focus a terminal pane, press `Ctrl+/`
2. Agent UI panel appears in the pane
3. Type a prompt and press Enter → real Claude response appears
4. Press Escape → returns to terminal mode

**Layer 2 MVP done when:** steps 1–4 pass with the `claude -p --resume` backend.
**Layer 2 polished when:** bare `/` detection works (#104), slash commands exist, streaming is live.

---

## Layer 3: Parallax Refactor

**Status: DONE**

| Task | Status | Reference |
|------|--------|-----------|
| Manifest-first refactor (editors write YAML, no ffmpeg) | **Done** | Parallax DEV_LOG 2026-04-12 |
| Manifest schema (Pydantic) + validator with feedback loop | **Done** | `packs/video/manifest_schema.py`, `manifest_validator.py`, 9 tests |
| Senior-only routing (drop Junior, simplify) | **Done** | HoP routes footage_edit directly to SeniorEditor on Sonnet |
| Parallax viewer app (companion pane in Plexi) | **Done** | Launched from command palette, companion pane |
| Cost reporting via cost_report events | **Done** | 11+ `cost_report` calls across `head_of_production.py`, `cost_tracker.py`, `improvement_officer.py` |
| SecretGet integration for API keys | Not started | Currently uses env vars — only Layer 3 item still pending |

### Verification steps for Layer 3
```bash
cd ~/Documents/GitHub/parallax
TEST_MODE=true python3.11 test/test_manifest_first.py       # 5 tests
TEST_MODE=true python3.11 test/test_manifest_validator.py    # 9 tests
TEST_MODE=true python3.11 test/test_regression.py            # 5 tests
```
All 19 should pass.

---

## Layer 4: Apps That Prove the Protocol

**Status: DONE — ~36 APPS ON ALPHA**

Mouse events and `delta_time` are in the protocol. The app ecosystem is live.

| Task | Status | Reference |
|------|--------|-----------|
| Mouse events protocol | **Done** | `MouseEvent` in draw protocol |
| `delta_time` in render frames | **Done** | `RenderContext.delta_time` |
| `get_state`/`set_state` protocol | **Done** | `src/process_app.rs` |
| `cost_report` protocol | **Done** | `src/cost_tracker.rs`, `~/.plexi-alpha/costs.jsonl` |
| Python SDK decorators (`@app.on_get_state`, etc.) | **Done** | `emit.cost_report()` |
| App Store (built-in) | **Shipped** | [#99](https://github.com/ianjamesburke/PLEXI/issues/99) |
| App Store: update management (version compare + badges) | **Done** | Version compare + UPDATE badges + 'u' keybinding. Verify after `just install-alpha`. |
| Test coverage for example apps (~22 of 36 apps lack `plexi_test.py` suites) | **In flight** | Partial coverage sprint merged — verify final count |

### Verification steps for Layer 4
1. Launch App Store from command palette → 32 apps listed
2. Install an app from the store → appears in `~/.plexi-alpha/apps/`
3. Open snake or aquarium → mouse events work, animation is smooth
4. Trigger a cost_report from a test app → check `~/.plexi-alpha/costs.jsonl`

---

## Layer 4.5: SDK Packaging & Protocol Stability

**Status: DONE (2026-04-14)**

Python + Rust SDK at 0.3.0 parity. Protocol spec stamped v1.

| Task | Status | Reference |
|------|--------|-----------|
| Python SDK packaged as `plexi-sdk` 0.2.0 | **Done** | `sdk/python/pyproject.toml`, `README.md`, `LICENSE`, `MANIFEST.in` |
| Vendored SDK sync script | **Done** | `scripts/sync-sdk.py` — 33 examples byte-identical to canonical |
| App infrastructure spec v1 | **Done** | `docs/specs/subsystems/app-infrastructure.md` (705 lines) |
| Rust SDK Cargo manifest publication-ready | **Done** | `sdk/rust/Cargo.toml` full publish metadata |
| **Rust SDK 0.2.0 protocol parity** | **Done** | `sdk/rust/src/lib.rs` — scroll, mouse, drop, state, cost_report, notification, feedback, log |
| **Python + Rust SDK 0.3.0** | **Done** | `spawn_app`, `BreakpointSet`/`pick_breakpoint`, `App::min_size`, `load_manifest_layout()` |
| Shell-config app v1 spec | **Done (spec only)** | `docs/specs/proposals/app-shell-config.md`. P3, idea-tier, not implemented |

### Verification steps for Layer 4.5

```bash
pip install -e sdk/python                           # Editable install of plexi-sdk 0.3.0
python3 scripts/sync-sdk.py --check                 # All vendored copies byte-identical
cd sdk/rust && cargo test                           # 5 breakpoint tests pass
```

---

## Layer 4.6: App Composition Primitive

**Status: DONE (2026-04-15)**

All spawn_app pieces landed in the `f8da18e` squash merge (PR #246). One app can now request that Plexi launch and place another app — fully end-to-end.

| Task | Status | Reference |
|------|--------|-----------|
| `DrawCommand::SpawnApp` + `SpawnParent`/`SpawnLayout`/`SpawnLifecycle` | **Done** | `src/app_protocol.rs` |
| `AppSpawnable` manifest table + `[app.spawnable]` | **Done** | `src/app_registry.rs`, `docs/specs/subsystems/app-infrastructure.md` |
| `pending_spawns` queue in `process_app.rs` | **Done** | `src/process_app.rs::take_pending_spawns()` |
| SDK: `Emitter.spawn_app` + `RenderContext.spawn_app` (Python + Rust) | **Done** | `sdk/python/plexi_sdk.py`, `sdk/rust/src/lib.rs` |
| Host dispatcher: pane creation, registry lookup, cascade/orphan walk | **Done** | `src/pane_ops.rs::dispatch_pending_spawns()` + `execute_spawn()` |
| File browser `→ text-editor` / `→ photo-viewer` wiring | **Done** | `src/file_browser/mod.rs` — emits `SpawnApp` on Enter for txt and image files |
| Typed-pipes spec | **Done (spec only)** | `docs/specs/subsystems/typed-pipes.md` — Phase 1 design |

### Verification steps for Layer 4.6

1. File browser: navigate to a `.txt` file, press Enter → Plexi spawns text-editor app in adjacent pane
2. File browser: navigate to an image file, press Enter → Plexi spawns photo-viewer in adjacent pane
3. Close file browser → text-editor and photo-viewer both close (cascade lifecycle)

---

## Layer 5: WASM/PWA (Back-burner)

**Status: DEFERRED. Revisit when multiplayer is needed.**

| Task | Status | Reference |
|------|--------|-----------|
| WASM Phase 1: feature-gate native deps | Deferred | [#105](https://github.com/ianjamesburke/PLEXI/issues/105), `docs/specs/proposals/wasm-pwa-deployment.md` |
| WASM Phase 2: WebSocket server mode | Not started | — |
| WASM Phase 3: WASM client with WS transport | Not started | — |
| WASM Phase 4: PWA manifest, service worker, touch | Not started | — |
| WASM Phase 5: Auth (Ed25519) | Not started | — |
| Directory sync (Tailscale + SpacetimeDB) | Not started | — |

---

## App Ecosystem (~36 apps on alpha)

### Games
snake, aquarium, sandfall, lichen, apiary, seedclock

### Utilities
clipboard-stack, port-watcher, pulse, process-monitor, calc, stopwatch, color-palette, json-viewer, diff-viewer, todo

### Productivity
pomodoro, markdown-preview, audio-player, weather, hacker-news

### Development
git-log, git-blame, navigator, pyflow, plexi-browser, text-editor (external Python), photo-viewer (external Rust)

### System
backlog-triage, permissions-viewer

### Dev Tools
spiral-viewer (render any app at 8 sizes on Fibonacci spiral)

### Plexi-native
parallax, learn-plexi, app-store, wikipedia, hello-app

---

## What's Next (Recommended Order)

### Immediate (this week)

1. **Host dispatcher for `spawn_app`** — drain `take_pending_spawns()` in `src/app.rs` (registry lookup, pane creation, cascade/orphan walk). This unblocks Layer 4.6 end-to-end.
2. **File browser wiring** — update `src/file_browser/mod.rs` to emit `spawn_app` on Enter for text/image files; demo the full file-browser → text-editor → photo-viewer composition flow.
3. **`sdk/python/SKILL.md`** — top-level agent guide: "how to build a Plexi app" self-documenting SDK reference for AI coding agents.
4. **Finish `claude -p --resume` backend swap** — streaming responses, session continuity (Layer 2)
5. **Bare `/` at empty prompt** ([#104](https://github.com/ianjamesburke/PLEXI/issues/104)) — low-effort UX win

### Near-term (next week)

- `docs/types/core/` — seed 6 core type TOML files to bootstrap the typed-pipes type registry
- SDK: publish `plexi-sdk` 0.3.0 to PyPI
- Parallax: SecretGet integration for API keys (only Layer 3 item still pending)
- Slash commands in agent mode (/status, /cost, /jobs)
- Cut a beta build (`just install-beta`) once the immediate list is green

### Long game (back burner)

- Layer 5 WASM Phases 1-5 (mobile/remote access)
- Directory-scoped workspace persistence (`.plexi/workspace.json` per project) — see `docs/specs/proposals/spatial-canvas.md`
- Agent orchestration trust system
- Agent replay & testing infrastructure
- **Shell-config app v1 implementation** — P3, spec at `docs/specs/proposals/app-shell-config.md` (filed 2026-04-14, not blocking)

---

## Spec Index

| Spec | Location | Status |
|------|----------|--------|
| App Infrastructure | `docs/specs/subsystems/app-infrastructure.md` | Active — v1 stamped 2026-04-14, source-of-truth for shipping protocol |
| Shell Config App | `docs/specs/proposals/app-shell-config.md` | Active — spec only, not implemented |
| Agent Mode | `docs/specs/subsystems/agent-mode.md` | Active — backend swap in flight |
| Agent Orchestration | `docs/specs/subsystems/agent-orchestration.md` | Draft — core logic ready |
| Companion App | `docs/mobile/ios-companion.md` | Reference only — replaced by WASM/PWA |
| WASM/PWA Deployment | `docs/specs/proposals/wasm-pwa-deployment.md` | Deferred — Phase 1 not started |
| Intelligence Protocol | `docs/specs/subsystems/intelligence-protocol.md` | Deferred — apps manage own LLM calls |
| Sync Architecture | `docs/specs/proposals/sync-architecture.md` | Draft — Phase 2+ |
| Agent Replay & Testing | `docs/specs/proposals/agent-replay-testing.md` | Draft — future |
| Parallax App | `parallax/docs/parallax-plexi-app-spec.md` | Active — app shipped |
| North Star | `~/.agents/skills/plexi-north-star/SKILL.md` | Active |
| Typed Pipes | `docs/specs/subsystems/typed-pipes.md` | Draft — Phase 1 design, not implemented |
