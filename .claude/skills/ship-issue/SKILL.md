---
name: ship-issue
description: "Full pipeline orchestrator. Spawns implement-issue → open-pr → validate-pr → merge-pr as sequential Plexi panes, watching each for exit and reading the issue body Ship Log to determine outcome before dispatching the next phase. Resumes mid-pipeline automatically. Input: issue number."
risk: medium
source: local
date_added: "2026-05-20"
---

# Dispatch Issue

Orchestrates the full ship pipeline for a single issue without manual handoffs.

**Entry:** `/ship-issue <issue-number>`

The skill runs a shell script that:
1. Reads the issue body's `## Ship Log` to determine which phase to resume at
2. Spawns each phase skill in a new Plexi terminal pane
3. Polls `plexi pane list` every 60s until the phase pane exits
4. Reads the Ship Log for the completion marker
5. Dispatches the next phase, or surfaces a failure notification

**Phases dispatched in order:**

| Phase | Skill | Completion marker |
|---|---|---|
| 1 | `/implement-issue <n>` | `[IMPLEMENTED]` |
| 2 | `/open-pr <branch>` | `[PR OPENED]` |
| 3 | `/validate-pr <pr>` | `[VALIDATED]` |
| 4 | `/merge-pr <pr>` | `[COMPLETE]` |

**validate-pr requires human interaction.** The orchestrator polls the Ship Log every 5 minutes and surfaces status updates while waiting. The user tests in the spawned pane; the orchestrator picks up after the pane exits.

**Resume behavior:** if the issue already has Ship Log entries, the orchestrator skips completed phases and starts at the right one. Call `/ship-issue <n>` at any point — it won't re-run phases that already succeeded.

**Hard reject:** if `validate-pr` hard-rejects a PR, the orchestrator fires a `plexi notify`, exits with a non-zero status, and leaves the issue re-labeled `ready`. The issue body contains the `## Prior Attempts` section for the next run.

---

## How to invoke

The skill runs the orchestrator script directly. When invoked, run:

```bash
bash /Users/ianburke/Documents/GitHub/PLEXI/.claude/skills/ship-issue/scripts/run.sh <issue-number>
```

Or from any worktree:
```bash
bash "$(git rev-parse --show-toplevel)/.claude/skills/ship-issue/scripts/run.sh" <issue-number>
```

The script logs each phase transition to stdout and fires a `plexi notify` on completion or failure.

---

## What the orchestrator does NOT do

- Does not make any implementation decisions — each phase skill owns its logic
- Does not retry on hard reject — that requires human review of the issue body
- Does not bypass the human testing gate in validate-pr — it waits for the pane to exit naturally
- Does not handle bundle mode specially — pass the issue number; implement-issue handles bundles internally

---

## Failure modes

**Phase pane exited but marker missing:** the phase skill crashed or the agent hit a context limit. The issue body will still have partial Ship Log entries. Call `/ship-issue <n>` again — it resumes from the last successful marker.

**Branch or PR number not in Ship Log:** the phase skill didn't write its completion entry. Check the issue body and add the missing entry manually, then re-run. Format:
```markdown
**Branch:** feature/<n>-short-description
**PR:** #<pr-number> — <url>
```

**Orchestrator itself crashes:** safe to re-run. It reads Ship Log state fresh on every invocation.
