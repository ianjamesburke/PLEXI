---
name: plexi-cli
description: "Operate a running Plexi host: panes, apps, contexts, notifications, workspace tools, and agent coordination."
skill_version: "5.0.12"
plexi_version: "0.3.5"
last_verified: "2026-10-06"
---

# Plexi CLI

Start with `plexi --help`, then run `plexi <command> --help` before using a
surface. Help is the reference for arguments and flags in the installed binary.

Run these commands from a Plexi pane when they act on a host. The pane's context
and connection are supplied automatically. For app state and host state, use the
CLI or app SDK; do not inspect Plexi profile files directly.

## Install

This binary carries the skill. One command writes it where Claude Code and
Codex load a user skill:

```bash
plexi skill install --agent claude
plexi skill install --agent codex
```

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
  one live instance when several panes of that app are open. Without `--pane`,
  two or more instances return `error_code` `ambiguous_instance` and a `panes`
  list; the call does not choose one. `plexi app open --new <app>` opens another
  instance when a later open would otherwise focus the one already up. A
  rejection exits 1.
- **Assistant permission** — list or show a pending grant:
  `plexi assistant permission list`, `plexi assistant permission show <id>`.
  When a tool call returns permission_required, print the pending_request_id
  and wait for the person at the desktop to decide. Do not approve, deny,
  or widen a grant from the terminal. `assistant permission resolve`,
  `needs-you resolve`, and `permissions allow` are refused and do not grant.
- **Needs you** — one list of everything waiting on the human:
  `plexi needs-you list --json`. Click approvals, agent questions, blocked
  runs, and a host integrity item share that record. An integrity item
  appears after a start when the previous host did not exit cleanly or the
  permission profile changed while it was down. Open items survive a host
  restart. The resolve JSON, when a desktop decision lands, includes
  `run_outcome`: `unblocked` when this host process still has the run, and
  `outcome_unknown` when that run was filed by a previous process.
  Integrity, questions, and blocked runs do not mint a grant. Do not edit
  profile files to create or approve an item. Do not resolve an item from
  the terminal.

  one live instance when several panes of that app are open. A rejection exits 1.
- **Assistant send** — submit a turn through the same composer and permission
  sheet as the desktop Assistant: `plexi assistant send --text "..."`. Omit
  `--conversation` to start a new conversation that is not the desktop
  transcript. `--conversation <id>` continues one caller-owned conversation
  (the phone relay uses this). `--desktop` appends to the desktop transcript
  and conflicts with `--conversation`. `--status-for <turn-id>` reads a turn
  that already returned `waiting_for_permission` and does not submit a prompt.
- **Assistant permission** — list, show, or resolve a pending grant:
  `plexi assistant permission list`, `plexi assistant permission show <id>`,
  `plexi assistant permission resolve <id> --choice once`. `once`, `session`,
  `always`, `deny`, and `revoke` are the choices. This is the observation
  seam for the desktop permission sheet.
- **Needs you** — one list of everything waiting on the human:
  `plexi needs-you list --json` and
  `plexi needs-you resolve <id> --approve` or `--deny`. Click approvals, agent questions, and blocked runs share that record.
  Resolving an id resolves it everywhere exactly once. Expired items are
  auto-denied. A repeat resolve returns the existing resolution.
  A paired phone passes `--from-phone` and may approve a question or a blocked run.
  An approval click is irreversible: the phone's approve returns `waiting on desktop` and the desktop sheet stays the grant. Deny is allowed from the phone.
- **MCP servers** — bridge a configured MCP server's tools onto the assistant's
  connector plane. Servers are declared in the channel profile's
  `mcp_servers.toml` and named by id; the host resolves the command, so an app
  can never supply argv. `plexi app open --mcp` is a different thing: it wraps a
  server in a viewer pane and exposes nothing to the assistant.
- **Phone relay** — pair a phone to this desktop: `plexi relay enable --url <wss-or-loopback-ws>`,
  then `plexi relay confirm` after comparing the fingerprint the phone shows.
  `relay pair` adds another phone. `relay revoke <device-id>` drops one phone.
  `relay status` prints the desktop session. `relay connect` attaches to the
  host's socket. `relay disable` stops the host from connecting on startup.
  Message bodies are sealed; the relay stores no plaintext.
- **Notifications** — show scoped information to the person using the host:
  `plexi notify --help`.
- **Workspace tools** — initialize a workspace, run named commands, and manage
  project secrets and routines: `plexi workspace --help`, `plexi run --help`,
  `plexi secret --help`, and `plexi routine --help`.
