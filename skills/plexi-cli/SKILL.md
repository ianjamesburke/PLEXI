---
name: plexi-cli
description: "Operate a running Plexi host: panes, apps, contexts, notifications, workspace tools, and agent coordination."
skill_version: "5.0.18"
plexi_version: "0.4.0"
last_verified: "2026-10-19"
---

# Plexi CLI

Start with `plexi --help`, then run `plexi <command> --help` before using a
surface. Help is the reference for arguments and flags in the installed binary.
When the target channel is known, use its `plexi-<channel>` shim for every
command below (for example, `plexi-alpha agent --help`). The bare `plexi`
command inherits the calling pane's routing; it may target a different host.

Run these commands from a Plexi pane when they act on a host. The pane's context
and connection are supplied automatically. For app state and host state, use the
CLI or app SDK; do not inspect Plexi profile files directly.

## Install

This binary carries the skill. One command writes it where Claude Code,
Codex, and Pi load a user skill:

```bash
plexi skill install --agent claude
plexi skill install --agent codex
plexi skill install --agent pi
```

## Feature map

- **Host launch** — `plexi host start --ephemeral --timeout-secs 15` waits for
  application readiness within its startup budget. A timeout returns nonzero;
  the spawned host may still be running, so inspect `plexi host status --json`
  before retrying. Status reports `ready`, `pane_count`, `pid`, and `socket`;
  on Windows the PID comes from the named-pipe owner. Capture process exit and
  stdout/stderr EOF with a deadline when driving CLI commands from a script.

- **Panes** — create terminals, control their input and focus, inspect them, and
  coordinate work: `plexi pane --help`. Driving an agent in another pane uses a
  two-command host-confirmed recipe (two commands, no sleeps, no capture polling,
  no manual `pane key <id> enter`):
  1. Boot with `ID=$(plexi pane new --agent <tier-alias> -n "<name>" --no-focus)` —
     the supported way to launch an agent into a fresh pane; blocks until the
     agent TUI is idle, so a printed id on stdout means ready-to-brief.
  2. Brief with `plexi pane send "$ID" "<brief>" --submit` — settles the pane,
     presses Enter (re-sending once if parked as a collapsed paste), and exits 0
     only once the host confirms the prompt left the input line.
  Running an ordinary shell command in another pane:
  `pane command <id> "<cmd>" --enter` is the same host-confirmed submit for a
  shell prompt (exits 0 only once the host confirms it ran).
  `pane state <id>` always returns the agent's own `claimed_state` (its
  `status` slot) together with the host's `observed_state` (process alive,
  output recency, child pids) — never trust the claim alone for liveness;
  add `--stale-after <secs>` to raise a typed `stale_claim` when the claim
  looks fresh but output has gone quiet. Pass `--worktree <branch>` to
  `plexi pane new` to create or attach an isolated git worktree via `wtp` and
  start the terminal pane inside it; closing the pane automatically reclaims the
  worktree when clean and unmoved, or retains it and prints
  `retained worktree: <path> (dirty|ahead)` when uncommitted changes or commits
  are present.
- **Lifecycle events** — `plexi pane wait <id> --until idle|blocked|exited
  --timeout <seconds>` waits for current state or a later matching event. It
  prints the matching JSON record and exits 0; timeout exits 2; permission,
  missing-pane, transport, and argument errors exit 1. `idle` describes agent
  readiness, not task success. `blocked` retains its typed reason. `exited`
  matches any recorded PTY exit (`status` is `{"code": <i32>}`, `{"signal": <i32>}`,
  or `"unknown"` when unavailable); it does not promise a successful exit. A retained exit remains queryable after the
  pane closes, for this host lifetime. `plexi pane events --follow [--pane <id>]`
  streams new lifecycle records as NDJSON, scoped to the caller's context and
  subject to event-stream consent. Stop, StopFailure, and SessionEnd remain
  distinct raw facts. Each invocation owns its subscription; interruption or
  a broken output pipe releases it. Events are not replayed after host restart.
- **Slots** — store a small, named pane result that another pane can inspect or
  wait for: `plexi pane slot --help`.
