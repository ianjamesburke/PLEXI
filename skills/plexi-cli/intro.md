# Plexi CLI

Start with `plexi --help`, then `plexi <command> --help` for arguments and
flags. When the target channel is known, use its `plexi-<channel>` shim for
every command (`plexi-alpha` reads `~/.plexi-alpha/` and `<repo>/.plexi-alpha/`;
`plexi-beta` reads `~/.plexi-beta/` and `<repo>/.plexi-beta/`; stable `plexi`
reads `~/.plexi/` and `<repo>/.plexi/`). Bare `plexi` inherits the calling
pane's routing and may target a different host. Run host commands from a Plexi
pane so the host supplies its context and connection. Use the CLI or app SDK
for state.

This binary carries the skill. Install it for your agent:

```bash
plexi skill install --agent claude
plexi skill install --agent codex
plexi skill install --agent pi
```

## Use a configured MCP server

For a configured MCP server, `plexi app open --mcp <server-id>` opens its tools,
resources and prompts when the MCP client experiment is enabled. Supply the
configured id, such as `filesystem`. Approve connection and endpoint requests
in Needs You, then retry in the pane. Use j/k, Enter, Tab and Esc to operate the
generated lists and forms; Copy copies the displayed result.

## Coordinate workers

Reuse an existing registry role when it fits. Inspect its settings before
spawning. Spawn returns a pane id immediately; collect its result separately.

```bash
plexi agent list
plexi agent show <slug>
plexi agent start <slug>
plexi agent create <slug> --cli pi --persona "<markdown>" --result-schema '<json>' --ephemeral
plexi agent spawn <slug> --task "<brief>" --subcontext "<name>"
plexi pane result wait <pane_id> --timeout 300
```

Keep each wait below your tool harness call limit. Exit 0 prints the result;
exit 2 means the slice expired, so report progress and repeat the same wait.
Inspect errors on exit 1. A timeout does not call for a replacement worker.

