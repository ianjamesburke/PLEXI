# What's Next

> Read this at the start of any Plexi session. It is the single anchor for orientation.
> Skill: `/whats-next` -- re-runs the audit and updates this file. Auto-updated by `/merge-pr` after every merge.

---

## Current State (2026-06-28)

**Sprint s50** ("Unified v1 landing"): 5/12 done. `stint status` reports 24 active todo/in-progress tasks, 46 backlog tasks, and 8 blocked tasks. `stint check` is green.

Open PRs that affect priority reading:

- `#2323` draft: WASM SDK v3 platform POCs (`0285`/`0287` lane).
- `#2314` open: palette scroll reset (`0280`).
- `#2282` open: collapsible subcontexts (`0241`).
- Other open PRs exist, but they are not the v1 marketplace/app-framework spine.

**Source-of-truth correction:** v1 marketplace is **not** hard-gated on CPython-in-WASM. The active PRM says v1 may ship reviewed-native Python apps with blunt trust labels and review. WASM remains the stronger sandbox/performance path and v2 trust upgrade.

---

## Verified State (audited 2026-06-28 against `PLEXI_CHANNEL=alpha plexi-alpha`)

Confirmed by running:

- Unknown capability validation fails closed.
- Clean local app package build/install works after removing generated dev artifacts.
- Pack-file install with git cloning works.
- Core pack install works.
- `plexi update apps` performs real git update work for global git-backed apps.
- Uninstall works.

Verified broken or not real yet:

- Fresh scaffold package/install is broken because `app init` creates `.venv` and package validation rejects `.venv/bin/python3` as a symlink. Owned by `0316`.
- Direct `plexi app install github:owner/repo` routes to the local-path handler and fails with ENOENT. Owned by `0316`.
- Update UX is split between `plexi app update` and `plexi update apps`. Owned by `0316`.
- Git installs require an exact tag/ref; no branch/HEAD fallback. Owned by `0316`.
- Workspace-scoped app update is blind to workspace installs. Owned by `0316`.
- Hosted registry browse/search currently hits a 404 endpoint. Owned by `0321`.
- Shared secrets resolver is not complete across apps, PTYs, `plexi run`, and AI broker. Owned by `0237`.
- Reviewed-native bypass scanning is not implemented. Owned by `0320`.
- License-aware update gating is not implemented. Owned by `0322`.
- Managed `ai.query` backend `"plexi"` is not implemented. Owned by `0323`.
- App/agent/skill package envelope is not specified. Owned by `0325`.

Do not describe secrets, hosted registry, paid updates, managed AI, or package envelopes as shipped.

---

## Path To Commercial Launch

### Track A -- v1: usable, free, shippable

The product a stranger can install, build an app in, and use with a free reviewed app. No money yet.

1. **Correct the plan before more work fans out.** `0319` reconciles the WASM marketplace gate with the PRM and decides how to split or re-scope `0286`.
2. **Authoring is self-documenting.** `0314` redesigns app UI boilerplate, `0313` makes the SDK/scaffold self-documenting, then `0299` rebuilds todo as the canonical demo.
3. **Distribution basics are clean.** `0316` fixes default scaffold packaging, direct GitHub install, update command split, tag fallback, and workspace-aware update.
4. **Trust is honest for reviewed-native v1.** `0320` adds bypass scanning and trust-label behavior for native Python packages. `0285` remains important, but it is not a prerequisite for reviewed-native v1.
5. **Secrets are real.** `0237` routes apps, PTYs, `plexi run`, and AI broker through one workspace/global resolver.
6. **Free hosted install exists.** `0321` stands up the smallest reviewed-native registry smoke path after `0319`, `0316`, and `0320`.
7. **First-user surface exists.** `0272` refreshes `plexiapp.com`; `0324` turns existing AI doctor/setup into a first-run path.

v1 is done when a stranger installs Plexi, an agent builds a working local app from the scaffold, and a reviewed free app installs from the hosted registry without an account.

### Track B -- v1.1: commercial launch

Starts after Track A's local distribution and free hosted install are real. Brokers money; never a dependency for running installed apps.

1. **Commercial model agreed.** `0315` writes the monetization and anti-fork spec. It is spec-only.
2. **Paid update gating exists.** `0322` implements license-aware registry update checks after `0315`, `0316`, and `0321`.
3. **Plexi-managed AI exists.** `0323` adds the opt-in `ai.query` `"plexi"` backend with account entitlements after `0315` and `0237`.
4. **Package envelope is specified.** `0325` defines apps/agents/skills package boundaries before build work assumes they are unified.
5. **WASM sandbox and cloud runtime mature.** `0285` and `0287` are still strategic, but they are the stronger sandbox/cloud lane, not the free reviewed-native v1 gate.

---

## Priority Stack

### P0 -- Ship These First

| Task | Title | Why |
|------|-------|-----|
| 0319 | planning: reconcile WASM marketplace gate with app-framework PRM | Stops future agents from following a roadmap that contradicts the active PRM. |
| 0313 | sdk: self-documenting flow | Agents need in-scaffold orientation before the demo rebuild. |
| 0314 | redesign app UI boilerplate | Blocks `0299`; every generated app starts here. |
| 0299 | rebuild todo app from scratch | Canonical demo; blocked by `0314`. |
| 0316 | app distribution fixes | Fresh scaffold package/install and direct GitHub install are broken. |
| 0280 | palette scroll reset | Visible regression on every palette open; PR `#2314` is already open. |

