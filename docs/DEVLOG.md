# Devlog

Landed-work history only. Newest entries first; do not rewrite old entries.
Live work, priority, and dependency state belongs exclusively to `.stint/`.

---

## 2026-07-26: Assistant context-root file access

`42ae578c` made the current Plexi context root the Assistant file tools' primary scope instead of limiting discovery to the channel's global app directory. Relative list/read/grep/write/edit paths now behave like a coding agent opened in the context root, while the global app directory remains an explicit auxiliary root. Canonical deepest-existing-prefix validation rejects symlink escapes for both reads and writes. The installed alpha host was exercised against `/Users/ianburke/temp_prod`: the Assistant used only `host.files.list` and `host.files.read` to find `hi.md` and return its exact contents.

## 2026-07-17 — Assistant app-building hardening (0413 gate dogfood)

Live dogfooding of the 0413 launch gate — "the assistant builds a small app in front of you" — exposed that the host Assistant could not actually author a Plexi app, and each fix went direct to alpha.

- **No app-authoring knowledge.** The Assistant's `DEFAULT_AGENT_PROMPT` never mentioned the `plexi app init`/`app check` lifecycle and forbade all CLI use, so "build me a game" produced a raw curses script. Added a compiled-in builtin skill (`src/assistant/builtin/build-plexi-app.md`, new `SkillSource::Builtin`) that every fresh install carries and that auto-matches build/make/game prompts, plus a prompt carve-out routing app authoring through the CLI.
- **Assistant was blind to terminal output.** `host.terminals.run` returned only `{ok}`, so the model guessed the scaffold path (`~/.plexi/` instead of `~/.plexi-alpha/`). Added read-only `host.terminals.read` and a read-after-every-run prompt rule; raised the broker tool-loop cap and made the forced-stop emit a visible message instead of ending the turn silently.
- **File authoring through PTY heredocs.** The Assistant had no file tools, so it typed whole `main.py` files into a visible terminal. Added app-dir-scoped, audit-logged `host.files.read`/`host.files.write`/`host.files.edit` (unique-string-replace, loud on zero/multi match); traversal and out-of-scope paths reject by name.
- **Badge/status color contract mismatch.** Apps could write `Badge(color="blue")`; the SDK accepted any string and the host decoder rejected all but five semantic roles, crashing the whole tree at render. Colors are now the theme's semantic roles (`accent/success/warning/danger/neutral` plus the `red/green/yellow` aliases the theme already defines); the SDK validates at construction with a `Literal` type and a named `ValueError`, and the host decoder accepts the aliases. `Banner` fails loud instead of coercing bad tones to neutral.
- **Doc-drift purge.** An audit found the theme/color contract was undocumented in any agent-facing file, plus stale pointers (wrong `CONFIG.md` path, a mangled `src/ui/AGENTS.md` link, a dead `substitute_note_tokens_static` security trap, an inaccurate exemplar-app list). Fixed all, stripped volatile line-numbers/counts from the agent docs, and added a root "No volatile numbers in docs" rule so the drift can't recur.

An SDK-vs-code audit also found that many `AUTHORING.md`-advertised widgets (`Markdown`, `Clickable`, `Footer`, `ButtonRow`, and several canvas-era classes with no `to_node()`) crash or no-op in the shipped declarative-tree mode — stranded debt from the native→WASM migration. Being fixed as a follow-up with an enumerate-every-component-and-assert-its-node-type guardrail so the class can't regress.

## 2026-07-11 — Durability + channel-routing fixes; assistant dogfooding audit

