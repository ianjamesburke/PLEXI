# What's Next

> Read this at the start of any Plexi session. It is the single anchor for orientation.
> Skill: `/whats-next` -- re-runs the audit and updates this file. Auto-updated by `/merge-pr` after every merge.

---

## Current State (2026-06-29)

**Sprint s50** ("Unified v1 landing"): 5/12 done per `stint status`. `stint status` reports 18 open active-pool tasks (13 ready todo + 5 blocked todo), 46 backlog tasks, and 0 actually in-progress active tasks. `stint check` is green.

Open PRs that affect priority reading:

- `#2353` open: toolbar button focus steal fix from external branch.
- `#2323` draft: WASM SDK v3 platform POCs (`0285`/`0287` lane).
- `#2314` open: palette scroll reset (`0280`).
- `#2282` open: collapsible subcontexts (`0241`).
- Other open PRs exist, but they are not the v1 marketplace/app-framework spine.

**Source-of-truth correction:** `0319` resolved the sequencing conflict. Free v1 marketplace work proceeds through reviewed-native Python apps with blunt trust labels, human review, and bypass-pattern checks. WASM remains the stronger sandbox/performance path and v2 trust upgrade.

---

## Progress Report (2026-06-29)

Sequential native sub-agent batch landed on `alpha`:

| Task | PR | Result |
|------|----|--------|
| 0313 | #2347 | SDK/scaffold self-documenting flow shipped. |
| 0314 | #2348 | ActionBar scaffold pattern and FooterKeys clipping fix shipped. |
| 0299 | #2349 | Todo rebuilt as the canonical SDK v3 demo app. |
| 0316 | #2350 | Scaffold packaging, direct GitHub/source install, update unification, ref fallback, and workspace-aware update shipped. |
| 0320 | #2351 | Reviewed-native bypass scanner and honest trust labels shipped. |
| 0321 | #2352 | Free hosted reviewed-native registry smoke path shipped. |

Alpha verification after #2352:

- `stint check` -> ok
- `git diff --check` -> clean
- `just check-config-docs`, `just check-cli-docs`, `just check-sdk-docs`, `just check-capability-docs` -> up to date
- `bash tools/check_docs_coverage.sh` -> all CLI commands covered
- `npm run build` in `website` -> passed
- `cargo test --bin plexi` -> 1401 passed, 0 failed, 1 ignored
- `cargo build` -> passed

Clean stopping point: free v1 local/demo/distribution/trust/hosted-registry spine is landed and verified on `alpha`. Remaining free-v1 launch gaps are secrets, onboarding, and public website polish.

New P0 added after live scaffold audit:

- `0326` owns the agent app-building loop: exact live/headless render parity, app-shell viewport contract, JSON state round-trip, channel/profile coherence, scaffolded app-local `AGENTS.md` / `.gitignore` / SDK drift metadata, CLI-baked self-validation instructions, and a three-agent acceptance run where agents build real apps from the product instructions.

---

## Verified State (audited 2026-06-29 against alpha)

Confirmed by running:

- Unknown capability validation fails closed.
- Fresh scaffold validate -> package -> package install works; generated `.venv` artifacts are excluded.
- Direct GitHub/source installs route through the git resolver.
- Pack-file install with git cloning works.
- Core pack install works.
- `plexi app update` / `plexi update apps` use the real git update path and handle workspace installs.
- Git installs fall back to default branch HEAD when a requested ref is missing.
- Reviewed-native package validation flags obvious subprocess/socket/path traversal bypasses.
- Free hosted reviewed-native registry smoke path is live in the website registry fixture.
- Uninstall works.

Verified broken or not real yet:

- Shared secrets resolver is not complete across apps, PTYs, `plexi run`, and AI broker. Owned by `0237`.
- License-aware update gating is not implemented. Owned by `0322`.
- Managed `ai.query` backend `"plexi"` is not implemented. Owned by `0323`.
- App/agent/skill package envelope is not specified. Owned by `0325`.
- The agent app-building/testing loop is not trustworthy enough yet: static `app check` can pass while a live pane shows a different layout, and ambient `PLEXI_CHANNEL` can point app tooling at the wrong profile SDK. Owned by `0326`.

Do not describe secrets, paid updates, managed AI, package envelopes, or app-builder validation parity as shipped.

---

## Path To Commercial Launch

### Track A -- v1: usable, free, shippable

The product a stranger can install, build an app in, and use with a free reviewed app. No money yet.

1. **App-building loop is exact.** `0326` makes `plexi app init` -> generated app `AGENTS.md` -> test/render/check/state/action validation match the real host pane one-to-one.
2. **Demo path is rebuilt.** `0313` shipped the self-documenting SDK/scaffold flow, `0314` shipped ActionBar/footer scaffold quality, and `0299` rebuilt todo as the canonical demo. `0326` is the new gate that proves those renders are faithful to the live host.
3. **Distribution basics are clean.** `0316` shipped default scaffold packaging, direct GitHub install, update command unification, tag fallback, and workspace-aware update.
4. **Trust is honest for reviewed-native v1.** `0320` shipped bypass scanning and trust-label behavior for native Python packages. `0285` remains important, but it is not a prerequisite for reviewed-native v1.
5. **Secrets are real.** `0237` routes apps, PTYs, `plexi run`, and AI broker through one workspace/global resolver.
6. **Free hosted install exists.** `0321` shipped the smallest reviewed-native registry smoke path.
7. **First-user surface exists.** `0272` refreshes `plexiapp.com`; `0324` turns existing AI doctor/setup into a first-run path.

