# Plexi v1.0 Roadmap

> **V1.0 = Phase 0 polished.** No AI, no agent infrastructure, no intelligence wiring. A tiling, terminal-native personal computing environment that a non-technical person can install, use, and build apps for. AI features move to v2.0.

**39 open issues.** Organized into 6 execution phases, ordered by dependency and impact.

---

## Phase 1: Critical Fixes (P0 + P1)

Ship-blocking bugs. Nothing else starts until these are green.

| # | Title | Load | Status |
|---|-------|------|--------|
| **1546** | `plexi -h` crashes; no-args spawns app with no escape hatch; need `plexi kill` | S | ready |
| **1545** | `plexi open config.toml` does nothing on machines without VS Code | S | ready |
| **1547** | Pane focus misbehaves on close and fullscreen toggle | M | ready |
| **1601** | Keep every text-owning overlay focused after CentralPanel renders | M | blocked (architectural) |
| **1599** | Defer overlay-unsafe app command side effects while modals own input | M | blocked by #1601 |

**Dependency chain:** #1601 (centralized focus-owner mapping) unblocks #1599 (defer side effects) and #1600 (Phase 3).

**Effort:** ~1 week focused work. P0s are small. #1547 and #1601 are the real work.

---

## Phase 2: Install Story

First impression for new users. If install is confusing, nothing else matters.

| # | Title | Load |
|---|-------|------|
| **1652** | Install popup should label the command "Run this in the terminal" | S (bundle) |
| **1643** | Install modal: "Check for success" button that auto-detects shell config | M |
| **1642** | install.sh final message should say "open Plexi", not "restart your terminal" | S (bundle) |
| **1550** | Add guardrails to prevent worktree/build-dir path errors in install.sh | S |

**Effort:** ~2-3 days. #1643 is the only non-trivial one.

---

## Phase 3: Core UX Bugs

Overlay, sidebar, config, and navigation bugs that make the product feel broken. Parallelizable after Phase 1 unblocks #1600.

### Overlays & Focus
| # | Title | Load |
|---|-------|------|
| **1600** | Move remaining keyboard-owning overlays into FocusLayer path | M (blocked by #1601) |
| **1653** | Keyboard shortcuts quick-help has incorrect entries | S (bundle) |
| **1626** | QuickNote blocked when another modal is open | S |
| **1639** | QuickNote missing shortcut hint for changing destination | S (bundle) |
| **1641** | Update notification should say "run plexi update" | S (bundle) |
| **1625** | Context inspector should pre-select active context on open | S |
| **1627** | Context inspector UI audit: OSC status, close button, title, set-root-path | M |

### Sidebar & Chrome
| # | Title | Load |
|---|-------|------|
| **1651** | Minimap window order sometimes reversed | S |
| **1633** | Sidebar item heights inconsistent | S |

### Config
| # | Title | Load |
|---|-------|------|
| **1640** | Duplicate clipboard destination in default config.toml | S (bundle) |
| **1587** | Alpha config stays default, beta as staging, add TOML migration | M |

### Terminal & Navigation
| # | Title | Load |
|---|-------|------|
| **1629** | Cmd+A full-pane selection + cross-pane selection persistence | M |
| **1635** | Auto-dismiss notification when originating pane is focused | S |
| **1549** | Strip trailing punctuation from rendered URLs | S |
| **1548** | QuickNote destination pane dies; should open new window | S |
| **1603** | Pane-spawning commands should split from PLEXI_PANE_ID | M |

**Effort:** ~2 weeks. Many S-load bundles can be batched. #1600, #1627, #1587, #1629, #1603 are the heavier items.

---

## Phase 4: Default Apps Polish

Every app that ships by default must work correctly and look good.

| # | Title | Load |
|---|-------|------|
| **1630** | Several default apps missing manifest.toml, skipped on load | S |
| **1659** | Bluesky: avatar images never load; post-fetch row stutter | S |
| **1654** | Bluesky: Esc key does not close the app | S (bundle) |
| **1646** | Bluesky: UI overhaul, thumbnails, footer, responsive layout | M |
| **1648** | Logs: UI spacing, uniform badge sizes, table padding | S |
| **1631** | Logs: Esc key does not close the app | S (bundle) |
| **1649** | Logs: add search and filter support | M |
| **1632** | Logs: copy mode, mouse selection, shift-click multi-line | M |
| **1650** | Backlog: use channel-aware path for backlog directory | S (bundle) |

**Effort:** ~1.5 weeks. #1630 first (apps must load), then Bluesky and Logs in parallel.

**Decision needed:** Audit the full default app list. Any app that isn't polished enough for v1 should be removed from defaults rather than fixed. POC apps belong in `examples/`, not in the install.

---

## Phase 5: SDK Stability

The SDK is the app-building surface. Layout and selection bugs undermine the "build apps for it" pitch.

| # | Title | Load |
|---|-------|------|
| **1527** | SDK layout fundamentals: headline alignment, character padding in boxes | M |
| **1645** | Text selection in PGAP apps + appbar/footer typography polish | M |

**Effort:** ~3-4 days. These touch the render pipeline so need careful testing.

---

## Phase 6: Welcome Screen & CLI Onboarding

The first thing a new user sees. Must guide them toward success.

| # | Title | Load |
|---|-------|------|
| **1575** | Redesign welcome screen | M (unblocked, was waiting on #1576 which is now closed) |
| **1660** | `plexi app dev` + `plexi app publish` commands | M |
| **1644** | Audit `plexi list` vs `plexi app list` overlap, clarify namespace | M |

**Effort:** ~1 week. Welcome screen is demo-critical. `app dev` enables the "build apps" story. `app publish` can be stubbed or deferred to v2 if needed.

---

## Bundle Batch

9 issues marked `bundle` (micro-changes verifiable by diff alone). Batch into 1-2 PRs at any point:

#1654, #1653, #1652, #1650, #1648 (if trivial), #1642, #1641, #1640, #1639, #1631

---

## Summary

| Phase | Issues | Est. Effort | Parallelizable |
|-------|--------|-------------|----------------|
| 1. Critical Fixes | 5 | ~1 week | No (dependency chain) |
| 2. Install Story | 4 | ~2-3 days | Yes (after Phase 1 P0s) |
| 3. Core UX Bugs | 16 | ~2 weeks | Yes (mostly independent) |
| 4. Default Apps | 9 | ~1.5 weeks | Yes (app-by-app) |
| 5. SDK Stability | 2 | ~3-4 days | Yes |
| 6. Welcome & CLI | 3 | ~1 week | Partially |
| **Total** | **39** | **~6-7 weeks** | |

**Execution order:** Phase 1 first (unblocks Phase 3). Phases 2, 4, 5 can run in parallel with Phase 3. Phase 6 last (benefits from all prior polish). Bundle batch anytime.

---

## What's NOT in v1.0

- **AI/Agent features (v2.0, 26 issues):** Agent panes, AiQuery, Crew, agent registry, PGAP skills, autonomous dispatch, agent manifests. Plus enablers like app launch args, theme namespace, routines CLI.
- **Future (70 issues):** Video/media, WASM, app marketplace, advanced permissions, navigation rework, website redesign, platform expansion, speculative ideas.

Query shortcuts:
```bash
# V1.0 remaining
gh issue list --label v1.0 --state open

# V2.0 scope
gh issue list --label v2.0 --state open

# Future
gh issue list --label future --state open

# V1.0 bundles (batch these)
gh issue list --label v1.0 --label bundle --state open
```
