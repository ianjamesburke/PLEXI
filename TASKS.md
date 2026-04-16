# Plexi v2.0 — Execution Plan

> Ordered by dependency. Parallel groups are items with no shared state — multiple agents can work these simultaneously. Checkpoints are observable tests, not "code is right."
>
> State is tracked in GitHub issues. This file tracks ORDER and DEPENDENCIES.
> Update this file when scope changes; close issues when work lands.

## Status key
- [ ] not started
- [~] in progress
- [x] done / landed on alpha

---

## Already landed on alpha

These are done. Listed here for reference, not as work items.

- [x] Typed pipes Phase 0 — `PipeWrite`/`PipeData` bidirectional IPC (PR #209)
- [x] `spawn_app` draw command — parent/child lifecycle (PR #209)
- [x] SDK 0.3.0 — breakpoints decorator, `spawn_app` helper (PR #207)
- [x] SecretGet API for external apps (PR #208)
- [x] Plexi IQ Stage 0 scaffolding — `src/plexi_iq/` module tree, `LlmBackend` trait (PR #207)
- [x] Notification system — urgency, Unix socket ingestion, action types, focus-pane (PR #222)
- [x] `docs/VISION.md` — foundational vision (PR #235)
- [x] `docs/specs/releases/plexi-v2.0.md` — full v2 technical spec (PR #235)
- [x] Protocol v2 §4: host event bus / `events.jsonl` (PR #259, closed #226)
- [x] Protocol v2 §10: protocol version negotiation (PR #254, closed #225)
- [x] Homebrew release automation — SHA256 + cask update on tag (PR #250)

---

## Phase 0 — Recursive Foundation (must land before Phase 1 and 2)

Everything in Phase 1 assumes the protocol surface exists and the event bus is wired. Phase 0 is the critical path.

- [ ] #260 Fractal PGAP: recursive instance nesting, capability-scoped agents, depth-native infrastructure
  - Blocks: #228 (Run primitive sits inside depth-scoped containers), #230 (capability enforcement requires depth context), #231 (Plexi IQ needs depth tree to orchestrate), all of Phase 3
  - Parallel with: #227 (OpenIntent is independent protocol work)
  - Done looks like: `ls .plexi/` in a nested project opens a distinct depth in Plexi's sidebar/tree; `events.jsonl` shows `AppSpawned` events with depth fields

- [ ] #227 Protocol v2 §3: OpenIntent Init payload
  - Blocks: #228 (Run primitive uses `run_id` in OpenIntent), #184 (file explorer needs OpenIntent for file handoff), #106 (Parallax needs OpenIntent for structured agent delegation)
  - Parallel with: #260
  - Done looks like: `plexi launch text-editor foo.md` opens the editor with `foo.md` pre-loaded; a Python app can read `init.open_intent.kind` on startup

---

## Phase 1 — Protocol v2 Sections (parallelize after Phase 0)

These can be worked in parallel by separate agents — they touch different subsystems. Each depends on the event bus (#259, already landed) and protocol version negotiation (#254, already landed).

- [ ] #228 Protocol v2 §5: Run primitive + run palette card
  - Blocks: #231 (Plexi IQ needs Run to track agent work), #234 (backlog/notification unification rides on Run)
  - Parallel with: #229, #230
  - Done looks like: an app can emit `RunCreate`; a run card appears in the notification palette (Cmd+Shift+N) showing status; `events.jsonl` records `RunCreated`/`RunUpdated`

- [ ] #229 Protocol v2 §6: rich notification action payloads
  - Blocks: #234 (backlog-as-notification needs action payloads)
  - Parallel with: #228, #230
  - Done looks like: a notification with `action: { kind: "OpenApp", app_id: "text-editor", intent: ... }` causes the correct app to launch when the action is activated from the palette

- [ ] #230 Protocol v2 §7: capability enforcement runtime prompts
  - Blocks: #142 (centralized tool permission system is the same problem from agent mode's side), Phase 3 capability manifest
  - Parallel with: #228, #229
  - Done looks like: an app requesting a capability it didn't declare in its manifest surfaces a runtime prompt to the user before the call proceeds; a denied capability returns an error, not a crash

---

## Phase 2 — Alpha Bug Fixes (fully parallel, no dependencies)

These are independent defects. Any agent can pick one up without touching another. Fix all P1s before RC; P2s before release; P3s are best-effort.

**P1 — must fix before RC**

- [ ] #240 Command palette: doesn't capture keyboard focus — systemic focus priority fix needed
  - Blocks: nothing downstream, but it breaks a core UX flow
  - Parallel with: all other alpha bugs
  - Done looks like: Cmd+P opens the palette and typing immediately filters without clicking first

- [ ] #123 Bug: Agent mode message send doesn't trigger LLM response
  - Blocks: #125 (the claude -p --resume replacement is moot if send is broken)
  - Parallel with: others except #125
  - Done looks like: type a message in agent mode, press Enter, response streams in

- [ ] #125 Replace `agent_llm.rs` with `claude -p --resume` subprocess wrapper
  - Blocks: #212 (Plexi IQ Stage 1 needs a working agent backend), #214 (streaming responses)
  - Parallel with: alpha bugs that don't touch agent_mode.rs
  - Done looks like: `Ctrl+/` opens agent mode, a message gets a streamed response from Claude via `claude -p --resume`; old `agent_llm.rs` is deleted

- [ ] #184 File explorer: refactor so app takes over SAME terminal instance, not a new auto-split
  - Blocks: #113 (Parallax viewer pattern depends on clean pane reuse)
  - Parallel with: other alpha bugs
  - Done looks like: pressing `e` or running `plexi open` in a pane replaces that pane with the file explorer; no phantom split appears

**P2 — fix before release**

- [ ] #236 Quick note: focus doesn't activate on pane navigation (Cmd+H/J/K/L)
  - Parallel with: all other alpha bugs
  - Done looks like: navigating to a Quick Note pane with Cmd+H/J/K/L immediately allows typing without an extra click

- [ ] #238 Hacker News: no way to back out of article preview
  - Parallel with: all other alpha bugs
  - Done looks like: pressing Escape or Backspace from article preview returns to the story list

- [ ] #239 Command palette: scroll bar positioning + arrow keys don't scroll past viewport
  - Parallel with: all other alpha bugs
  - Done looks like: arrow key navigation in the palette scrolls the list smoothly past 10+ items

- [ ] #128 WASM build broken on alpha: `agent_mode` references cfg-gated-out modules
  - Parallel with: all other alpha bugs
  - Done looks like: `cargo build --target wasm32-unknown-unknown` exits 0

- [ ] #139 Agent mode: first line of ZSH prompt not restored after exiting agent mode
  - Parallel with: all other alpha bugs
  - Done looks like: toggle Ctrl+/ twice; the terminal prompt renders correctly with no truncated top line

- [ ] #177 lichen: memory leak causes interface crash on long runs
  - Parallel with: all other alpha bugs
  - Done looks like: lichen runs for 30 minutes without crashing; `instruments` or `heaptrack` shows flat allocation over time

- [ ] #178 learn-plexi: block global Plexi keybindings during lesson, show escape hint
  - Parallel with: all other alpha bugs
  - Done looks like: inside the learn-plexi tutorial, Cmd+P does not open the command palette; an escape hint is visible in the lesson UI

- [ ] #181 pulse: 'audio off' error — pygame/numpy not found (system Python 3.9 vs Homebrew)
  - Parallel with: all other alpha bugs
  - Done looks like: pulse plays audio without errors on a clean macOS install where only Homebrew Python is set up

- [ ] #121 Bug: hot reload doesn't apply changes to git-log app source
  - Parallel with: all other alpha bugs
  - Done looks like: edit `~/.plexi-alpha/apps/git-log/git_log.py`, save → app restarts within ~250ms showing the change

**P3 — best effort**

- [ ] #237 File explorer: default to 75% upper split on open
  - Done looks like: file explorer opens with browser taking ~75% height, preview/actions in lower 25%

- [ ] #182 UI: pane resize handle — not centered, inconsistent cursor, black sliver
  - Done looks like: drag handle is visually centered on the split line; cursor changes to resize cursor on hover; no black gap

- [ ] #122 Bug: Finder Service 'Open in Plexi' doesn't appear in right-click menu
  - Done looks like: right-click a folder in Finder → Services → "Open in Plexi" appears and works

- [ ] #118 Show cwd in agent mode UI header
  - Done looks like: agent mode overlay shows the current working directory path in the header bar

---

## Phase 3 — SDK & Tooling (can run parallel to Phase 1–2)

These don't block protocol work but must land before integrations in Phase 4.

- [ ] #253 SDK overhaul: Pydantic-typed event/draw system with crash handling and LSP ergonomics
  - Blocks: #244 (deploy story is moot without a shippable SDK), #215 (Parallax migration should target the new SDK), #201 (ctx.list() position params are part of the overhaul)
  - Parallel with: Phase 1 protocol sections, Phase 2 bug fixes
  - Done looks like: a Python app written with the new SDK gets autocomplete for all draw commands in VSCode/Pyright; a crash in an app handler surfaces a structured error, not a silent hang

- [ ] #244 SDK deploy story Phase 2: shared SDK via `PYTHONPATH` in `ProcessApp::launch`
  - Blocks: nothing downstream directly, but vendors out of individual apps once this lands
  - Parallel with: nothing in Phase 3 (depends on #253)
  - Done looks like: removing `plexi_sdk.py` from an app's directory doesn't break it; `ProcessApp::launch` injects the SDK path automatically

- [ ] #247 plexi secrets CLI + Keychain-backed scoped secret management
  - Blocks: #215 (Parallax migration needs the CLI to exist first)
  - Parallel with: #253, #244
  - Done looks like: `plexi secrets set ANTHROPIC_API_KEY` stores to Keychain; `plexi secrets get ANTHROPIC_API_KEY` retrieves it; a Python app receives it via `SecretGet` without touching env vars

- [ ] #142 Agent mode: centralized tool permission system with hardwired attack-surface floor
  - Blocks: nothing downstream in v2.0 directly (feeds into Plexi IQ trust model)
  - Parallel with: #253, #247
  - Done looks like: agent mode shows a permission summary on first activation; hardwired denies (e.g. `rm -rf /`) cannot be overridden by user settings

- [ ] #104 Bare `/` at empty prompt detection for agent mode
  - Blocks: nothing critical
  - Parallel with: all of Phase 3
  - Done looks like: typing `/` at the empty agent prompt shows a slash-command picker, not a path

- [ ] #201 SDK: `ctx.list()` has no position parameters, silently collides in split layouts
  - Blocks: nothing critical — UX correctness fix
  - Parallel with: #253 (ideally lands in the same SDK overhaul)
  - Done looks like: two `ctx.list()` calls in the same app render in distinct regions without overlapping

- [ ] #202 manifest: add `[app.launch].size_fraction` for full-pane apps
  - Blocks: nothing critical
  - Parallel with: all of Phase 3
  - Done looks like: setting `size_fraction = 1.0` in a manifest causes the app to open full-pane without manual resizing

---

## Phase 4 — Integrations (depends on Phase 1 + Phase 3)

These are integration-layer issues. They require the protocol surface (Phase 1) and SDK (Phase 3) to be stable.

- [ ] #212 Plexi IQ Stage 1: minimum viable loop with both backends
  - Blocks: #231 (scope lock is a prerequisite conversation, but implementation is here)
  - Parallel with: #113, #215 (Parallax is independent)
  - Done looks like: Ctrl+/ in agent mode routes to Plexi IQ; IQ dispatches to `claude -p --resume`; response streams back; `events.jsonl` records `AgentTurn`

- [ ] #211 Plexi IQ Stage 0.5: add `mod plexi_iq;` to `src/main.rs`
  - Blocks: #212 (IQ module must be wired into main before Stage 1 works end-to-end)
  - Parallel with: nothing — must land first in this phase
  - Done looks like: `cargo build` on alpha succeeds with `mod plexi_iq;` in `src/main.rs`; no feature flag required

- [ ] #231 Protocol v2 §9: Plexi IQ Stage 1 scope lock
  - Blocks: #212 depends on the spec being closed
  - Parallel with: #211
  - Done looks like: the issue is closed with a spec comment confirming what Stage 1 ships (both backends wired, no trust float, no replay)

- [ ] #214 Layer 2: stream `claude -p --resume` responses into agent mode
  - Blocks: nothing downstream in v2.0
  - Parallel with: #212, #231
  - Done looks like: a long agent response streams token-by-token into the agent mode overlay instead of appearing all at once

- [ ] #106 Parallax manifest-first refactor — execute handoff doc with Sonnet agent
  - Blocks: #113 (Parallax viewer is the UI for the refactored Parallax), #215
  - Parallel with: #212, #214
  - Done looks like: running a Parallax session writes a `project.yaml` manifest before any assembly; re-running from the manifest produces the same output

- [ ] #113 Parallax viewer + linked terminal pattern (replaces in-app chat UI)
  - Blocks: #215 (migration should happen against the new viewer)
  - Parallel with: #212, #214
  - Done looks like: `plexi open parallax` in a Parallax project opens the viewer pane alongside a linked terminal; the in-app chat UI is removed

- [x] #215 Parallax: migrate env-var secrets to SecretGet API
  - Blocks: nothing downstream
  - Parallel with: nothing in Phase 4 (depends on #247 in Phase 3, #113 above)
  - Done looks like: Parallax session starts without `ANTHROPIC_API_KEY` in the environment; it fetches the key via `SecretGet` at runtime; `.env` file is no longer required

- [ ] #200 Platform: clipboard, paste, text selection, and mouse events across apps
  - Blocks: nothing, but needed for polish before release
  - Parallel with: all of Phase 4
  - Done looks like: Cmd+C in a list-based app copies the selected item to system clipboard; pasting into a terminal pane works as expected

- [ ] #234 Unify backlog items with notification/Run signal flow
  - Blocks: nothing
  - Parallel with: all of Phase 4 (depends on #228 Run primitive and #229 action payloads landing first)
  - Done looks like: a `.plexi/backlog` item surfaced as a notification has an action button that opens it as a Run

---

## Phase 5 — Release Gate

These must all be true before tagging v2.0. Observable, not aspirational.

- [ ] All P1 alpha-bugs closed: #240, #123, #125, #184
- [ ] Protocol v2 sections merged: #227, #228, #229, #230
- [ ] Protocol v2 tracking issue closed: #224
- [ ] Fractal PGAP foundation shipped: #260
- [ ] Plexi IQ Stage 1 working with both backends: #212
- [ ] SDK overhaul shipped or explicitly deferred with a clear note in ROADMAP.md: #253
- [ ] Secrets CLI shipped: #247 (Parallax migration #215 can slip to v2.1)
- [ ] `events.jsonl` writes events during a real session (verified: launch 3 apps, confirm 3 `AppSpawned` entries in the log)
- [ ] `cargo build --release` exits 0 on a clean clone of `alpha`
- [ ] All 24 app protocol tests pass: `python3 examples/*/tests/test_*.py`
- [ ] Homebrew tap installs v2.0 clean: `brew upgrade --cask plexi` on a machine running v1.x lands on v2.0 without manual steps
