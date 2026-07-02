# What's Next

> Read this at the start of any Plexi session. It is the single anchor for orientation.
> Skill: `/whats-next` -- re-runs the audit and updates this file. Auto-updated by `/merge-pr` after every merge.
> Landed-work history lives in `docs/DEVLOG.md`; this file stays forward-looking.

---

## Current State (2026-07-01)

**Sprint s50** ("Unified v1 landing"): 5/12 done per `stint status`. `alpha` is merged at:

- `7ddb7c3e` feat: route ai broker through workspace secrets (#2354)

New this session: an inter-pane comms deep-dive filed the **app-dev DX chain** (`0330 -> 0331 -> 0332 -> 0215`) and the comms unification task (`0327`). These are now the top of the priority stack (see below). Full findings are in the task bodies.

Open PRs that affect priority reading:

- `#2353` open: toolbar button focus steal fix from external branch.
- `#2323` draft: WASM SDK v3 platform POCs (`0285` / `0287` lane).
- `#2318` open: stats idle-heartbeat filtering (`0282`).
- `#2316` open: todo app space-to-toggle regression (`0281`).
- `#2314` open: palette scroll reset (`0280`).
- `#2282` open: collapsible subcontexts (`0241`), currently has a failed build check.
- `#1604` open: Windows port from external branch.

Recently landed work and verified-state notes: see `docs/DEVLOG.md` (2026-06-30 entry).

Not real yet:

- License-aware update gating. Owned by `0322`.
- Managed `ai.query` backend `"plexi"`. Owned by `0323`.
- App/agent/skill package envelope. Owned by `0325`.
- First-run onboarding. Owned by `0324`.
- Public website refresh. Owned by `0272`.

---

## Path To Commercial Launch

### Track A -- v1: usable, free, shippable

The product a stranger can install, build an app in, and use with a free reviewed app. No money yet.

1. **App-building loop is exact.** `0326` shipped `plexi app init` -> generated app `AGENTS.md` -> test/render/check/state/action/hot-reload validation against the real host pane. The DX chain (`0330 -> 0331 -> 0332`) now audits, pipelines, and de-drifts that loop end-to-end.
2. **Demo path is rebuilt.** `0313`, `0314`, and `0299` shipped the scaffold and canonical todo demo path.
3. **Distribution basics are clean.** `0316` shipped default scaffold packaging, direct GitHub install, update command unification, tag fallback, and workspace-aware update.
4. **Trust is honest for reviewed-native v1.** `0320` shipped bypass scanning and trust-label behavior for native Python packages.
5. **Secrets are real.** `0237` shipped workspace/global resolver behavior for app-facing secrets, `plexi run`, PTYs, and the AI broker.
6. **Free hosted install exists.** `0321` shipped the smallest reviewed-native registry smoke path.
7. **First-user surface exists.** `0324` turns existing AI doctor/setup into a first-run path. `0272` refreshes `plexiapp.com`.

v1 is done when a stranger installs Plexi, an agent builds a working local app from the scaffold with self-validation that matches the live host, and a reviewed free app installs from the hosted registry without an account.

### Track B -- v1.1: commercial launch

Starts after Track A's local distribution and free hosted install are real. Brokers money; never a dependency for running installed apps.

1. **Commercial model agreed.** `0315` writes the monetization and anti-fork spec.
2. **Paid update gating exists.** `0322` implements license-aware registry update checks after `0315`, `0316`, and `0321`.
3. **Plexi-managed AI exists.** `0323` adds the opt-in `ai.query` `"plexi"` backend with account entitlements after `0315`.
4. **Package envelope is specified.** `0325` defines apps/agents/skills package boundaries before build work assumes they are unified.
5. **WASM sandbox and cloud runtime mature.** `0285` and `0287` are still strategic, but they are the stronger sandbox/cloud lane, not a prerequisite for free reviewed-native v1.

---

## Priority Stack

### P0 -- Ship These First

**App-dev DX chain (dispatch in order; each unblocks the next):**

| Order | Task | Title | Status |
|-------|------|-------|--------|
| 1 | 0330 | audit: app-dev CLI path end-to-end functional audit + open-behavior fix | ready |
| 2 | 0331 | infra: agent-drives-agent E2E pipeline for app-building sessions | blocked by 0330 |
| 3 | 0332 | docs: app-authoring guidance consolidation + drift gates | blocked by 0331 |
| 4 | 0215 | Build a Plexi app-authoring benchmark + case-study directory | blocked by 0330, 0331 |

**Standalone P0:**

| Task | Title | Why |
|------|-------|-----|
| 0327 | refactor: unify inter-pane comms on the event bus | Three-lane comms model (events / binary pipes / PTY); WASM-transition prerequisite. |
| 0324 | onboarding: first-run AI doctor and app-install guidance | Last first-user product gap after install/demo/distribution/secrets. |
| 0280 | fix: palette scroll position persists between opens | Visible regression; PR `#2314` is open but needs fresh validation/merge decision. |

### P1 -- Core Feature Completeness

| Task | Title | Blocked By | Why |
|------|-------|------------|-----|
| 0328 | sdk: UI component coverage audit + placeholder primitives | -- | Polaris-class component vocabulary + `SetShortcuts` host chrome; kills per-app hand-rolling. |
| 0241 | feat: collapsible subcontexts with top-level sidebar numbering | -- | v1 navigation primitive; PR `#2282` exists but currently has a failed build. |
| 0315 | spec: marketplace monetization + anti-fork model | -- | Unblocks commercial-launch tasks `0322`, `0323`, and `0325`. |
| 0285 | wasm: WASM-native Python SDK + CPython-in-WASM runtime | -- | Strategic sandbox/performance lane; draft PR `#2323` exists. |
| 0272 | feat: website visual refresh | -- | Shareable public surface for first users; still backlog, not `stint next` ready. |

### P2 -- Important, Not Blocking Free v1

| Task | Title | Notes |
|------|-------|-------|
| 0311 | fix: Cmd+P open app with no active terminal is silent no-op | Real UX bug and ready. |
| 0317 | feat: LiveNote inline markdown editor for scratchpad | Ready, useful but not launch-critical. |
| 0296 | chore: make core app install set canonical | Ready after prior blockers; app-framework hygiene. |
| 0295 | fix: normalize WASM POC manifests to current schema | WASM lane hygiene. |
| 0297 | chore: remove release-workflow leftovers | Release-flow hygiene. |
| 0238 | testing: systematic coverage sprint | Important test debt. |
| 0240 | refactor: PlexiInput router + FocusLayer | Blocks `0250`, `0258`, `0259`. |
| 0257 | refactor: host state-machine extraction | Architecture debt. |
| 0225-0229 | assistant registry/scopes/persistence/skills | Phase 3 intelligence lane. |

### P3 -- Polish / Backlog

`0298`, `0310`, file explorer backlog (`0007`, `0150`), terminal features (`0247`, `0258`, `0259`, `0246`), UI polish (`0243`, `0245`, `0252`, `0263`), input refactors (`0260`, `0261`, `0264`), infra hygiene (`0265-0270`), and WASM effects (`0230-0234`). Run `stint list` for the full set.

---

## Finding First Users

Gaps before sharing publicly:

1. **Onboarding:** `0324` turns existing `plexi ai doctor` / `plexi ai setup` into a first-run path.
2. **Website:** `0272` refreshes `plexiapp.com`.
3. **Stale pipeline cleanup:** validate or close the old open regression PRs (`#2314`, `#2316`, `#2318`) so the board reflects reality.

**First-user critical path:** 0324 -> 0272 -> share.

**Next recommended task:** claim `0330` (app-dev CLI path audit). It heads the P0 DX chain — everything downstream (agent-drive pipeline, guidance consolidation, benchmark corpus) is sequenced behind it.

---

## Blocked Chains

```
0330 (app-dev CLI path E2E audit)
  └─ 0331 (agent-drives-agent E2E pipeline)
       ├─ 0332 (authoring guidance consolidation + drift gates)
       └─ 0215 (benchmark + case-study directory; also blocked by 0330)

0315 (commercial monetization spec)
  ├─ 0322 (license-aware paid update checks)
  ├─ 0323 (Plexi-managed ai.query backend)
  └─ 0325 (app/agent/skill package envelope spec)

0285 (WASM sandbox/performance lane)
  └─ 0286 (paid/WASM full registry/client/payment epic)
       └─ 0287 (cloud streaming runtime)

0240 (PlexiInput router)
  └─ 0250, 0258, 0259
```

---

## Key Reference Docs

| Doc | What it covers |
|-----|----------------|
| `NORTH_STAR.md` | Vision, phases, local-first constraint, v1 reviewed-native / v2 WASM boundary |
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

Run `/whats-next` at the start of any session -- it re-runs `stint list` + open-PR check and rewrites the Priority Stack. `/merge-pr` runs the same routine after every merge. Landed-work history goes to `docs/DEVLOG.md` (append a dated entry), never accumulates here. Do not hand-edit the Priority Stack unless you are correcting verified roadmap drift from live code/tasks/docs, and always update stint tasks first when new work is discovered.
