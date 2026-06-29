---
name: dispatch
description: Use when the user wants to run one or more tasks in parallel lanes. Each lane is a separate pane in a sub-context. Lanes can be issue numbers (/implement-issue), stint IDs (/implement-stint), bundles of issues, or any arbitrary prompt. No argument type is required — dispatch is general-purpose.
---

# Dispatch

Open parallel agent lanes inside a sub-context. Each lane self-orchestrates its own work.

## Invocation

```
/dispatch 1671                          # ship GitHub issue #1671
/dispatch 1671 1679                     # two issues in parallel
/dispatch 0171                          # implement stint task 0171
/dispatch "add dark mode to the editor" # arbitrary prompt as a lane
/dispatch 1671 0171 "write release notes"  # mixed — any combination
```

Dispatch is general-purpose. Arguments can be issue numbers, stint IDs, or free-form prompts. Dispatch does not auto-pick; the user always provides the lanes.

---

## Step 1 — Alpha gate (skip for non-code tasks)

If any lane is a GitHub issue or stint task, run:

```bash
git status --porcelain
git log origin/alpha..HEAD --oneline
```

If either has output: print `ALPHA BLOCKED — working tree is dirty or has unpushed commits.` and stop.

Skip this check when all lanes are arbitrary prompts with no code-shipping intent.

---

## Step 2 — Resolve each lane

For each argument, determine its type and the command to run in its window:

| Argument form | Type | Window command |
|---|---|---|
| Numeric (e.g. `1671`) | GitHub issue | `c '/implement-issue 1671'` |
| Zero-padded 4-digit (e.g. `0171`) or `stint-NNNN` | Stint task | `c '/implement-stint 0171'` |
| Multiple issues (e.g. `1671 1679` flagged as bundle) | Bundled PR | `c '/implement-issue 1671 1679'` |
| Anything else | Arbitrary prompt | `c 'the prompt text'` |

**GitHub issue validation** — for numeric args, confirm open and not in-progress:

```bash
gh issue view <N> --json number,title,state,labels \
  --jq '{number, title, state, labels: [.labels[].name]}'
```

If `CLOSED`: stop, tell the user. If `in progress` label: stop, tell the user.

**Stint validation** — for stint args, confirm the task exists and is claimable:

```bash
stint show <id>
```

Skip validation for arbitrary-prompt lanes.

Print what's about to be dispatched:
```
Dispatching:
  #1671  — fix(infra/skills): implement-issue preflight optimization  [implement-issue]
  0171   — refactor render pipeline                                   [implement-stint]
  lane 3 — "write release notes"                                      [prompt]
```

---

## Step 3 — Name the sprint

Pick a short descriptive name for this dispatch batch. Base it on what the lanes have in common — shared area, theme, or feature. Examples: `sdk-frames`, `cli-completions`, `app-protocol`. If nothing ties them together, fall back to `dispatch/arg1+arg2+...`.

---

## Step 4 — Create sub-context and open lanes

Create a sub-context as a child of the current context (portal splits below). All lanes open as windows inside it. The orchestrator pane stays in the parent context, untouched.

```bash
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}

# Build window args — one per lane (or one shared window for a bundle).
WINDOW_ARGS=()
for CMD in <resolved lane commands>; do
  WINDOW_ARGS+=("--window" "$CMD")
done

# Create sub-context with all lanes atomically.
# --parent: child of current context. -d: portal splits below.
# Returns {"context_id": ..., "windows": [...]}.
CTX_RESPONSE=$($PLEXI context new "$SPRINT_NAME" --parent -d --path "$PWD" "${WINDOW_ARGS[@]}")

# Extract context ID for monitoring.
CTX_ID=$(echo "$CTX_RESPONSE" | jq -r '.context_id')
echo "Sprint context: $CTX_ID"
```

**Bundle mode:** if the user explicitly groups multiple issues into one PR, open ONE window: `c '/implement-issue N1 N2 N3'`.

For code-shipping lanes the pipeline self-orchestrates inline:

```
implement-issue / implement-stint → open-pr → validate-pr (notify user, wait) → merge-pr
```

Arbitrary-prompt lanes just run to completion and close.

---

## Step 5 — Monitor lane progress

After lanes open, the orchestrator can check in at any time:

**Agent state** (requires `plexi agent hook install --claude-code` to be set up):
```bash
$PLEXI agent status
# Shows: PANE_ID  AGENT       STATE    SESSION_ID  DETAIL
# STATE is working / blocked / idle. DETAIL is the active tool (e.g. "Edit: main.rs").
```

**Lane pane IDs** (cross-reference against agent status):
```bash
$PLEXI pane list --context $CTX_ID
# Returns JSON array with pane_id, name, type for every pane in the sprint context.
LANE_PANE_IDS=$($PLEXI pane list --context $CTX_ID | jq -r '.[].pane_id')
```

**Read a slot artifact** a lane has written:
```bash
$PLEXI pane slot read status <pane_id>   # e.g. "PR opened: #1234"
$PLEXI pane slot read result <pane_id>   # structured JSON result
$PLEXI pane slot list <pane_id>          # all slots for a pane (JSON)
```

**Lanes write their own slots** using `$PLEXI_PANE_ID` (set in every Plexi terminal):
```bash
$PLEXI pane slot write status "PR opened: #1671"
$PLEXI pane slot write result '{"pr": 1671, "merged": true}'
```

When instructing a lane to report back, include in its prompt: *"Write a `status` slot update at each pipeline milestone using `plexi pane slot write status <message>`."*

---

## Notes

- Dispatch is fire-and-forget after lanes open. You will be notified for validation handoffs, failures, and user-action states; successful merge panes close quietly.
- If a lane crashes mid-pipeline: re-run `/dispatch N` for that argument. implement-issue/implement-stint will detect the in-progress state and ask for takeover confirmation.
- To add a lane to an existing dispatch: `bash .claude/skills/dispatch/scripts/add-to-dispatch.sh <pane_id> <cmd>`
