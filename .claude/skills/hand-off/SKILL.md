---
name: hand-off
description: Split the current pane, launch a command in the new pane, confirm it started, then close self. Used by the ship pipeline (open-pr hands off to validate-pr) and as a general Plexi pane utility. Requires PLEXI_PANE_ID — must run inside a Plexi pane.
source: local
---

# Hand-Off

Split the current pane, start work in the new pane, confirm it's running, then close self.

## Invocation

```
/hand-off /validate-pr 123   # hands off to validate-pr (most common pipeline use)
/hand-off /some-skill        # any slash command
/hand-off echo "hello"       # any shell command
```

If invoked with no argument, ask: "What should the new pane run?"

---

## Step 1 — Require Plexi context

```bash
: ${PLEXI_PANE_ID:?Must run inside a Plexi pane}
PLEXI=plexi${PLEXI_CHANNEL:+-$PLEXI_CHANNEL}
MY_ID=$PLEXI_PANE_ID
REPO_DIR=$(git rev-parse --show-toplevel 2>/dev/null || echo "$PWD")
```

If `PLEXI_PANE_ID` is unset, stop with: "This skill must run inside a Plexi pane."

---

## Step 2 — Resolve the command

Use the argument verbatim as the command. Derive a short label from it:

```
CMD='/validate-pr 123'
LABEL='validate-pr #123'
```

For a bare integer argument (legacy): treat as `/implement-issue N`.

---

## Step 3 — Split and launch

```bash
NEW_ID=$($PLEXI terminal \
  --layout split_h \
  --from-pane-id $MY_ID \
  --cwd "$REPO_DIR" \
  --no-focus \
  "c \"$CMD\"")
$PLEXI pane name $NEW_ID "$LABEL"
```

---

## Step 4 — Confirm running

Poll pane capture until output is non-empty. 10s timeout at 0.5s intervals.

```bash
for i in $(seq 1 20); do
  OUTPUT=$($PLEXI pane capture $NEW_ID --lines 3 2>/dev/null)
  if [[ -n "$OUTPUT" ]]; then
    echo "Hand-off confirmed: pane $NEW_ID is active."
    break
  fi
  sleep 0.5
done
# Proceed regardless — pane was created and command was sent.
```

---

## Step 5 — Close self

```bash
$PLEXI pane close $MY_ID
```

This is the last command. The current pane closes immediately after.
