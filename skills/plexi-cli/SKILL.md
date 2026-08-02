---
name: plexi-cli
description: Operate a running Plexi host: panes, apps, contexts, notifications, workspace tools, and agent coordination.
skill_version: "5.0.1"
plexi_version: "0.2.4"
last_verified: "2026-08-01"
---

# Plexi CLI

Start with `plexi --help`, then run `plexi <command> --help` before using a
surface. Help is the reference for arguments and flags in the installed binary.

Run these commands from a Plexi pane when they act on a host. The pane's context
and connection are supplied automatically. For app state and host state, use the
CLI or app SDK; do not inspect Plexi profile files directly.

## Feature map

- **Panes** — create terminals, control their input and focus, inspect them, and
  coordinate work: `plexi pane --help`. Running a command in another pane:
  `pane command <id> "<cmd>" --enter` is the host-confirmed submit for an
  ordinary shell command (exits 0 only once the host confirms it ran);
  `pane send --submit` is the same contract for driving an interactive TUI,
  and `pane new --agent` is the dedicated verb for booting another agent.
- **Slots** — store a small, named pane result that another pane can inspect or
  wait for: `plexi pane slot --help`.
- **Locks** — coordinate host-owned named leases between panes with FIFO grants,
  timeouts, and pane-death cleanup: `plexi lock --help`.
- **Contexts** — create or enter scoped project spaces, including pre-populated
  sub-contexts: `plexi context --help`.
- **Apps** — scaffold, check, test, open, package, install, and inspect apps:
  `plexi app --help`.
- **Notifications** — show scoped information to the person using the host:
  `plexi notify --help`.
- **Workspace tools** — initialize a workspace, run named commands, and manage
  project secrets and routines: `plexi workspace --help`, `plexi run --help`,
  `plexi secret --help`, and `plexi routine --help`.
- **Agents** — install workspace definitions and report or inspect agent state:
  `plexi agent --help`.
- **Configuration and diagnostics** — inspect configuration, AI setup, app
  health, and updates: `plexi config --help`, `plexi ai --help`,
  `plexi doctor --help`, and `plexi update --help`.
- **Marketplace and tool registry** — search and publish apps, manage an account,
  and refresh CLI-tool knowledge: `plexi account --help`, `plexi registry --help`.
- **Notes** — capture, browse, and process scratchpad notes: `plexi note --help`
  and `plexi notes --help`.

The release gate verifies these feature-map entry points:

```
pane
pane slot
lock
context
app
notify
workspace
run
secret
routine
agent
config
ai
doctor
update
account
registry
note
notes
```

## Worked examples

### Initialize, open, and check an app

Create a workspace, scaffold an app, open it in a live pane, then check both the
running pane and the generated app. `app init` prints the app path and pane ID;
this example captures both instead of assuming where the workspace stores apps.

```bash
mkdir hello-workspace
cd hello-workspace
plexi workspace init

INIT_OUTPUT=$(plexi app init hello --open)
printf '%s\n' "$INIT_OUTPUT"
APP_DIR=$(printf '%s\n' "$INIT_OUTPUT" | sed -n "s/^Created app 'hello' at //p")
PANE_ID=$(printf '%s\n' "$INIT_OUTPUT" | sed -n 's/^[[:space:]]*\([0-9][0-9]*\)$/\1/p' | head -1)

plexi pane state "$PANE_ID"
plexi app check "$APP_DIR"
```

### Create a named sub-context with a terminal grid

Create four terminal panes in one tiled sub-context. The context name is the
argument to `context sub`; name the returned pane IDs directly.

```bash
SQUAD=$(plexi context sub release-train --agents 4 --command 'exec zsh' --layout tiled)
CONTEXT_ID=$(printf '%s' "$SQUAD" | jq -r '.context_id')
PLANNER=$(printf '%s' "$SQUAD" | jq -r '.panes[0]')
IMPLEMENTER=$(printf '%s' "$SQUAD" | jq -r '.panes[1]')
REVIEWER=$(printf '%s' "$SQUAD" | jq -r '.panes[2]')
VERIFIER=$(printf '%s' "$SQUAD" | jq -r '.panes[3]')

plexi pane name "$PLANNER" planner
plexi pane name "$IMPLEMENTER" implementer
plexi pane name "$REVIEWER" reviewer
plexi pane name "$VERIFIER" verifier

CONTEXTS=$(plexi context list)
printf '%s' "$CONTEXTS" | jq --argjson id "$CONTEXT_ID" '.[] | select(.context_id == $id)'
plexi pane list --context "$CONTEXT_ID"
```

### Signal and wait for a pane's result

Slots are small, durable, host-managed rendezvous points associated with a pane.
Use one when another pane needs a simple, observable completion signal: publish a
named value, then wait for a value that matches. A wait is level-triggered, so it
also succeeds when the value was written before the wait began.

Slots are not a stream or an app-to-app data channel. Use the event bus for
structured app data and typed pipes for bulk binary data. Delete a slot when its
value is no longer useful.

```bash
WORKER=$(plexi pane new 'exec zsh' --name report-builder --no-focus)
plexi pane slot write result ready --pane-id "$WORKER"
RESULT=$(plexi pane slot wait result "$WORKER" --until '^ready$' --timeout 300)
printf 'worker result: %s\n' "$RESULT"
plexi pane close "$WORKER"
```

### Review and clean stale slot files

Slot values are stored as files in the workspace's channel data, one directory
per pane. They remain while their pane is live. After panes close, review stale
directories first, then remove them through the CLI.

```bash
plexi workspace clean --dry-run
plexi workspace clean
```

### Post and dismiss a scoped notification

Use a notification for information intended for the person using the host, not
for structured agent-to-agent data.

```bash
NOTICE=$(plexi notify --title 'Review ready' --body 'The branch is ready to inspect.' \
  --scope context --timeout 30)
plexi notify dismiss "$NOTICE"
```