`0367` made workspace saves atomic and durable (#2384); `0365` routed CLI commands by binary channel (#2385). Same evening, three live dogfooding sessions against the alpha assistant produced a 15-task audit batch (`0368`–`0382`): the headline finding is that the host-side v3.7 tool protocol (`ExposeTools`/`AiTool`, stint 0227) was never wrapped in the Python SDK, so zero apps can expose connector tools; the assistant also lacks terminal-input, internet, and pane-targeting tools, and the permission modal lacks keyboard support. Full detail on each task.

## 2026-07-11: Assistant E2E and affordable-model verification

`0359` added a deterministic, non-egui Assistant harness around the real `AssistantApp` effect path and gated tool dispatcher. Tests script model/tool outcomes and capture typed events for agent and model selection, skills offered and activated, calls and JSON arguments, permission decisions, tool results, native host effects, results observed by the model, and final outcomes. Assertions reject missing, repeated, reordered, unexpected, or wrong-argument calls. Connector result injection fails before dispatch because connector success requires a registered app; HostHarness owns that integration layer.

Initial real-host checks used an isolated `pr-3590` install in `/tmp/plexi-0359-live`; the fixed Ollama rerun used `pr-2383`. The local route used `ollama/gemma4:latest`. The cheap cloud route used `openrouter/qwen/qwen3.6-flash` and cost $0.000463125. No secret value was logged or recorded.

The first Ollama tool-result follow-up returned HTTP 400 with `{"error":"Value looks like object, but can't find closing '}' symbol"}`. Plexi was sending OpenAI-style history to Ollama's native `POST /api/chat`: `function.arguments` was a JSON string and tool results used `tool_call_id`. Ollama's [native tool-calling format](https://docs.ollama.com/capabilities/tool-calling) requires an arguments object and identifies results with `tool_name`. A minimal replay against Ollama 0.30.7 reproduced the boundary exactly: string arguments returned the same HTTP 400; changing only arguments to an object returned HTTP 200 and a final Gemma answer. The Ollama adapter now converts arguments to objects, translates results to `tool_name`, and synthesizes stable IDs because Ollama omits them. A fresh installed `pr-2383` run at `e16114f5` loaded `local-verify` plus `pane-check`, completed `host.apps.open` and `host.panes.list` across both result-feedback turns, created File Browser pane 8, and ended with the model confirming pane 8 was present.

| Contract | Ollama `gemma4:latest` | OpenRouter `qwen/qwen3.6-flash` |
|---|---|---|
| Agent/model routing | PASS: `local-verify`, low tier, concrete Ollama route | PASS: `cheap-verify`, low tier, concrete OpenRouter route |
| Settings read/write | PASS: workspace settings loaded; active agent survived restart | PASS: `/model low` applied a session write; workspace settings loaded |
| Conversation restart | PASS: completed 6-turn conversation, final answer, Assistant pane, and File Browser pane 9 restored | PASS: completed 10-turn conversation, active agent, and final answer restored |
| Skill activation | PASS: workspace `pane-check` instructions loaded | PASS: workspace `pane-check` instructions loaded |
| Native host operation | PASS: fixed rerun created File Browser pane 9; `host.panes.list` returned pane 9 | PASS: `host.apps.open` created pane 7; `host.panes.list` returned pane 7 |
| Result back to model | PASS after adapter fix: Gemma observed both results and produced a semantic success outcome | PASS: model observed both results and produced a semantic success outcome |

Typed real-model traces:

```toml
[[trace]]
backend = "ollama"
agent = "local-verify"
tier = "low"
model = "gemma4:latest"
skill_activated = "pane-check"
calls = [
  { name = "host.apps.open", arguments = { app = "file_browser", layout = "split_h", args = [] } },
  { name = "host.panes.list", arguments = {} },
]
permissions = [
  { name = "host.apps.open", decision = "persisted_allow" },
  { name = "host.panes.list", decision = "persisted_allow" },
]
host_effects = [
  { name = "host.apps.open", pane_id = 9, result = "ok" },
  { name = "host.panes.list", contains_pane_id = 9, result = "ok" },
]
model_observed_results = ["host.apps.open:ok", "host.panes.list:contains_pane_9"]
final = { class = "success", semantic = "opened_pane_is_present_in_list" }

[[trace]]
backend = "openrouter"
agent = "cheap-verify"
tier = "low"
model = "qwen/qwen3.6-flash"
skill_activated = "pane-check"
calls = [
  { name = "host.apps.open", arguments = { app = "file_browser", layout = "split_h", args = [] } },
  { name = "host.panes.list", arguments = {} },
]
permissions = [
  { name = "host.apps.open", decision = "persisted_allow" },
  { name = "host.panes.list", decision = "persisted_allow" },
]
host_effects = [
  { name = "host.apps.open", pane_id = 7, result = "ok" },
  { name = "host.panes.list", contains_pane_id = 7, result = "ok" },
]
model_observed_results = ["host.apps.open:ok", "host.panes.list:contains_pane_7"]
final = { class = "success", semantic = "opened_pane_is_present_in_list" }
```

Live-driver gaps found during the run: the native permission sheet exposes semantic buttons but `pane key` cannot focus or activate them, and a PR-signed app can block in Keychain ACL lookup before opening a provider socket. The verification used persisted broker grants and a launchd-scoped environment injection for the existing OpenRouter key, then removed that launchd value after teardown.

## 2026-07-11: Assistant identity and shared scene verbs landed

`0226` (#2375) added scoped Assistant settings. `0362` (#2376) replaced feature-specific scene actions with generic open, text, key, context, symbolic-handle, assertion, and report primitives. `0225` (#2377) added the file-backed agent registry, model-tier routing, `/agent`, and `/effort`; validation also made agent and effort choices survive reopen, rejected partial agent-definition overwrites, and kept PR installs compatible with Rust 1.97's new float-literal warning.

## 2026-07-10 — Explorer native media viewers landed

`0349` (#2366) completed the explorer-as-window-manager loop: image, video, and audio files opened from File Explorer now land in native Rust viewer panes (`image-viewer` fit/zoom/pan, `video-player` and `audio-player` play/pause/seek) instead of bouncing to the macOS opener, and closing a viewer returns focus to the explorer with its selection intact. The branch was 15 commits stale (predated the money-path merge run); rebased cleanly onto alpha with one expected touch-point in `pane_ops/create.rs` (shared with `0245`'s bug bundle) resolving without conflict, re-verified 1527/1527 `cargo test --bin plexi` green post-rebase. Manually exercised on `plexi-pr-2366`: log trace confirmed correct `launch_app_by_id_with_layout` routing for all three viewer types, `close_tile` returning focus to the file_browser tile on both audio closes, and unsupported types (`.json`, `.pdf`) still falling through to the OS opener. This was the last tracked P1 gap in the Epoch 1 (free v1) finish line.

## 2026-07-10 — First-party monetization landed

`0355` (#2374) built the first-party sell-side on top of the merged buy-side: `website/src/server/products.ts` `ensureAppProduct` creates Polar products under Plexi's org with `metadata.app_id` and upserts `app_products` (idempotent PATCH on re-run, free apps rejected — the seam 0344 reuses); `ensureAiProProduct` wires the recurring $10/mo AI Pro product to the existing `POLAR_AI_PRO_PRODUCT_ID`; operator CLI `npm run commerce -- set-app/ensure-ai-pro`. Deliberately no `src/cli/app.rs` change — the client binary must not hold seller Polar/DB creds. **Closed #2370's never-mock gap**: completed real Polar sandbox checkouts (test card via Playwright) and recorded genuine `order.paid`/`order.refunded`/`subscription.created` shapes (real `platform_fee_amount`=110 → net 1090, correcting the fabricated 1140). 26/26 website tests. Remaining to go live is ops only: provision Polar org/product-ids/webhook-secret + private bucket, flip `SALES_ENABLED`. Live-verified: org token omits `organization_id`; buyer email must be deliverable.

## 2026-07-10 — Money-path buy-side + Polar AUP split

Five PRs landed via a sequential subagent run, moving Epoch 3's buy-side onto alpha: `0339` Polar merchant-of-record (#2370 — checkout/webhooks/402 envelope/gated artifact download/`002_commerce.sql`), `0347` legal surface (#2369 — ToS/privacy/refund/DMCA), `0325` package envelope spec (#2371), `0245` host bug bundle (#2372 — FooterKeys wrap, OpenArtifact symlink containment, event-subscriber identity split, cli-renderer degraded-ready + temp-file cleanup), and `0252` v1 polish (#2373 — app-reported pip-status SDK effect + UI gallery smoke). `0339` was validated **live against the Polar sandbox** (auth → product create → checkout create; `metadata.app_id` round-trips; org token must omit `organization_id`; buyer email must be deliverable).

Key discovery: **Polar's AUP bars the third-party-marketplace model** (outside sellers with payouts owed back) — Polar is first-party MoR only. This split Epoch 3: first-party monetization (`0355`, ready — sell Plexi's own apps + AI Pro now) vs the deferred third-party economy (`0352` payout-rail decision → `0344`/`0353`). New tasks filed: `0352`, `0353`, `0354`, `0355`; `0344` re-scoped so its third-party product creation blocks on `0352`. `#2370` caveats before sales go live: fixtures are schema-grounded not sandbox-recorded (closed in `0355`), Polar/bucket provisioning still needed, `SALES_ENABLED` keeps it dark.

## 2026-07-09 — Agent pip color swap

`0350` landed in #2368: agent activity pips now flash yellow while working and sit solid green while idle — the reverse of the prior default. `Colors::from_config` swaps the fallback (`pip_working` → `cfg.yellow`, `pip_idle` → `cfg.green`); explicit `[theme]` overrides are untouched. `PipStatus::as_agent_state()` (the app-reported green/yellow/red pip status apps set via the SDK) was swapped alongside it so a green pip still reads as color-faithful "idle" and yellow as "working" under the new palette. Verified via exact-value unit tests plus a live IPC round-trip against the installed PR build (`plexi agent report --state working` correctly flipped pane state); a full on-screen pixel screenshot was blocked by a macOS Screen Recording permission gap in the validating session, not a code issue. Last P0 in the queue — none remain.

## 2026-07-08 — Native pane-key driving

`0351` landed in #2367: `plexi pane key` on a native (builtin/WASM) app pane was a silent no-op — the host queued `PlexiEvent::Key`, which native apps never read (they consume `egui::InputState` via `App::handle_key`). The `KeyPane` handler now synthesizes an `egui::InputState` and drives the app's real `handle_key`, the same path a physical keystroke takes; responses report `disposition: consumed|passthrough` so drive-host validation can detect ignored keys. Unblocks validating PR #2366 (`0349`, explorer native viewers), which exposed the gap.

---

## 2026-07-02 — Epoch 1 close bundle

`0330`, `0331`, `0332`, `0335`, `0328`, `0334`, `0346`, and `0215` landed in #2360:

- App-authoring DX now has sibling-split app opens, canonical SDK authoring docs with drift gates, agent-driven E2E capture, and benchmark prompt/scorecard scaffolding.
- The shipped app set was culled into maintained exemplars, with Logs rebuilt, CSV/Wikipedia/core survivors checked, Snake/Tetris restored as examples, and stale dev apps kept out of PR/alpha installs.
- SDK v3 gained the native placeholder/component coverage slice and host renderer fixes for tables, text entry focus, button affordance, CSV scrolling, and over-eager app hung status.
- Core registry packages are generated under the website registry with bytecode/cache artifacts excluded; the remaining post-merge gap is a production install smoke after alpha deploy.

## 2026-07-02 — pane new --tab anchoring fix

- `0337` (#2357): `plexi pane new --tab --from <pane-id>` now anchors to the caller pane's window instead of the currently active window, matching the existing `--window` and split behavior. Also fixed two related cwd-fallback bugs where the tab path fell back to the ambient active window/router context instead of the target window's own context when no explicit `--cwd` was given.
- `0304` (#2332, landed earlier): confirmed still correct on inspection — the `--window` path already anchored to `from_pane_id` correctly, so this task only needed the `--tab` path fix.

## 2026-06-30 — Free v1 spine + app-builder hardening

Free v1 local/demo/distribution/trust/hosted-registry spine landed on `alpha`:

| Task | PR | Result |
|------|----|--------|
| 0313 | #2347 | SDK/scaffold self-documenting flow shipped. |
| 0314 | #2348 | ActionBar scaffold pattern and FooterKeys clipping fix shipped. |
| 0299 | #2349 | Todo rebuilt as the canonical SDK v3 demo app. |
| 0316 | #2350 | Scaffold packaging, direct GitHub/source install, update unification, ref fallback, and workspace-aware update shipped. |
| 0320 | #2351 | Reviewed-native bypass scanner and honest trust labels shipped. |
| 0321 | #2352 | Free hosted reviewed-native registry smoke path shipped. |
| 0237 | #2354 | Workspace/global secrets now flow through command runs, PTYs, and the OpenRouter broker. |

App-builder hardening also landed on `alpha`:

- `0326` shipped scaffold-local `AGENTS.md`, `.gitignore`, drift metadata, fixtures, semantic ActionBar/FooterKeys boilerplate, host probes, headless check/render coverage, and hot-reload guidance.
- SDK semantic chrome shipped in `src/render/app_chrome.rs`; app init/check now exercise host-native semantic components.
- `plexi app check` gates current scaffolds on semantic proof components and seeded render/action probes.

### Verified by validation runs

- Fresh scaffold validate -> package -> package install works; generated `.venv` artifacts are excluded.
- Direct GitHub/source installs route through the git resolver.
- Pack-file install with git cloning works.
- Core pack install works.
- `plexi app update` / `plexi update apps` use the real git update path and handle workspace installs.
- Reviewed-native package validation flags obvious subprocess/socket/path traversal bypasses.
- Free hosted reviewed-native registry smoke path is live in the website registry fixture.
- Agent app-building loop is trustworthy enough for v1: generated app instructions, drift metadata, headless check/render, JSON seed state, real host state/action/key probes, and same-pane hot reload were verified by three sequential app-build trials.
- Workspace secrets resolver now works for command-run, OpenRouter broker lookup, and GUI terminal panes after zsh startup overwrites.

### Source-of-truth correction

`0319` resolved the sequencing conflict. Free v1 marketplace work proceeds through reviewed-native Python apps with blunt trust labels, human review, and bypass-pattern checks. WASM remains the stronger sandbox/performance path and v2 trust upgrade.