`settings.toml` `[permissions]` posture (`default_posture`, `allow`, `ask`, `deny`) is snapshotted onto each spawned worker pane (intersected with its parent's posture) and enforced by the host permission monitor on Plexi CLI and host-brokered capabilities (`secret.get`, `secret.list`, `pane.send`, `app call`). Harness-native tools (such as `pi` `bash`/`read`/`write`/`edit`, `claude -p` with no permission mode, `codex` workspace-write and network, and `agy`) execute inside the harness process and are outside Plexi's permission gate.

For an interactive agent without a registry role, use the host-confirmed
launch and submit sequence:

```bash
ID=$(plexi pane new --agent <tier-alias> -n "<name>" --no-focus)
plexi pane send "$ID" "<brief>" --submit
```

The first command waits until the agent is idle; the second confirms the
prompt left its input line. Avoid sleeps, capture polling, and manual Enter.
`pane state` reports both claimed and observed state; an idle agent has not
necessarily completed its task. Use `--stale-after` to detect a stale claim.

Print-mode workers ask their parent, wait for a mailbox reply in bounded
slices, and finish with a validated result:

```bash
plexi agent ask "<question>"
plexi pane message recv --timeout 300
plexi pane message send <worker-pane> "<answer>"
plexi agent done --result '<json>' --summary "<text>"
```

Mailbox receive exits 0 with a JSON array, 2 on timeout, and 1 on error.
The parent sends the reply. Role-scoped `agent call` and `agent handoff` use
`can_call` and `can_handoff` allowlists. Handoff completes and closes the
caller only after a compatible successor launches. Tool children attach to
the parent's child strip; Cmd+Down opens that view and Cmd+Escape returns.

## Run a stored team

`agent team run <name>` looks for a team in the nearest workspace, then the
active channel profile, then built-ins. A global or built-in team can run from
any directory; its default `RESULT.md` is written under the active profile's
`sessions/teams/` directory. Pass a relative `--out` to place the report under
the current directory instead.

```bash
plexi agent team show consult-experts
plexi agent team run consult-experts --question "Which option is safest?"
```

## Inspect apps and request human decisions

Discover live tools and their schemas before calling them. A mutating call
may return `permission_required` and a `pending_request_id`; show it to the
person and wait for the desktop decision. Agents cannot approve their own
requests. Creative answers settle questions but never grant permissions.
After an answer, re-read resource state before retrying a mutation.

```bash
plexi app tools --json
plexi app call <app_id> <tool> --input '<json object>' --json
plexi needs-you list --json
plexi needs-you show <id>
plexi needs-you reconcile
```

Completed continuations must not repeat their mutation. For app-to-app data,
use event streams or typed binary pipes. `pane send` is for driving a terminal
a person is watching.

## Open files and native surfaces

You run inside a pane, so prefer a Plexi surface over an OS app or an external
editor. Built-in pane types open any file:

```bash
plexi app open text-editor <path>     # text, markdown, code (.md gets live preview)
plexi app open image-viewer <path>
plexi app open video-player <path>
plexi app open audio-player <path>
plexi app open file_browser           # no path: browses the pane's directory
```

Built-in pane types do not appear in `plexi app list` (that lists installed
apps only), and `text-editor` with no path opens a fresh scratch note. Read an
open app pane with `plexi pane state <id>`; `pane capture` is terminal-only.

## Capture and handle notes

Plexi has a built-in notes editor. A note is a Markdown file in the tier of the
current directory (its nearest ancestor holding `.plexi/`, else the global
tier), and the CLI is the sanctioned way to read and retire it. Never read or
delete a note through its backing file path directly.

```bash
plexi note "a thought"            # capture (shorthand for `plexi note add`)
plexi note list                   # print every visible note path, newest first
plexi note show <path>            # print one note's contents
plexi note open                   # fzf picker in the focused pane, opens in $EDITOR
```

To handle a person's notes: run `plexi note list`, then `plexi note show` each
path and act on what it says. After a note has been addressed, tell the person
it can likely be deleted and run `plexi note rm <path>` only on their yes —
`rm` is irreversible. `show` and `rm` refuse any path outside a notes tier
visible from the current directory, so pass exactly the paths `list` printed.

## Inspect the host and manage capacity

Use `host status --json` to check readiness after a startup timeout before
retrying. Capture rendered pixels with `host screenshot`; use `pane state`
for semantics. Use `pane layout` for the active strip's membership, widths,
bounds, geometry generation, and input owner without pane content. Updates
preserve the channel; restart after rollback to run
the restored package. To recover a pre-migration workspace with an older build,
run `plexi-alpha host workspace-rollback --to-channel recovery` with a new
channel name. The export is the earlier workspace; later changes stay in Alpha.

```bash
plexi host start --ephemeral --timeout-secs 15
plexi host status --json
plexi host screenshot --output <path>
plexi pane state <id> --stale-after 60
plexi lease acquire lanes --max 3 --pid $$
plexi lease release lanes --pid $$
```

A lease acquire exits 2 with `status: paused` when admission must wait; do
not spawn work until it succeeds. Exit 1 is an error. Leases belong to the
holder process and are reclaimed when it exits. Use one host for all lanes
sharing a capacity limit.

## Draft a workflow from a session

The host captures CLI argv with caller pane, agent session, timestamps and the
CLI exit code. Secret-setting values, secret options, known secret names and
uppercase environment assignments such as `API_KEY=value` are redacted before
recording. Ordinary grants such as `agents.ping=allow` remain readable. Interrupted calls, including a host shutdown,
retain an unknown exit status. This diagnostic trace is not an authorization log.

```bash
plexi debug session show --cli
plexi debug session show --cli --pane <id> --utc
plexi debug session show --cli --pane <id> --json
plexi skill draft --from-session latest --pane <id>
plexi skill draft --from-session latest --as command --out <path>
```

Session text uses shell-quoted commands with the executable basename, clips long
arguments, and shows local timestamps with a zone and a date for older rows.
The header identifies the resolved session and its full recorded time range.
Use `--utc` for UTC timestamps or `--json` for complete recorded argv.

Drafts default to the calling pane (`PLEXI_PANE_ID`). Use `--pane` to target a
different pane or `--all-panes` to include the whole session; outside a pane,
omitting both includes all panes. Agent bookkeeping and pane polling are omitted
unless `--include-internal` is set. Consecutive identical calls collapse with a
repeat note. Failed and incomplete calls are omitted unless `--include-failed`
is set, which keeps them as review comments. Redacted calls always stay comments.

Drafts print to stdout unless `--out` names a new file. They never install a skill
or edit `<repo>/.plexi-<channel>/commands.toml` / `~/.plexi-<channel>/commands.toml`
(stable: `<repo>/.plexi/commands.toml` / `~/.plexi/commands.toml`). Review pane ids
and paths, replace redacted values through the secret workflow, and choose a
task-specific name for suggested `--subcontext` arguments. Session inspection and
drafting do not add calls to the trace they are reading.

The command map below comes from this binary's CLI and experiment ownership.
Use each command's `--help` for its full argument contract. Installed skills
omit entries for experiments that are off.

## Inspect secret names and restart

`plexi secret list --json` returns `entries` and `skipped`. An out-of-scope cwd
still returns user-scope names. Use `--global`, change directory into the pane's
context, or select an owned context with `--context <id|name|root>`. An agent
needs a `secret.list` approval to use an explicit context; ask the human to
review the request in Needs you, then retry. Listing never returns values.

For `secret get`, retry once after the human approves the pending request.
A not-set or Keychain authentication error ends that request: follow the printed
repair guidance instead of repeatedly asking for approval. A changed binary
signature may require authorizing Plexi in Keychain Access or re-running
`secret set NAME` with the current channel binary. Keychain authentication may
show a system dialog; it does not always use Touch ID.

Folder bindings require the `folder_secrets` experiment, which is off by
default. Disabled bindings appear as a hidden count. Enable it with
`plexi experiments enable folder_secrets` and restart using `plexi host restart`.
A restart restores the saved workspace. From a pane of the host being restarted,
`--yes` confirms that the pane will close; a detached coordinator completes the
restart. From another host or terminal, restart waits and prints the new pid and
pane count (`--json` is supported).
