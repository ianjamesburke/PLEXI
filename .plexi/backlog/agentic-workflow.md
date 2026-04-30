# Backlog: Agentic Workflow

Items captured 2026-04-29. These are related threads around PLEXI becoming an agentic orchestration layer for AI coding jobs.

---

## PLEXI CLI: system-wide shortcut layer (Raycast for CLI)
Trigger PLEXI commands from anywhere in the OS via a global keybind. Entry point for all agentic workflow shortcuts below — nothing else in this cluster lands without it.

## Ephemeral pane: spawn Claude Code instance for an issue
`plexi work <issue>` opens a non-spatial ephemeral pane (inventory bar) running a Claude Code session scoped to that issue. Pane becomes permanent if the job is promoted. Depends on: PLEXI CLI + pane ADT changes.

## PLEXI notification channel: surface AI questions to user
Agent pushes a notification with context + proposed change + yes/no/edit prompt. User answers via PLEXI notification UI. Required for the agentic PR review loop when the agent is uncertain how to apply a comment.

## Actionable issue classifier
LLM gate that runs before dispatching a Claude Code instance: is this issue actionable? Is it scoped enough for an agent? Only send issues that pass. Log pass/fail decisions to train the gate over time.

## Notification guessing system / personal agent training
Every time the user answers an agent notification (apply this comment? yes/no/edit), log the input + decision. Build a dataset that makes the agent progressively more autonomous for this user's preferences.

## Automated PR review apply loop (PLEXI-native)
After PR creation: wait for configured reviewer bots (gemini-code-assist, claude-code-review), classify comments by severity, auto-apply high-severity ones via a new Claude Code session, re-push, merge. Surface uncertain comments as PLEXI notifications rather than blocking.