- **Structured pane results** — declare a JSON Schema at spawn (`plexi pane new --result-schema <json-or-path>`)
  or brief time (`plexi pane result schema <json-or-path> [--pane-id <id>]`).
  The worker writes its structured JSON object once with `plexi pane result write <json>`:
  the host validates it against the declared schema synchronously and rejects
  malformed or schema-violating payloads back to the agent (`exit 1` with the
  validation error on stderr) so the agent can self-correct before any consumer
  sees it. Consumers read the validated object with `plexi pane result <id> [--wait --timeout <secs>]`
  (`exit 0` with JSON on stdout, `exit 2` with `no_result_yet` or `timeout` when
  not yet written, `exit 1` with `invalid_result` when the last write was
  rejected).
- **Mailbox** — send and receive structured inter-agent messages on a per-pane
  FIFO queue without clobbering: `plexi pane message send <id> "<payload>" [--submit]`
  stamps `from_pane_id` host-side from socket peer ancestry, returns `queued` or
  `delivered` ack state, and with `--submit` wakes an idle target agent without
  interrupting a busy one. `plexi pane message recv --timeout 300`
  drains the caller's mailbox as a JSON array and blocks host-side until a message arrives
  with zero polling (exits 0 on messages, 2 on timeout, 1 on error).
- **Contexts** — create or enter scoped project spaces, including pre-populated
  sub-contexts: `plexi context --help`.
- **Apps** — scaffold, adopt, check, test, open, package, install, and inspect apps:
  `plexi app --help`. `plexi app init --adopt <dir>` writes or refreshes
  `plexi.scaffold.toml` for an existing app directory without altering source
  files so `plexi app check` can enforce version, template, and channel drift.
  `plexi app info <id>` prints the manifest and populates
  its tools field from the live tool registry when a pane of `<id>` is running
  in the context, or explicitly labels its static `@tools.tool` source scan
  `manifest-only` when not running.
- **App state** — read or replace a file-backed app's state document, so a human
  and an agent can drive the same app: `plexi app state --help`. Only apps that
  declare a `[state]` section are addressable; the path is resolved from the
  manifest and the calling context, never passed in. A running app picks the
  write up on its own event loop.
- **App tools** — inspect or call the tools running apps expose in a context:
  `plexi app tools [--app <id>] [--context <id>] [--json]` snapshots the live
  tool registry over the command socket (qualified tool name, owning `app_id`
  and `pane_id`, description, and full JSON input schema). Call a live tool and
  get its JSON result with `plexi app call <app_id> <tool> --input '<json object>'`.
  It uses the same dispatcher as the Assistant, scoped to your pane's context.
  The app sees `pane:<id>` from the host credential or peer; a missing pane is
  never `user`. Identity fields in `--input` are ignored. `--json` prints the
  structured reply (`error_code`, `pending_request_id`). `--pane <id>` addresses
  one live instance when several panes of that app are open. A rejection exits 1.
  Add `--wait-approved` to wait up to two minutes for a real desktop decision,
  then execute that exact request once. Denial, timeout or caller exit cancels
  the wait; do not retry a waiting call in parallel. Without the flag, an ask
  returns `permission_required` immediately. "Allow for this session" permits
  different arguments only for the same caller, tool, app instance, workspace,
  context and resource until Plexi quits; the person can revoke it in Permissions.
<!-- experiment: assistant -->
- **Assistant send** — submit a turn through the same composer and permission
  sheet as the desktop Assistant: `plexi assistant send --text "..."`. Omit
  `--conversation` to start a new conversation that is not the desktop
  transcript. `--conversation <id>` continues one caller-owned conversation
  (the phone relay uses this). `--desktop` appends to the desktop transcript
  and conflicts with `--conversation`. `--status-for <turn-id>` reads a turn
  that already returned `waiting_for_permission` and does not submit a prompt.
  A phone turn has no approval, grant, or permission field.
- **Assistant permission** — list or show a pending grant:
  `plexi assistant permission list`, `plexi assistant permission show <id>`.
  When a tool call returns permission_required, print the pending_request_id
  and wait for the person at the desktop. Do not approve, deny, or widen a
  grant from the terminal. `assistant permission resolve` is refused and does
  not grant. `needs-you resolve --approve` is refused for every kind;
  `--deny` still denies.
