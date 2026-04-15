# Plexi 2.0 — Release Scope

> ## ⚠️ PROTECTED SPEC — DO NOT EDIT WITHOUT CAUSE
>
> This document is the single source of truth for **what ships in Plexi 2.0 and what does not**. It is intentionally narrow. Every item here is either in-scope or explicitly deferred — there is no third category.
>
> **Changes are strictly discouraged.** If you are about to edit this file, stop and ask: does the scope need to change, or does the implementation need to catch up to the scope? Almost always, it's the second one.
>
> **If you must edit:**
> 1. The edit must be approved by the repo owner (enforced via `CODEOWNERS`).
> 2. Every edit must bump `Last updated` at the top of the file.
> 3. Every edit must add a `[DECISION]` entry in `DEV_LOG.md` explaining what changed and why.
> 4. Scope additions require a paired edit to `docs/specs/protocol-v2.md` (the technical contract) or one of the referenced sub-specs. Scope additions to this file alone are not valid — this doc indexes, it does not define.
> 5. Checklist status updates (pending → in-progress → done) are the ONLY edits that don't need all four steps above. They still need a DEV_LOG entry when an item flips to "done."

**Status:** Draft
**Last updated:** 2026-04-14
**Owner:** plexi-core (ianjamesburke)
**Release target:** Plexi 2.0 — 3 months from `protocol-v2.md` date

---

## What Plexi 2.0 Is

Plexi 2.0 is **the agent-native release**. It takes v1 — a polished spatial terminal multiplexer with an external app protocol — and adds the four load-bearing primitives that v1 lacks: structured spawn intent (`OpenIntent`), a host-side event bus (`events.jsonl`), a stateful multi-step task container (`Run`), and rich notification actions that can wrap and resume Runs. On top of those, it lands Plexi IQ Stage 1 (the in-host agent orchestrator), typed pipes Phase 1 (apps compose without code changes), a capability enforcement pass (runtime permission prompts), and protocol version negotiation.

Everything in v2 exists to serve one sentence from `VISION.md`: **"one install, three interfaces."** Apps become skills, agents become apps, the host orchestrates. Read `docs/specs/protocol-v2.md` §TL;DR for the 2-paragraph version; read it in full for the contract.

The explicit design constraint: **the SDK barely changes.** The host gets smarter; apps stay dumb. Python stays zero-dependency stdlib. Rust SDK gets a parity pass in Month 3 but lags in polish.

---

## Source-of-Truth Index

This doc **references** rather than duplicates. When in doubt, the linked file is authoritative. This doc's only job is to tell you what's in scope for 2.0 and where each piece is spec'd.

| Role | File | What it defines |
|---|---|---|
| Compass | [`docs/VISION.md`](../VISION.md) | Why Plexi exists. The six non-negotiables every v2 decision is checked against. |
| Protocol contract | [`docs/specs/protocol-v2.md`](protocol-v2.md) | The v2 wire format. `OpenIntent`, event bus, `Run`, rich notifications, version negotiation, capability enforcement. §1 is the authoritative in-scope / deferred list. §12 is the ship order. |
| App infrastructure | [`docs/specs/app-infrastructure.md`](app-infrastructure.md) | The v1 contract v2 depends on. Manifest, protocol events, draw commands, env vars. Not edited by v2 — only added to. |
| Typed pipes | [`docs/specs/typed-pipes.md`](typed-pipes.md) | Phase 1 (manifest `[app.io]`, auto-wire, linking matrix) ships with v2. Phase 0 already shipped on alpha. |
| Agent orchestration | [`docs/specs/agent-orchestration.md`](agent-orchestration.md) | Plexi IQ design. §4 trust/risk floats are **deferred to v2.1** — v2 uses binary prompts. |
| Agent mode UI | [`docs/specs/agent-mode.md`](agent-mode.md) | Per-pane Ctrl+/ overlay. v2 wires it to Plexi IQ; UI surface otherwise unchanged. |
| Manifest schema | [`schemas/plexi-manifest-schema.json`](../../schemas/plexi-manifest-schema.json) | JSON Schema validation. v2 adds `protocol_version`, `observes`, `create_runs`, `open_intent_kinds`, `[app.skill]`, `[app.agent]`. |
| Next release | [`docs/specs/protocol-v2.1.md`](protocol-v2.1.md) | UI primitives (viewers, editors, canvas). **Explicitly not v2.0.** Listed here only so you know it exists and won't drift. |

### Plexi IQ

There is no standalone `plexi-iq.md` file. Plexi IQ is spec'd inline at **`protocol-v2.md` §9**. The implementation lives in `src/plexi_iq/` (stubbed by PR #207, filled in during Month 3). If you need the IQ design, read `protocol-v2.md` §9, then `agent-orchestration.md` for the deeper rationale — but remember v2 only ships Stage 1 (in-host orchestrator on `claude -p --resume`), not PGAP.

