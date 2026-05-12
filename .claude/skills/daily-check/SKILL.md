# Daily Check

Triage report for three recurring maintenance concerns: docs freshness, improve
patterns, and untriaged GitHub issues. Everything is read-only — nothing auto-applies.

## Invocation

  /daily-check

No arguments. Always runs all three agents.

---

## Step 1 — Read freshness state

Read `.claude/docs-freshness.toml` in the PLEXI repo root. Extract:
- `docs.readme` — last version README was reviewed
- `docs.plexi_cli_skill` — last version plexi-cli skill was reviewed
- `docs.webapp` — last version webapp docs were reviewed
- `meta.improve_last_scanned` — ISO date of last improve triage

Get current version:
  grep -m1 '^version' Cargo.toml | grep -oE '[0-9]+\.[0-9]+\.[0-9]+'

If `docs-freshness.toml` does not exist, treat all last-reviewed values as `0.0.0`
and `improve_last_scanned` as 30 days ago.

---

## Step 2 — Spawn three parallel sub-agents

Run all three simultaneously. Each returns a formatted section. Combine in Step 3.

--------

### Agent A — Docs Freshness

**Inputs:** `docs-freshness.toml`, `CHANGELOG.md`, current version from `Cargo.toml`

**Steps:**

1. For each tracked surface, extract the CHANGELOG block between `last_reviewed`
   version and `current` version (inclusive of current, exclusive of last_reviewed).
   CHANGELOG format: `## [X.Y.Z] — YYYY-MM-DD` headers, bullet entries below.

