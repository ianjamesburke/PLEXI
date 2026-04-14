# Plexi Roadmap

Reference document linking layers of work to specs and issues. This file tracks sequencing, dependencies, and **how to verify each layer works**. The specs have the details.

**Last updated:** 2026-04-14

---

## Status Snapshot

| Layer | Status | What's blocking |
|---|---|---|
| Layer 0 — Unblocked Now | **6/6 done** | — |
| Layer 1 — App Protocol Testing | **Done** (24 tests, 4 apps) | — |
| Layer 2 — Agent Mode in Terminal | **Shipped (Warp-style). LLM backend: `claude -p --resume` swap in flight** | Finish backend swap, streaming, slash commands |
| Layer 3 — Parallax Refactor | **Done** (manifest-first, validator, Senior-only routing, Parallax viewer app) | — |
| Layer 4 — Apps That Prove the Protocol | **Done** (mouse events + delta_time, 32 apps on alpha) | App Store update management (in flight); test coverage gap (~22 apps); Rust SDK protocol parity |
| Layer 4.5 — SDK Packaging & Protocol Stability | **Done** (Python SDK packaged 0.2.0, protocol spec v1, Rust SDK polished) | Rust SDK protocol parity with Python 0.2.0 |
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
| App Store: update management (version compare + badges) | **In flight (this session)** | Parallel agent implementing — verify after merge |
| Test coverage for example apps (~22 of 32 apps lack `plexi_test.py` suites) | **In flight (this session)** | Coverage sprint running in parallel — verify final count after merge |
| Rust SDK protocol parity with Python 0.2.0 | **P1 — not started** | Missing: `scroll`, `mouse_down`/`mouse_up`, `drop`, `get_state`/`set_state`, `cost_report`, `notification`, `feedback`, `log`. See DEV_LOG 2026-04-14 |

### Verification steps for Layer 4
1. Launch App Store from command palette → 32 apps listed
2. Install an app from the store → appears in `~/.plexi-alpha/apps/`
3. Open snake or aquarium → mouse events work, animation is smooth
4. Trigger a cost_report from a test app → check `~/.plexi-alpha/costs.jsonl`

---

## Layer 4.5: SDK Packaging & Protocol Stability

**Status: DONE (2026-04-14)**

The Python SDK is now a real PyPI-shaped package and the app protocol is stamped at v1. External devs can `pip install plexi-sdk` for editor/linter support; runtime apps continue to vendor `plexi_sdk.py` next to their entry file. A sync script enforces byte-equality between the canonical SDK and every vendored copy.

| Task | Status | Reference |
|------|--------|-----------|
| Python SDK packaged as `plexi-sdk` 0.2.0 | **Done** | `sdk/python/pyproject.toml`, `README.md`, `LICENSE`, `MANIFEST.in` (commit `e49de37`) |
| Vendored SDK sync script | **Done** | `scripts/sync-sdk.py` — 31 examples byte-identical to canonical (commit `4496e2b`) |
| App infrastructure spec v1 | **Done** | `docs/specs/app-infrastructure.md` (705 lines, every shipping event/command documented, commit `a7a7e22`) |
| Rust SDK Cargo manifest publication-ready | **Done** | `sdk/rust/Cargo.toml` full publish metadata; `cargo publish --dry-run --allow-dirty` clean (commit `2bc52ee`). **Source kept at 0.1.0 — not bumped.** |
| Rust SDK protocol parity with Python 0.2.0 | **Not started** | Listed under Layer 4 — blocks any Rust example app needing new commands |
| Shell-config app v1 spec | **Done (spec only)** | `docs/specs/app-shell-config.md` (commit `7b13b10`). P3, idea-tier, not implemented |

### Verification steps for Layer 4.5

```bash
pip install -e sdk/python                    # Editable install of plexi-sdk 0.2.0
python3 scripts/sync-sdk.py --check          # All vendored copies byte-identical
cd sdk/rust && cargo publish --dry-run --allow-dirty   # Packages cleanly
```

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

### Immediate (this week)

1. **Finish `claude -p --resume` backend swap** — streaming responses, session continuity (Layer 2)
2. **Rust SDK protocol parity with Python 0.2.0** — add `scroll`, `mouse_down`/`mouse_up`, `drop`, `get_state`/`set_state`, `cost_report`, `notification`, `feedback`, `log` to `sdk/rust/src/lib.rs` before bumping the crate
3. **Bare `/` at empty prompt** ([#104](https://github.com/ianjamesburke/PLEXI/issues/104)) — low-effort UX win
4. **v2 new-tab rendering bug** — file as a P1 GitHub issue first, then fix (surfaced this session, not yet tracked)
5. **App Store: update management** — version compare + update badges (in flight via parallel agent — verify after merge)
6. **Test coverage sprint** — close the ~22-app gap with `plexi_test.py` suites (in flight via parallel agent — verify after merge)

### Near-term (next week)

- SDK: publish `plexi-sdk` 0.2.0 to PyPI ([#169](https://github.com/ianjamesburke/PLEXI/issues/169))
- Parallax: SecretGet integration for API keys (only Layer 3 item still pending)
- Slash commands in agent mode (/status, /cost, /jobs)
- Cut a beta build (`just install-beta`) once the immediate list is green

### Long game (back burner)

- Layer 5 WASM Phases 1-5 (mobile/remote access)
- Directory-scoped workspace persistence (`.plexi/workspace.json` per project) — see `docs/specs/spatial-canvas.md`
- Agent orchestration trust system
- Agent replay & testing infrastructure
- **Shell-config app v1 implementation** — P3, spec at `docs/specs/app-shell-config.md` (filed 2026-04-14, not blocking)

---

## Spec Index

| Spec | Location | Status |
|------|----------|--------|
| App Infrastructure | `docs/specs/app-infrastructure.md` | Active — v1 stamped 2026-04-14, source-of-truth for shipping protocol |
| Shell Config App | `docs/specs/app-shell-config.md` | Active — spec only, not implemented |
| Agent Mode | `docs/specs/agent-mode.md` | Active — backend swap in flight |
| Agent Orchestration | `docs/specs/agent-orchestration.md` | Draft — core logic ready |
| Companion App | `docs/specs/companion-app.md` | Reference only — replaced by WASM/PWA |
| WASM/PWA Deployment | `docs/specs/wasm-pwa-deployment.md` | Deferred — Phase 1 not started |
| Intelligence Protocol | `docs/specs/intelligence-protocol.md` | Deferred — apps manage own LLM calls |
| Sync Architecture | `docs/specs/sync-architecture.md` | Draft — Phase 2+ |
| Agent Replay & Testing | `docs/specs/agent-replay-testing.md` | Draft — future |
| Parallax App | `parallax/docs/parallax-plexi-app-spec.md` | Active — app shipped |
| North Star | `~/.agents/skills/plexi-north-star/SKILL.md` | Active |