<!-- /experiment -->
- **Permissions** — the live permission monitor, the same rows as the Permissions app:

  ```bash
  plexi permissions list --json
  plexi permissions reset <id>
  plexi permissions revoke <id>
  plexi permissions allow <id>  # agents do not grant
  ```

  Reset clears a stored denial. Revoke removes an allow. Allow turns a denial
  into an allow for a human terminal. From a pane, a child of that pane, or
  with `PLEXI_CALL_CREDENTIAL`, reset and allow file a Needs you item and do
  not grant. Revoke runs from either caller. Open the app with
  `plexi app open permissions`. Editing `grants.toml` or
  `permission-audit.jsonl` does not grant a permission. The host ignores a
  file whose signature does not match, asks again, and files Needs you. A
  declared non-sensitive capability is auto-granted as one of these rows. A
  sensitive capability is absent from the list until a person grants it.
  `permissions.toml` is not a second store: deleting it leaves the list and the
  gate unchanged. `plexi secret get` cannot read `plexi:host:*`; a workspace id
  of `host` is reserved.
- **Needs you** — `plexi needs-you list --json` prints everything waiting on
  the human; `plexi needs-you show <id> [--json]` inspects one item (open or
  settled) with its structured `details`, `answer`, and linked `continuation`.
  Click approvals, agent questions, blocked runs, permission-widen requests,
  integrity alerts, and keychain prompts share that record. `plexi needs-you ask`
  files a structured `question` or `blocked_run` item (`--choice`, `--allow-free-text`,
  `--preview-ref`, `--correlation-id`) and optionally binds a durable continuation
  record (`--operation-id`, `--session-id`, `--expected-revision`, `--resume-condition`)
  sealed under `<profile>/host/continuations.json`. `plexi needs-you answer <id> --answer <text>`
  settles a question or blocked run without minting a grant; unauthenticated agent
  panes cannot forge an answer (`permission_denied`). `plexi needs-you reconcile --json`
  returns durable continuation records paired with their `NeedsYouRecord` after a
  session or host restart, and `plexi needs-you complete <operation_id> --receipt <json>`
  marks a continuation completed. Expired items are auto-denied. Open rows are
  sealed under the profile `host` directory and listed again after a restart. From
  the terminal, command socket, or phone, `needs-you resolve --approve` is refused
  (permission_denied) for every kind: the row stays open and a refusal is recorded.
  Only a click in the host window approves (Allow once on the approval banner for
  click approvals; Approve on the sidebar Needs-you row or in Needs attention for
  the rest, including Keychain recovery). `--deny` from the terminal or phone settles
  questions, blocked runs, click approvals, and permission-widen requests; it
  is refused for integrity and keychain items. A phone does not approve or
  answer questions.
<!-- experiment: mcp_client -->
- **MCP servers** — bridge a configured MCP server's tools onto the assistant's
  connector plane. Servers are declared in the channel profile's
  `mcp_servers.toml` and named by id; the host resolves the command, so an app
  can never supply argv. `plexi app open --mcp` is a different thing: it wraps a
  server in a viewer pane and exposes nothing to the assistant.
<!-- /experiment -->
<!-- experiment: relay -->
- **Phone relay** — pair a phone to this desktop: `plexi relay enable --url <wss-or-loopback-ws>`,
  then `plexi relay confirm` after comparing the fingerprint the phone shows.
  `relay pair` adds another phone. `relay revoke <device-id>` drops one phone.
  `relay status` prints the desktop session. `relay connect` attaches to the
  host's socket. `relay disable` stops the host from connecting on startup.
  Message bodies are sealed; the relay stores no plaintext.
<!-- /experiment -->
- **Notifications** — show scoped information to the person using the host:
  `plexi notify --help`.
- **Workspace tools** — initialize a workspace, run named commands, and manage
  project secrets: `plexi workspace --help`, `plexi run --help`, and
  `plexi secret --help`. `secret list` first reconciles its index with the
  keychain, so it also shows keys added outside Plexi. A legacy-format secrets
  index is migrated the first time `secret get` or `secret list` runs, not at
  CLI startup.
<!-- experiment: routines -->
  Scheduled commands: `plexi routine --help`.