2. Filter entries by surface relevance:

   **README** — flag entries matching:
     feat(, ux(, fix(, docs(readme), sec(, chore(config)
   
   **plexi-cli skill** — flag entries matching:
     feat(terminal), feat(pane), feat(notify), feat(context), feat(workspace),
     feat(app), feat(quick-note), fix( on any CLI-exposed command
   
   **webapp** — flag all `feat(` and `docs(` entries until real docs land
     (current docs page is a holding stub — any user-visible change is relevant)

3. For each surface, output one verdict:
   - `CURRENT` — no relevant entries since last_reviewed
   - `NEEDS REVIEW (N changes)` — list the relevant entries
   - `STALE` — version gap > 10 releases with no review

**Output format:**

  ## Docs Freshness

  README (last reviewed: 3.6.11 → current: 3.6.19): NEEDS REVIEW (6 changes)
    - feat(quick-note): visual polish & keyboard navigation (#1124)
    - feat(navigation): pane focus history — Cmd+[ / Cmd+] (#1119)
    - feat(config): all 6 theme presets + docs link (#1117)
    - feat(navigation): remap tab cycle to Cmd+Shift+H/L (#1107)
    - fix(mcp-renderer): reduce handshake timeout, surface server stderr (#1106)
    - feat(quick-note): backend — FocusLayer modal, config routing (#1105)

  plexi-cli skill (last reviewed: 3.6.14 → current: 3.6.19): NEEDS REVIEW (1 change)
    - feat(terminal): new_window layout — parallel dispatch spawns windows (#1158)

  webapp (last reviewed: 3.6.19 → current: 3.6.19): CURRENT
    (holding page — no action needed until real docs land)

--------

### Agent B — Improve Triage

**Inputs:** `meta.improve_last_scanned` from `docs-freshness.toml`, daily log files

**Steps:**

1. Find all `~/Documents/github/daily_log/YYYY-MM-DD_*.md` files with a date
   component newer than `improve_last_scanned`. Skip `.jsonl` files.

2. Scan each file for friction signals:
   - Explicit corrections: "that's wrong", "don't do that", "stop", "no not that"
   - Retries: same tool called 3+ times in a row, or direction reversal on a task
   - Unresolved `/improve` invocations (skill invoked but no CLAUDE.md commit followed)
   - Multi-turn clarification on something that should have been clear

3. Group signals by pattern across all sessions (not by session). A pattern seen in
   3 sessions outranks one seen in 1.

4. Output top 5 patterns max. For each: description, session count, last seen date,
   and a one-line suggested fix (rule for CLAUDE.md or workflow fix for a skill).

If no logs newer than `improve_last_scanned`: output `No new logs since last scan.`

**Output format:**

  ## Improve Triage (logs since 2026-05-01)

  1. [3 sessions] Agent used relative path with git -C, failed when CWD ≠ repo root
     Last seen: 2026-05-11
     Suggested: add lesson to CLAUDE.md — always use absolute paths with git -C

  2. [2 sessions] Brainstorming skill skipped on "just a small change" rationalization
     Last seen: 2026-05-10
     Suggested: tighten using-superpowers rule — "small change" is not an exemption

--------

### Agent C — Issue Triage

**Steps:**

1. Fetch open issues:
     gh issue list --state open --json number,title,labels --limit 200

2. Filter for issues with NO priority label (P0, P1, P2, P3, P4 absent).

3. Group by type label (bug / enhancement / idea). Issues with no type label go
   in a fourth group: unlabeled.

4. Sort each group by issue number ascending (oldest untriad first).

**Output format:**

  ## Untriaged Issues (no priority label)

  bugs (2):
    #1045 — fix(pane): focus not restored after modal close
    #1061 — fix(terminal): cursor blink rate ignores system preference

  enhancements (3):
    #1033 — feat(context): auto-name from git remote
    #1044 — feat(workspace): remember last open page per context
    #1052 — feat(pane): drag-to-reorder panes within a window

  ideas (1):
    #998 — idea: inline diff view for file changes

  unlabeled (0):

---

## Step 3 — Assemble and present report

Print:

  # Daily Check — YYYY-MM-DD (vX.Y.Z)

  [Agent A output]

  [Agent B output]

  [Agent C output]

  ---
  ## What to do next

  Mark a doc as reviewed — edit .claude/docs-freshness.toml and commit:
    readme = "X.Y.Z"
    plexi_cli_skill = "X.Y.Z"

  Apply an improve pattern:
    /improve (describe the pattern)

  Triage an issue:
    gh issue edit <N> --add-label "P2"

---

## Step 4 — Offer to update improve_last_scanned

After presenting the report, ask once:

  "Update `improve_last_scanned` to today in .claude/docs-freshness.toml
  and commit? (marks these logs as scanned)"

If yes: edit the file, set `improve_last_scanned = "YYYY-MM-DD"` (today's date),
commit with message:
  chore: daily-check scan YYYY-MM-DD

---

## Tracked surfaces

| Surface | File | Relevant CHANGELOG prefixes |
|---------|------|---------------------------|
| README | PLEXI/README.md | feat(, ux(, fix(, docs(readme), sec( |
| plexi-cli skill | ~/.agents/skills/plexi-cli/SKILL.md | feat(terminal), feat(pane), feat(notify), feat(context), feat(workspace), feat(app), fix( |
| webapp | plexi-webapp/src/pages/docs.astro | feat(, docs( (placeholder) |

To add a surface: add an entry to `[docs]` in `.claude/docs-freshness.toml`
and a row to this table.

---

## Launchd drift watcher

A background script runs every 6 hours via launchd and spawns a `/daily-check`
pane when docs version drift is detected.

**Status: not yet active — depends on `plexi terminal --cwd` (#1168).**

Once #1168 ships, set up:
  Script:  ~/dotfiles/scripts/plexi-drift-check.sh
  Plist:   ~/dotfiles/launchd/com.plexi.drift-check.plist
  Install: ln -sf ~/dotfiles/launchd/com.plexi.drift-check.plist \
             ~/Library/LaunchAgents/ && launchctl load ~/Library/LaunchAgents/com.plexi.drift-check.plist

Script logic (see docs/superpowers/specs/2026-05-12-daily-check-design.md):
- Read docs-freshness.toml versions vs Cargo.toml current version
- If drift found and no daily-check pane already open:
    plexi-alpha terminal --cwd "$PLEXI_REPO" --layout new_window "c '/daily-check'"
- If no drift: exit silently
