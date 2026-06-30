# What's Next

> Read this at the start of any Plexi session. It is the single anchor for orientation.
> Skill: `/whats-next` -- re-runs the audit and updates this file. Auto-updated by `/merge-pr` after every merge.

---

## Current State (2026-06-30)

**Sprint s50** ("Unified v1 landing"): 5/12 done per `stint status`. `stint status` reports 17 active tasks, 45 backlog tasks, and 5 blocked tasks. `alpha` is merged at:

- `7ddb7c3e` feat: route ai broker through workspace secrets (#2354)

Just merged:

- `#2354` / `0237`: workspace env secrets resolver. PR build artifacts and the feature worktree were cleaned up by `just merge-pr 2354`.

Open PRs that affect priority reading:

- `#2353` open: toolbar button focus steal fix from external branch.
- `#2323` draft: WASM SDK v3 platform POCs (`0285` / `0287` lane).
- `#2318` open: stats idle-heartbeat filtering (`0282`).
- `#2316` open: todo app space-to-toggle regression (`0281`).
- `#2314` open: palette scroll reset (`0280`).
- `#2282` open: collapsible subcontexts (`0241`), currently has a failed build check.
- `#1604` open: Windows port from external branch.

**Source-of-truth correction:** `0319` resolved the sequencing conflict. Free v1 marketplace work proceeds through reviewed-native Python apps with blunt trust labels, human review, and bypass-pattern checks. WASM remains the stronger sandbox/performance path and v2 trust upgrade.

---

## Recently Landed

Free v1 local/demo/distribution/trust/hosted-registry spine is landed on `alpha`:

| Task | PR | Result |
|------|----|--------|
| 0313 | #2347 | SDK/scaffold self-documenting flow shipped. |
| 0314 | #2348 | ActionBar scaffold pattern and FooterKeys clipping fix shipped. |
| 0299 | #2349 | Todo rebuilt as the canonical SDK v3 demo app. |
| 0316 | #2350 | Scaffold packaging, direct GitHub/source install, update unification, ref fallback, and workspace-aware update shipped. |
| 0320 | #2351 | Reviewed-native bypass scanner and honest trust labels shipped. |
| 0321 | #2352 | Free hosted reviewed-native registry smoke path shipped. |
| 0237 | #2354 | Workspace/global secrets now flow through command runs, PTYs, and the OpenRouter broker. |

App-builder hardening also landed on `alpha`:

- `0326` shipped scaffold-local `AGENTS.md`, `.gitignore`, drift metadata, fixtures, semantic ActionBar/FooterKeys boilerplate, host probes, headless check/render coverage, and hot-reload guidance.
- SDK semantic chrome shipped in `src/render/app_chrome.rs`; app init/check now exercise host-native semantic components.
- `plexi app check` gates current scaffolds on semantic proof components and seeded render/action probes.

---

## Verified State

Confirmed by recent validation runs:

- Fresh scaffold validate -> package -> package install works; generated `.venv` artifacts are excluded.
- Direct GitHub/source installs route through the git resolver.
- Pack-file install with git cloning works.
- Core pack install works.
- `plexi app update` / `plexi update apps` use the real git update path and handle workspace installs.
- Reviewed-native package validation flags obvious subprocess/socket/path traversal bypasses.
- Free hosted reviewed-native registry smoke path is live in the website registry fixture.
- Agent app-building loop is trustworthy enough for v1: generated app instructions, drift metadata, headless check/render, JSON seed state, real host state/action/key probes, and same-pane hot reload were verified by three sequential app-build trials.
- Workspace secrets resolver now works for command-run, OpenRouter broker lookup, and GUI terminal panes after zsh startup overwrites.

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

1. **App-building loop is exact.** `0326` shipped `plexi app init` -> generated app `AGENTS.md` -> test/render/check/state/action/hot-reload validation against the real host pane.
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

| Task | Title | Why |
|------|-------|-----|
| 0324 | onboarding: first-run AI doctor and app-install guidance | Last first-user product gap after install/demo/distribution/secrets. |
| 0280 | fix: palette scroll position persists between opens | Visible regression; PR `#2314` is open but needs fresh validation/merge decision. |

### P1 -- Core Feature Completeness

| Task | Title | Blocked By | Why |
|------|-------|------------|-----|
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

`0298`, `0310`, file explorer backlog (`0007`, `0150`), terminal features (`0247`, `0258`, `0259`, `0246`), UI polish (`0243`, `0245`, `0252`, `0263`), input refactors (`0260`, `0261`, `0264`), infra hygiene (`0265-0270`), WASM effects (`0230-0234`), and benchmarks (`0215`). Run `stint list` for the full set.

---

## Finding First Users

Gaps before sharing publicly:

1. **Onboarding:** `0324` turns existing `plexi ai doctor` / `plexi ai setup` into a first-run path.
2. **Website:** `0272` refreshes `plexiapp.com`.
3. **Stale pipeline cleanup:** validate or close the old open regression PRs (`#2314`, `#2316`, `#2318`) so the board reflects reality.

**First-user critical path:** 0324 -> 0272 -> share.

**Next recommended task:** claim `0324` first-run onboarding. It is ready, user-facing, and now the highest-leverage first-user gap after `0237` landed.

---

## Blocked Chains

```
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
