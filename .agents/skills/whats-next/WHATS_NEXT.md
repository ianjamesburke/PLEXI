# What's Next

> Read this at the start of any Plexi session. It is the single anchor for orientation.
> Skill: `/whats-next` -- re-runs the audit and updates this file. Auto-updated by `/merge-pr` after every merge.
> Landed-work history lives in `docs/DEVLOG.md`. Sprints are retired (2026-07-01): sequencing is priority + `blocked_by` + this file's Arc. The north star is `NORTH_STAR.md`.

---

## Current State (2026-07-11, dogfooding audit lands 15 assistant/UX tasks)

`alpha` is at `56b994dc`: workspace saves are atomic (`0367`), CLI commands route by binary channel (`0365`), and the Assistant E2E harness with local/cheap-model verification (`0359`) is landed. Details in `docs/DEVLOG.md`.

**Free v1 finish line: one verification pass remains.** `0358` (P1, small) — production hosted-registry install smoke from a clean machine after deploying alpha. It is the only tracked Epoch 1 gap.

**Dogfooding verdict (three live sessions, 2026-07-11):** the assistant-as-operator story has a load-bearing hole — the host's v3.7 tool protocol (`ExposeTools`/`AiTool`, landed in `0227`) was never wrapped in the Python SDK, so **zero apps can expose connector tools** (`0369`, P1). The assistant also cannot type into a terminal it opens (`0376`, P1), has no internet fetch (`0381`), can't target an existing pane (`0374`), and has no answer for "do you take MCP connectors?" (`0382`, scope task). Permission modal and assistant-UX papercuts round out the batch (`0372`/`0373`/`0375`/`0377`/`0378`/`0379`/`0380`).

**Commercial launch deferred post-v2 (decision 2026-07-11):** Ian is the sole publisher for now and no sales ship at launch. `0344`/`0352`/`0353`/`0356` moved to backlog, tagged `post-v2`. No code stubs needed — sales are already dark behind `SALES_ENABLED=false` and the publish/package seams (`publisher` field, `PublishClient`) are landed. Polar AUP constraint unchanged: first-party MoR only; third-party payouts need `0352`'s rail decision when the marketplace opens.

Open PRs affecting priority reading:

- `#2353` open: toolbar button focus-steal fix (external branch).
- `#2323` draft: WASM SDK v3 platform POCs (`0285`/`0287` lane).
- `#2318` open: stats idle-heartbeat filtering (`0282`).
- `#2316` open: todo app space-to-toggle regression (`0281`).
- `#2282` open: collapsible subcontexts (`0241`), failed build check.
- `#1604` open: Windows port (external branch).

Not real yet: first-party sales live (`0356`), production registry install smoke (`0358`), app connector tools reachable from any app (`0369`), managed `ai.query` backend (`0323`), third-party publisher economy (`0344`/`0352`/`0353`).

---

## The Arc

Every epoch feeds the next; the whole line points at `NORTH_STAR.md` ("the last app you'll ever need" — a portable, ownable computing environment where the marketplace is how it gets apps and makes money). Tasks are indented under the outcome they serve; nested tasks are blocked by their parent.

### Epoch 1 — Land v1 (now) — one verification pass from done

A stranger installs, an agent builds an app first try, a free reviewed app installs from the hosted registry.

- First-user surface is landed; detail in `docs/DEVLOG.md`.
- `0358` production hosted-registry install smoke after alpha deploy — the last tracked gap, and the release gate.

### Epoch 2 — Intelligence (NORTH_STAR Phase 3; runs parallel to Epoch 3)

The host Assistant becomes the workspace operator: typed host tools behind the permission broker, named agent personas, skills, app connectors.

- Registry/routing/settings/history/skills/host-tools/E2E-harness are landed (`0225`–`0229`, `0359`).
- **The connector-tool chain — the epoch's current spine:**
  - `0369` Python SDK wrapper for `ExposeTools`/`AiTool`/`ToolResult` — unblocks every app exposing tools
    - `0370` calculator exposes its operations as tools (first real connector)
    - `0371` connector-tools POC app under `apps/dev/`
  - `0382` scope: MCP servers as assistant tool sources (design decision, likely rides the `0369` connector path)
- **Assistant operator gaps (each independently shippable):**
  - `0376` terminal send-input tool (wraps existing `RunInLinkedTerminal`) — makes "run a command" real
  - `0381` internet fetch tool (wraps existing `HttpRequest`/`net.http`)
  - `0374` pane-targeting for `host.apps.open`/`host.panes.open`
  - `0380` slash-command output visible to the assistant
- **Assistant/permission UX (trust surface for everything above):**
  - `0377` permission modal keyboard nav; `0378` modal responsive sizing
  - `0373` interactive permissions manager (replaces `/revoke <target_id>`)
  - `0372` `/model` interactive picker; `0375` `/compact` progress/success feedback

### Testing foundation: shared headless/live vocabulary

Agents can drive and verify the host through one scene language. Generic verbs and symbolic handles, normalized Process/native/WASM semantics, the installed-host backend, and the full-host coverage audit are landed (`0362`, `0363`, `0364`, `0361`).

