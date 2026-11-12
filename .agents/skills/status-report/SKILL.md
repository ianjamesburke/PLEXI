---
name: status-report
description: Owner is at the desk and wants what needs them. Run in the project-manager lead session when Ian types /status-report, "status report" or "sr". Reads the decisions ledger and lead log, replies with state, changes, a decisions table, Ian's hands-on steps; then records his rulings.
---

# Status report

Lead session only. No agents spawned for this.

## Steps
1. Read `.stint/runs/truth-scribe/decisions.md` and the tail of `.stint/runs/truth-scribe/lead-log.md` since the last `status report given` marker. Do not re-derive from scratch.
2. Reply with exactly this, in order, nothing else:
   - (a) One line: overall state.
   - (b) At most five bullets: what changed since last report (merged PRs and stints as number plus title, incidents).
   - (c) Table `# | Decision | Options | My pick`, one row per open decision. Each names its stint or PR number with title. Options in a few words. One pick.
   - (d) "Your hands": numbered steps only Ian can do (tests to run, settings to click), line by line.
   - (e) One line: "Answer with row numbers, e.g. `yes to all`, `yes except 4`, `7: <comment>`."
3. Four or fewer open decisions: also ask via AskUserQuestion, one question per decision, pick first and marked "(Recommended)". Built-in "Other" is how Ian comments or overrides. More than four: table only.
4. On his answer: move the row to Ruled (date plus his words), or rewrite it if he commented. Log the ruling to the lead log. Act on it. Append `status report given <UTC timestamp>` to the lead log.

## Ledger: `.stint/runs/truth-scribe/decisions.md`
- Sections `## Open` and `## Ruled`.
- Open rows: table `# | Decision | Options | My pick | Raised`.
- Row numbers stable for the life of a decision, never reused.
- Lead adds a row the moment something needs Ian, removes it when ruled. Workers never edit it.

## Quiet rule (between reports)
On any turn triggered by an agent result or background event: handle internally, end with at most one short line. No restating pending decisions, no status summary.
Interrupt Ian unprompted only for:
- an incident touching his installs or data
- a security finding that changes what he should do now
- a visible build ready for his eye
Every other new decision goes to the ledger and waits for the next `/status-report`.
