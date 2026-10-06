---
name: plexi-cli
description: "Operate a running Plexi host: panes, apps, contexts, notifications, workspace tools, and agent coordination."
skill_version: "5.0.9"
plexi_version: "0.3.5"
last_verified: "2026-10-06"
---

# Plexi CLI

Start with `plexi --help`, then run `plexi <command> --help` before using a
surface. Help is the reference for arguments and flags in the installed binary.

Run these commands from a Plexi pane when they act on a host. The pane's context
and connection are supplied automatically. For app state and host state, use the
CLI or app SDK; do not inspect Plexi profile files directly.

## Feature map

- **Host launch** — `plexi host start --ephemeral --timeout-secs 15` waits for
  application readiness within its startup budget. A timeout returns nonzero;
  the spawned host may still be running, so inspect `plexi host status --json`
  before retrying. Status reports `ready`, `pane_count`, `pid`, and `socket`;
  on Windows the PID comes from the named-pipe owner. Capture process exit and
  stdout/stderr EOF with a deadline when driving CLI commands from a script.

- **Panes** — create terminals, control their input and focus, inspect them, and
  coordinate work: `plexi pane --help`. Running a command in another pane:
  `pane command <id> "<cmd>" --enter` is the host-confirmed submit for an
  ordinary shell command (exits 0 only once the host confirms it ran);
  `pane send --submit` is the same contract for driving an interactive TUI,
  and `pane new --agent` is the dedicated verb for booting another agent.
  `pane state <id>` always returns the agent's own `claimed_state` (its
  `status` slot) together with the host's `observed_state` (process alive,
  output recency, child pids) — never trust the claim alone for liveness;
  add `--stale-after <secs>` to raise a typed `stale_claim` when the claim
  looks fresh but output has gone quiet.
- **Lifecycle events** — `plexi pane wait <id> --until idle|blocked|exited
  --timeout <seconds>` waits for current state or a later matching event. It
  prints the matching JSON record and exits 0; timeout exits 2; permission,
  missing-pane, transport, and argument errors exit 1. `idle` describes agent
  readiness, not task success. `blocked` retains its typed reason. `exited`
  matches any recorded PTY exit, including explicitly unknown status; it does
  not promise a successful exit. A retained exit remains queryable after the
  pane closes, for this host lifetime. `plexi pane events --follow [--pane <id>]`
  streams new lifecycle records as NDJSON, scoped to the caller's context and
  subject to event-stream consent. Stop, StopFailure, and SessionEnd remain
  distinct raw facts. Each invocation owns its subscription; interruption or
  a broken output pipe releases it. Events are not replayed after host restart.
- **Slots** — store a small, named pane result that another pane can inspect or
  wait for: `plexi pane slot --help`.
- **Contexts** — create or enter scoped project spaces, including pre-populated
  sub-contexts: `plexi context --help`.
- **Apps** — scaffold, check, test, open, package, install, and inspect apps:
  `plexi app --help`. `plexi app info <id>` prints the manifest and the tools
  declared in that app's source.
- **App state** — read or replace a file-backed app's state document, so a human
  and an agent can drive the same app: `plexi app state --help`. Only apps that
  declare a `[state]` section are addressable; the path is resolved from the
  manifest and the calling context, never passed in. A running app picks the
  write up on its own event loop.
- **App tools** — call a tool a running app exposes and get its JSON result:
  `plexi app call <app_id> <tool> --input '<json object>'`. It uses the same
  dispatcher as the Assistant, scoped to your pane's context. The app sees
  `pane:<id>` from the host credential or peer; a missing pane is never
  `user`. Identity fields in `--input` are ignored. `--json` prints the
  structured reply (`error_code`, `pending_request_id`). `--pane <id>` addresses
  one live instance when several panes of that app are open. A rejection exits 1.
- **Assistant permission** — list, show, or resolve a pending grant:
  `plexi assistant permission list`, `plexi assistant permission show <id>`,
  `plexi assistant permission resolve <id> --choice once`. `once`, `session`,
  `always`, `deny`, `deny_always`, and `revoke` are the choices. This is the observation
  seam for the desktop permission sheet. `deny` refuses that call only.
  `deny_always` stores a denial.
- **Permissions** — the live permission monitor, the same rows as the Permissions app:

  ```bash
  plexi permissions list --json
  plexi permissions reset <id>
  plexi permissions revoke <id>
  plexi permissions allow <id>
  ```

  Reset clears a stored denial. Revoke removes an allow. Allow turns a denial
  into an allow. From a pane (`PLEXI_PANE_ID`) or with `PLEXI_CALL_CREDENTIAL`,
  reset and allow file a Needs you item and leave the decision unchanged.
  Revoke runs from either caller. Open the app with `plexi app open permissions`.