<!-- /experiment -->
- **Agents** — create, list, inspect, and spawn registry-backed agent definitions (`AGENT.md` + `settings.toml` with `[harness]`), inspect closed runs, reopen historical sessions, and report or inspect agent state:
  `plexi agent --help`. Agents live in two tiers (workspace `.plexi/agents/<slug>/` overrides global `<profile>/agents/<slug>/`, resolved by walking up from cwd stopping at `$HOME`).
  1. Run `plexi agent list [--json]` first to see whether a matching agent already exists in the workspace or global tier.
  2. If not, create one with `plexi agent create <slug> --cli <pi|claude|codex|agy> [--model <model>] [--fallback <cli>] [--ephemeral] [--result-schema <json-or-path>] --persona "<markdown>"` (or `--persona-file <path>`; defaults to workspace `.plexi/agents/<slug>/`; pass `--global` for reusable user-wide agents).
  3. Inspect with `plexi agent show <slug> [--json]` before spawning to see the resolved tier, path, harness settings, `AGENT.md` body, and exact command line `spawn` will run.
  4. Spawn with `plexi agent spawn <slug> --task "<brief>" [--subcontext <name>] [--ephemeral]` and collect the structured return with `plexi pane result wait <pane_id> --timeout 300`. `spawn` resolves `<slug>` through the two-tier registry, creates or reuses a sub-context with a parent `PortalPane`, stamps `PLEXI_PARENT_PANE` and `PLEXI_EPHEMERAL=1`, injects `AGENT.md` (`--append-system-prompt` for `pi` and `claude`; inlined `# Persona` for `codex` and `agy`) plus the `# Plexi worker contract` and any `result_schema`, and boots the configured harness.
  Workers report structured completion via `plexi agent done --result <json> --summary "<text>"` (validates
  against `result_schema`, writes `completed` and `summary` slots, and exits `0` so the
  ephemeral pane closes and the sub-context auto-collapses when all workers succeed; non-zero
  crashes keep the pane and sub-context open in `Error` state). Workers escalate to their
  parent master agent via `plexi agent ask "<question>"` (Tier 1 `Blocked`, waking the parent's
  `pane wait` without filing human `NeedsYou`), while `plexi needs-you` is Tier 2 human
  escalation. On startup (`[agents] restore_mode = "lazy"` by default), active-context agent
  panes with a captured `session_id` auto-resume in their restored `cwd` via their Harness
  Adapter (`claude --resume <id>`, `codex resume <id>`, `pi --session <id>`, `agy --resume <id>`),
  mid-turn `Working` panes transition to `Interrupted` with their `tail_preview`, ephemeral
  `Completed` workers are skipped, and background/sub-context agent panes restore in
  `[Plexi Standby]` (preserving slots and `Blocked` / `NeedsYou` status rollups on parent
  portals) until focused, `Enter`, or `plexi pane send`. Closing an agent pane appends a record
  to `<profile>/agent_history.jsonl`; inspect closed runs with
  `plexi agent history [--limit N] [--harness <name>] [--role <name>] [--json]` and reopen any
  historical run into a live resumed pane with
  `plexi agent reopen <run-id|session-id> [--subcontext <name>] [--json]`. `agent report --event` preserves
  a provider lifecycle event separately from its UI state; `--blocked-reason`
  supplies a typed reason. Read `agent report --help` before using these optional
  fields.
<!-- experiment: leads -->
  Agent heads and runs: `agent head create` stores a
  named head under `.plexi/agents` with `--grant tool=allow|ask|deny`. `AGENT.md`
  is guidance and grants nothing. `agent run spawn` starts a run; repeat the
  same `--admission` to get that run back, and a second admission while it is
  active returns `assignment_conflict`. `agent run finish` stops it. `agent delegate`
  starts a temporary child with a subset of the parent run's grants.
  `agent conversation --head <id> --as <other>` is refused when the ids differ.
  `agent assign --head <id> --input task.json` queues work with no pane open.
  `agent cancel --id <task>` stops it before the next tool. A lead cannot read,
  write, or message another lead.
<!-- /experiment -->
<!-- experiment: leads, assistant -->
  Ask-tier calls wait on `plexi assistant permission list`. Do not resolve those
  requests from an agent pane. `assistant send --head <id>` runs one model turn
  in that head's conversation. `assistant open --head <id>` opens an Assistant
  pane bound to that head.
<!-- /experiment -->
<!-- experiment: command_view -->
  `command-view` lists heads, runs, and queued tasks from the agents API.
  `command-view open` shows the same rows in a pane. `command-view send <lead> <text>`
  runs a real turn in that head's conversation. `command-view cancel <run>` stops
  that run before the next tool. `command-view resolve` and `command-view allow`
  do not grant, and both are refused when `PLEXI_PANE_ID` is set.
<!-- /experiment -->
<!-- experiment: ledger -->
- **AI ledger** — `plexi ledger` prints per-client totals for tokens, cost,
  runs, and wall time from this channel's ledger. `plexi ledger summary --help`
  groups by client or kind. A token count is a positive number or the word
  `unknown`. The command reads the local ledger file and does not need a
  running host.
