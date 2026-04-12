# Plexi Roadmap

Reference document linking layers of work to specs and issues. This file tracks sequencing, dependencies, and **how to verify each layer works**. The specs have the details.

**Last updated:** 2026-04-12 mid-session

---

## Status Snapshot

| Layer | Status | What's blocking |
|---|---|---|
| Layer 0 — Unblocked Now | **6/6 done** (3 PRs awaiting verify+merge) | User installs and tests #108, #109, #110 |
| Layer 1 — App Protocol Testing | **Done** | — |
| Layer 2 — Agent Mode in Terminal | **Scaffolding done, LLM backend in PR #108** | Verification + Claude Code research (could change backend) |
| Layer 3 — Parallax Refactor | **Manifest-first + validator + Senior-only routing done** | Plexi app shell to wrap the pipeline |
| Layer 4 — Apps That Prove the Protocol | **Protocol done, no app uses it yet** | First consumer = Parallax app |
| Layer 5 — Multiplayer + Companion | **Replaced by WASM/PWA strategy** | Phase 1 in PR #109; back-burner per user |

---

## Layer 0: Unblocked Now

| Task | Type | Status | Reference |
|------|------|--------|-----------|
| Self-closing panes via OSC title | Code (~35 LOC) | **Done** (0e6cddc) | [#90](https://github.com/ianjamesburke/PLEXI/issues/90) closed |
| App protocol test harness (`plexi_test.py`) | Code (~360 LOC) | **Done** (PR #101 merged) | [#100](https://github.com/ianjamesburke/PLEXI/issues/100) |
| Issue triage cleanup | Ops | **Done** (13 closed, 9 cross-referenced) | DEV_LOG 2026-04-11 |
| Hot reload for app development | Code | **Done in #103, merged to alpha** | [#83](https://github.com/ianjamesburke/PLEXI/issues/83) |
| Theme: surface-specific hover tokens | Code (small) | **Done in #103, merged to alpha** | [#70](https://github.com/ianjamesburke/PLEXI/issues/70) |
| Finder Service "Open in Plexi" | Code (Rust + objc2) | **In PR #110, awaiting verify+merge** | — |

### Verification steps for Layer 0
After installing `layer-merged` and the open PRs:

1. **Hot reload** — open Plexi, run a Python app (e.g. wikipedia), edit `~/.plexi-alpha/apps/wikipedia/wikipedia.py` to change a hardcoded string, save → app restarts and shows new string within ~250ms
2. **Hover tokens** — hover over a row in the file browser → row background should be visibly distinct from non-hovered rows
3. **Self-closing panes** — run `printf '\e]0;plexi:close\x07'` in a terminal pane → pane closes
4. **Finder Service** — right-click any folder in Finder → Services → "Open in Plexi" → Plexi opens with that folder as a new context
5. **Test harness** — `python3 examples/wikipedia/tests/test_wikipedia.py` → 7 tests pass

**Layer 0 done when:** all 5 verifications pass.

---

## Layer 1: App Protocol Testing

**Status: DONE**

| Task | Status | Reference |
|------|--------|-----------|
| `plexi_test.py` test harness | Done | PR #101, merged |
| Test cases for existing Python apps | Done | 24 tests across hello-app, git-log, plexi-browser, wikipedia |
| CI integration | Deferred | Lower priority — local testing works fine for now |
| Rust universal test harness | Filed | [#102](https://github.com/ianjamesburke/PLEXI/issues/102) — Layer 1+ |

### Verification steps for Layer 1
```bash
python3 examples/wikipedia/tests/test_wikipedia.py     # 7 tests
python3 examples/git-log/tests/test_git_log.py         # 5 tests
python3 examples/plexi-browser/tests/test_plexi_browser.py  # 6 tests
python3 examples/hello-app/tests/test_hello_app.py     # 6 tests
```

All 24 should pass. They use the installed apps at `~/.plexi-alpha/apps/`, so apps must be installed first.

**Unlocks:** All app development iteration, Parallax app, app store

---

## Layer 2: Agent Mode in Terminal

**Status: SCAFFOLDING DONE, LLM BACKEND IN PR #108 (UNVERIFIED)**

| Task | Status | Reference |
|------|--------|-----------|
| `Ctrl+/` mode switching, agent UI shell | **Done in #103, merged to alpha** | `src/agent_mode.rs`, `agent_ui.rs`, `agent_context.rs` |
| LLM call backend (Anthropic, async, secret lookup) | **In PR #108, awaiting verify+merge** | `src/agent_llm.rs` |
| Bare `/` at empty prompt (instead of `Ctrl+/`) | Open | [#104](https://github.com/ianjamesburke/PLEXI/issues/104) |
| **Driving Claude Code as backend (research)** | **Research in flight** | Could replace the custom Anthropic client entirely |
| Slash commands (/status, /cost, /jobs) | Not started | Agent Mode spec §4 |
| Agent context loading (lazy index) | Not started | Agent Mode spec §5 |
| Background job tracker | Not started | Agent Mode spec §7 |
| Trust/risk scoring | Not started | [Agent Orchestration spec](docs/specs/agent-orchestration.md) |

### Verification steps for Layer 2 (after #108 merges)
1. Set `ANTHROPIC_API_KEY` via secrets manager (or `plexi secret set ANTHROPIC_API_KEY <key>`)
2. Open Plexi, focus a terminal pane, press `Ctrl+/`
3. Agent UI panel appears in the pane (replaces terminal view)
4. Type "What directory am I in?" and press Enter
5. Real Sonnet response appears within ~2-5 seconds, mentioning the actual cwd
6. With NO API key set: same flow shows a friendly system message explaining how to set it (no crash)
7. Press Escape → returns to terminal mode

**Layer 2 MVP done when:** all 7 verifications pass.
**Layer 2 polished when:** bare `/` detection works (#104), slash commands exist, background jobs run.

### Open architectural question: Claude Code as backend

If the in-flight research confirms that `claude -p --resume <session_id>` benefits from prompt caching and supports tool use, **we should scrap PR #108's custom Anthropic client and replace it with a Claude Code subprocess wrapper.** This would inherit all of Claude Code's tool use (Edit/Bash/Read/Grep) for free, use the user's existing auth, and dramatically reduce per-turn cost via cache reuse. Decision pending research.

---

## Layer 3: Parallax Refactor

**Status: ALL CORE WORK DONE, PLEXI APP SHELL NEXT**

| Task | Status | Reference |
|------|--------|-----------|
| Manifest-first refactor (editors write YAML, no ffmpeg) | **Done** | Parallax DEV_LOG 2026-04-12, root cause was `_get_tools()` returning `["...", "ffmpeg"]` |
| Manifest schema (Pydantic) + validator with feedback loop | **Done** | `packs/video/manifest_schema.py`, `manifest_validator.py`, 9 tests |
| Senior-only routing (drop Junior, simplify) | **Done** | HoP routes footage_edit directly to SeniorEditor on Sonnet |
| Cost reporting via cost_report events | Not started | Parallax → Plexi cost protocol exists in Layer 4, Parallax doesn't emit yet |
| SecretGet integration for API keys | Not started | Currently uses env vars |
| **Parallax Plexi app shell** (chat UI in a pane) | **Not started — next milestone** | The thing that makes Parallax usable from inside Plexi |

### Verification steps for Layer 3
**Pipeline-only verification (works today):**
```bash
cd ~/Documents/GitHub/parallax
TEST_MODE=true python3.11 test/test_manifest_first.py       # 5 tests
TEST_MODE=true python3.11 test/test_manifest_validator.py    # 9 tests
TEST_MODE=true python3.11 test/test_regression.py            # 5 tests
```
All 19 should pass.

**End-to-end with real API (manual):**
```bash
TEST_MODE=false python3.11 -c "from core.head_of_production import HeadOfProduction; HoP = HeadOfProduction(); HoP.execute({'type': 'footage_edit', 'brief': '...'})"
```
Should produce a valid manifest (validator passes), generate stills (real API calls), and assemble a playable MP4.

**Layer 3 fully done when:** Parallax runs as a Plexi app — `~/.plexi-alpha/apps/parallax/` exists, you launch it from the command palette, chat with it in a pane, give it a brief, see it work. That's the "Plexi app shell" task above.

---

## Layer 4: Apps That Prove the Protocol

**Status: PROTOCOL DONE, NO APP USES IT YET**

| Task | Status | Reference |
|------|--------|-----------|
| `get_state`/`set_state` protocol implementation | **Done in #103, merged** | `src/process_app.rs` undo/redo stacks, `Cmd+Z`/`Cmd+Shift+Z` |
| `cost_report` protocol implementation | **Done in #103, merged** | `src/cost_tracker.rs`, writes to `~/.plexi-alpha/costs.jsonl` |
| Python SDK `get_state`/`set_state` decorators | **Done in #103, merged** | `@app.on_get_state`, `@app.on_set_state`, `emit.cost_report()` |
| **First app that actually USES get_state** (Parallax) | Not started | Required to verify the protocol works end-to-end |
| App Store (built-in) | Not started — scoped | [#99](https://github.com/ianjamesburke/PLEXI/issues/99) |
| Manifest validator hook for Plexi apps (mirror of Parallax pattern) | Not scoped | Could promote to SDK primitive |

### Verification steps for Layer 4
**Protocol exists (works today):**
1. Open a Python app in Plexi
2. Press `Cmd+Z` → no crash (no-op since no app implements `get_state` yet)
3. Trigger a `cost_report` from a test app → check `~/.plexi-alpha/costs.jsonl` has the entry

**Protocol used by a real app (the actual goal):**
1. Open the Parallax Plexi app
2. Type a brief, click "Generate"
3. See the agent work, see costs accumulate in real time in the status bar
4. Make a manual edit (e.g. select a clip, change duration)
5. Press `Cmd+Z` → edit reverts
6. Quit Plexi, reopen → conversation history persists (`persistent` bucket survived)

**Layer 4 fully done when:** Parallax app demonstrates all four state buckets (`user_state` for selection, `derived` for timeline pixels, `session` for playback head, `persistent` for manifest YAML).

---

## Layer 5: Multiplayer + Companion (WASM/PWA Replacement)

**Status: BACK BURNER PER USER. PHASE 1 PR EXISTS.**

The native iOS companion app is replaced by a WASM/PWA strategy: compile Plexi's UI to WebAssembly, serve as a Progressive Web App installable from Safari/Chrome on any device. Same Rust codebase, no App Store, works on Android too.

| Task | Status | Reference |
|------|--------|-----------|
| WASM Phase 1: feature-gate native deps | **In PR #109, awaiting verify+merge** | [#105](https://github.com/ianjamesburke/PLEXI/issues/105), `docs/specs/wasm-pwa-deployment.md` |
| WASM Phase 2: WebSocket server mode (axum) | Not started | Backend serves the JSON protocol over WS instead of stdin/stdout |
| WASM Phase 3: WASM client with WS transport | Not started | Real WASM build that talks to backend |
| WASM Phase 4: PWA manifest, service worker, touch | Not started | Mobile UX |
| WASM Phase 5: Auth (Ed25519 from companion-app spec) | Not started | Secure remote access |
| Directory sync (Tailscale + SpacetimeDB) | Not started | Multiplayer foundation |
| Presence (who's in this directory) | Not started | After sync |
| Agent orchestration trust system | Not started | [Orchestration spec](docs/specs/agent-orchestration.md) |

### Verification steps for Layer 5
**Phase 1 (after #109 merges):**
```bash
cd /Users/ianburke/Documents/GitHub/PLEXI
cargo build                                           # native still works
cargo build --target wasm32-unknown-unknown           # WASM compiles cleanly
```
Both should pass with no errors.

**Phase 2-5:** Out of scope until later. The spec at `docs/specs/wasm-pwa-deployment.md` has the full plan.

---

## Layer 6: Agent Replay & Testing Infrastructure

**Status: SPEC WRITTEN, NOT STARTED**

This is the meta-layer that makes everything else iterable. Spec at `docs/specs/agent-replay-testing.md`.

| Task | Status |
|------|--------|
| Phase 0: Foundations (run capture format, span schema) | Not started |
| Phase 1 MVP: Record + replay (no fork yet) | Not started |
| Phase 2: Fork from span, swap component, diff | Not started |
| Phase 3: Iterability gate (refuse-by-default when below required mode) | Not started |
| Phase 4: Fidelity spectrum + budget-directed selection | Not started |
| Phase 5: Replay browser app | Not started |
| Phase 6: Aggregated insights ("last 100 runs, find patterns") | Not started |
| Composition patterns spec (panels, teams, topology iteration) | **Not yet written** — extension to agent-replay-testing.md |

### Verification steps for Layer 6
Cannot verify until Phase 1 MVP exists. The minimum bar:
```bash
plexi replay record &lt;app&gt;             # captures a run
plexi replay list                       # shows captured runs
plexi replay show &lt;run_id&gt;             # shows spans
plexi replay replay &lt;run_id&gt;           # re-runs from cassette
```

---

## What's Next (Recommended Order)

### Immediate (this session or next)

1. **Verify and merge PRs #108, #109, #110.** They're sitting open. PR #108 changes everything about how agent mode feels. Just install + test the verifications above.
2. **Resolve the Claude Code research question.** If the answer is YES, scrap #108's custom client and rebuild agent mode as a Claude Code wrapper. This is potentially a much bigger win than anything else on the roadmap right now.

### Next milestone: Parallax Plexi app shell (Layer 3 → Layer 4)

This is the killer demo. Concretely:

1. Create `~/.plexi-alpha/apps/parallax/` with `manifest.toml`, `parallax.py` entry point
2. Build a chat UI: `ctx.text()` for messages, `ctx.list()` for conversation, input field at the bottom
3. Wire it to invoke `core.head_of_production.HeadOfProduction.execute(brief)` from the Parallax repo
4. Stream status updates back into the conversation as they happen
5. Implement `get_state` / `set_state` so the conversation history persists in the `persistent` bucket
6. Emit `cost_report` events as the pipeline incurs API costs
7. Render the manifest as a timeline view (later — MVP is text-only)

**Verification:** open Parallax in Plexi, type a brief, watch it generate a video.

### Then: incremental polish

- Bare `/` at empty prompt (#104)
- Cost dashboard / status bar
- Agent mode background jobs (the spec is written)
- Parallax timeline GUI (separate from chat)
- App store (#99)

### Long game (back burner)

- Layer 5 WASM Phases 2-5 (mobile)
- Layer 6 Agent Replay (after we have enough runs to make it valuable)
- Composition patterns (panels, teams, topology iteration) — spec extension, not implementation

---

## Spec Index

| Spec | Location | Status |
|------|----------|--------|
| App Infrastructure | `docs/specs/app-infrastructure.md` | Active — Phase 2 in progress |
| Agent Mode | `docs/specs/agent-mode.md` | Draft — implementation in flight |
| Agent Orchestration | `docs/specs/agent-orchestration.md` | Draft — core logic ready, improvement officer deferred |
| Companion App | `docs/specs/companion-app.md` | **Reference only — replaced by WASM/PWA** |
| WASM/PWA Deployment | `docs/specs/wasm-pwa-deployment.md` | Active — Phase 1 in PR #109 |
| Intelligence Protocol | `docs/specs/intelligence-protocol.md` | **Deferred** — apps manage own LLM calls |
| Sync Architecture | `docs/specs/sync-architecture.md` | Draft — Phase 2+ |
| Telegram Integration | `docs/specs/telegram-integration.md` | Reference — companion app preferred |
| Agent Replay & Testing | `docs/specs/agent-replay-testing.md` | Draft — Layer 6 vision |
| Parallax App | `parallax/docs/parallax-plexi-app-spec.md` | Draft — implementation now unblocked |
| Parallax Packaging | `parallax/docs/parallax-plexi-packaging.md` | Draft — Phase 1 (manifest-first) done |
| North Star | `~/.agents/skills/plexi-north-star/SKILL.md` | Active — ship order updated 2026-04-11 |