v1 is done when a stranger installs Plexi, an agent builds a working local app from the scaffold with self-validation that matches the live host, and a reviewed free app installs from the hosted registry without an account.

### Track B -- v1.1: commercial launch

Starts after Track A's local distribution and free hosted install are real. Brokers money; never a dependency for running installed apps.

1. **Commercial model agreed.** `0315` writes the monetization and anti-fork spec. It is spec-only.
2. **Paid update gating exists.** `0322` implements license-aware registry update checks after `0315`, `0316`, and `0321`.
3. **Plexi-managed AI exists.** `0323` adds the opt-in `ai.query` `"plexi"` backend with account entitlements after `0315` and `0237`.
4. **Package envelope is specified.** `0325` defines apps/agents/skills package boundaries before build work assumes they are unified.
5. **WASM sandbox and cloud runtime mature.** `0285` and `0287` are still strategic, but they are the stronger sandbox/cloud lane, not a prerequisite for free reviewed-native v1.

---

## Priority Stack

### P0 -- Ship These First

| Task | Title | Why |
|------|-------|-----|
| 0326 | app builder loop: exact render parity, state round-trip, and agent self-validation | Highest priority: agents must be able to build apps and trust `app check` / rendered artifacts as a one-to-one representation of the real host pane. Done requires scaffolded app `AGENTS.md`, `.gitignore`, SDK drift metadata, and three independent agent app-build trials from product instructions only. |
| 0237 | workspace env secrets resolver | External API apps and AI broker key flow need one resolver; promote from backlog before dispatch. |
| 0324 | first-run AI doctor and app-install guidance | First-user onboarding path after install. |
| 0280 | palette scroll reset | Visible regression on every palette open; PR `#2314` is already open. |

### P1 -- v1 Core Completeness

| Task | Title | Blocked By | Why |
|------|-------|------------|-----|
| 0272 | website visual refresh | -- | Shareable public surface. |
| 0241 | collapsible subcontexts | -- | v1 UX primitive; PR `#2282` is open. |
| 0315 | marketplace monetization + anti-fork spec | -- | Gates commercial-launch build tasks, not v1 free install. |
| 0285 | WASM-native Python SDK + CPython-in-WASM | -- | Strategic sandbox/performance lane; not a prerequisite for reviewed-native v1. |

### P2 -- Important, Not Blocking Free v1

| Task | Title | Notes |
|------|-------|-------|
| 0322 | license-aware registry update checks | Track B; blocked by `0315`, `0316`, `0321`. |
| 0323 | Plexi-managed `ai.query` backend | Track B; blocked by `0315`, `0237`. |
| 0325 | app/agent/skill package envelope spec | Track B; blocked by `0315`. |
| 0311 | Cmd+P open app with no active terminal | Real UX bug, but not the main launch spine. |
| 0238 | systematic coverage sprint | Important test debt. |
| 0240 | PlexiInput router + FocusLayer | Blocks `0250`, `0258`, `0259`. |
| 0257 | HostModel state-machine extraction | Architecture debt. |
| 0225-0229 | assistant registry/scopes/persistence/skills | Phase 3 intelligence lane; `0227` is already done. |
| 0295 / 0296 / 0297 | WASM manifest/install/workflow cleanup | Hygiene around the WASM lane. |

### P3 -- Polish / Backlog

Everything else: file explorer backlog (`0007`, `0150`), terminal features (`0247`, `0258`, `0259`, `0246`), UI polish (`0243`, `0245`, `0252`, `0263`), input refactors (`0260`, `0261`, `0264`), infra hygiene (`0265-0270`, `0310`), WASM effects (`0230-0234`), benchmarks (`0215`, now mostly subsumed by `0326`). Run `stint list` for the full set.

---

## Finding First Users

Gaps before sharing publicly:

1. **Agent app-building loop:** `0326` makes scaffold instructions, generated app `AGENTS.md`, render/check artifacts, JSON state, and live host panes agree.
2. **Secrets path:** `0237` makes external-API apps viable.
3. **Onboarding:** `0324` turns existing `plexi ai doctor` / `plexi ai setup` into a first-run path.
4. **Website:** `0272` refreshes `plexiapp.com`.

**First-user critical path:** 0326 -> 0237 + 0324 + 0272 -> share.

**Next recommended task:** `0326` app builder loop. It is ready, P0, and should land before more generated-app or marketplace-demo work.

---

## Blocked Chains

```
0315 (commercial monetization spec)
  ├─ 0322 (license-aware paid update checks)
  ├─ 0323 (Plexi-managed ai.query backend)
  └─ 0325 (app/agent/skill package envelope spec)

0237 (shared secrets resolver)
  └─ 0323 (Plexi-managed ai.query backend)

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
