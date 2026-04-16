# Plexi Specs — Index

Single source of truth for where every spec lives. Start here for any spec question.
**This is the single source of truth for where every spec lives.** If you're looking for a spec, start here. If you're writing a new one, use the table at the bottom to pick the right bucket.

Four-bucket taxonomy. Every spec has one clear reason to exist; pick the bucket by *purpose*, not by version.

---

## Releases

| File | Version | Status | Summary |
|------|---------|--------|---------|
| [`releases/plexi-v2.1.md`](releases/plexi-v2.1.md) | v2.1 | Implemented | UI primitives: PushTransform, MeasureText, viewport/text_input/tabs/grid/modal, feature negotiation |
| [`releases/plexi-v2.2.md`](releases/plexi-v2.2.md) | v2.2 | Draft | Rich text, clip regions, multiline input, IME, input layering, PyPI SDK |
| [`releases/plexi-v2.3.md`](releases/plexi-v2.3.md) | v2.3 | Draft (speculative) | Spatial canvas, node graph, video primitives, WASM/PWA target |
Each release has **two files**: a short protected *scope checklist* and a long *technical contract*. They play different roles and should never be merged.

| Release | Scope (protected checklist) | Contract (technical spec) |
|---|---|---|
| **Plexi 2.0** — Recursive `.plexi` instance foundation (Fractal PGAP, depth tree POC, lifecycle, render summaries, embedded instance spike, capability containers, event bus, OpenIntent, Runs, depth notifications, typed pipes, Plexi IQ) | [`releases/plexi-v2.0-scope.md`](releases/plexi-v2.0-scope.md) | [`releases/plexi-v2.0.md`](releases/plexi-v2.0.md) |
| **Plexi 2.1** — UI primitives (viewport, text_input, tabs, grid, modal, PushTransform/PopTransform, exact text measurement) | *not yet* | [`releases/plexi-v2.1.md`](releases/plexi-v2.1.md) |

**Scope vs contract:**
- **Scope** — short, protected by `CODEOWNERS`, narrow "what ships / what doesn't" checklist. Indexes the contract and sub-specs; never defines. Edits require approval.
- **Contract** — long-form technical specification. Defines protocol additions, primitives, ship order, open questions. Anyone can propose edits via PR.

The scope doc is the answer to "can I add X to this release?" The contract doc is the answer to "how does X actually work?" If they disagree, the scope wins — the contract catches up to it.

---

## Subsystems

| File | Scope |
|------|-------|
| [`app-infrastructure.md`](app-infrastructure.md) | App registry, manifest format, launch lifecycle, pipe wires |
Load-bearing mechanisms that stay alive across releases. Each has a status header indicating which release it shipped in (or is drafted for). Release contracts link to these instead of restating them.

- [`subsystems/app-infrastructure.md`](subsystems/app-infrastructure.md) — v1 out-of-process app protocol: manifest, events, draw commands, lifecycle
- [`subsystems/typed-pipes.md`](subsystems/typed-pipes.md) — inter-app typed channels (Phase 0 shipped; Phase 1 for v2.0)
- [`subsystems/fractal-pgap.md`](subsystems/fractal-pgap.md) — recursive `.plexi` instance nesting, depth-native navigation, capability-scoped agents, and embedded PGAP instances; v2.0 roadmap lives in [`roadmaps/fractal-pgap/`](roadmaps/fractal-pgap/)
- [`subsystems/agent-orchestration.md`](subsystems/agent-orchestration.md) — directory-scoped agent networks, trust/risk scoring, delegation flow
- [`subsystems/agent-mode.md`](subsystems/agent-mode.md) — per-pane agent mode UI and slash commands
- [`subsystems/intelligence-protocol.md`](subsystems/intelligence-protocol.md) — PGAP routing, budget enforcement, cost ledger (deferred to v2.1+)

---

## Proposals

Proposals live under `proposals/` and are promoted to release specs when scoped and accepted.

| File | Topic | Target |
|------|-------|--------|
| `proposals/input-layering.md` | Key dispatch priority tiers | v2.2 §7.5 |
| `proposals/spatial-canvas.md` | Infinite zoomable canvas | v2.3 §1 |
| `proposals/wasm-pwa-deployment.md` | WASM/PWA build target | v2.3 §4 |
| `proposals/media-primitives.md` | Video frame, waveform, playhead | v2.3 §3 |

---

## Feature → Spec mapping

| Feature | Spec |
|---------|------|
| `core_v1` | v2.0 |
| `open_intent_v1` | v2.0 |
| `event_bus_v1` | v2.0 |
| `runs_v1` | v2.0 |
| `typed_pipes_v1` | v2.0 |
| `ui_primitives_v1` | v2.1 |
## `roadmaps/` — implementation tracks for large efforts

Large proposals or subsystems that need multiple PRs get a roadmap folder with end-to-end testable slices. Roadmaps are not release contracts; they are execution plans that reference proposals, subsystems, release contracts, and source files.

- [`roadmaps/fractal-pgap/`](roadmaps/fractal-pgap/) — v2.0 execution plan for Fractal PGAP / recursive instance nesting

---

## Where to put a new spec

| You are writing... | Put it in... |
|---|---|
| The short protected checklist for a numbered release | `releases/plexi-vX.Y-scope.md` |
| The full technical contract for a numbered release | `releases/plexi-vX.Y.md` |
| A deep design for a mechanism shipping in a release | `subsystems/<name>.md` |
| An exploratory idea that may or may not land | `proposals/<name>.md` |
| A multi-PR execution plan for a proposal or subsystem | `roadmaps/<name>/` |
| A design doc for a specific app that already exists | **Nowhere** — code is the spec once the app ships. Historical design docs belong in git history, not `docs/specs/`. |
| A spec for a separate product (e.g., iOS companion, hosted service) | Not here — use `docs/mobile/`, `docs/web/`, etc. |

## Promotion path

When a **proposal** graduates to "we're actually shipping this in release X":
1. Move the file from `proposals/` to `subsystems/`.
2. Add a status header to the top: `**Shipped in:** Plexi vX.Y` or `**Draft for:** Plexi vX.Y`.
3. Update the release contract (`releases/plexi-vX.Y.md`) to link to the subsystem doc by its new path.
4. Add an entry to the release scope (`releases/plexi-vX.Y-scope.md`) if the item is in-scope and needs tracking.

When a **subsystem** reaches shipped-and-stable status, nothing moves — it just gets a different status header and becomes part of the permanent record.

---

**Everywhere else in the repo that talks about specs should link here first.** `ROADMAP.md`, `CLAUDE.md`, release notes, PR descriptions — all should point at this index rather than deep-linking into a specific file. If you find a stale cross-reference, update it to come through this index.
