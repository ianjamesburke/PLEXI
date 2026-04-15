# Plexi Specs — Index

**This is the single source of truth for where every spec lives.** If you're looking for a spec, start here. If you're writing a new one, use the table at the bottom to pick the right bucket.

Three-bucket taxonomy. Every spec has one clear reason to exist; pick the bucket by *purpose*, not by version.

---

## `releases/` — authoritative contracts per version

Each release has **two files**: a short protected *scope checklist* and a long *technical contract*. They play different roles and should never be merged.

| Release | Scope (protected checklist) | Contract (technical spec) |
|---|---|---|
| **Plexi 2.0** — Agent-native orchestration (OpenIntent, Runs, event bus, rich notifications, capability enforcement, typed pipes Phase 1, Plexi IQ Stage 1, protocol version negotiation) | [`releases/plexi-v2.0-scope.md`](releases/plexi-v2.0-scope.md) | [`releases/plexi-v2.0.md`](releases/plexi-v2.0.md) |
| **Plexi 2.1** — UI primitives (viewport, text_input, tabs, grid, modal, PushTransform/PopTransform, exact text measurement) | *not yet* | [`releases/plexi-v2.1.md`](releases/plexi-v2.1.md) |

**Scope vs contract:**
- **Scope** — short, protected by `CODEOWNERS`, narrow "what ships / what doesn't" checklist. Indexes the contract and sub-specs; never defines. Edits require approval.
- **Contract** — long-form technical specification. Defines protocol additions, primitives, ship order, open questions. Anyone can propose edits via PR.

The scope doc is the answer to "can I add X to this release?" The contract doc is the answer to "how does X actually work?" If they disagree, the scope wins — the contract catches up to it.

---

## `subsystems/` — deep design docs that span versions

Load-bearing mechanisms that stay alive across releases. Each has a status header indicating which release it shipped in (or is drafted for). Release contracts link to these instead of restating them.

- [`subsystems/app-infrastructure.md`](subsystems/app-infrastructure.md) — v1 out-of-process app protocol: manifest, events, draw commands, lifecycle
- [`subsystems/typed-pipes.md`](subsystems/typed-pipes.md) — inter-app typed channels (Phase 0 shipped; Phase 1 for v2.0)
- [`subsystems/agent-orchestration.md`](subsystems/agent-orchestration.md) — directory-scoped agent networks, trust/risk scoring, delegation flow
- [`subsystems/agent-mode.md`](subsystems/agent-mode.md) — per-pane agent mode UI and slash commands
- [`subsystems/intelligence-protocol.md`](subsystems/intelligence-protocol.md) — PGAP routing, budget enforcement, cost ledger (deferred to v2.1+)

---

## `proposals/` — ideas not yet committed to a release

Exploratory design. Each file is a pitch that may or may not become a subsystem. **No promise of shipping.** A proposal graduates to a subsystem when an actual release commits to it (see "Promotion" below).

- [`proposals/input-layering.md`](proposals/input-layering.md) — host-owned keyboard routing stack (fixes alpha-bugs #240, #236, consume_key/TextEdit class); targets v2.0 §7.5
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
- [`proposals/secrets-manager.md`](proposals/secrets-manager.md) — Keychain-backed secrets with global/directory scoping and runtime injection into agent sandboxes
- [`proposals/plexi-iq.md`](proposals/plexi-iq.md) — in-process agent harness: dual-backend (native Anthropic API + claude -p proxy), pane-native agent mode, tool dispatch against the live app protocol

---

## Where to put a new spec

| You are writing... | Put it in... |
|---|---|
| The short protected checklist for a numbered release | `releases/plexi-vX.Y-scope.md` |
| The full technical contract for a numbered release | `releases/plexi-vX.Y.md` |
| A deep design for a mechanism shipping in a release | `subsystems/<name>.md` |
| An exploratory idea that may or may not land | `proposals/<name>.md` |
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