- **Agents** — install workspace definitions, report or inspect agent state,
  and operate agent heads: `plexi agent --help`. `agent head create` stores a
  named head under `.plexi/agents` with `--grant tool=allow|ask|deny`. `AGENT.md`
  is guidance and grants nothing. `agent run spawn` starts a run; repeat the
  same `--admission` to get that run back, and a second admission while it is
  active returns `assignment_conflict`. `agent run finish` stops it. `agent delegate`
  starts a temporary child with a subset of the parent run's grants. Ask-tier
  calls wait on `plexi assistant permission list`. Do not resolve those
  requests from an agent pane. `assistant send --head <id>` runs one model turn
  in that head's conversation. `assistant open --head <id>` opens an Assistant
  pane bound to that head. `agent conversation --head <id> --as <other>` is
  refused when the ids differ.   `command-view` lists heads, runs, and queued tasks from the agents API.
  `command-view open` shows the same rows in a pane. `command-view send <lead> <text>`
  runs a real turn in that head's conversation. `command-view cancel <run>` stops
  that run before the next tool. `command-view resolve` and `command-view allow`
  do not grant, and both are refused when `PLEXI_PANE_ID` is set. `agent assign --head <id> --input task.json` queues work with no pane open. `agent cancel --id <task>` stops it before the next tool. A lead cannot read, write,
  or message another lead. `agent report --event` preserves
  a provider lifecycle event separately from its UI state; `--blocked-reason`
  supplies a typed reason. Read `agent report --help` before using these optional
  fields.
- **AI ledger** — `plexi ledger` prints per-client totals for tokens, cost,
  runs, and wall time from this channel's ledger. `plexi ledger summary --help`
  groups by client or kind. A token count is a positive number or the word
  `unknown`. The command reads the local ledger file and does not need a
  running host.
  `assistant send --client` and `--kind` override the tags for one run;
  omitted, the client comes from `[ai] client` and the kind is `output`.
  Send does not require `app open assistant` first: with no pane named, the
  host reuses an Assistant in the context or creates a hidden one. A pane id
  that does not exist still fails.
- **Connectors** — connect a registered service with OAuth:
  `plexi connector --help`. The current `stub` connector is a loopback-only
  test issuer. Login prints a credential reference, never an access or refresh
  token; `status` prints that same safe reference; and `revoke` removes the
  local credential even if its remote revoke attempt fails. Mobile connector
  operations explicitly report that they are not yet supported.
- **Agents** — install workspace definitions and report or inspect agent state:
  `plexi agent --help`. `agent report --event` preserves a provider lifecycle
  event separately from its UI state; `--blocked-reason` supplies a typed reason.
  Read `agent report --help` before using these optional fields. When a managed
  Pi hook is installed, Pi's built-in MCP client automatically receives the
  pane-scoped host MCP endpoint, so context-reachable app tools are available as
  Pi MCP tools without configuring a second tool protocol.
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
assistant
assistant send
assistant permission
assistant permission list
assistant permission show
assistant permission resolve
needs-you
needs-you list
needs-you resolve
relay
relay connect
relay confirm
relay revoke
relay pair
relay enable
relay disable
relay status
notify
workspace
run
secret
connector
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

### Bind a secret to a folder

`plexi secret set NAME --folder <path>` reads the value from stdin (or a hidden
prompt) and stores it in the OS keychain, or in the labeled encrypted-file
fallback when Secret Service is unavailable. `list` prints names and folders.
A new pane a person starts inside that folder receives the name as an
environment variable. Same-user native processes are not isolated from each
other: a process running as this user can still read a value a pane already
holds. Folder secrets stop accidental injection into the wrong directory.

From a pane, `secret exec` and `secret grant` return `permission_denied`.
They do not start a command, record an allow, or print the value. `secret
read --agent <id>` returns `permission_required` until a person records an
allow. A pane cannot spawn another pane that receives a folder secret.
`list`, a refusal, and the audit log never print the value.

```bash
plexi secret set FOLDER_TOKEN --folder /path/to/project
plexi secret list
plexi secret read FOLDER_TOKEN --agent reader --folder /path/to/project
plexi secret rm FOLDER_TOKEN --folder /path/to/project
```

### Test a desktop OAuth connector against the local stub issuer

With a loopback stub issuer already running, start sign-in. The issuer redirects
the browser to a one-time loopback callback; pass `--no-browser` to copy the
displayed URL into a browser yourself. The command prints only a credential
reference. Revoke once the test is complete.

```bash
plexi connector login stub --issuer http://127.0.0.1:8765
plexi connector status stub
plexi connector revoke stub
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

### Create an agent head and claim a run

Heads live in the workspace `.plexi/agents` directory. Grants are the permission
gate's authority. The same admission id returns the original run.

```bash
plexi agent head create lead --display-name Lead --description 'Lead agent' --grant agents.ping=allow --grant agents.review=ask --json
plexi agent head list --all --json
plexi agent run spawn --head lead --admission adm-1 --client-ref acme --kind output --input-tokens 11 --output-tokens 4 --json
plexi agent run list --json
plexi agent run show run_example --json
plexi agent delegate --parent-run run_example --name scout --grant agents.ping=allow --json
plexi agent run finish run_example --json
plexi assistant open --head lead
plexi assistant send --head lead --text 'status?'
plexi agent conversation --head lead --json
plexi command-view --json
plexi command-view send lead-a "status?"
plexi command-view cancel run_example
plexi command-view resolve pending_example
plexi command-view allow --tool assistant.turn
plexi agent assign --head lead --input task.json --json
plexi agent cancel --id task_example --json
```

### Summarize AI ledger usage

`plexi ledger` prints this channel's per-client totals. `ledger summary`
groups by client or kind. `--since` keeps rows at or after a date. Each group
reports runs, input and output tokens, cost, and wall time. A token count that
was not measured is the word `unknown`. Wall time is null in JSON when it was
not recorded. A client or kind that was never tagged is the null group.

```bash
plexi ledger
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
