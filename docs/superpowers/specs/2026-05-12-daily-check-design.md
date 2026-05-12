# Daily Check — Design Spec

**Date:** 2026-05-12  
**Status:** Approved, pending implementation plan

---

## Problem

Several maintenance tasks need to happen regularly but have no reliable trigger:

1. Docs drift — README, plexi-cli skill, and webapp docs fall behind as PRs ship
2. Improve backlog — friction patterns surface in conversation logs but never get acted on
3. Issue triage — new issues accumulate without priority labels, blocking the ship skill

Without a system, these accumulate silently until something breaks or the debt is too large to address quickly.

---

## Solution Overview

A `/daily-check` skill that orchestrates three parallel read-only sub-agents and assembles their output into a single triage report. Nothing auto-applies. The user reviews and decides what to act on.

A companion launchd background job checks for docs version drift every 6 hours and spawns a `plexi terminal` pane with `/daily-check` pre-typed when drift is detected — non-intrusive, no interruption.

---

## Component 1: Freshness Tracking File

**Location:** `.claude/docs-freshness.toml` (committed to the PLEXI repo)

```toml
# Last version at which each surface was reviewed and verified current.
# Updated manually after a review pass (prompted by /daily-check report).

[docs]
readme = "3.6.19"           # PLEXI/README.md
plexi_cli_skill = "3.6.19"  # ~/.agents/skills/plexi-cli/SKILL.md
webapp = "3.6.19"           # plexi-webapp/src/pages/docs.astro (placeholder — no real docs yet)

[meta]
improve_last_scanned = "2026-05-12"  # ISO date; logs newer than this are scanned by improve agent
```

Rules:
- `readme`, `plexi_cli_skill`, `webapp` — semver strings matching CHANGELOG section headers
- `improve_last_scanned` — ISO date; the improve agent only reads logs newer than this
- Updated by committing a change to this file after a review pass
- The `/daily-check` skill reads this file at the start of every run; it never writes to it automatically

---

## Component 2: `/daily-check` Skill

**Location:** `.claude/skills/daily-check/SKILL.md`

### Invocation

```
/daily-check
```

No arguments. Always runs all three agents.

### Step 1 — Read freshness state

Read `.claude/docs-freshness.toml`. Get current version from `Cargo.toml` (`grep -m1 "^version"`). If the file doesn't exist yet, treat all `last_verified` values as `"0.0.0"` and `improve_last_scanned` as 30 days ago.

### Step 2 — Spawn three parallel sub-agents

#### Agent A: Docs Freshness

Inputs: `docs-freshness.toml`, `CHANGELOG.md`, current version

Steps:
1. For each tracked doc, extract CHANGELOG entries between `last_verified` and `current`
2. Filter for entries relevant to that surface:
   - README: `feat(`, `ux(`, `fix(`, `docs(readme)`, any user-facing behavior change
   - plexi-cli skill: `feat(terminal)`, `feat(pane)`, `feat(notify)`, `feat(context)`, `feat(workspace)`, `feat(app)`, `fix(` on any CLI-exposed command, any new CLI flag
   - webapp: `docs(`, `feat(` (for now, flag anything — docs page is a stub)
3. For each doc, output one of:
   - `CURRENT` — no relevant changes since last_verified
   - `NEEDS REVIEW` — N relevant changelog entries listed
   - `STALE` — version gap > 10 releases with no review

Output format:
```
## Docs Freshness

README (last reviewed: 3.6.11 → current: 3.6.19): NEEDS REVIEW
  - feat(quick-note): visual polish & keyboard navigation (#1124)
  - feat(navigation): pane focus history (#1119)
  - feat(config): all 6 theme presets + docs link (#1117)
  [... N more]

plexi-cli skill (last reviewed: 3.6.14 → current: 3.6.19): NEEDS REVIEW
  - feat(terminal): new_window layout (#1158)
  [... N more]

webapp (last reviewed: 3.6.19): CURRENT (holding page — no action needed)
```

#### Agent B: Improve Triage

Inputs: `improve_last_scanned` date from `docs-freshness.toml`, `~/Documents/github/daily_log/`

Steps:
1. Find all `YYYY-MM-DD_*.md` log files newer than `improve_last_scanned`
2. Scan for friction signals:
   - Multiple retries or direction changes on the same task
   - Explicit "that's wrong", "don't do that", "stop" corrections
   - Tasks that took >3 back-and-forth turns to resolve
   - Any `/improve` invocations that didn't result in a committed rule change
3. Group by pattern (not by session). A pattern that appeared in 3 sessions is one entry, ranked higher.
4. Output top 5 patterns max, each with: pattern description, frequency, example session date, suggested rule/fix

