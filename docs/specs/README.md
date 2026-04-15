# Plexi Specs

Three-bucket taxonomy. Every spec has one clear reason to exist; pick the bucket by *purpose*, not by version.

## `releases/`

Authoritative contract for each shipped or in-progress version. Short index documents that reference the deep subsystem specs rather than restating them. One file per release.

- [`releases/plexi-v2.0.md`](releases/plexi-v2.0.md) — Plexi 2.0: orchestration layer (OpenIntent, Runs, event bus, rich notifications, capabilities, typed pipes Phase 1, Plexi IQ Stage 1, protocol version negotiation)
- [`releases/plexi-v2.1.md`](releases/plexi-v2.1.md) — Plexi 2.1: UI primitives (viewport, text_input, tabs, grid, modal, measure_text_exact) — ships after v2.0

## `subsystems/`

Deep design documents for load-bearing mechanisms that span versions. Each has a status line indicating which release it shipped in (or is drafted for). Release docs link to these instead of restating them.

- [`subsystems/app-infrastructure.md`](subsystems/app-infrastructure.md) — v1 out-of-process app protocol: manifest, events, draw commands, lifecycle
- [`subsystems/typed-pipes.md`](subsystems/typed-pipes.md) — inter-app typed channels (Phase 0 shipped, Phase 1 for v2.0)
- [`subsystems/agent-orchestration.md`](subsystems/agent-orchestration.md) — directory-scoped agent networks, trust/risk scoring, delegation flow
- [`subsystems/agent-mode.md`](subsystems/agent-mode.md) — per-pane agent mode UI and slash commands
- [`subsystems/intelligence-protocol.md`](subsystems/intelligence-protocol.md) — PGAP routing, budget enforcement, cost ledger (deferred to v2.1+)

## `proposals/`

Ideas being explored, not yet committed to a release. Each is a design document that may or may not become a subsystem. No promise of shipping.

- [`proposals/spatial-canvas.md`](proposals/spatial-canvas.md) — recursive zoom, 2D grid navigation, discrete LOD rendering
- [`proposals/chat-primitive.md`](proposals/chat-primitive.md) — UI primitive for LLM conversations in panes
- [`proposals/core-text-editor-primitive.md`](proposals/core-text-editor-primitive.md) — built-in text editor primitive
- [`proposals/core-advanced-ui-sdk.md`](proposals/core-advanced-ui-sdk.md) — egui-backed widgets beyond the draw protocol
- [`proposals/core-layout-presets.md`](proposals/core-layout-presets.md) — pre-baked pane split layouts
- [`proposals/app-focus-manager.md`](proposals/app-focus-manager.md) — priority-aware attention manager app
- [`proposals/app-shell-config.md`](proposals/app-shell-config.md) — ZDOTDIR-based shell addon manager app
- [`proposals/wasm-pwa-deployment.md`](proposals/wasm-pwa-deployment.md) — WASM app support + web client (long-term)
- [`proposals/sync-architecture.md`](proposals/sync-architecture.md) — SpacetimeDB-backed collaborative workspaces (long-term)
- [`proposals/agent-replay-testing.md`](proposals/agent-replay-testing.md) — deterministic agent replay + regression testing
- [`proposals/telegram-integration.md`](proposals/telegram-integration.md) — Telegram bridge bot app

---

## Where to put a new spec

| You are writing... | Put it in... |
|---|---|
| The ship list for a numbered release | `releases/plexi-vX.Y.md` |
| A deep design for a mechanism shipping in a release | `subsystems/<name>.md` |
| An exploratory idea that may or may not land | `proposals/<name>.md` |
| A design doc for a specific app that already exists | **Nowhere** — code is the spec once the app ships. Historical design docs belong in git history, not `docs/specs/`. |
| A spec for a separate product (e.g., mobile companion) | Not here — use `docs/mobile/`, `docs/web/`, etc. |

## When promoting a proposal to a subsystem

When a proposal graduates to "we're actually shipping this in release X," move it from `proposals/` to `subsystems/` and add a status header: `**Shipped in:** Plexi vX.Y` or `**Draft for:** Plexi vX.Y`. Release docs should then link to the subsystem doc rather than the original proposal.