<!-- /experiment -->
<!-- experiment: assistant, ledger -->
  `assistant send --client` and `--kind` override the tags for one run;
  omitted, the client comes from `[ai] client` and the kind is `output`.
<!-- /experiment -->
<!-- experiment: assistant -->
  Send does not require `app open assistant` first: with no pane named, the
  host reuses an Assistant in the context or creates a hidden one. A pane id
  that does not exist still fails.
<!-- /experiment -->
- **Configuration and diagnostics** — inspect configuration, AI setup, app
  health, and updates: `plexi config --help`, `plexi ai --help`,
  `plexi doctor --help`, and `plexi update --help`.
  Every `config` verb defaults to the workspace config when run in a workspace,
  otherwise the channel-global config; `--global` selects only the latter.
- **Tool registry** — refresh CLI-tool knowledge: `plexi registry --help`.
<!-- experiment: marketplace -->
- **Marketplace** — search, browse, and publish apps: `plexi app search`,
  `plexi app browse`, `plexi app publish`.
<!-- /experiment -->
<!-- experiment: cloud_account -->
- **Account** — optional cloud account: `plexi account --help`.
<!-- /experiment -->

- **Notes** — capture and browse notes across their two storage tiers
  (`~/.plexi/notes/` and `<context-root>/.plexi/notes/`): `plexi note --help`.
  `plexi note "text"` writes into the tier the working directory belongs to;
  `plexi note list` also rolls up any tier nested below it plus the global tier,
  and `plexi note open` picks one with fzf. Use `plexi note add <text>` when the
  text is itself `list`, `open`, or `add`.
- **Events** — subscribe to an app's event stream and declare/emit events:
  `plexi events --help`.