### P1 -- v1 Core Completeness

| Task | Title | Blocked By | Why |
|------|-------|------------|-----|
| 0320 | reviewed-native bypass scanner and trust labels | -- | Required for honest reviewed-native v1 marketplace. |
| 0237 | workspace env secrets resolver | -- | Needed for external API apps and AI broker key flow. |
| 0321 | free hosted registry smoke path | 0319, 0316, 0320 | First real hosted free install path. |
| 0324 | first-run AI doctor and app-install guidance | 0316 | First-user onboarding path. |
| 0272 | website visual refresh | -- | Shareable public surface. |
| 0241 | collapsible subcontexts | -- | v1 UX primitive; PR `#2282` is open. |
| 0315 | marketplace monetization + anti-fork spec | -- | Gates commercial-launch build tasks, not v1 free install. |
| 0285 | WASM-native Python SDK + CPython-in-WASM | -- | Strategic sandbox/performance lane; no longer described as the v1 marketplace hard gate. |

### P2 -- Important, Not Blocking Free v1

| Task | Title | Notes |
|------|-------|-------|
| 0322 | license-aware registry update checks | Track B; blocked by `0315`, `0316`, `0321`. |
| 0323 | Plexi-managed `ai.query` backend | Track B; blocked by `0315`, `0237`. |
| 0325 | app/agent/skill package envelope spec | Track B; blocked by `0315`, `0319`. |
| 0311 | Cmd+P open app with no active terminal | Real UX bug, but not the main launch spine. |
| 0238 | systematic coverage sprint | Important test debt. |
| 0240 | PlexiInput router + FocusLayer | Blocks `0250`, `0258`, `0259`. |
| 0257 | HostModel state-machine extraction | Architecture debt. |
| 0225-0229 | assistant registry/scopes/persistence/skills | Phase 3 intelligence lane; `0227` is already done. |
| 0295 / 0296 / 0297 | WASM manifest/install/workflow cleanup | Hygiene around the WASM lane. |

### P3 -- Polish / Backlog

Everything else: file explorer backlog (`0007`, `0150`), terminal features (`0247`, `0258`, `0259`, `0246`), UI polish (`0243`, `0245`, `0252`, `0263`), input refactors (`0260`, `0261`, `0264`), infra hygiene (`0265-0270`, `0310`), WASM effects (`0230-0234`), benchmarks (`0215`). Run `stint list` for the full set.

---

## Finding First Users

Gaps before sharing publicly:

1. **Roadmap truth:** `0319` must remove the false WASM hard-gate framing.
2. **One bug-free demo path:** `0314` -> `0313` -> `0299`.
3. **Install flow reliability:** `0316` makes the default scaffold and GitHub/update flows work.
4. **Trust story:** `0320` makes reviewed-native package review honest.
5. **Secrets path:** `0237` makes external-API apps viable.
6. **Website:** `0272` refreshes `plexiapp.com`.
7. **Onboarding:** `0324` turns existing `plexi ai doctor` / `plexi ai setup` into a first-run path.
8. **Free hosted install:** `0321` gives users something real to install from the registry.

**First-user critical path:** 0319 -> 0316 + 0320 + 0313/0314 -> 0299 -> 0237 -> 0272 + 0324 -> 0321 -> share.

---

## Blocked Chains

```
0314 (boilerplate redesign)
  └─ 0299 (todo rebuild -- canonical demo)

0319 (PRM/task graph reconciliation)
0316 (distribution fixes)
0320 (reviewed-native bypass scanner)
  └─ 0321 (free hosted registry smoke path)
       └─ 0322 (license-aware paid update checks)

0315 (commercial monetization spec)
  ├─ 0322 (license-aware paid update checks)
  ├─ 0323 (Plexi-managed ai.query backend)
  └─ 0325 (app/agent/skill package envelope spec)

0237 (shared secrets resolver)
  └─ 0323 (Plexi-managed ai.query backend)

0316 (distribution fixes)
  └─ 0324 (first-run onboarding guidance)

0285 (WASM sandbox/performance lane)
  └─ 0286 (full registry/client/payment epic -- currently still blocked in stint)
       └─ 0287 (cloud streaming runtime)

0240 (PlexiInput router)
  └─ 0250, 0258, 0259
```

---

## Key Reference Docs

| Doc | What it covers |
|-----|----------------|
| `NORTH_STAR.md` | Vision, phases, local-first constraint, v1 reviewed-native / v2 WASM boundary |
| `docs/app-framework-marketplace.md` | App framework + marketplace PRM; resolves roadmap conflicts |
| `docs/marketplace-hosted.md` | Hosted registry, paid apps, AI subscription spec |
| `docs/workspace-env-secrets.md` | Shared resolver contract for secrets |
| `docs/wasm-runtime.md` | WASM runtime spec |
| `docs/assistant-host-app.md` | Assistant app spec |
| `sdk/python/SDK_V3.md` | SDK v3 API reference |
| `src/testing/TESTING.md` | Test infra reference |

---

## How To Update This File

Run `/whats-next` at the start of any session -- it re-runs `stint list` + open-PR check and rewrites the Priority Stack. `/merge-pr` runs the same routine after every merge. Do not hand-edit the Priority Stack unless you are correcting verified roadmap drift from live code/tasks/docs, and always update stint tasks first when new work is discovered.