### Epoch 3 — Commercial launch (Track B) — deferred post-v2 (decision 2026-07-11)

The registry brokers money; never a dependency for running installed apps. Spec: `docs/marketplace-monetization.md` (`0315`, done). **Constraint (2026-07-10): Polar's AUP bars the third-party-marketplace model; Polar is first-party MoR only.**

**Decision (2026-07-11): Ian is the only publisher for now; no sales at launch.** The whole money surface is deferred post-v2 — no code stubs required: first-party sales are already code-complete behind `SALES_ENABLED=false` (`website/src/server/env.ts`), the package envelope already carries `publisher`, and `PublishClient`/`Submission` (`src/app/marketplace.rs`) already serve the first-party publish path. The door is open; nothing in the free path references the deferred work.

- Buy-side foundation + first-party monetization code: landed (`0339`/`0347`/`0325`/`0355`).
- Backlogged post-v2: `0356` (sales go-live ops), `0352` (payout-rail decision) → `0344` (third-party submission pipeline), `0353` (clawback).
- Still active (door-keeping code + recurring-revenue groundwork): `0322` paid-download host gating → `0341` marketplace app + paywall; `0354` subscription active-status gating; `0323` managed `ai.query` backend.

### Epoch 4 — The Platform (WASM, mobile, hosted)

Same app contract, sandboxed runtime — the only way apps run on iOS (in-process WASM) and hosted (same typed contract over WebSocket).

- Transport-agnostic contract pre-paid: `0327`, `0336`, `0348` landed.
- **The WASM lane**
  - `0285` WASM-native Python SDK + CPython-in-WASM runtime (draft PR `#2323`)
    - `0286` WASM bundle distribution through the registry (also after `0322`, `0344`)
      - `0287` cloud streaming runtime (server-side apps, thin clients)
- **Trust-rail riders**
  - `0333` biometric user-verification effect (Touch ID/Face ID; keychain-bound secrets)

### Epoch 5 — The Portable Instance

Your whole environment runs as a server (local or cloud, same architecture); thin clients attach from anywhere. SpacetimeDB persistence/sync. No stint tasks yet — deliberately: everything above lands first.

Maintenance (input debt, hygiene, polish) deliberately does not appear here — it advances no epoch. It lives in `stint list` with correct priorities and blockers.

---

## Priority Stack (flat view)

P0: none.
P1: `0358` (release gate: production install smoke), `0369` (SDK connector-tool wrapper — Epoch 2 chain head), `0376` (assistant terminal input), `0285` (draft PR), `0341`*, `0286`* (deep-blocked: `0344` now backlog), `0287`*.
P2: `0322`, `0354`, `0323`, `0317`, `0295`, `0297`, `0374`, `0375`, `0377`, `0381`, `0368`, `0370`*, plus backlog.
P3 and below: `0371`*, `0372`, `0373`, `0378`, `0379`, `0380`, `0382`, `0310`, `0318`, `0357`, `0360`, `0366`, post-v2 money cluster (`0344`, `0352`, `0353`, `0356`), plus backlog in `stint list`.
(* = blocked; see the Arc for what unblocks them.)

**v1 release gate (decision 2026-07-11):** `0369` → `0376` → `0377` → `0358`. The first three close the assistant's broken-promise gaps (advertised capabilities that don't work: connector tools unreachable, no terminal input, mouse-only permission modal); `0358` runs last so the install smoke validates the build containing them. Everything else in the `0368`–`0382` dogfooding batch is post-v1 polish.

**Next recommended task:** `0369` — SDK connector-tool wrapper, head of the release-gate chain.

---

## Key Reference Docs

| Doc | What it covers |
|-----|----------------|
| `NORTH_STAR.md` | The ten commandments, phases, v1 reviewed-native / v2 WASM boundary |
| `docs/DEVLOG.md` | Landed-work history (moved out of this file) |
| `docs/app-framework-marketplace.md` | App framework + marketplace PRM; resolves roadmap conflicts |
| `docs/marketplace-hosted.md` | Hosted registry, paid apps, AI subscription spec |
| `docs/marketplace-monetization.md` | Monetization + anti-fork model; the payout-rail decision (`0352`) records here |
| `src/workspace/AGENTS.md` | Environment secrets resolver reference (retired from `docs/workspace-env-secrets.md`, stint `0408`) |
| `docs/assistant-host-app.md` | Assistant spec: connectors, permissions, slash commands |
| `docs/wasm-runtime.md` | WASM runtime spec |
| `sdk/python/SDK_V3.md` | SDK v3 API reference |
| `src/testing/TESTING.md` | Test infra reference |

---

## How To Update This File

Run `/whats-next` at the start of any session -- it re-runs `stint list` + open-PR check and rewrites the Arc + Priority Stack. `/merge-pr` runs the same routine after every merge. Landed-work history goes to `docs/DEVLOG.md` (append a dated entry), never accumulates here. Sprints are retired: do not create sprint files or `sprint:` fields; new work slots into the Arc under the outcome it serves, with `blocked_by` wiring. Always update stint tasks first when new work is discovered.
