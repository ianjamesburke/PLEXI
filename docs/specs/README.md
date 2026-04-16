# Plexi Specs — Index

Single source of truth for where every spec lives. Start here for any spec question.

Three buckets: **releases**, **subsystems**, **proposals**. Every spec has one clear reason to exist; pick the bucket by purpose, not by version.

---

## Releases

The authoritative technical contract for a numbered release.

| File | Version | Status | Summary |
|------|---------|--------|---------|
| [`releases/plexi-v3.0.md`](releases/plexi-v3.0.md) | v3.0 | Draft (active target) | Pane ADT, PGAP v3, directory-scoped secrets, host-owned media + binary side channel, Plexi IQ wired, five example apps + quick-note. |

All v2.x release specs have been removed. v2.x work on `alpha` is frozen as `v2-last` and retired. v3.0 is the clean cut.

---

## Subsystems

Load-bearing mechanisms that stay alive across releases. Currently empty — v3.0 consolidates all subsystem design inline. Subsystem docs will be extracted here if they outgrow the release spec.

---

## Proposals

Speculative ideas that may or may not land in a future release. Proposals make no claim about being implemented.

| File | Topic |
|------|-------|
| [`proposals/spatial-canvas.md`](proposals/spatial-canvas.md) | Infinite zoomable canvas |
| [`proposals/wasm-pwa-deployment.md`](proposals/wasm-pwa-deployment.md) | WASM / PWA build target |
| [`proposals/sync-architecture.md`](proposals/sync-architecture.md) | Plexi Teams: cross-machine sync via SpacetimeDB |
| [`proposals/agent-replay-testing.md`](proposals/agent-replay-testing.md) | Deterministic agent testing via PGAP JSON replay |

---

## Where to put a new spec

| You are writing... | Put it in... |
|---|---|
| The technical contract for a numbered release | `releases/plexi-vX.Y.md` |
| A deep design for a mechanism shipping in a release | `subsystems/<name>.md` |
| An exploratory idea that may or may not land | `proposals/<name>.md` |
| A design doc for a specific app that already exists | Nowhere — code is the spec once the app ships. |

## Promotion path

When a proposal graduates to "we're actually shipping this in release X":
1. Move the file from `proposals/` to `subsystems/`.
2. Add a status header: `**Shipped in:** Plexi vX.Y` or `**Draft for:** Plexi vX.Y`.
3. Reference it from the release spec.

---

**Everywhere else in the repo that talks about specs should link here first.** If you find a stale cross-reference, update it to come through this index.
