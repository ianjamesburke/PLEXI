# Plexi CLI

Start with `plexi --help`, then `plexi <command> --help` for arguments and
flags. When the target channel is known, use its `plexi-<channel>` shim for
every command. Bare `plexi` inherits the calling pane's routing and may target
a different host. Run host commands from a Plexi pane so the host supplies
its context and connection. Use the CLI or app SDK for state.

This binary carries the skill. Install it for your agent:

```bash
plexi skill install --agent claude
plexi skill install --agent codex
plexi skill install --agent pi
```

## Coordinate workers

Reuse an existing registry role when it fits. Inspect its settings before
spawning. Spawn returns a pane id immediately; collect its result separately.

```bash
plexi agent list
plexi agent show <slug>
plexi agent create <slug> --cli pi --persona "<markdown>" --result-schema '<json>' --ephemeral
plexi agent spawn <slug> --task "<brief>" --subcontext "<name>"
plexi pane result wait <pane_id> --timeout 300
```

Keep each wait below your tool harness call limit. Exit 0 prints the result;
exit 2 means the slice expired, so report progress and repeat the same wait.
Inspect errors on exit 1. A timeout does not call for a replacement worker.

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

## Inspect the host and manage capacity

Use `host status --json` to check readiness after a startup timeout before
retrying. Capture rendered pixels with `host screenshot`; use `pane state`
for semantics. Updates preserve the channel; restart after rollback to run
the restored package.

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

The command map below comes from this binary's CLI and experiment ownership.
Use each command's `--help` for its full argument contract. Installed skills
omit entries for experiments that are off.
