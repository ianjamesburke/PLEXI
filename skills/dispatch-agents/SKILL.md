---
name: dispatch-agents
description: Use when facing 2+ independent tasks to dispatch as sub-agents. Creates a Plexi layout with orchestrator on top, parallel agents as vertical columns below, and sequential agents stacked within each column.
---

# Dispatch Agents

## Prerequisites

You must be inside a Plexi pane (`$PLEXI_PANE_ID` is set). Use the channel-specific binary matching your socket (e.g. `plexi-beta` if `PLEXI_SOCKET` points to `~/.plexi-beta/`).

## Layout

```
┌──────────────────────────────────────┐
│           Orchestrator (you)          │
├─────────────┬─────────────┬──────────┤
│   Agent 1   │   Agent 2   │  Agent N │  ← parallel
├─────────────┤             │          │
│  Agent 1b   │             │          │  ← sequential (within column)
└─────────────┴─────────────┴──────────┘
```

## Setup

```bash
ORCH=$PLEXI_PANE_ID

# Parallel agents: first splits below orchestrator, rest split right
A1=$(plexi terminal --layout split_v --from-pane-id $ORCH --no-focus)
plexi pane name $A1 "Agent 1: <task>"

A2=$(plexi terminal --layout split_h --from-pane-id $A1 --no-focus)
plexi pane name $A2 "Agent 2: <task>"

A3=$(plexi terminal --layout split_h --from-pane-id $A2 --no-focus)
plexi pane name $A3 "Agent 3: <task>"
```

## Sequential agents (within a column)

Task B depends on A's output — stack below the parallel pane:

```bash
A1B=$(plexi terminal --layout split_v --from-pane-id $A1 --no-focus)
plexi pane name $A1B "Agent 1b: <next task>"
```

## Dispatching (interactive pane)

Always use `c` (not `claude` or `cl`) — it's an alias for `claude --dangerously-skip-permissions`:

```bash
plexi pane send $A1 "c 'your task instructions here'" && plexi pane key $A1 enter
```

## Dispatching (headless)

When you don't need a visible pane — evaluation, verification, one-shot tasks:

```bash
OUTPUT=$(cd /path/to/worktree && claude -p --model <model> --dangerously-skip-permissions "prompt")
```

Returns stdout directly. Use for: model comparisons, automated CI-like checks, verifier agents. No pane needed, no permission prompts, output lands in your context.

## Choosing pane vs headless

| Use case | Method |
|----------|--------|
| Long-running ship-issue cycle | Pane (user watches progress) |
| Quick verify/review of a diff | Headless `-p` |
| Model evaluation (Haiku vs Sonnet) | Headless `-p` |
| Task that needs user interaction | Pane |
| Coder/verifier split | Coder in pane, verifier headless |

## Parallel vs sequential

- **Parallel** — tasks don't share state or files → separate columns
- **Sequential** — task B needs task A's output → same column, stacked below with `split_v`

## Common patterns

**Coder + Verifier split:**
```bash
# Coder works in a pane
CODER=$(plexi terminal --layout split_v --from-pane-id $ORCH --no-focus)
plexi pane name $CODER "Coder: #<issue>"
plexi pane send $CODER "c 'ship <N>'" && plexi pane key $CODER enter

# After coder finishes, verify headlessly
RESULT=$(cd /path/to/worktree && claude -p --model sonnet --dangerously-skip-permissions \
  "Review the diff on this branch against alpha. Check: correctness, missed edge cases, stale references. Report pass/fail with specifics.")
```

**Multi-model evaluation:**
```bash
for MODEL in claude-haiku-4-5-20251001 claude-sonnet-4-6; do
  echo "=== $MODEL ==="
  cd /path/to/frozen-worktree && claude -p --model $MODEL --dangerously-skip-permissions "prompt"
done
```