Output format:
```
## Improve Triage (logs since 2026-05-01)

1. [3 sessions] Agent reads worktree path as relative, fails when CWD ≠ repo root
   Last seen: 2026-05-11. Suggested fix: add lesson to CLAUDE.md — always use absolute paths with git -C.

2. [2 sessions] Skill invoked but not followed — brainstorming skipped on "just a small change"
   Last seen: 2026-05-10. Suggested fix: tighten using-superpowers rule for small-change rationalization.

[... up to 5]
```

If no logs newer than `improve_last_scanned`: output `No new logs since last scan.`

#### Agent C: Issue Triage

Steps:
1. `gh issue list --state open --json number,title,labels --limit 200`
2. Filter for issues with no P0/P1/P2/P3/P4 label
3. Group by type label (bug / enhancement / idea / unlabeled)
4. Sort by issue number ascending (oldest first)

Output format:
```
## Untriaged Issues (no priority label)

bugs (N):
  #1045 — fix(pane): focus not restored after modal close
  #1061 — fix(terminal): cursor blink rate ignores system preference

enhancements (N):
  #1033 — feat(context): auto-name from git remote
  ...

ideas (N):
  ...

unlabeled (N):
  ...
```

### Step 3 — Assemble report

Concatenate the three agent outputs under a single header:

```
# Daily Check — 2026-05-12 (v3.6.19)

[Agent A output]
[Agent B output]
[Agent C output]

---
## Actions

To mark a doc as reviewed, update .claude/docs-freshness.toml and commit:
  readme = "3.6.19"
  plexi_cli_skill = "3.6.19"

To apply an improve pattern: /improve (describe the pattern)
To triage an issue: gh issue edit <N> --add-label "P2"
```

### Step 4 — Update improve_last_scanned

After the report is assembled, prompt the user:

> "Update `improve_last_scanned` to today's date in `docs-freshness.toml`? (Marks these logs as reviewed)"

If yes: edit the file and commit with message `chore: daily-check scan 2026-05-12`.

---

## Component 3: Launchd Drift Watcher

**Purpose:** Detect version drift every 6 hours, spawn a `/daily-check` pane when found.

**Prerequisite:** Depends on `plexi terminal --cwd` flag (issue #1168). The drift watcher should not be activated until #1168 is shipped.

**Files:**
- Script: `~/dotfiles/scripts/plexi-drift-check.sh`
- Plist: `~/dotfiles/launchd/com.plexi.drift-check.plist` (symlinked to `~/Library/LaunchAgents/`)

**Script logic:**

```bash
#!/bin/bash
# Reads docs-freshness.toml, compares to current Cargo.toml version.
# If any doc is behind, spawns a /daily-check pane in Plexi.

PLEXI_REPO="$HOME/Documents/GitHub/PLEXI"
FRESHNESS="$PLEXI_REPO/.claude/docs-freshness.toml"

current=$(grep -m1 '^version' "$PLEXI_REPO/Cargo.toml" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
readme=$(grep 'readme' "$FRESHNESS" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
skill=$(grep 'plexi_cli_skill' "$FRESHNESS" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')

if [ "$current" != "$readme" ] || [ "$current" != "$skill" ]; then
  plexi-alpha terminal --cwd "$PLEXI_REPO" --layout new_window "c '/daily-check'"
fi
```

**Plist:** `StartInterval` = 21600 (6 hours), `RunAtLoad` = false, `WorkingDirectory` = `$HOME`.

**Behavior:**
- Runs silently if docs are current
- Spawns exactly one pane titled `"Daily Check — v{old} → v{current}"` if drift found
- Does not spawn a second pane if one is already open (script checks `plexi pane list` first)

---

## Surfaces Tracked

| Surface | File | Relevant CHANGELOG prefixes |
|---------|------|---------------------------|
| README | `PLEXI/README.md` | `feat(`, `ux(`, `fix(`, `docs(readme)` |
| plexi-cli skill | `~/.agents/skills/plexi-cli/SKILL.md` | `feat(terminal)`, `feat(pane)`, `feat(notify)`, `feat(context)`, `feat(workspace)`, `feat(app)`, `fix(` on CLI commands |
| webapp | `plexi-webapp/src/pages/docs.astro` | Placeholder — flag any `feat(` or `docs(` until real docs land |

Adding a new surface: add an entry to `docs-freshness.toml` and a row to this table.

---

## What This Does Not Do

- Does not auto-apply improve rules to CLAUDE.md or skill files
- Does not auto-label GitHub issues
- Does not update docs itself
- Does not run on a schedule within Claude Code (the launchd watcher is external)
- Does not block or interrupt an in-progress session

---

## Dependencies

| Dependency | Status | Blocks |
|-----------|--------|--------|
| `plexi terminal --cwd` (#1168) | Open | Launchd drift watcher |
| `docs-freshness.toml` initial file | New — created in implementation | Everything |
| `/daily-check` skill | New — created in implementation | Everything |