- **Needs you** — one list of everything waiting on the human:
  `plexi needs-you list --json` and
  `plexi needs-you resolve <id> --approve` or `--deny`. Click approvals, agent questions, and blocked runs share that record.
  Resolving an id resolves it everywhere exactly once. Expired items are
  auto-denied. A repeat resolve returns the existing resolution.
- **MCP servers** — bridge a configured MCP server's tools onto the assistant's
  connector plane. Servers are declared in the channel profile's
  `mcp_servers.toml` and named by id; the host resolves the command, so an app
  can never supply argv. `plexi app open --mcp` is a different thing: it wraps a
  server in a viewer pane and exposes nothing to the assistant.
- **Notifications** — show scoped information to the person using the host:
  `plexi notify --help`.
- **Workspace tools** — initialize a workspace, run named commands, and manage
  project secrets and routines: `plexi workspace --help`, `plexi run --help`,
  `plexi secret --help`, and `plexi routine --help`.
- **Agents** — install workspace definitions and report or inspect agent state:
  `plexi agent --help`. `agent report --event` preserves a provider lifecycle
  event separately from its UI state; `--blocked-reason` supplies a typed reason.
  Read `agent report --help` before using these optional fields.
- **AI ledger** — totals for tokens, cost, runs, and wall time from this
  channel's ledger, grouped by client or kind: `plexi ledger summary --help`.
  The command reads the local ledger file and does not need a running host.
  `assistant send --client` and `--kind` override the tags for one run;
  omitted, the client comes from `[ai] client` and the kind is `output`.
  Send does not require `app open assistant` first: with no pane named, the
  host reuses an Assistant in the context or creates a hidden one. A pane id
  that does not exist still fails.
- **Configuration and diagnostics** — inspect configuration, AI setup, app
  health, and updates: `plexi config --help`, `plexi ai --help`,
  `plexi doctor --help`, and `plexi update --help`.
  Every `config` verb defaults to the workspace config when run in a workspace,
  otherwise the channel-global config; `--global` selects only the latter.
- **Marketplace and tool registry** — search and publish apps, manage an account,
  and refresh CLI-tool knowledge: `plexi account --help`, `plexi registry --help`.
- **Notes** — capture and browse notes across their two storage tiers
  (`~/.plexi/notes/` and `<context-root>/.plexi/notes/`): `plexi note --help` and
  `plexi notes --help`. `note` writes into the tier the working directory belongs
  to; `notes list` also rolls up any tier nested below it plus the global tier.
- **Events** — subscribe to an app's event stream, declare/emit events, and print
  the singleton host MCP config for this pane's scoped credential (exposes event
  tools and `<app_id>__<tool>` for live apps in the pane's workspace):
  `plexi events --help`.

The release gate verifies these feature-map entry points:

```
pane
pane slot
context
app
notify
workspace
run
secret
routine
events
agent
config
ai
ledger
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

### Bridge an MCP server's tools to the assistant

Declare the server in the channel profile's `mcp_servers.toml` (next to
`config.toml`; user-owned, format in `docs/CONFIG.md`), then open the MCP bridge
app with the server ids as trailing arguments. Its tools arrive on the connector
plane as `mcp.<server>.<tool>`, ask-gated and never auto-allowed. The bridge app
must declare the `mcp.client` capability.

```toml
# ~/.plexi-<channel>/mcp_servers.toml
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
```

```bash
plexi app open <mcp-bridge-app-id> -- filesystem
```

### Post and dismiss a scoped notification

Use a notification for information intended for the person using the host, not
for structured agent-to-agent data. A visible notification interrupts unless
the host's notification focus mode is enabled; notifications do not carry a
severity or urgency flag.

```bash
NOTICE=$(plexi notify --title 'Review ready' --body 'The branch is ready to inspect.' \
  --scope context --timeout 30)
plexi notify dismiss "$NOTICE"
```

### Summarize AI ledger usage

Group this channel's ledger by client. `--since` keeps rows at or after a
date. Each group reports runs, input and output tokens, cost, and wall time
(`null` when that field was not recorded). A client or kind that was never
tagged is the null group.

```bash
plexi ledger summary --by client --since 2026-01-01 --json
plexi assistant send --text "hello" --client narrative --kind output
```

## Installation health

Unix installations configure shell completions for new terminal sessions.
Host startup restores a missing shared agent-hook script; use `plexi agent hook
install` to register hooks with an agent explicitly.

`plexi host status --json` distinguishes the invoking CLI, installed package and
running host by build identity. `plexi doctor --json` reports package validation
and identity mismatches, including a running host newer than a rolled-back install.

```bash
plexi update
plexi update --rollback
plexi uninstall --yes
```

Updates preserve the installation channel and custom destination. Rollback restores
the retained previous package; restart the host to use it. Uninstall removes this
channel's recorded package and launchers and retains user data. It preserves other
channels and unrelated development binaries.
