# What's Next

> Read this at the start of any Plexi session. It is the single anchor for orientation.
> Skill: `/whats-next` -- re-runs the audit and updates this file. Auto-updated by `/merge-pr` after every merge.
> Landed-work history lives in `docs/DEVLOG.md`. Sprints are retired (2026-07-01): sequencing is priority + `blocked_by` + this file's Arc. The north star is `NORTH_STAR.md`.

---

## Current State (2026-07-10, money-path + explorer viewers landed)

`alpha` is at the `#2366` merge (`bf7cb0a4`). Six PRs landed this session — the full money-path buy-side/sell-side/legal bundle (0325/0339/0347/0245/0252/0355, detail in `docs/DEVLOG.md`) plus `0349` explorer native viewers (`#2366`, rebased onto post-money-path alpha, re-verified 1527/1527 tests green, manually exercised on `plexi-pr-2366` — image/video/audio launch, resolver routing, close-return-to-explorer focus, and OS fallthrough all confirmed correct via log trace).

**Free v1 finish line is now effectively complete.** The last tracked P1 gap (`0349`) is merged. Remaining gap: production hosted-registry install smoke after deploying alpha — not yet a stint task.

**Critical constraint — the Polar AUP wall.** Polar is a merchant of record for **first-party** digital products only; its AUP **bars the marketplace model** (third-party sellers collecting with payouts owed back). Epoch 3 is split: Plexi can sell **its own** apps + the AI Pro subscription on Polar now (`0355`, done); paying outside publishers their 85% needs a different rail (Stripe Connect / etc.) and is deferred to `0352`.

**First-party sales are code-complete, dark behind `SALES_ENABLED`.** `0356` (new, P1) is the single ops runbook to go live: production Polar org, product creation (`ensure-ai-pro` + `set-app`), private Railway artifact bucket, webhook registration, confirm legal live, flip the switch, real smoke purchase+refund. Pure provisioning, no code.

Other open PRs affecting priority reading:

- `#2353` open: toolbar button focus-steal fix (external branch).
- `#2323` draft: WASM SDK v3 platform POCs (`0285`/`0287` lane).
- `#2318` open: stats idle-heartbeat filtering (`0282`).
- `#2316` open: todo app space-to-toggle regression (`0281`).
- `#2282` open: collapsible subcontexts (`0241`), failed build check.
- `#1604` open: Windows port (external branch).

Not real yet: first-party sales live (needs `0356` provisioning), production hosted-registry install smoke after alpha deploy, managed `ai.query` backend (`0323`), the entire third-party publisher economy (`0344`/`0352`/`0353`).

---

## The Arc

Every epoch feeds the next; the whole line points at `NORTH_STAR.md` ("the last app you'll ever need" — a portable, ownable computing environment where the marketplace is how it gets apps and makes money). Tasks are indented under the outcome they serve; nested tasks are blocked by their parent.

### Epoch 1 — Land v1 (now) — effectively done

A stranger installs, an agent builds an app first try, a free reviewed app installs from the hosted registry.

- First-user surface is landed (onboarding, website, registry go-live, palette scroll, app-builder DX, exemplar apps, SDK coverage, hosted Core catalog, native pane-key driving, agent pip color, explorer native media viewers `0349`). Detail in `docs/DEVLOG.md`.
- **Missing tracked gap:** production hosted-registry smoke after deploy from alpha. Create a stint if it can't fold into release verification.

### Epoch 2 — Intelligence (NORTH_STAR Phase 3; runs parallel to Epoch 3)

The host Assistant becomes the workspace operator: typed host tools behind the permission broker, named agent personas, skills, app connectors.

- `0225` agent registry + model routing
- `0226` settings scopes (user/workspace/local/session)
- `0228` conversation persistence + history
- `0229` skills + host tools (pane/app/terminal operations)

### Epoch 3 — Commercial launch (Track B)

The registry brokers money; never a dependency for running installed apps. Spec: `docs/marketplace-monetization.md` (`0315`, done). **Constraint (2026-07-10): Polar's AUP bars the third-party-marketplace model; Polar is first-party MoR only.** The path splits:

**Buy-side foundation — landed (0339/0347/0325 merged this session).**

**First-party monetization — landed (`0355`, #2374 merged).** Polar product seam under Plexi's org + AI Pro wiring + operator CLI; `#2370`'s fixtures replaced with real sandbox-recorded order/subscription shapes (never-mock gap closed). **Remaining to go live:** `0356` (new, P1) — provision Polar org/product-ids/webhook-secret + private Railway bucket, confirm legal live, then flip `SALES_ENABLED` (ops, not a code task).
  - `0322` host account-gated paid downloads — unblocks once `0339` lands
    - `0341` marketplace app + paywall handoff
  - `0354` verify AI Pro subscription gates on *active status*, not row presence
  - `0323` Plexi-managed `ai.query` backend (recurring-revenue surface)

**Third-party publisher economy (deferred until opening the marketplace to outside publishers):**
- `0352` publisher payout rail (Stripe-Connect-vs-etc **decision**) + onboarding + tax — the gate for everything below
  - `0344` publisher submission pipeline + review queue; its third-party Polar product creation is blocked on `0352`
  - `0353` refund/chargeback clawback from publisher balance

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
P1: `0356` (ops: go-live provisioning), `0241` (open PR needs fixing), `0285` (draft PR), `0322` (paid-download host gating), `0344`, `0341`*.
P2 and below: `0354` (subscription active-gating), `0352`, `0353`, `0323`, `0317`, `0295`, `0297`, `0357` (P3, sudo-noise investigation), plus the backlog in `stint list`.
(* = blocked; see the Arc for what unblocks them.)

**Next recommended task:** free v1's last coding gap (`0349`) is merged — the finish line is real pending only a post-deploy registry smoke. The top remaining item is **`0356`**: ops-only, provision Polar production + the private bucket and flip `SALES_ENABLED` — that's the entire distance between merged code and live revenue. For coding work, `0322` (host paid-download gating) + `0354` (subscription active-gating) extend the money path host-side; `0344` (publisher pipeline) is ready but its third-party product creation stays blocked on `0352`.

---

## Key Reference Docs

| Doc | What it covers |
|-----|----------------|
| `NORTH_STAR.md` | The ten commandments, phases, v1 reviewed-native / v2 WASM boundary |
| `docs/DEVLOG.md` | Landed-work history (moved out of this file) |
| `docs/app-framework-marketplace.md` | App framework + marketplace PRM; resolves roadmap conflicts |
| `docs/marketplace-hosted.md` | Hosted registry, paid apps, AI subscription spec |
| `docs/marketplace-monetization.md` | Monetization + anti-fork model; the payout-rail decision (`0352`) records here |
| `docs/package-envelope.md` | App/agent/skill package envelope spec (`0325`) |
| `docs/workspace-env-secrets.md` | Shared resolver contract for secrets |
| `docs/wasm-runtime.md` | WASM runtime spec |
| `sdk/python/SDK_V3.md` | SDK v3 API reference |
| `src/testing/TESTING.md` | Test infra reference |

---

## How To Update This File

Run `/whats-next` at the start of any session -- it re-runs `stint list` + open-PR check and rewrites the Arc + Priority Stack. `/merge-pr` runs the same routine after every merge. Landed-work history goes to `docs/DEVLOG.md` (append a dated entry), never accumulates here. Sprints are retired: do not create sprint files or `sprint:` fields; new work slots into the Arc under the outcome it serves, with `blocked_by` wiring. Always update stint tasks first when new work is discovered.
</content>
</invoke>