<!-- experiment: mcp_client -->
  `plexi events mcp-config` prints the singleton host MCP config for this pane's
  scoped credential (exposes event tools and `<app_id>__<tool>` for live apps in
  the pane's workspace).
<!-- /experiment -->

The release gate verifies these feature-map entry points:

```
pane
pane slot
pane result
pane result schema
pane result write
pane result read
pane result wait
pane message
pane message send
pane message recv
context
app
<!-- experiment: assistant -->
assistant
assistant send
assistant permission
assistant permission list
assistant permission show
assistant permission resolve
<!-- /experiment -->
needs-you
needs-you list
needs-you resolve
<!-- experiment: relay -->
relay
relay connect
relay confirm
relay revoke
relay pair
relay enable
relay disable
relay status
<!-- /experiment -->
notify
workspace
run
secret
<!-- experiment: routines -->
routine
<!-- /experiment -->
events
agent
config
ai
<!-- experiment: ledger -->
ledger
<!-- /experiment -->
debug
doctor
update
<!-- experiment: cloud_account -->
account
<!-- /experiment -->
registry
note
```

<!-- experiment: folder_secrets -->
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

<!-- /experiment -->

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

### Create a named sub-context with an agent squad

Create terminal panes in one sub-context window. Panes are named
`<name>-1`, `<name>-2`, … by default, and the layout defaults from `--agents`
(`tiled` when the count fills a square grid such as 4 or 9, `columns` otherwise
so no grid cell is left empty).

```bash
SQUAD=$(plexi context sub release-train --agents 3 --command 'exec zsh')
CONTEXT_ID=$(printf '%s' "$SQUAD" | jq -r '.context_id')

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
RESULT=$(plexi pane slot wait result --pane-id "$WORKER" --until '^ready$' --timeout 300)
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

<!-- experiment: mcp_client -->
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

<!-- /experiment -->

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

### Inspect a recorded host session

Every host run writes a durable flight trace to
`~/.plexi-<channel>/sessions/<sess-id>.jsonl` (`sessions/latest.jsonl` points at
the newest). `plexi debug session list [--json]` lists sessions with first/last
time, contexts, panes, and attached agent session ids. `plexi debug session show
[<sess-id>|latest] [--pane <id>] [--join-agents] [--json]` prints one
chronological timeline of app events, tool calls, permission asks, desktop
decisions and uses, and pane/context lifecycle. Rows share `{seq, ts,
host_session_id, context_id, pane_id, call_id}`. `--pane` keeps that pane's rows
plus the calls it made. `--join-agents` interleaves the pi and Claude transcripts
named by recorded agent session ids. Neither verb needs a running host.

<!-- experiment: ledger -->
### Summarize AI ledger usage

`plexi ledger` prints this channel's per-client totals. `ledger summary`
groups by client or kind. `--since` keeps rows at or after a date. Each group
reports runs, input and output tokens, cost, and wall time. A token count that
was not measured is the word `unknown`. Wall time is null in JSON when it was
not recorded. A client or kind that was never tagged is the null group.

```bash
plexi ledger
plexi ledger summary --by client --since 2026-01-01 --json
<!-- experiment: ledger, assistant -->
plexi assistant send --text "hello" --client narrative --kind output
<!-- /experiment -->
```

<!-- /experiment -->

### Agents (`plexi agent --help`)

Read `plexi agent --help` for the create/spawn flow. Spawn returns immediately
with a pane id; it does not wait for the task to finish. Wait in bounded slices:
run `plexi pane result wait <pane_id> --timeout 300`, choosing a timeout well
under your tool harness's call limit. Exit 0 means done and prints the result;
exit 2 means the slice timed out, so report progress and re-run the wait for the
same pane. Exit 1 is an error to inspect. Never use one long wait or spawn a
replacement just because a slice expired.

A print-mode worker that calls `plexi agent ask "<question>"` waits for its
parent's reply with `plexi pane message recv --timeout 300`. This blocks until
a message arrives (exit 0 with a JSON array) or the slice expires (exit 2).
Repeat on exit 2; inspect errors on exit 1. The parent replies with
`plexi pane message send <worker-pane> "<answer>"`.

1. Run `plexi agent list` first to see whether a matching agent already exists in the workspace or global tier.
2. If not, create one with `plexi agent create <slug> --cli <pi|claude|codex|agy> [--model <model>] --persona "<markdown>" [--result-schema '<json>'] [--ephemeral]` (defaults to workspace `.plexi/agents/<slug>/`; pass `--global` for reusable user-wide agents).
3. Inspect with `plexi agent show <slug>` before spawning if unsure what CLI or model it uses.
4. Spawn with `plexi agent spawn <slug> --task "<brief>" [--subcontext <name>] [--ephemeral]` and collect the structured return with `plexi pane result wait <pane_id> --timeout 300`.

<!-- experiment: leads -->
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
<!-- experiment: leads, assistant -->
plexi assistant open --head lead
plexi assistant send --head lead --text 'status?'
<!-- /experiment -->
plexi agent conversation --head lead --json
<!-- experiment: leads, command_view -->
plexi command-view --json
plexi command-view send lead-a "status?"
plexi command-view cancel run_example
plexi command-view resolve pending_example
plexi command-view allow --tool assistant.turn
<!-- /experiment -->
plexi agent assign --head lead --input task.json --json
plexi agent cancel --id task_example --json
```

<!-- /experiment -->

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

## Resource leases

Acquire capacity before spawning a lane or starting a build:

```bash
plexi lease acquire lanes --max 3 --pid $$
plexi lease acquire cargo-build --max 1 --pid $$
plexi lease release cargo-build --pid $$
plexi lease release --pid $$
```

Replies are JSON. Exit 0 means acquired/released; exit 2 with `status: paused`
means stop admitting work and retry later, without spawning or queueing panes.
Exit 1 is an error. Without `--pid`, the calling shell (the CLI's parent) owns
the lease. An explicit PID must be the CLI or an ancestor authenticated by the
host's socket. Use a dedicated subshell for each concurrent lane; acquiring the
same name again from the same holder is idempotent, not another slot.

A pool's first holder fixes its maximum until the pool empties. A different
`--max` fails with `lease_max_mismatch`. `release [name]` releases that holder's
named lease, or all its leases when the name is omitted. The host reclaims leases
when a holder exits, including a killed process; PID reuse does not retain them.
Leases belong to the running host and end when it stops. Use the same host for
all lanes sharing a cap. The existing machine-wide Cargo file lock remains in
place across hosts and worktrees.

The host samples machine-wide allocated swap bytes and live `cargo`/`rustc`
processes off the UI thread. Admission pauses at 256 MiB of swap allocation
growth within a rolling 60-second window, or at four concurrent build processes.
It never uses free-memory percentage. Sampling starts on the first lease request;
initial swap allocation establishes the baseline. Existing work is never killed.
Sampling failures pause new acquisition; explicit release remains available.
A paused request is not retained: the caller decides when to retry.
