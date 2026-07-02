# What's Next

> Read this at the start of any Plexi session. It is the single anchor for orientation.
> Skill: `/whats-next` -- re-runs the audit and updates this file. Auto-updated by `/merge-pr` after every merge.
> Landed-work history lives in `docs/DEVLOG.md`. Sprints are retired (2026-07-01): sequencing is priority + `blocked_by` + this file's Arc. The north star is `NORTH_STAR.md`.

---

## Current State (2026-07-02)

`alpha` is at `c7e926f5`. Since the last audit: the pane `new --tab` anchoring fix (`0337`, #2357) landed, plus a monetization spec doc (`0315`, done). The free v1 spine (scaffold, demo apps, packaging, trust labels, hosted registry smoke path, secrets, onboarding, website) is landed — see `docs/DEVLOG.md`. The v1 finish line: **a stranger installs Plexi, an agent builds a working app from the scaffold on the first try, and a reviewed free app installs from the hosted registry without an account.**

Open PRs that affect priority reading:

- `#2353` open: toolbar button focus steal fix from external branch.
- `#2323` draft: WASM SDK v3 platform POCs (`0285` / `0287` lane).
- `#2318` open: stats idle-heartbeat filtering (`0282`).
- `#2316` open: todo app space-to-toggle regression (`0281`).
- `#2314` open: palette scroll reset (`0280`).
- `#2282` open: collapsible subcontexts (`0241`), currently has a failed build check.
- `#1604` open: Windows port from external branch.

Not real yet: license-aware update gating (`0322`), managed `ai.query` backend (`0323`), package envelope (`0325`).

---

## The Arc

Every epoch feeds the next; the whole line points at `NORTH_STAR.md` ("the last app you'll ever need" — a portable, ownable computing environment where the marketplace is how it gets apps and makes money). Tasks are indented under the outcome they serve; nested tasks are blocked by their parent.

### Epoch 1 — Land v1 (now)

A stranger installs, an agent builds an app first try, a free reviewed app installs from the hosted registry.

- **Prove "agents build apps first try"** (the DX chain — dispatch in order)
  - `0342` `plexi host start` — one-command host launch with declarative boot layout (the live-E2E primitive) — **ready, P0, in progress**
    - `0343` drive-host skill — codified agentic live PR-testing loop
  - `0330` app-dev CLI path E2E audit + open-placement fix (default: sibling split) — **ready, next up**
    - `0331` agent-drives-agent E2E pipeline (isolated instance, parent as user-proxy, session capture)
      - `0332` authoring guidance consolidation + drift gates (dead links, effect-name drift, one canonical doc, CI-gated)
      - `0215` app-authoring benchmark + case-study directory (prompt library, scorecards, baseline sweep; also blocked by 0330)
- **Make shipped apps the exemplar tier** (apps are the de facto docs)
  - `0335` logs app rebuild — flagship SDK exemplar (v2 colors/labels/sorting restored, app-id filter, channel-aware) — **ready**
  - `0296` canonicalize the core app install set (blocked by done `0277`)
    - `0334` core-app cull + exemplar conformance sweep (delete POCs, survivors pass all gates; stats gauge fixes)
  - `0328` SDK component coverage audit (Polaris-class vocabulary, intrinsic size contract, `SetShortcuts` native chrome) — **ready**
- **First-user surface** (onboarding `0324` and website `0272` landed 2026-07-01)
  - `0280` palette scroll regression (PR `#2314` needs validation/merge decision) — **P0**
- **First-user critical path:** validate + merge `#2314` -> share.

### Epoch 2 — Intelligence (NORTH_STAR Phase 3; runs parallel to Epoch 3)

The host Assistant becomes the workspace operator: typed host tools behind the permission broker, named agent personas, skills, app connectors.

- `0225` agent registry + model routing
- `0226` settings scopes (user/workspace/local/session)
- `0228` conversation persistence + history
- `0229` skills + host tools (pane/app/terminal operations)
- (`0227` app connectors — done)

### Epoch 3 — Commercial launch (Track B)

The registry brokers money; never a dependency for running installed apps. Spec: `docs/marketplace-monetization.md` (0315, done 2026-07-02) — no client-side licensing, Polar as merchant of record, the paid download is the product.

- `0338` website account service (magic-link + device-code flow) — **ready, epoch head**
  - `0339` Polar checkout/webhooks/gated downloads (the 402 envelope)
    - `0322` host: account-gated paid downloads (also after `0340`)
      - `0341` marketplace app + paywall handoff (also after `0327`)
  - `0340` host `plexi account` CLI + license-machinery deletion
  - `0344` publisher submission pipeline + review queue (`plexi app publish` → admin approval; also after `0339`)
  - `0323` Plexi-managed `ai.query` backend (also after `0339`)
- `0325` app/agent/skill package envelope spec — unblocked

### Epoch 4 — The Platform (WASM, mobile, hosted)

Same app contract, sandboxed runtime. This is what makes the marketplace mobile-friendly (in-process WASM is the only way apps run on iOS) and hosted (same typed contract over WebSocket).

- **Pre-pay the toll: one transport-agnostic contract**
  - `0327` unify inter-pane comms on the event bus (events / binary pipes / PTY, delete the middle tier) — **ready, P0-priority standalone**
    - `0336` manifest `[launch] on_launch` policy (focus_existing / per-context / always_new; duplicates ride bus subscriptions)
- **The WASM lane**
  - `0285` WASM-native Python SDK + CPython-in-WASM runtime (draft PR `#2323`)
    - `0286` registry/client/payment epic for WASM apps
      - `0287` cloud streaming runtime (server-side apps, thin clients)
- **Trust-rail riders** (design lands on whatever Epoch 2 + trust gates establish)
  - `0333` biometric user-verification effect (Touch ID/Face ID via LocalAuthentication; keychain-bound secrets)

### Epoch 5 — The Portable Instance

Your whole environment runs as a server (local or cloud, same architecture); thin clients attach from anywhere. SpacetimeDB persistence/sync. No stint tasks yet — deliberately: everything above must land first. This epoch is where "hosted, mobile-friendly marketplace" matures into "your working life on any device" — the north star's inheritance layer stands on it.

Maintenance (input debt, hygiene, polish) deliberately does not appear here — it advances no epoch. It lives in `stint list` with correct priorities and blockers.

---

## Priority Stack (flat view)

P0: `0342` (in progress), `0343`*, `0330` (chain head), `0327`, `0280`.
P1: `0335`, `0331`*, `0328`, `0241` (PR needs fixing), `0338`, `0285`, `0332`*, `0339`*, `0340`*, `0341`*.
P2 and below: run `stint list`.
(* = blocked; see the Arc for what unblocks them.)

**Next recommended task:** `0330` — heads the P0 DX chain; `0335` (logs) and `0327` (event bus) are the parallel-friendly picks.

---

## Key Reference Docs

| Doc | What it covers |
|-----|----------------|
| `NORTH_STAR.md` | The ten commandments (portable formats, ergonomics, CLI-complete, local-first, no ambient authority…), phases, v1 reviewed-native / v2 WASM boundary |
| `docs/DEVLOG.md` | Landed-work history (moved out of this file) |
| `docs/app-framework-marketplace.md` | App framework + marketplace PRM; resolves roadmap conflicts |
| `docs/marketplace-hosted.md` | Hosted registry, paid apps, AI subscription spec |
| `docs/workspace-env-secrets.md` | Shared resolver contract for secrets |
| `docs/wasm-runtime.md` | WASM runtime spec |
| `docs/assistant-host-app.md` | Assistant app spec |
| `sdk/python/SDK_V3.md` | SDK v3 API reference |
| `src/testing/TESTING.md` | Test infra reference |

---

## How To Update This File

Run `/whats-next` at the start of any session -- it re-runs `stint list` + open-PR check and rewrites the Arc + Priority Stack. `/merge-pr` runs the same routine after every merge. Landed-work history goes to `docs/DEVLOG.md` (append a dated entry), never accumulates here. Sprints are retired: do not create sprint files or `sprint:` fields; new work slots into the Arc under the outcome it serves, with `blocked_by` wiring. Always update stint tasks first when new work is discovered.