---

## In Scope for 2.0 — Authoritative List

The full list with rationale lives in `protocol-v2.md` §1. Mirrored here for visibility:

1. **Protocol version negotiation** — `protocol-v2.md` §10
2. **Host event bus** — `protocol-v2.md` §4 (new `events.jsonl`, `EventSubscribe` / `EventData`)
3. **`OpenIntent` payload on `Init`** — `protocol-v2.md` §3
4. **`Run` primitive** — `protocol-v2.md` §5 (dumb store, draw commands, JSONL log)
5. **Rich notification actions** — `protocol-v2.md` §6 (typed action enum, `run_id` binding)
6. **Capability enforcement pass** — `protocol-v2.md` §7 (runtime prompts, `permissions.json`, `observes`)
7. **Typed pipes Phase 1** — `typed-pipes.md` §2.3+ (manifest wiring, auto-wire, linking matrix UI)
8. **Plexi IQ Stage 1** — `protocol-v2.md` §9 (in-host orchestrator, `claude -p --resume` backend)
9. **`[app.skill]` + `[app.agent]` manifest sections** — installable skills and agents
10. **SDK 0.4.0** — `OpenIntent` + `Run` convenience methods, all examples migrated
11. **Migration pass** — all bundled example apps bumped to `protocol_version = 2`

### V2 Product (ships with or after the protocol work)

The protocol items above are the engineering contract. These are the user-facing surface that lands alongside them:

- **App registry + `plexi install`** — remote registry, `plexi install <id>` and `plexi install --local` (#233)
- **App Store app** — discoverable catalog, developer publishing, one-click install
- **Plexi Intelligence (PGAP)** — hosted LLM gateway: audit, budget, model routing. **Note:** the *gateway* is v2 product; the *protocol-level* `intelligence-protocol.md` routing is deferred to v2.1. v2 ships the user-facing story; v2.1 ships the full routing layer.
- **Billing — Plexi Credits or BYOK** — credits (Anthropic wrapper) or bring-your-own-key
- **`@agent` syntax** — in agent mode, `@agentname` invokes installed `[app.agent]` apps; resolves to running instance first (#232)

---

## Explicitly Deferred — Not in 2.0

The full list lives in `protocol-v2.md` §1 "Explicitly deferred to v2.1+" and §15 "Defers". Mirrored here so you can scan without jumping files:

- **PGAP as a protocol-level intelligence gateway** (`intelligence-protocol.md`, #213) — v2 ships the user-facing "Plexi Intelligence" product but individual apps still make their own LLM calls. v2.1 routes all calls through PGAP.
- **Trust/risk float learning** (`agent-orchestration.md` §4) — v2 uses binary Yes once/Yes always/No prompts. The data (`PermissionPrompted` events) is logged so v2.1 can train without a migration.
- **Agent replay testing** (`agent-replay-testing.md`) — the event bus makes this free once it exists, but building the replayer is v2.1.
- **WASM/PWA deployment** (`wasm-pwa-deployment.md`) — back-burner until multiplayer.
- **SpacetimeDB sync** (`sync-architecture.md`) — shared workspaces across machines.
- **Chat primitive** (`chat-primitive.md`) — typed pipes are the only composition primitive in v2.
- **Core text editor primitive** (`core-text-editor-primitive.md`)
- **Advanced UI SDK egui widgets** (`core-advanced-ui-sdk.md`, #132)
- **Spatial canvas Option B/C** (`spatial-canvas.md`) — v2 stays on Option A as background.
- **Protocol v2.1 UI primitives** (`protocol-v2.1.md`) — viewers, editors, canvas. Separate release.
- **Notification undo** (#223)
- **App-focus manager, app-shell config, core layout presets, core-text-editor** — all referenced by `protocol-v2.md` §15 as valid specs that are not required for 2.0.
- **`SpawnLifecycle::Prompt`** — stays stubbed as `Orphan` in v2.

---

## Non-Goals — Will Never Be in Protocol v2

From `protocol-v2.md` §1, reproduced verbatim because they're load-bearing philosophical lines:

1. **No generic RPC layer between arbitrary apps.** Request/Reply stays scoped to `app_api.rs` (host-mediated capability calls). App↔app data flows through typed pipes.
2. **No chat/message bus primitive.** Typed pipes are the only composition primitive.
3. **No host-side workflow orchestration.** Runs are dumb containers; orchestration lives in Plexi IQ.

---

## Release Checklist

This is the thing that tracks 2.0 completion. Each line is a discrete, verifiable deliverable. Flip the status box as work lands; the "done" state for each item is defined under it.

### Month 1 — Plumbing

- [ ] **M1.1 — Protocol version negotiation** — `Init` carries `protocol_version: u32`; manifests declare it; host reads and negotiates. **Done when:** a v1 app launched against a v2 host receives `protocol_version = 1` and a v2 app receives 2, with a stderr warning when the manifest omits the field. Spec: `protocol-v2.md` §10.
- [ ] **M1.2 — Host event bus** — `EventSubscribe` / `EventData` draw commands, `~/.plexi-alpha/events.jsonl` JSONL log, background writer thread, bounded channel with drop-on-full. **Done when:** `tail -f events.jsonl` during a session shows a coherent stream of `AppSpawned`, `AppClosed`, `PipeWrite`, `NotificationEmitted`, `ApiCall` events. Spec: `protocol-v2.md` §4.
- [ ] **M1.3 — `OpenIntent` on `Init`** — field added, palette/CLI/SpawnApp all construct it, SDK (Python + Rust) exposes read access on init. **Done when:** `plexi launch text-editor foo.md` opens `foo.md` via `OpenIntent::File` without text-editor reading argv. Spec: `protocol-v2.md` §3.
- [ ] **M1.4 — `Run` primitive** — `RunCreate`/`RunUpdate`/`RunComplete` draw commands, `runs.jsonl` log, in-memory index, notification palette run-card rendering. **Done when:** the §5 video editor scenario runs end-to-end (agent → run → spawn → blocked-on-user → notification → resume → complete). Spec: `protocol-v2.md` §5.

### Month 2 — Surface

- [ ] **M2.1 — Rich notification actions** — closed enum (`Focus`, `Confirm`, `TextInput`, `Dismiss`, `ResumeRun`, `OpenIntent`, `RunCommand`, `ExternalUrl`), `run_id` on `Notification`, palette integration. **Done when:** a notification with `ResumeRun` action resumes a blocked run in one click. Spec: `protocol-v2.md` §6.
- [ ] **M2.2 — Capability enforcement pass** — runtime Yes once / Yes always / No prompt flow, `~/.plexi-alpha/permissions.json` persistence, `observes` capability gates event bus subscriptions, `OpenIntent` path validated against directory scope at the host boundary. **Done when:** an app trying to read a file outside its workspace scope is refused with a visible prompt and the decision persists across restarts. Spec: `protocol-v2.md` §7.
- [ ] **M2.3 — Typed pipes Phase 1** — `[app.io]` manifest parsing, auto-wiring of matching kind+name pairs, linking matrix overlay UI. **Done when:** two unrelated example apps compose via matching kind+name with no code changes. Spec: `typed-pipes.md` §2.3+.

### Month 3 — Intelligence

- [ ] **M3.1 — `[app.skill]` manifest section** — skills are invokable capabilities; registry indexes them; Plexi IQ can discover. **Done when:** an app declaring `[app.skill]` appears in IQ's skill registry and is invokable via agent mode. Spec: `protocol-v2.md` §9.
- [ ] **M3.2 — `[app.agent]` manifest section** — installable agent apps with system prompt + tool allowlist. **Done when:** an installed agent app can be invoked via `@agentname` in agent mode and gets its declared tool scope. Spec: `protocol-v2.md` §9.
- [ ] **M3.3 — Plexi IQ Stage 1 — in-host orchestrator** — `src/plexi_iq/` filled in (Stage 0 scaffolding from PR #207 becomes Stage 1 implementation), `claude -p --resume` backend, agent mode integration, Run lifecycle, `/approve` / `/deny` / `/status` / `/jobs` slash commands. **Done when:** agent mode can delegate a task to parallax, track it as a Run, and surface completion via notification. Spec: `protocol-v2.md` §9.
- [ ] **M3.4 — SDK 0.4.0** — Python + Rust convenience methods for `OpenIntent` and `Run`. Rust SDK brought to parity with Python protocol surface added in v2. **Done when:** `pip install plexi-sdk==0.4.0` and `cargo add plexi-sdk@0.4.0` both expose `spawn_app(..., open_intent=...)` and `run_create/update/complete` helpers. Spec: `protocol-v2.md` §3, §5.
- [ ] **M3.5 — Migration pass** — all bundled example apps bumped to `protocol_version = 2`, stress-tested under v2 host, DEV_LOG entry, `CHANGELOG.md` section, Homebrew cask updated. **Done when:** `just install-alpha` installs a clean v2 host with all 36+ example apps running.

### Product / Cross-cutting

- [ ] **P.1 — App registry + `plexi install`** — remote registry, `plexi install <id>`, `plexi install --local` (#233).
- [ ] **P.2 — App Store app** — discoverable catalog, developer publishing flow, one-click install integration.
- [ ] **P.3 — Plexi Intelligence product surface** — user-facing credits + BYOK, billing page, model selector. (Protocol-level PGAP routing deferred to v2.1.)
- [ ] **P.4 — `@agent` syntax in agent mode** — resolves to running instance first, falls back to new spawn (#232).
- [ ] **P.5 — Agent flow visualizer app** — validation app that subscribes to `AppSpawned` + `AgentTurn` + `PipeWrite` events and renders the graph. Not must-ship; must-exist-during-testing. Proves the event bus is sufficient.

### Hardening / Ops

- [ ] **O.1 — Alpha bug burn-down** — every issue labeled `alpha-bug` resolved before RC.
- [ ] **O.2 — RC pipeline** — `alpha` → `v2` branch → `v2.0.0-rc.N` tag via `just bump` → promote to `2.0.0` via `just release`. No new CI infra. Document the convention in `CLAUDE.md` or this file.
- [ ] **O.3 — DEV_LOG + CHANGELOG complete** — every M1–M3 item has a DEV_LOG `[DECISION]` entry on land, CHANGELOG has a `## [2.0.0] — YYYY-MM-DD` section summarizing user-facing changes.

---

## What Validates Each Item

Copied from `protocol-v2.md` §12 "What validates each item" — duplicated here because this is the spec you'll actually have open when testing:

- **Event bus:** `tail -f ~/.plexi-alpha/events.jsonl` during any session shows a coherent stream.
- **OpenIntent:** `plexi launch text-editor foo.md` opens foo.md without text-editor reading argv.
- **Run:** the video editor scenario in `protocol-v2.md` §5 runs end-to-end.
- **Rich notifications:** a notification with `ResumeRun` action resumes a blocked run in one click.
- **Capability:** trying to read a file outside scope from an app returns a permission error, prompts the user, and the decision persists.
- **Typed pipes:** two unrelated example apps compose via matching kind+name with no code changes.
- **Plexi IQ:** agent mode can delegate a task to parallax, track it as a Run, and surface completion.

---

## Change Process — Editing This Document

Repeated here because this section governs this file's lifecycle:

1. **`CODEOWNERS` enforces review.** Any PR touching this file requires the repo owner's review. GitHub will block merge otherwise.
2. **`Last updated` bump is required.** Every edit must update the date at the top.
3. **Paired edit rule.** Scope changes (adding items to "In Scope" or moving items from "Deferred" to "In Scope") require a simultaneous edit to `protocol-v2.md` or the relevant sub-spec. This file indexes; it does not define.
4. **DEV_LOG entry required.** Every edit that isn't a checklist status flip requires a `[DECISION]` entry in `DEV_LOG.md` with the new scope boundary and the reason.
5. **Checklist status flips are exempt from rules 3 and 4**, but flipping an item to "done" requires a DEV_LOG entry documenting what validates it.
6. **Do not delete items from "Explicitly Deferred."** If something genuinely no longer belongs in v2.1+, move it to `docs/research/` or archive with a note. The deferred list is the second-most-important part of this document.

---

## References — The Full v2 File Set

Every file that a v2 reader may need to open, in read order:

1. `docs/VISION.md` — **read first, always**
2. `docs/specs/plexi-v2.md` — this file (scope)
3. `docs/specs/protocol-v2.md` — the protocol contract
4. `docs/specs/typed-pipes.md` — Phase 1 details
5. `docs/specs/agent-orchestration.md` — IQ rationale (note: §4 floats are deferred)
6. `docs/specs/agent-mode.md` — UI surface for IQ
7. `docs/specs/app-infrastructure.md` — the v1 contract v2 extends
8. `schemas/plexi-manifest-schema.json` — manifest validation
9. `ROADMAP.md` — weekly operational view (this spec is the contract; roadmap is the calendar)
10. `DEV_LOG.md` — newest-first decision journal; read top ~100 lines before editing any v2 file

**Implementation entry points:** `src/app_protocol.rs` (types), `src/process_app.rs` (subprocess handling), `src/pane_ops.rs` (dispatch), `src/app_registry.rs` (manifest), `src/app_permissions.rs` (enforcement), `src/notification_log.rs`, `src/notify_socket.rs`, new `src/event_log.rs` (bus, M1.2), new `src/run_store.rs` (runs, M1.4), `src/plexi_iq/` (orchestrator, M3.3 — Stage 0 stubs land via PR #207).

---

**End of scope spec.** When Plexi 2.0 ships, every unchecked box above must be checked or explicitly moved to v2.1 via the change process. Nothing is done until the box is ticked.
