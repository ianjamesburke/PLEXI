# Plexi Roadmap

Reference document linking layers of work to specs and issues. This file tracks sequencing, dependencies, and **how to verify each layer works**. The specs have the details.

**Last updated:** 2026-04-12 end-of-day

---

## Status Snapshot

| Layer | Status | What's blocking |
|---|---|---|
| Layer 0 — Unblocked Now | **6/6 done** | — |
| Layer 1 — App Protocol Testing | **Done** (24 tests, 4 apps) | — |
| Layer 2 — Agent Mode in Terminal | **Shipped (Warp-style). LLM backend: `claude -p --resume` swap in flight** | Finish backend swap, streaming, slash commands |
| Layer 3 — Parallax Refactor | **Done** (manifest-first, validator, Senior-only routing, Parallax viewer app) | — |
| Layer 4 — Apps That Prove the Protocol | **Done** (mouse events + delta_time, 32 apps on alpha) | App Store update management; test coverage for new apps |
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

**Status: SHIPPED (WARP-STYLE). LLM BACKEND SWAP IN FLIGHT.**

The Warp-style inline agent UI is live — bytes injected into the alacritty grid. The LLM backend is being swapped from the custom Anthropic client to `claude -p --resume` (session-resuming subprocess wrapper).

| Task | Status | Reference |
|------|--------|-----------|
| `Ctrl+/` mode switching, agent UI shell | **Done** | `src/agent_mode.rs`, `agent_ui.rs`, `agent_context.rs` |
| LLM backend: `claude -p --resume` subprocess wrapper | **In flight this session** | Replaces custom Anthropic client |
| Streaming responses | Not started | Depends on backend swap |
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
| Cost reporting via cost_report events | Not started | Parallax doesn't emit yet |
| SecretGet integration for API keys | Not started | Currently uses env vars |

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

**Status: DONE — 32 APPS ON ALPHA**

Mouse events and `delta_time` are in the protocol. The app ecosystem is live.

| Task | Status | Reference |
|------|--------|-----------|
| Mouse events protocol | **Done** | `MouseEvent` in draw protocol |
| `delta_time` in render frames | **Done** | `RenderContext.delta_time` |
| `get_state`/`set_state` protocol | **Done** | `src/process_app.rs` |
| `cost_report` protocol | **Done** | `src/cost_tracker.rs`, `~/.plexi-alpha/costs.jsonl` |
| Python SDK decorators (`@app.on_get_state`, etc.) | **Done** | `emit.cost_report()` |
| App Store (built-in) | **Shipped** | [#99](https://github.com/ianjamesburke/PLEXI/issues/99) |
| App Store: update management (version compare + badges) | **Not started** | Next milestone |
| Test coverage for new apps (10 apps need `plexi_test.py` suites) | **Not started** | — |

### Verification steps for Layer 4
1. Launch App Store from command palette → 32 apps listed
2. Install an app from the store → appears in `~/.plexi-alpha/apps/`
3. Open snake or aquarium → mouse events work, animation is smooth
4. Trigger a cost_report from a test app → check `~/.plexi-alpha/costs.jsonl`

---

## Layer 5: WASM/PWA (Back-burner)

**Status: DEFERRED. Revisit when multiplayer is needed.**

| Task | Status | Reference |
|------|--------|-----------|
| WASM Phase 1: feature-gate native deps | Deferred | [#105](https://github.com/ianjamesburke/PLEXI/issues/105), `docs/specs/wasm-pwa-deployment.md` |
| WASM Phase 2: WebSocket server mode | Not started | — |
| WASM Phase 3: WASM client with WS transport | Not started | — |
| WASM Phase 4: PWA manifest, service worker, touch | Not started | — |
| WASM Phase 5: Auth (Ed25519) | Not started | — |
| Directory sync (Tailscale + SpacetimeDB) | Not started | — |

---

## App Ecosystem (32 apps on alpha)

### Games
snake, aquarium, sandfall, lichen, apiary, seedclock

### Utilities
clipboard-stack, port-watcher, pulse, process-monitor, calc, stopwatch, color-palette, json-viewer, diff-viewer, todo

### Productivity
pomodoro, markdown-preview, audio-player, weather, hacker-news

### Development
git-log, git-blame, navigator, pyflow, plexi-browser

### Plexi-native
parallax, learn-plexi, app-store, wikipedia, hello-app

---

## What's Next (Recommended Order)

### Immediate

1. **Finish `claude -p --resume` backend swap** — streaming responses, session continuity
2. **Bare `/` at empty prompt** (#104) — low-effort UX win
3. **App Store: update management** — version compare + update badges for installed apps
4. **Test coverage** — write `plexi_test.py` suites for the 10 newest apps (snake, aquarium, sandfall, etc.)

### Near-term

- SDK: publish to PyPI with type stubs ([#169](https://github.com/ianjamesburke/PLEXI/issues/169))
- Slash commands in agent mode (/status, /cost, /jobs)
- Parallax: cost_report events + SecretGet for API keys

### Long game (back burner)

- Layer 5 WASM Phases 1-5 (mobile/remote access)
- Directory-scoped workspace persistence (`.plexi/workspace.json` per project)
- Agent orchestration trust system
- Agent replay & testing infrastructure

---

## Spec Index

| Spec | Location | Status |
|------|----------|--------|
| App Infrastructure | `docs/specs/app-infrastructure.md` | Active |
| Agent Mode | `docs/specs/agent-mode.md` | Active — backend swap in flight |
| Agent Orchestration | `docs/specs/agent-orchestration.md` | Draft — core logic ready |
| Companion App | `docs/specs/companion-app.md` | Reference only — replaced by WASM/PWA |
| WASM/PWA Deployment | `docs/specs/wasm-pwa-deployment.md` | Deferred — Phase 1 not started |
| Intelligence Protocol | `docs/specs/intelligence-protocol.md` | Deferred — apps manage own LLM calls |
| Sync Architecture | `docs/specs/sync-architecture.md` | Draft — Phase 2+ |
| Agent Replay & Testing | `docs/specs/agent-replay-testing.md` | Draft — future |
| Parallax App | `parallax/docs/parallax-plexi-app-spec.md` | Active — app shipped |
| North Star | `~/.agents/skills/plexi-north-star/SKILL.md` | Active |
