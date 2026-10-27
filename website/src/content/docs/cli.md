---
title: CLI Reference
description: Complete reference for all plexi subcommands and flags.
order: 7
---

The `plexi` CLI is the primary way to interact with a running Plexi instance from the terminal, and to manage workspaces and local apps from outside the UI. This reference is generated from the complete public command inventory; hidden implementation commands are omitted, and beta-only entries are labelled where they appear.

Stable v1 covers the tiling host, panes, subcontexts, status hooks, Quick Note, local app runtime, and CLI/MCP app wrappers. Assistant, marketplace, MCP client, and routine surfaces are beta-gated and do not appear in stable help. Use `plexi-beta` or an explicit worktree channel only when testing those gated surfaces.

Each channel has its own binary, profile (`~/.plexi`, `~/.plexi-alpha`, `~/.plexi-beta`), and workspace directory (`<repo>/.plexi`, `<repo>/.plexi-alpha`, `<repo>/.plexi-beta`). A channel-named binary always targets its own profile and workspace directory; the bare `plexi` binary honors an explicit `PLEXI_SOCKET` / `PLEXI_CHANNEL` when run inside a Plexi pane.

## `plexi lease`

Acquire process-lifetime capacity before starting work. Pauses exit with code 2

| Subcommand | Description |
|---|---|
| `acquire` | Acquire one slot. The calling shell owns it by default; no work is queued |
| `release` | Release this holder's leases (or just the named pool) |

### `plexi lease acquire`

Acquire one slot. The calling shell owns it by default; no work is queued

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes |  |
| `--max` | string | yes |  |
| `--pid` | string | no | Holder PID; must be this CLI or one of its ancestors |

### `plexi lease release`

Release this holder's leases (or just the named pool)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | no |  |
| `--pid` | string | no |  |

## `plexi assistant`

Submit a text turn to the running host Assistant

| Subcommand | Description |
|---|---|
| `send` | Submit through the same composer, model, and permission path as the desktop Assistant |
| `tool` | Run one Assistant host tool through the host permission gate |
| `permission` | Observe and resolve pending permission requests. This is not `plexi agent request` |

### `plexi assistant send`

Submit through the same composer, model, and permission path as the desktop Assistant

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--text` | string | no | Prompt text. Omitted when `--status-for` only reads a pending turn |
| `--request-id` | string | no |  |
| `--pane-id` | string | no | Target pane id or name |
| `--context-id` | string | no | Target context id or name |
| `--conversation` | string | no | Caller-owned conversation. A phone session passes one stable id. Omit to start a new conversation that is not the desktop transcript |
| `--desktop` | flag | no | Append this turn to the desktop Assistant conversation |
| `--status-for` | string | no | Read the outcome of a turn that already returned waiting_for_permission. Does not submit a new prompt |
| `--client` | string | no | Ledger client tag for this run. Omitted uses `[ai] client` |
| `--kind` | string | no | Ledger run kind: `system` or `output`. Omitted means `output` |

### `plexi assistant tool`

Run one Assistant host tool through the host permission gate.

Example: plexi assistant tool host.files.edit --input '{"path":"/tmp/note.md","old_string":"a","new_string":"b"}'

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Tool name, for example `host.editors.list` or `host.files.edit` |
| `--input` | string | yes | JSON object passed as the tool input |

### `plexi assistant permission`

Observe and resolve pending permission requests. This is not `plexi agent request`

| Subcommand | Description |
|---|---|
| `list` | List pending permission requests as JSON |
| `show` | Show one pending permission request as JSON |
| `resolve` | Resolve one pending request: once, session, always, or deny. The host refuses every choice from the terminal |

#### `plexi assistant permission list`

List pending permission requests as JSON

#### `plexi assistant permission show`

Show one pending permission request as JSON

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes |  |

#### `plexi assistant permission resolve`

Resolve one pending request: once, session, always, or deny. The host refuses every choice from the terminal

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes |  |
| `--choice` | string | yes |  |

## `plexi relay`

Pair a phone through the Plexi relay and deliver its messages to this desktop

| Subcommand | Description |
|---|---|
| `connect` | Connect outbound to a phone relay and forward paired messages to the Assistant |
| `confirm` | Confirm a phone that redeemed the pairing code shown by `relay connect` |
| `revoke` | Revoke a paired phone. It must pair again and be confirmed |
| `pair` | Start a pairing code for another phone. Phones already paired stay paired |
| `enable` | Remember the relay URL and connect it from the host on startup |
| `disable` | Stop connecting to the relay when the host starts |
| `status` | Print the desktop's relay status |

### `plexi relay connect`

Connect outbound to a phone relay and forward paired messages to the Assistant

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--url` | string | no | Relay websocket URL (`ws://` locally, `wss://` in production) |

### `plexi relay confirm`

Confirm a phone that redeemed the pairing code shown by `relay connect`

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pairing_id>` | string | no | Pairing id. Omit to confirm the phone currently waiting |

### `plexi relay revoke`

Revoke a paired phone. It must pair again and be confirmed

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<device_id>` | string | yes | Device id printed when the phone was confirmed |

### `plexi relay pair`

Start a pairing code for another phone. Phones already paired stay paired

### `plexi relay enable`

Remember the relay URL and connect it from the host on startup

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--url` | string | no | Relay websocket URL (`ws://` locally, `wss://` in production) |

### `plexi relay disable`

Stop connecting to the relay when the host starts

### `plexi relay status`

Print the desktop's relay status

## `plexi command-view`

Show every registered agent and its runs. Cancel finishes an active run.

The rows come from the agents API agent records and runs. `open` shows the same projection in a pane. `cancel` stops that run. `resolve` and `allow` never grant, and an agent pane is refused.

Example: plexi command-view --json Example: plexi command-view --follow Example: plexi command-view open Example: plexi command-view cancel run_example

| Subcommand | Description |
|---|---|
| `open` | Open the command view pane in the active context |
| `cancel` | Stop a live run before its next tool call |
| `resolve` | Resolve a pending approval. An agent pane is refused, and this command does not grant |
| `allow` | Record an allow. An agent pane is refused, and this command does not grant |

### `plexi command-view open`

Open the command view pane in the active context

### `plexi command-view cancel`

Stop a live run before its next tool call.

Example: plexi command-view cancel run_example

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<run>` | string | yes | Run id from `command-view --json` |

### `plexi command-view resolve`

Resolve a pending approval. An agent pane is refused, and this command does not grant.

Example: plexi command-view resolve pending_example

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Pending request id |

### `plexi command-view allow`

Record an allow. An agent pane is refused, and this command does not grant.

Example: plexi command-view allow --tool assistant.turn

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--tool` | string | no | Tool name. Ignored. The command does not write a grant |

## `plexi run`

Run a named command from `<repo>/.plexi-<channel>/commands.toml` or `~/.plexi-<channel>/commands.toml` (or global scripts).

Resolution and precedence rules:

(1) Global tier: `config_dir()/commands.toml` (`~/.plexi-alpha/commands.toml` on alpha, `~/.plexi-beta/commands.toml` on beta, `~/.plexi/commands.toml` on stable) plus executable files in `config_dir()/scripts/` (`~/.plexi-<channel>/scripts/`, or `~/.plexi/scripts/` on stable).

(2) Workspace tier: `<workspace>/.plexi-<channel>/commands.toml` (`<workspace>/.plexi/commands.toml` on stable), where `<workspace>` is the nearest ancestor of the starting `cwd` containing a `.plexi-<channel>/` directory (`.plexi/` on stable) via `resolve_workspace_root_stopping_at`. The upward walk stops at `$HOME` and the passwd login home directory so `~` is never treated as a workspace. Fallback (`resolve_commands_workspace_root`): `cwd` itself when `cwd/.plexi-<channel>/commands.toml` (or `cwd/.plexi/commands.toml` on stable) exists and is distinct from the global `commands.toml`.

(3) Starting directory by surface: `plexi run` in a terminal uses the shell's `cwd`. The command palette (`Cmd+P`) uses the focused pane's `cwd`, then falls back to the active context root (`effective_commands_workspace_root`); `Edit commands` opens that workspace's `commands.toml` (creating it from template if absent), or the global `commands.toml` when outside a workspace. `[keys]` `"<chord>" = "run:<name>"` bindings in `~/.plexi-<channel>/config.toml` (or `<repo>/.plexi-<channel>/config.toml`; `~/.plexi/config.toml` on stable) use the adopted/host workspace root, falling back to the active context root (`run_user_command_by_name`, validated by `plexi config check` against reserved shortcuts).

(4) Precedence: workspace `commands.toml` > global `commands.toml` > global `scripts/`, merged by command name. `plexi run` with no arguments lists both scopes (`[workspace]` and `[global]`); `--json` prints `[{name, scope, description, run, steps, secrets}]` (secret names only, never values). Any secrets listed in a command definition are injected as environment variables automatically when the command runs.

(5) Live reload: `commands.toml` and `scripts/` are re-read from disk on every command palette open and every `plexi run` or keybinding invocation — no host restart is needed.

Multi-step commands (`steps = [...]`) execute each Plexi CLI invocation or shell snippet sequentially, substituting `$1`, `$2`, … and `$@` from extra arguments, and stop at the first failing step with `step <i>/<n> failed: <cmd> (exit <code>)`.

Example: plexi run dev Example: plexi run play Example: plexi run --json

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<command>` | string | no | Command name to run (omit to list available commands) |
| `<extra_args>` | string (repeatable) | no | Extra arguments forwarded to the command as $1, $2, … and $@ positional params |

## `plexi workspace`

Set up a .plexi/ workspace in your project folder.

Run this once inside your project directory to enable workspace-scoped secrets and commands.

| Subcommand | Description |
|---|---|
| `init` | Set up a .plexi/ workspace in the current directory |
| `clean` | Remove pane slot files for panes that are no longer open |

### `plexi workspace init`

Set up a .plexi/ workspace in the current directory.

Run this once inside your project folder. It creates `<repo>/.plexi-<channel>/` (or `<repo>/.plexi/` on stable) with `workspace.toml`, `secrets.toml`, `apps.toml`, and `commands.toml` so that secrets, apps, and commands are scoped to this project.

### `plexi workspace clean`

Remove pane slot files for panes that are no longer open

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--dry-run` | flag | no | Print slot directories that would be removed without deleting them |

## `plexi changes`

Preview and accept an agent's file edit before it is written.

`propose` stores a change set and does not modify the file. `preview` prints the diff. `accept` writes it after the permission gate admits the same edit. If the file changed since propose, accept reports `stale` until `refresh`. `revert` restores a committed set.

| Subcommand | Description |
|---|---|
| `allow` | Record a permission-gate allow for one exact edit |
| `propose` | Prepare an edit. The file is not modified |
| `preview` | Print the prepared diff and whether the set is stale |
| `accept` | Write a pending change set after the permission gate admits it |
| `refresh` | Rebase a stale change set onto the file's current text |
| `revert` | Restore the file to the text from before accept |
| `profile` | Print this binary's profile directory |

### `plexi changes allow`

Record a permission-gate allow for one exact edit

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--agent` | string | yes | Agent id the grant and the later audit row name |
| `--file` | string | yes | File the edit applies to |
| `--old` | string | yes | Exact text to replace. It must occur once |
| `--new` | string | yes | Replacement text |

### `plexi changes propose`

Prepare an edit. The file is not modified

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--agent` | string | yes | Agent id recorded on the change set |
| `--file` | string | yes | File the edit applies to |
| `--old` | string | yes | Exact text to replace. It must occur once |
| `--new` | string | yes | Replacement text |

### `plexi changes preview`

Print the prepared diff and whether the set is stale

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Change set id from `propose` |

### `plexi changes accept`

Write a pending change set after the permission gate admits it

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Change set id from `propose` |

### `plexi changes refresh`

Rebase a stale change set onto the file's current text

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Change set id from `propose` |

### `plexi changes revert`

Restore the file to the text from before accept

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Change set id from `propose` |

### `plexi changes profile`

Print this binary's profile directory.

Channel-suffixed binaries use their own directory (`~/.plexi-pr-N`). Scripts should ask here instead of assuming `~/.plexi`.

## `plexi secret`

Store and retrieve secrets (API keys, passwords, tokens).

Secrets use macOS Keychain or, on Linux, Secret Service with a labeled encrypted-file fallback. Global, workspace, and folder scopes share that Linux backend. See https://plexiapp.com/docs/secrets/#linux-storage for backend selection and legacy plaintext migration. Folder secrets are an experiment, off by default in V1. Enable with `plexi experiments enable folder_secrets`, then `plexi host restart` (add `--yes` from a pane on that host). `secret set NAME --folder DIR` binds a value to a directory. New human terminal panes opened there or below it can receive the value as an environment variable. Folder-secret grant/read commands are experimental and are not supported in V1.

Same-user native processes are not isolated from each other. A process running as this user can still read a value that a pane already holds. Folder secrets stop accidental injection into the wrong directory.

| Subcommand | Description |
|---|---|
| `set` | Save a secret to the platform secret store |
| `get` | Print a stored secret's value to stdout |
| `list` | Show stored secrets |
| `delete` | Delete a stored secret |
| `rm` | Remove a folder-scoped secret |
| `grant` | Ask for an allow on a folder secret. This command does not record one |
| `read` | Read a folder secret as an agent or app. Without a grant this prints `permission_required` and writes an audit row that names the secret but not its value |
| `exec` | Run a command with the environment a new terminal pane in `--cwd` receives, including folder secrets for that directory |

### `plexi secret set`

Save a secret to the platform secret store.

Plexi will prompt you to type the value (hidden). On macOS the secret is stored in the system keychain; on Linux it uses Secret Service or a labeled encrypted-file fallback. See https://plexiapp.com/docs/secrets/#linux-storage for storage and migration details. Terminal injection is opt-in in Secrets.

Use --from-env to read the value from an existing environment variable instead of typing it. Use --global to make the secret available across all projects, not just the current one.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes | Name for this secret — also the environment variable name it will be injected as |
| `--from-env` | flag | no | Read the value from the environment variable named FRIENDLY_NAME instead of prompting |
| `--global` | flag | no | Store this secret globally so it's available in all projects, not just this one |
| `--alias` | string | no | Use a different platform-store entry name than the canonical env var name. On macOS this can reuse an existing Keychain entry. Example: plexi secret set OPENAI_API_KEY --alias openai_personal |
| `--folder` | string | no | Bind the secret to this directory. The value is stored in the OS keychain (or the labeled encrypted-file fallback) and injected into new human terminal panes opened in this directory or a subdirectory. Requires experiment folder_secrets, off by default in V1. Enable: plexi experiments enable folder_secrets, then plexi host restart (add --yes from a pane on that host). Example: plexi secret set OPENAI_API_KEY --folder ~/code/client-a |

### `plexi secret get`

Print a stored secret's value to stdout.

Looks up the secret for the current project first, then falls back to the global store. Use --global to read only from the global store.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes | Name of the secret to read |
| `--global` | flag | no | Read from the global store only, skipping the project-level lookup |

### `plexi secret list`

Show stored secrets.

Inside a workspace, shows project secrets plus user-scope secrets. Outside a workspace, falls back to user-scope secrets. Use --global to show only user-scope secrets from any directory.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--context` | string | no | List names in an explicit owned context (id, name or root). Agents need approval |
| `--global` | flag | no | Show only globally-stored user-scope secrets |

### `plexi secret delete`

Delete a stored secret.

Use --global to delete a globally-stored secret (one stored with `secret set --global`). Without --global, deletes the workspace-scoped entry for the current project.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes |  |
| `--global` | flag | no | Delete from the global store instead of the project-scoped store |

### `plexi secret rm`

Remove a folder-scoped secret

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Environment variable name |
| `--folder` | string | yes | Directory the secret was bound to |

### `plexi secret grant`

Ask for an allow on a folder secret. This command does not record one.

Returns `permission_denied` and writes an audit row with the secret name and folder, never the value. This process does not record an allow. An agent pane cannot grant itself a secret, including a secret bound to another folder.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Environment variable name |
| `--agent` | string | no | Agent id. Stored as `agent:<id>` |
| `--app` | string | no | App id. Stored as `app:<id>` |
| `--folder` | string | no | Directory the secret was bound to. Required when the name exists in more than one folder |

### `plexi secret read`

Read a folder secret as an agent or app. Without a grant this prints `permission_required` and writes an audit row that names the secret but not its value

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Environment variable name |
| `--agent` | string | no | Agent id. Stored as `agent:<id>` |
| `--app` | string | no | App id. Stored as `app:<id>` |
| `--folder` | string | no | Directory the secret was bound to. Required when the name exists in more than one folder |

### `plexi secret exec`

Run a command with the environment a new terminal pane in `--cwd` receives, including folder secrets for that directory.

Refused with `permission_denied` when the caller is a pane agent, including a child of that pane that cleared `PLEXI_PANE_ID`. The command is not started and no secret value is printed. Same-user native processes are not isolated from each other.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--cwd` | string | yes | Working directory of the spawned command, and the pane cwd used for injection |
| `<command>` | string (repeatable) | no | Command and arguments |

## `plexi routine`

Manage workspace routines — scheduled shell commands.

Routines are declared in `routines.toml` inside the workspace channel directory — `{workspace_channel_dir}/routines.toml`, e.g. `.plexi-alpha/routines.toml` on the alpha channel — and run automatically on schedule. **Requires Plexi to be running** — there is no background daemon. Routines only fire while the host process is open.

Use `plexi routine list` to see configured routines, or `plexi routine run <name>` to fire one manually.

> **Beta-gated:** Routines are a post-v1 beta surface. This reference is included for beta and worktree testing; it is not available from the stable v1 channel.

### Routine file format (`{workspace_channel_dir}/routines.toml`)

The file lives in the workspace channel directory beside the rest of the workspace state — e.g. `.plexi-alpha/routines.toml` on the alpha channel.

```toml
[[routine]]
name      = "morning-sync"
command   = "./scripts/sync.sh"
schedule  = "daily at 09:00"
context   = "work"   # optional: fires into this context wherever it is; skipped if no context by that name exists
ephemeral = true     # optional: close the spawned pane when the command exits
enabled   = false    # optional: keep the routine but never fire it (`plexi routine disable`)
```

`plexi routine add` / `remove` / `enable` / `disable` edit this file for you, validating the schedule against the same parser the scheduler uses and preserving hand-written comments.

A routine never stacks panes: while the previous run's pane is still alive, due fires are skipped (with one notification per skip streak), and the routine fires again on the first tick after that run ends. Ephemeral panes close themselves when the command exits; a non-ephemeral pane holds its routine until its shell session ends or the pane is closed.

### Schedule formats

| Format | Example |
|---|---|
| `every N seconds` (or `Ns`) | `every 30 seconds` |
| `every N minutes` (or `Nm`) | `every 5 minutes` |
| `every N hours` (or `Nh`)   | `every 2 hours` |
| `every minute` / `every hour` | `every minute` |
| `daily at HH:MM`  | `daily at 09:00` |
| `weekdays at HH:MM` | `weekdays at 09:00` |
| `weekends at HH:MM` | `weekends at 10:30am` |
| `weekly on <day> at HH:MM` | `weekly on monday at 09:00` |
| `monthly on N at HH:MM`    | `monthly on 1 at 08:00` |
| 5-field cron `m h dom mon dow` | `0 9 * * 1-5` |

Singular unit names (`every 1 minute`) and am/pm times (`daily at 9am`) are accepted; day names take short or full spellings (`mon` / `monday`).

| Subcommand | Description |
|---|---|
| `list` | List routines defined in the workspace's routines.toml with their schedule and next fire time |
| `run` | Manually trigger a named routine from the workspace's routines.toml |
| `add` | Add a routine to the workspace's routines.toml |
| `remove` | Remove a routine from the workspace's routines.toml |
| `enable` | Re-enable a disabled routine (removes its `enabled = false` key) |
| `disable` | Disable a routine without deleting it (sets `enabled = false`; it never fires until re-enabled) |

### `plexi routine list`

List routines defined in the workspace's routines.toml with their schedule and next fire time

### `plexi routine run`

Manually trigger a named routine from the workspace's routines.toml.

The routine fires exactly like a scheduled run: into its configured context when one is named (erroring if that context does not exist), otherwise into the caller's context.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Name of the routine to run |
| `--force` | flag | no | Fire the routine even when it is disabled |

### `plexi routine add`

Add a routine to the workspace's routines.toml.

The schedule is validated against the same parser the scheduler uses, so an accepted routine is guaranteed to fire. Hand-written comments elsewhere in the file are preserved.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Unique name for the routine |
| `--command` | string | yes | Shell command the routine runs |
| `--schedule` | string | yes | When to run — e.g. "every 30m", "daily at 09:00", "0 9 * * 1-5" |
| `--context` | string | no | Context to fire into (default: the active context at fire time) |
| `--ephemeral` | flag | no | Close the spawned pane when the command exits |

### `plexi routine remove`

Remove a routine from the workspace's routines.toml

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Name of the routine to remove |

### `plexi routine enable`

Re-enable a disabled routine (removes its `enabled = false` key)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Name of the routine to enable |

### `plexi routine disable`

Disable a routine without deleting it (sets `enabled = false`; it never fires until re-enabled)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Name of the routine to disable |

## `plexi agent`

Create, inspect, and spawn agent definitions (`AGENT.md` + `settings.toml`).

An agent is a directory containing `AGENT.md` (persona / system prompt) and
`settings.toml` (`[agent]` metadata + `[harness]` CLI worker config).

Two tiers exist, and workspace definitions shadow global ones with the same name:
- Workspace: `<workspace>/.plexi/agents/<name>/`
- Global:    `~/.plexi-<channel>/agents/<name>/` (pass `--global` to `create`)

End-to-end worker workflow:
plexi agent create researcher --cli pi --model openrouter/~z-ai/glm-flash-latest \
--persona "You are a researcher." \
--result-schema '{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}}'
PANE=$(plexi agent spawn researcher --task "Summarize AgentRegistry")
plexi agent wait "$PANE" --timeout 300

`agent start` without a first message and task-seeded `agent spawn` are both interactive adapter launches;
only an explicitly configured Script job is one-shot.
Spawn returns immediately with a pane id, before the worker is done.
Wait in bounded slices with `plexi agent wait` without capture/send polling.
Exit 0 means done; exit 2 means report progress and re-run the same wait;
exit 1 is an error to inspect. Never use one long wait or respawn on timeout.

| Subcommand | Description |
|---|---|
| `team` | Inspect or run a stored team of independent ranking agents. |
| `next` | Focus the next live agent needing attention (needs-you, blocked, done-unseen) |
| `fleet` | Show tasks and their workers, ordered by attention, with blocking reasons |
| `create` | Create a new agent definition (`AGENT.md` + `settings.toml`) in the workspace or global tier. |
| `flags` | Show supported custom harness flags and safety rules for an agent or adapter |
| `list` | List active agents from both workspace and global tiers |
| `show` | Show the resolved definition for `<name>`, its directory path, and its exact `spawn` command |
| `call` | Call an agent role and return its validated completion or blocking question as JSON |
| `message` | Reply through a child agent's structured mailbox |
| `handoff` | Transfer the current task to a permitted successor role |
| `start` | Start an interactive agent session in a new pane from the agent registry (`<workspace>/.plexi/agents/<name>/` or global) |
| `spawn` | Spawn a CLI worker agent from the agent registry (`<workspace>/.plexi/agents/<name>/` or global) |
| `done` | Validate a worker's structured result against its schema, write completion slots, and exit cleanly |
| `result` | Read a completed agent's persisted result (`RESULT.md` or `result.json` with `--json`) |
| `wait` | Wait for one or more child agent panes to finish via `plexi agent done` and pull their results |
| `ask` | Escalate a blocking question to the parent master agent (Tier 1 `Blocked` state) |
| `report` | Report agent state for this pane to the host |
| `status` | Show current agent state for all panes |
| `history` | Show closed agent runs and sessions from `~/.plexi-<channel>/agent_history.jsonl` |
| `reopen` | Reopen a closed agent run or session from `agent_history.jsonl` in its saved working directory |
| `hook` | Install or uninstall agent hook integrations |

### `plexi agent team`

Inspect or run a stored team of independent ranking agents.

Team v0 TOML is resolved from workspace channel teams/<name>.toml,
then profile teams/<name>.toml, then built-in consult-experts.
Fields: version = 0; options = ["Proceed", "Experiment", "Defer"].
Each [[members]] has role, harness (pi/claude/codex/agy/script),
grants = ["agents.ping=allow", "agents.review=deny"], optional model.
Script members require command: a shell command whose stdout is exactly
{"ranking":["Proceed","Experiment","Defer"],"rationale":"why"}.
Scripts receive PLEXI_AGENT_TASK, PLEXI_TEAM_OPTIONS and PLEXI_TEAM_RUN_ID.
Real harnesses report the same JSON through agent done.
At least three members independently rank every option once. Borda awards
N-1 down to 0 points. Optional tie_break = "Defer" selects a tied leader;
default: last option. If that option is not tied, use the last tied option.
Optional quorum = 2 requires that many valid ballots; default: all members.
The host always writes RESULT.md with COMPLETE, PARTIAL, or FAILED status,
member outcomes and valid ballots. Below quorum there is no winner.
Failed/timed-out member panes stay open for inspection.
Grants use existing delegation admission; a team is not authority.
Format v0 may change. consult-experts uses pi, claude and codex by default.

| Subcommand | Description |
|---|---|
| `show` | Show the resolved v0 team, including roles, harnesses, options and grants |
| `run` | Run independent team members in one task and wait for RESULT.md |

#### `plexi agent team show`

Show the resolved v0 team, including roles, harnesses, options and grants

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes |  |

#### `plexi agent team run`

Run independent team members in one task and wait for RESULT.md

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes |  |
| `--question` | string | yes |  |
| `--out` | string | no | Output directory (default: team-results/<run-id> in a workspace, or the channel profile's sessions/teams/ outside one) |
| `--option` | string (repeatable) | no | Replace the stored options; repeat for each option (at least two) |
| `--timeout` | string | no | Host-side deadline in seconds; unfinished members are reported as timed out Default: `600`. |

### `plexi agent next`

Focus the next live agent needing attention (needs-you, blocked, done-unseen)

### `plexi agent fleet`

Show tasks and their workers, ordered by attention, with blocking reasons

| Subcommand | Description |
|---|---|
| `dismiss` | Dismiss a failed, stalled, or completed agent run from the fleet view while keeping it in `agent history` |

#### `plexi agent fleet dismiss`

Dismiss a failed, stalled, or completed agent run from the fleet view while keeping it in `agent history`

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<target>` | string | no | Pane ID (`#215` or `215`) to dismiss (omit when `--all-failed` is set) |
| `--all-failed` | flag | no | Dismiss all failed, stalled, and exited-without-report runs |

### `plexi agent create`

Create a new agent definition (`AGENT.md` + `settings.toml`) in the workspace or global tier.

Writes `<workspace>/.plexi/agents/<name>/` (or `~/.plexi-<channel>/agents/<name>/` with `--global`)
and validates the directory through `AgentDefinition::load_dir`. Refuses to overwrite an
existing agent directory.

Complete `settings.toml` example:
```toml
[agent]
id = "researcher"
display_name = "Researcher"
description = "Fast codebase researcher"

[harness]
cli = "pi"
model = "openrouter/~z-ai/glm-flash-latest"
fallback = "claude"
ephemeral = true
result_schema = '{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}}'
```

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Agent slug (`a-z`, `0-9`, `-`) |
| `--cli` | string | yes | CLI harness to launch (`pi`, `claude`, `codex`, `agy`, or `script`). Interactive harnesses (`pi`, `claude`, `codex`, `agy`) remain interactive in start/spawn; only explicit script jobs are one-shot |
| `--model` | string | no | Model identifier passed verbatim via `--model` |
| `--fallback` | string | no | Fallback CLI harness when the primary binary is not on PATH (`pi`, `claude`, `codex`, `agy`, `script`) |
| `--description` | string | no | Short human-readable description of the agent's role |
| `--persona` | string | no | Inline persona text written to `AGENT.md` (mutually exclusive with `--persona-file`) |
| `--persona-file` | string | no | Path to a markdown file whose contents are written to `AGENT.md` (mutually exclusive with `--persona`) |
| `--result-schema` | string | no | Optional JSON Schema string or path to a schema file enforced on `plexi agent done` |
| `--ephemeral` | flag | no | Auto-close the worker pane on clean exit (`0`) |
| `--display-name` | string | no | Human-facing display name in `[agent]` (defaults to `<name>`) |
| `--global` | flag | no | Write to the global profile registry (`~/.plexi-<channel>/agents/<name>/`) instead of the workspace |
| `--cwd` | string | no | Directory used to resolve the workspace root (defaults to current working directory) |
| `--flag` | string (repeatable) | no | Optional custom harness flag(s) passed to the adapter (can be repeated) |

### `plexi agent flags`

Show supported custom harness flags and safety rules for an agent or adapter.

When `<target>` is an agent slug, displays its resolved safe custom flags. When `<target>` is a harness adapter (`pi`, `claude`, `codex`, `agy`, `script`) or omitted, displays the adapter options inventory, reserved options, and security rules.

Example: plexi agent flags Example: plexi agent flags claude Example: plexi agent flags researcher

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<target>` | string | no | Agent slug or harness adapter name (`pi`, `claude`, `codex`, `agy`, `script`) |
| `--cwd` | string | no | Directory used to resolve the workspace root (defaults to current working directory) |

### `plexi agent list`

List active agents from both workspace and global tiers.

Prints each agent's name, tier (`workspace` or `global`), CLI + model (or `assistant` when no `[harness]` table is present), description, and whether it shadows a global agent.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--cwd` | string | no | Directory used to resolve the workspace root (defaults to current working directory) |

### `plexi agent show`

Show the resolved definition for `<name>`, its directory path, and its exact `spawn` command

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Agent name to inspect |
| `--cwd` | string | no | Directory used to resolve the workspace root (defaults to current working directory) |

### `plexi agent call`

Call an agent role and return its validated completion or blocking question as JSON

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<role>` | string | yes |  |
| `--task` | string | yes |  |
| `--worktree` | string | no | Create a managed worktree for the child |
| `--async` | flag | no | Return the child pane ID immediately |

### `plexi agent message`

Reply through a child agent's structured mailbox

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Target pane id or name |
| `<text>` | string | yes |  |
| `--wait` | flag | no | Wait for the child's next completion or question |

### `plexi agent handoff`

Transfer the current task to a permitted successor role

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<role>` | string | yes |  |
| `--task` | string | yes |  |

### `plexi agent start`

Start an interactive agent session in a new pane from the agent registry (`<workspace>/.plexi/agents/<name>/` or global).

Resolves `<name>` by walking up from `--cwd` (or the current directory) to the project root (stopping at `$HOME`), then `~/.plexi-<channel>/agents/<name>/`, then built-in. Launches the agent's configured `[harness]` (`pi`, `claude`, `codex`, `agy`) interactively in the project directory with `AGENT.md` persona applied, the `# Plexi worker contract`, and `PLEXI_*` env set.

`agent start` without a first message and task-seeded `agent spawn` are both interactive adapter launches; only an explicitly configured Script job is one-shot. Child results report via `plexi agent done` and are collected with `plexi agent wait` without capture/send polling.

Example: plexi agent start researcher

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Agent name in `<workspace>/.plexi/agents/<name>/` or `~/.plexi-<channel>/agents/<name>/` |
| `<prompt>` | string | no | Optional initial message to send in the interactive session |
| `--message` | string | no | Optional initial message to send in the interactive session |
| `--cwd` | string | no | Working directory for workspace lookup and the interactive agent pane |

### `plexi agent spawn`

Spawn a CLI worker agent from the agent registry (`<workspace>/.plexi/agents/<name>/` or global).

Loads `<name>` through `AgentRegistry` (workspace shadowing global), refuses agents without a `[harness]` table, composes the CLI command with persona and `# Plexi worker contract`, and launches the worker pane. Codex workers (including calls and team members) use `codex --sandbox workspace-write -c sandbox_workspace_write.network_access=true`. They can write in their workspace and use the network; Plexi adds no profile writable root.

Task-seeded `agent spawn` and `agent start` without a first message are both interactive adapter launches; only an explicitly configured Script job is one-shot. Results report via `plexi agent done` and are collected with `plexi agent wait` or parent idle-batch delivery without capture/send polling.

Example: plexi agent spawn researcher --task "Audit auth module" --subcontext auth-audit --ephemeral

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Agent name in `<workspace>/.plexi/agents/<name>/` or `~/.plexi-<channel>/agents/<name>/` |
| `--task` | string | yes | Task brief passed to the spawned agent harness |
| `--subcontext` | string | no | Sub-context name to spawn into |
| `--ephemeral` | flag | no | Auto-close the pane on clean exit (`0`) so the sub-context collapses when all workers finish |
| `--from` | string | no | Parent/caller pane id or name (defaults to `$PLEXI_PANE_ID`) |
| `--cwd` | string | no | Working directory for workspace lookup and the spawned worker pane |

### `plexi agent done`

Validate a worker's structured result against its schema, write completion slots, and exit cleanly.

Validates `--result` against the pane's `result_schema` (when present), writes the `completed` and `summary` pane slots, and transitions the pane to `Idle` so an ephemeral worker can exit `0` and trigger sub-context auto-collapse.

Example: plexi agent done --result '{"status":"pass"}' --summary "Auth audit complete"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--result` | string | yes | Structured JSON result payload |
| `--summary` | string | no | Human- and master-readable completion summary written to the `summary` slot |
| `--pane-id` | string | no | Target pane id or name (defaults to `$PLEXI_PANE_ID`) |

### `plexi agent result`

Read a completed agent's persisted result (`RESULT.md` or `result.json` with `--json`).

Reads `<workspace>/.plexi/results/<pane-or-run-id>/RESULT.md` (and `result.json`), defaulting to the most recently completed agent result when `<pane>` is omitted.

Example: plexi agent result Example: plexi agent result 42 --json

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane>` | string | no | Pane ID, agent role name, or run/session ID (defaults to latest completed run) |
| `--pane` | string | no | Pane ID, agent role name, or run/session ID (alias for positional `<PANE>`) |

### `plexi agent wait`

Wait for one or more child agent panes to finish via `plexi agent done` and pull their results.

Blocks until every specified child pane has reported completion (or exited/failed) or `--timeout` expires. Results pulled via `agent wait` are consumed so the host does not re-deliver them as a duplicate idle-batch turn to the parent pane.

Example: plexi agent wait 42 43 --timeout 300

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_ids>` | string (repeatable) | yes | Child pane IDs or names to wait for |
| `--timeout` | string | no | Timeout in seconds Default: `300`. |

### `plexi agent ask`

Escalate a blocking question to the parent master agent (Tier 1 `Blocked` state).

Transitions this worker pane to `Blocked`, writes the `question` slot, and wakes the `PLEXI_PARENT_PANE` master agent waiting on `plexi pane wait` without filing a human-facing `NeedsYou` alert.

Example: plexi agent ask "Should we preserve the legacy v1 token endpoint?"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<question>` | string | yes | Question text for the parent master agent |
| `--pane-id` | string | no | Target pane id or name (defaults to `$PLEXI_PANE_ID`) |

### `plexi agent report`

Report agent state for this pane to the host.

Called internally by hook scripts. Requires PLEXI_SOCKET and PLEXI_PANE_ID to be set in the environment.

Example: plexi agent report --state working --agent claude-code

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--state` | string | yes | State to report: working, blocked, needs_you, idle, interrupted, failed, or completed |
| `--agent` | string | no | Agent name (e.g. "claude-code") Default: `unknown`. |
| `--detail` | string | no | Active tool detail (optional, from hook event JSON) |
| `--session-id` | string | no | Session ID (optional, from hook event JSON) |
| `--event` | string | no | Raw provider lifecycle event, retained separately from the UI state |
| `--blocked-reason` | string | no | Explicit blocking reason; state-only callers remain supported |

### `plexi agent status`

Show current agent state for all panes.

Queries the host for all panes that have reported agent state via hooks. Formats as a table with pane ID, agent name, state, and session ID.

Example: plexi agent status Example: plexi agent status --blocked

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--blocked` | flag | no | Show only blocked panes |
| `--working` | flag | no | Show only working panes |
| `--idle` | flag | no | Show only idle panes |

### `plexi agent history`

Show closed agent runs and sessions from `~/.plexi-<channel>/agent_history.jsonl`.

Example: plexi agent history Example: plexi agent history --limit 20 --json

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--limit` | string | no | Maximum number of recent closed runs to display (0 = all) Default: `50`. |

### `plexi agent reopen`

Reopen a closed agent run or session from `agent_history.jsonl` in its saved working directory.

Resolves the matching entry by `run_id` or harness `session_id` and spawns a new pane running the harness's resume command (`claude --resume <id>`, `codex resume <id>`, `pi --session <id>`, `agy --resume <id>`).

Example: plexi agent reopen sess-claude-42 Example: plexi agent reopen run-7-1729100000000

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<target>` | string | yes | Run ID (`run-...`) or harness session ID to reopen |
| `--subcontext` | string | no | Optional sub-context name to reopen the session into |

### `plexi agent hook`

Install or uninstall agent hook integrations.

install: patches the selected agent config with lifecycle hook registrations, routing them to plexi agent report.

uninstall: removes all PLEXI hook entries from the selected agent config.

Example: plexi agent hook install --claude-code Example: plexi agent hook install --codex --pi Example: plexi agent hook uninstall --claude-code

| Subcommand | Description |
|---|---|
| `install` | Install PLEXI agent-state hook integrations |
| `uninstall` | Remove PLEXI agent-state hook integrations |

#### `plexi agent hook install`

Install PLEXI agent-state hook integrations

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--claude-code` | flag | no | Install Claude Code hooks (PreToolUse, PostToolUse, SessionStart, UserPromptSubmit, PermissionRequest, Stop, StopFailure, SessionEnd) |
| `--codex` | flag | no | Install Codex hooks (SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, Stop) |
| `--pi` | flag | no | Install Pi extension hooks (session, agent, and tool lifecycle events) |

#### `plexi agent hook uninstall`

Remove PLEXI agent-state hook integrations

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--claude-code` | flag | no | Remove Claude Code hooks |
| `--codex` | flag | no | Remove Codex hooks |
| `--pi` | flag | no | Remove Pi extension hooks |

## `plexi context`

Manage the active context (the folder and project scope tied to the current pane)

| Subcommand | Description |
|---|---|
| `rename` | Rename a context (defaults to the calling pane's context) |
| `close` | Close a context, requesting desktop confirmation when configured |
| `park` | Park a context and move focus to an unparked neighbor |
| `unpark` | Unpark a context and focus it |
| `new` | Open a new context with an optional name |
| `sub` | Create a sub-context under the current one, pre-populated with panes |
| `open` | Switch the current pane to a context at the given path |
| `set-root` | Change the root folder for the active context |
| `current` | Print the id and name of the current pane's context as JSON |
| `describe` | Set the description for the active context |
| `zoom` | Zoom into a sub-context by its context id or name |
| `zoom-out` | Zoom out of the current sub-context to the parent |
| `push` | Push a pane into a new sub-context |
| `list` | List all open contexts as a JSON array |

### `plexi context rename`

Rename a context (defaults to the calling pane's context)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes |  |
| `--context-id` | string | no | Target context id or name (defaults to the calling pane's context) |

### `plexi context close`

Close a context, requesting desktop confirmation when configured

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<context_id>` | string | no | Target context id or name (defaults to the calling pane's context) |

### `plexi context park`

Park a context and move focus to an unparked neighbor

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<context_id>` | string | no | Target context id or name (defaults to the calling pane's context) |

### `plexi context unpark`

Unpark a context and focus it

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<context_id>` | string | no | Target context id or name (defaults to the calling pane's context) |

### `plexi context new`

Open a new context with an optional name.

Examples: plexi context new "sprint"                          # top-level context plexi context new "sprint" --parent                 # child of current context (no-focus) plexi context new "sprint" --parent=main -d         # child of "main", portal splits below plexi context new "sprint" --parent --window "echo a" --window "echo b"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | no | Name for the new context. Defaults to the directory basename |
| `--path` | string | no | Root path for the new context. Defaults to current working directory |
| `--parent` | string | no | Create as a child of a context (the new context is its sub-context). Bare `--parent` uses the current context (reads PLEXI_CONTEXT_NAME from env); use `--parent=<name>` to target another context by name |
| `--window` | string (repeatable) | no | Command to run in each pre-populated window. Repeatable |
| `--focus` | flag | no | Focus (zoom into) the new sub-context after creation. Default: stay in current pane |
| `--from` | string | no | Pane id or name to anchor the portal split at (requires --parent). Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the parent context's focused pane |
| `--down` / `-d` | flag | no | Split portal below instead of right (requires --parent) |
| `--left` / `-l` | flag | no | Split portal left (requires --parent) |
| `--up` / `-u` | flag | no | Split portal above (requires --parent) |
| `--right` / `-r` | flag | no | Split portal right — explicit (default, requires --parent) |

### `plexi context sub`

Create a sub-context under the current one, pre-populated with panes.

One command spins up a scoped squad: N panes in a single tiled window inside the new sub-context, each running the same command (or its own). Unlike `context new --parent --window`, this creates exactly N panes — no spare terminal — and roots them at the caller's cwd.

Examples: plexi context sub agentsquad --agents 3 --command cm plexi context sub review --agents 2 --command "cm review" --command "cm test" plexi context sub build --agents 4 --layout columns --focus

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Name for the new sub-context |
| `--path` | string | no | Root path for the sub-context. Defaults to the caller's cwd |
| `--agents` | string | no | Number of panes to open inside the new sub-context Default: `1`. |
| `--command` | string (repeatable) | no | Command to launch in each pane. Give it once to apply to every pane, or exactly --agents times to give each pane its own command |
| `--layout` | string | no | How the panes are arranged inside the sub-context's window. Defaults to `tiled` when the pane count fills a square grid (4, 9, …) and `columns` otherwise so no grid cell is left empty |
| `--focus` | flag | no | Zoom into the new sub-context after creation. Default: stay put |
| `--from` | string | no | Pane id or name to anchor the portal split at. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the parent's focused pane |

### `plexi context open`

Switch the current pane to a context at the given path

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no |  |

### `plexi context set-root`

Change the root folder for the active context

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no |  |

### `plexi context current`

Print the id and name of the current pane's context as JSON

### `plexi context describe`

Set the description for the active context

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<text>` | string | yes | Description text |

### `plexi context zoom`

Zoom into a sub-context by its context id or name

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<context_id>` | string | yes | Target context id or name |

### `plexi context zoom-out`

Zoom out of the current sub-context to the parent

### `plexi context push`

Push a pane into a new sub-context.

Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | no | Name for the new sub-context. Defaults to the pane name |
| `--pane-id` | string | no | Pane id or name to push. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane |

### `plexi context list`

List all open contexts as a JSON array

## `plexi app`

Manage your Plexi apps — open, install, list, scaffold, and inspect

| Subcommand | Description |
|---|---|
| `open` | Open an app or tool in a new pane |
| `trust` | Pre-approve a raw `.wasm` component's host imports without a prompt |
| `install` | Install an app from a local path, a remote source, or a pack file |
| `uninstall` | Remove an installed app by id |
| `list` | Show all installed apps with their versions |
| `prune` | Report obsolete first-party pre-v3 apps that launch-time reseeding quarantines |
| `render` | Render an app headlessly (JSON frame tree by default, or PNG with --png) |
| `check` | Check a local app with manifest, scaffold metadata, SDK, and render-size checks |
| `test` | Run an app's AppHarness tests with `uv run --with pytest pytest tests/` |
| `info` | Show details about an installed app: manifest fields and tool status |
| `state` | Read or replace a file-backed app's state document |
| `init` | Create a new app from a template, or adopt an existing app directory |
| `validate` | Check a Plexi app directory or .plexipkg package for errors before publishing or installing |
| `inspect` | Show the trust sheet for a local app directory or .plexipkg package |
| `package` | Build a distributable .plexipkg package from an app directory |
| `freeze` | Export your currently installed apps as a single TOML snapshot for sharing or backup |
| `publish` | Validate, package, and submit an app to the Plexi marketplace |
| `browse` | Browse every public app in the hosted marketplace |
| `search` | Search the public marketplace catalog |
| `update` | Pull git-backed installed apps to their latest source revision |
| `action` | Send a semantic action to a running app pane |
| `call` | Call a tool an app exposes and print its JSON result |
| `tools` | Snapshot the live tool registry exposed by running app panes in the current context |

### `plexi app open`

Open an app or tool in a new pane.

Pass an app id (e.g. `plexi app open calc`) or a path to an app directory containing a manifest.toml. Use `--mcp` to wrap an MCP server, or `--cli` to open any CLI tool with a Plexi UI.

Default placement is a sibling split to the right — the calling pane is never taken over. Pass a direction flag (--down/--left/--up/--right), --tab, or --window to override; the app's manifest `[launch] placement` applies when no flag is given.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<type_id>` | string | no | App id or path to open (mutually exclusive with --mcp and --cli) |
| `--mcp` | string (repeatable) | no | Explore a configured stdio MCP server in a Plexi pane.  Example: plexi app open --mcp filesystem |
| `--cli` | string | no | Wrap a CLI tool in a Plexi pane with a visual UI.  Example: plexi app open --cli git |
| `--down` / `-d` | flag | no | Split below |
| `--left` / `-l` | flag | no | Split left |
| `--up` / `-u` | flag | no | Split up |
| `--right` / `-r` | flag | no | Split right |
| `--tab` | flag | no | New tab |
| `--window` | flag | no | New window |
| `--split` | string | no | Split direction or placement (down, left, up, right, tab, window) |
| `--from` | string | no | Open the new pane relative to this pane id or name. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane |
| `<extra_args>` | string (repeatable) | no | Extra arguments passed through to the app (only valid with an app id) |

### `plexi app trust`

Pre-approve a raw `.wasm` component's host imports without a prompt.

The non-interactive form of the review `plexi app open <file.wasm>` runs at a TTY: it persists Green grants for every host interface the component imports, scoped to the wasm's parent directory — the same store and scope an installed host checks at open time. Meant for automation (release gates, scene runners) pre-approving a vetted, repo-committed fixture so an installed host opens it without a human at a terminal. It never opens a pane; production interactive review is unchanged.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Path to the `.wasm` component to trust |

### `plexi app install`

Install an app from a local path, a remote source, or a pack file.

Local path: `plexi app install ./my-app` — copies the app dir into Plexi's store. Remote source: `plexi app install github:owner/repo` — fetches and installs from GitHub. Pack file: `plexi app install --pack core` — installs from a pack file or the built-in core pack. Workspace pack: `plexi app install` (no args) — installs from `<repo>/.plexi-<channel>/apps.toml` (or `<repo>/.plexi/apps.toml` on stable).

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<spec_or_path>` | string | no | Source to install: a local path, GitHub spec (github:owner/repo), or bare app id. Omit to install from the workspace pack (`<repo>/.plexi-<channel>/apps.toml`, or `<repo>/.plexi/apps.toml` on stable) |
| `--pack` | string | no | Install from a pack file or 'core' |
| `--refresh` | flag | no | With --pack: re-extract already-installed `local:` apps from this binary's embedded tree, replacing the installed copy. The update path for bundled core apps on stable channels |
| `--yes` / `-y` | flag | no | Skip the trust-sheet confirmation prompt. Required for non-interactive (scripted) installs — without a terminal the install fails closed instead of proceeding silently |

### `plexi app uninstall`

Remove an installed app by id.

Example: plexi app uninstall github-tree

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | App id to remove (use `plexi app list` to see installed ids) |
| `--yes` / `-y` | flag | no | Skip the confirmation prompt |

### `plexi app list`

Show all installed apps with their versions

### `plexi app prune`

Report obsolete first-party pre-v3 apps that launch-time reseeding quarantines

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--dry-run` | flag | no | Show candidates without removing anything |

### `plexi app render`

Render an app headlessly (JSON frame tree by default, or PNG with --png)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<app>` | string | yes | App id or local path to render (e.g. "calc" or "./my-app") |
| `--size` | string | no | Image dimensions as WxH (e.g. 500x500) Default: `800x600`. |
| `--state` | string | no | Pre-seed the app's state from a JSON file before rendering |
| `--output` | string | no | Where to save the output (default: stdout) |
| `--png` | flag | no | Render to a PNG image instead of JSON (default: JSON) |

### `plexi app check`

Check a local app with manifest, scaffold metadata, SDK, and render-size checks.

This is the compiler-like gate for generated Plexi apps. It checks the manifest, warns on missing or stale `plexi.scaffold.toml`, inspects Python SDK usage without importing app code, and renders the app at small and normal pane sizes. Run it with an explicit alpha or PR channel so the SDK/profile under test is not ambient.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | Local app directory to check (default: current directory) Default: `.`. |
| `--size` | string (repeatable) | no | Render size to check as WxH. Repeat to override the default matrix |
| `--png-dir` | string | no | Write PNG snapshots for each checked size into this directory |

### `plexi app test`

Run an app's AppHarness tests with `uv run --with pytest pytest tests/`.

Runs the Python tests in the app's `tests/` directory (the `tests/test_app.py` scaffolded by `plexi app init`). AppHarness spawns the app as a real subprocess and checks it renders without overlap; see `plexi_sdk/testing.py`. Exits nonzero on failure so CI can gate on it.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | App directory to test (default: current directory) Default: `.`. |
| `--snapshot` | flag | no | Update stored snapshots instead of comparing against them |

### `plexi app info`

Show details about an installed app: manifest fields and tool status.

Populates `tools` from the live tool registry when a pane of this app is running in the current context; otherwise labels the section as manifest-only (or static source scan) so an unopened app is never misreported as declaring no tools. Run `plexi app tools` to snapshot all live tools in the current context.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes |  |

### `plexi app state`

Read or replace a file-backed app's state document.

Only apps that declare a `[state]` section are addressable. The state path is resolved from the manifest and the calling context — callers never pass a path, and there is no flag to address another context.

| Subcommand | Description |
|---|---|
| `get` | Print an app's current state document to stdout |
| `set` | Replace an app's state document, reading from a file or stdin |

#### `plexi app state get`

Print an app's current state document to stdout

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<app>` | string | yes | App id (from `plexi app list`) |
| `--scope` | string | no | Which declared scope to read. Defaults to the app's first declared scope; a scope the app did not declare is an error |

#### `plexi app state set`

Replace an app's state document, reading from a file or stdin

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<app>` | string | yes | App id (from `plexi app list`) |
| `<file>` | string | no | File to read the new document from. Reads stdin when omitted |
| `--scope` | string | no | Which declared scope to write. Defaults to the app's first declared scope; a scope the app did not declare is an error |

### `plexi app init`

Create a new app from a template, or adopt an existing app directory.

Scaffolds the folder structure and files you need to build a Plexi app: manifest.toml, main.py, tests/test_app.py, AGENTS.md, .gitignore, and plexi.scaffold.toml drift metadata.

Pass `--adopt <dir>` (or `--adopt` inside an app directory) to write or refresh `plexi.scaffold.toml` for a pre-existing app without creating or modifying any source files.

By default, a newly scaffolded app is placed in your workspace's app directory. If no workspace is detected, pass --global to scaffold into the global registry.

Use --open to launch it in a split-right pane after scaffolding. See `sdk/python/AUTHORING.md` for the canonical authoring guide.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | no |  |
| `--adopt` | flag | no | Write or refresh `plexi.scaffold.toml` for an existing app directory without altering source files |
| `--wasm` | string | no | Create a Rust WASM component app with the Plexi WASM SDK |
| `--lang` | string | no | App language/template: `python` (declarative UI). Use `--wasm <name>` for a Rust WASM component Default: `python`. |
| `--global` | flag | no | Scaffold into the global app registry instead of the workspace |
| `--open` | flag | no | Open the app in a split-right pane after scaffolding |
| `--no-open` | flag | no | Deprecated compatibility flag. App init no longer opens by default |
| `--from` | string | no | Open the new pane relative to this pane id or name. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane |

### `plexi app validate`

Check a Plexi app directory or .plexipkg package for errors before publishing or installing.

A directory is validated in place. A `.plexipkg` file is extracted to a temp dir with path-safety checks and verified end-to-end: descriptor, content hashes, manifest, entry point, and capability strings.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | App directory or .plexipkg file to check (default: current directory) Default: `.`. |

### `plexi app inspect`

Show the trust sheet for a local app directory or .plexipkg package.

Validates first (fail-closed), then prints what the app is, what runtime it uses with a blunt trust label, and every capability it declares — the same sheet shown before `plexi app install` proceeds.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | App directory or .plexipkg file to inspect |

### `plexi app package`

Build a distributable .plexipkg package from an app directory.

Validates the directory first (fail-closed), then writes `<id>-<version>.plexipkg` containing the app files plus a generated PACKAGE.toml with per-file sha256 checksums.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | App directory to package |
| `--out` | string | no | Output file path (default: ./<id>-<version>.plexipkg) |

### `plexi app freeze`

Export your currently installed apps as a single TOML snapshot for sharing or backup.

Like `pip freeze` — captures exactly what's installed so you can replay it later with `plexi app install`.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Destination path for the TOML snapshot file |

### `plexi app publish`

Validate, package, and submit an app to the Plexi marketplace.

Reads the `[marketplace]` manifest section (publisher, visibility, price), validates the directory, builds a `.plexipkg`, and submits it. Without a configured `[marketplace].submit_url` the package is prepared locally but not uploaded — the artifact path is printed.

> **Beta-gated:** Marketplace publishing and catalog browsing are beta surfaces. This reference is included for beta and worktree testing; it is not available from the stable v1 channel.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | App directory to publish (default: current directory) Default: `.`. |

### `plexi app browse`

Browse every public app in the hosted marketplace

> **Beta-gated:** Marketplace publishing and catalog browsing are beta surfaces. This reference is included for beta and worktree testing; it is not available from the stable v1 channel.

### `plexi app search`

Search the public marketplace catalog

> **Beta-gated:** Marketplace publishing and catalog browsing are beta surfaces. This reference is included for beta and worktree testing; it is not available from the stable v1 channel.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<query>` | string | yes | Substring matched against app id, name, description, and tags |

### `plexi app update`

Pull git-backed installed apps to their latest source revision.

Canonical app update command. Resolves workspace-local apps when run inside a workspace, and skips installed apps that are not git checkouts.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | no | App id to update (omit to update all installed apps visible here) |

### `plexi app action`

Send a semantic action to a running app pane.

Unlike `pane command` (which sends raw text), `app action` delivers a structured semantic event directly to the app's event handler — no keystroke simulation.

Example: plexi app action 42 refresh Example: plexi app action 42 navigate-to /some/path

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id or name of the target app pane (from `plexi pane list`) |
| `<action>` | string | yes | Action name to invoke (e.g. "refresh", "navigate-to", "add-item") |
| `<args>` | string (repeatable) | no | Optional arguments forwarded to the action handler |

### `plexi app call`

Call a tool an app exposes and print its JSON result.

Runs through the same tool dispatcher the Assistant uses, scoped to the calling pane's context (the active context when run outside a pane). The host stamps the caller identity from the pane credential or peer ancestry (`pane:<id>`). A missing pane is never the human `user`. The app sees that identity, never one taken from the input. Exits 1 with `error: <message>` when the tool or the app rejects the call.

Example: plexi app call chess chess.state Example: plexi app call chess chess.play --input '{"game_id":"game-1","expected_revision":0,"operation_id":"op-1","move":"e2e4"}'

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<app_id>` | string | yes | App id that exposes the tool (from `plexi app info`) |
| `<tool>` | string | yes | Tool name as the app declares it (e.g. `chess.state`) |
| `--input` | string | no | Tool input as a JSON object Default: `{}`. |
| `--pane` | string | no | Pane id or name that owns the tool when more than one instance of the app is open |
| `--wait-approved` | flag | no | Wait up to two minutes for desktop approval, then execute once |

### `plexi app tools`

Snapshot the live tool registry exposed by running app panes in the current context.

Reads the same context-scoped `ToolDispatcher` registry that agents and `plexi app call` see: exposed tool name (including `<name>@<pane_id>` disambiguation when multiple panes expose the same tool), owning app id and pane id, description, and full JSON input/output schemas.

Example: plexi app tools Example: plexi app tools --json Example: plexi app tools --app calc --json

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--app` | string | no | Filter to tools exposed by this app id |
| `--context` | string | no | Inspect a specific context id or name instead of the caller's context |

## `plexi account`

Optional account that links this desktop to relay and cloud features.

Free apps install without an account. Login requires the accounts backend enabled (`[marketplace].account_backend = "plexi"`); otherwise it fails closed with a clear message.

> **Beta-gated:** Marketplace account management is a beta surface. This reference is included for beta and worktree testing; it is not available from the stable v1 channel.

| Subcommand | Description |
|---|---|
| `status` | Show whether you are logged in |
| `login` | Log in to your marketplace account via emailed sign-in link |
| `logout` | Log out and clear the local session |

### `plexi account status`

Show whether you are logged in

> **Beta-gated:** Marketplace account management is a beta surface. This reference is included for beta and worktree testing; it is not available from the stable v1 channel.

### `plexi account login`

Log in to your marketplace account via emailed sign-in link.

Runs the device-code flow: plexiapp.com emails a link, you click it in any browser, and the CLI stores the session. Magic-link login creates the account on first use — there is no separate signup.

> **Beta-gated:** Marketplace account management is a beta surface. This reference is included for beta and worktree testing; it is not available from the stable v1 channel.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--email` | string | no | Account email (falls back to [marketplace].account_email in config) |

### `plexi account logout`

Log out and clear the local session

> **Beta-gated:** Marketplace account management is a beta surface. This reference is included for beta and worktree testing; it is not available from the stable v1 channel.

## `plexi registry`

Watch installed CLI tools for changes to their available commands and options

| Subcommand | Description |
|---|---|
| `watch` | Check installed CLI tools for changes to their help output and update Plexi's knowledge of them |

### `plexi registry watch`

Check installed CLI tools for changes to their help output and update Plexi's knowledge of them

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<cli>` | string | no | Only check this one CLI tool instead of all of them |

## `plexi pane`

Control panes — list, focus, send input, capture output, and more.

Driving an agent in another pane (two commands, no sleeps, no capture polling): 1. Boot: `ID=$(plexi pane new --agent <tier-alias> -n "<name>" --no-focus)` Blocks until the agent TUI is idle; a printed id means ready-to-brief. 2. Brief: `plexi pane send "$ID" "<brief>" --submit` Settles the pane, presses Enter, and exits 0 only once the host confirms the prompt left the input line. 3. Wait: `plexi pane wait "$ID" --until idle --timeout 300` (or `plexi pane slot wait <slot> --pane-id "$ID" --until '<regex>'`)

For an ordinary shell command in a terminal pane, use `plexi pane command <id> "<cmd>" --enter`.

| Subcommand | Description |
|---|---|
| `hide` | Hide a pane (defaults to the calling pane) |
| `reveal` | Reveal a hidden pane (defaults to the calling pane) |
| `zoom` | Focus a pane and toggle fullscreen zoom, or enter its sub-context |
| `swap` | Focus a pane and swap it with a directional neighbor |
| `wait` | Wait for current idle, blocked, or exited state; print its lifecycle record. Exit 0 on match, 2 on timeout, 1 on invalid predicate or plumbing failure |
| `events` | Follow brokered pane lifecycle records as NDJSON until interrupted |
| `new` | Open a terminal pane, the Agents pane (`agents`), or launch an agent with `--agent` |
| `name` | Rename a pane |
| `list` | List all open panes as a JSON array |
| `tree` | Show the parent/child session provenance forest across open panes (`pstree` / `ps --forest` style) |
| `focus` | Move the visible focus to a specific pane |
| `heartbeat` | Configure a host-owned recurring prompt for a terminal pane |
| `close` | Close a pane. Omit the pane id to close the pane you are currently in |
| `send` | Type text into another pane as if it came from the keyboard, or pass `--submit` for host-confirmed prompt submission |
| `self` | Print the id of the pane you are currently in |
| `info` | Print details about a pane (default: the current pane, or the previously focused pane) as JSON |
| `capture` | Capture the last N lines of a pane's output as a JSON array |
| `status` | Report one composite working, idle, blocked, or unknown verdict as JSON |
| `key` | Send a key press to a pane |
| `drop` | Drop a local file or image URL onto a pane |
| `click` | Inject a synthetic pointer click into an app pane, for driving canvas interaction without OS-level automation |
| `drag` | Drag the pointer across an app pane through the production input path: press, N intermediate moves, release — delivered one frame at a time |
| `command` | Send an ordinary shell command to a terminal pane as if typed from the keyboard |
| `state` | Return the current UI state of a pane as JSON |
| `slot` | Manage host-managed named file slots for a pane |
| `message` | Send and receive structured messages via a host-managed per-pane FIFO mailbox |
| `result` | Read a pane's validated structured result |

### `plexi pane hide`

Hide a pane (defaults to the calling pane)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Target pane id or name (defaults to the calling pane) |

### `plexi pane reveal`

Reveal a hidden pane (defaults to the calling pane)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Target pane id or name (defaults to the calling pane) |

### `plexi pane zoom`

Focus a pane and toggle fullscreen zoom, or enter its sub-context

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Target pane id or name (defaults to the calling pane) |

### `plexi pane swap`

Focus a pane and swap it with a directional neighbor

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<direction>` | string | yes |  |
| `--pane-id` | string | no | Target pane id or name (defaults to the calling pane) |

### `plexi pane wait`

Wait for current idle, blocked, or exited state; print its lifecycle record. Exit 0 on match, 2 on timeout, 1 on invalid predicate or plumbing failure

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Target pane id or name |
| `--until` | string | yes | Predicate: idle, blocked, or exited (any exit, including unknown status) |
| `--timeout` | string | no | Host deadline in seconds; must be finite and positive Default: `300`. |

### `plexi pane events`

Follow brokered pane lifecycle records as NDJSON until interrupted

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--follow` | flag | yes |  |
| `--pane` | string | no | Limit deliveries to one pane id or name in the caller's allowed context |

### `plexi pane new`

Open a terminal pane, the Agents pane (`agents`), or launch an agent with `--agent`.

`--agent <tier-alias>` is the supported way to launch an agent into a fresh pane: the host runs the command in the new pane and prints the pane id only once the agent reports idle, so a printed id on stdout means ready-to-brief with `plexi pane send <id> "<brief>" --submit` (two commands, no sleeps, no capture polling).

Examples: plexi pane new --agent c-large -n "worker" --no-focus   # agent pane, id printed once ready-to-brief plexi pane new --worktree feature/0668-test             # terminal in isolated wtp worktree plexi pane new                                          # empty terminal, split right plexi pane new "npm run dev" -n "dev"                   # terminal with command, named plexi pane new -d                                       # split below

For apps use `plexi app open`. For MCP servers use `plexi app open --mcp`.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<cmd>` | string | no | Shell command to run, or `agents` to open the built-in Agents pane |
| `--agent` | string | no | Launch an agent in the new pane and block until it reports ready-to-brief.  The value is a shell command — normally a size-tier alias such as `c-large` or `codex-small` — run in the new pane verbatim. The pane id is printed only once the agent has booted, so a successful id on stdout means the pane is ready to receive a brief via `plexi pane send <id> "<brief>" --submit`. On boot timeout the reason and the created pane id go to stderr, stdout stays empty, and the exit code is 2 so the caller can close the pane.  Requires a running Plexi host (PLEXI_SOCKET). |
| `--boot-timeout` | string | no | How long to wait for the agent to report ready, in seconds. Defaults to 60 when omitted (the host owns the default) |
| `--name` / `-n` | string | no | Name the pane |
| `--down` / `-d` | flag | no | Split below instead of right |
| `--left` / `-l` | flag | no | Split left |
| `--up` / `-u` | flag | no | Split up |
| `--right` / `-r` | flag | no | Split right (explicit, same as default) |
| `--tab` | flag | no | New tab |
| `--window` | flag | no | New window |
| `--overlay` | flag | no | Overlay pane |
| `--from` | string | no | Pane id or name to split relative to. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane |
| `--ephemeral` / `-e` | flag | no | Close the pane when the command finishes |
| `--no-focus` | flag | no | Keep focus on the current pane |
| `--cwd` | string | no | Working directory |
| `--worktree` | string | no | Create or check out an isolated git worktree for <BRANCH> via `wtp` and start the pane inside it. When the pane closes, the host automatically reclaims the worktree if clean and unmoved, or retains it and reports its path if dirty or ahead |
| `--result-schema` | string | no | Declare a JSON Schema (inline JSON object or path to a `.json` file) that the pane's structured return (`plexi pane result write`) must satisfy before the host accepts it |
| `--on-load` | string (repeatable) | no | Chained steps to run sequentially after the pane opens (reuses the command step runner) |
| `<extra_args>` | string (repeatable) | no | Extra arguments forwarded to the command (for example after `--`) |

### `plexi pane name`

Rename a pane.

With one argument, renames the current pane: plexi pane name "My Project" With two arguments, renames any pane by id or name: plexi pane name 42 "My Project"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<first>` | string | yes | Pane id or name (from `plexi pane list`) or the new name if renaming the current pane |
| `<second>` | string | no | New name when a pane id or name is given as the first argument |

### `plexi pane list`

List all open panes as a JSON array.

Filter by context: `--context` (no value) returns panes in the caller's context (reads PLEXI_CONTEXT_ID from env). `--context <id-or-name>` filters to a specific context.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--context` | string | no | Filter by context id or name. With no argument, reads PLEXI_CONTEXT_ID from env (caller's context). With an argument, returns panes in that specific context |

### `plexi pane tree`

Show the parent/child session provenance forest across open panes (`pstree` / `ps --forest` style).

Displays each root pane and its spawned child panes with `spawned_by`, agent role, state, and title. Pass `--json` to output the nested forest as JSON.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--context` | string | no | Filter by context id or name. With no argument, reads PLEXI_CONTEXT_ID from env (caller's context) |

### `plexi pane focus`

Move the visible focus to a specific pane.

This moves what the user sees on screen — it does not change which pane an agent is running in. An agent calling this from pane A remains in pane A; the user just sees pane B highlighted.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id or name to focus (from `plexi pane list`) |
| `--back` | flag | no | Step back through pane focus history |
| `--forward` | flag | no | Step forward through pane focus history |

### `plexi pane heartbeat`

Configure a host-owned recurring prompt for a terminal pane.

Example: plexi pane heartbeat 42 --every 5m --text "cycle"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id or name to prompt (from `plexi pane list`) |
| `--every` | string | no | Interval such as 30s, 5m, or 1h |
| `--text` | string | no | Prompt text submitted on each beat |
| `--while-idle-only` | flag | no | Only submit when the shared agent detector reports idle (default) |
| `--off` | flag | no | Disable the pane heartbeat |

### `plexi pane close`

Close a pane. Omit the pane id to close the pane you are currently in.

If the pane was created with `plexi pane new --worktree <BRANCH>`, the host automatically reclaims the worktree via `wtp` when clean and unmoved, or retains it and prints the retained worktree path when dirty or ahead.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id or name to close (from `plexi pane list`). Defaults to the current pane if not given |

### `plexi pane send`

Type text into another pane as if it came from the keyboard, or pass `--submit` for host-confirmed prompt submission.

Pass `--submit` (`-s`) when briefing an agent or submitting a prompt in a terminal pane: the host settles the pane, presses Enter (re-sending once if the text was parked as a collapsed paste), and exits 0 only once the prompt has left the input line — no `sleep` or `pane key <id> enter` needed.

Without `--submit`, terminal panes receive raw PTY bytes and app panes receive one egui text input event after the host focuses the target pane. For an ordinary shell command, see also `plexi pane command <id> "<cmd>" --enter`.

Examples: plexi pane send 42 "Read the brief and implement the fix" --submit plexi pane send 42 "partial text"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id or name to send text to (from `plexi pane list`) |
| `<text>` | string | yes | Text to type into the pane (prefer `--submit` to submit with confirmation) |
| `--submit` / `-s` | flag | no | Press Enter for you and wait until the host confirms the prompt left the pane's input line.  The host settles the pane, submits, and re-sends Enter once if the text is still parked as a collapsed paste. Exits 0 only when the submission is confirmed; on an unconfirmed submit it exits non-zero and prints the observed input line to stderr. Terminal panes only. |
| `--paste` | flag | no | Paste literal text, including newlines, as one edit. App panes only; does not read or change the system clipboard |

### `plexi pane self`

Print the id of the pane you are currently in.

Useful in scripts: MY_PANE=$(plexi pane self)

### `plexi pane info`

Print details about a pane (default: the current pane, or the previously focused pane) as JSON.

Example: plexi pane info 42

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id or name to describe. Defaults to the current pane (PLEXI_PANE_ID) |
| `--previous` | string | no | Return info for a previously focused pane. With no value, returns the immediately previous pane (step 1). Provide an integer N to walk back N steps in focus history.  Examples: plexi pane info --previous      # pane focused 1 step ago plexi pane info --previous 3    # pane focused 3 steps ago |

### `plexi pane capture`

Capture the last N lines of a pane's output as a JSON array.

Defaults to the current pane when no pane id is given.

Example: plexi pane capture --lines 50 42

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id or name to capture output from. Defaults to the current pane |
| `--lines` | string | no | How many lines to read from the end of the output Default: `50`. |
| `--full-output` | flag | no | Preserve trailing empty lines (by default they are stripped) |
| `--from-cursor` | string | no | Read only lines written after this cursor value. Get the cursor from a previous capture response. When set, the response is always JSON object format |
| `--plain` | flag | no | Print only captured lines on stdout. The next cursor is printed to stderr |

### `plexi pane status`

Report one composite working, idle, blocked, or unknown verdict as JSON

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id or name to inspect |

### `plexi pane key`

Send a key press to a pane.

For terminal panes, injects the keystroke into the terminal. For app panes, delivers a structured key event.

To submit a prompt or command with host confirmation, prefer `plexi pane send <id> "<text>" --submit` (or `plexi pane command <id> "<cmd>" --enter`) instead of chaining `pane send` + `pane key <id> enter`: `--submit` settles the pane, handles collapsed-paste Enter, and exits 0 only once the host confirms the input line cleared.

Key formats: single character ("h"), named key ("enter", "escape", "space", "up", "down", "left", "right", "backspace", "plus", "minus", "equals"), or chord ("ctrl+c").

Example: plexi pane key 42 escape

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id or name to send the key to (from `plexi pane list`) |
| `<key>` | string | yes | Key to press |

### `plexi pane drop`

Drop a local file or image URL onto a pane

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Target pane id or name |
| `<path_or_url>` | string | yes |  |

### `plexi pane click`

Inject a synthetic pointer click into an app pane, for driving canvas interaction without OS-level automation.

Two mutually exclusive targeting modes:

PANE-PIXEL coordinates (origin at the pane's top-left) — the honest primitive, since it exercises the same fit=contain/fit=fill `canvas_transform` inversion a real click goes through. The host injects a real pointer move + press + release into the live egui pass, never a parallel resolver.

`--node <node_id>` — targets a specific Button/TextInput/ListView node by the id `plexi pane state` reports, so a caller never has to compute pixel geometry. The host resolves the node's on-screen rect during the next render pass and fails loudly if the id is missing or not an interactive role.

Only app panes accept clicks today.

Examples: plexi pane click 42 120 80 plexi pane click 42 --node 5

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id or name to click (from `plexi pane list`) |
| `<x>` | string | no | X offset in pane pixels from the pane's top-left corner |
| `<y>` | string | no | Y offset in pane pixels from the pane's top-left corner |
| `--node` | string | no | Node id to click (from `plexi pane state`), instead of pixel coordinates |
| `--button` | string | no | Pointer button: "left", "right", or "middle" Default: `left`. |

### `plexi pane drag`

Drag the pointer across an app pane through the production input path: press, N intermediate moves, release — delivered one frame at a time.

Endpoints are pane-pixel coordinates ("x,y" from the pane's top-left) or semantic node ids from `plexi pane state` (the drag targets the node's rendered bounds center).

Example: plexi pane drag 42 --from 40,120 --to 260,120 --steps 12

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id or name to drag in (from `plexi pane list`) |
| `--from` | string | no | Start position "x,y" in pane pixels |
| `--from-node` | string | no | Start node id (from `plexi pane state`) |
| `--to` | string | no | End position "x,y" in pane pixels |
| `--to-node` | string | no | End node id (from `plexi pane state`) |
| `--steps` | string | no | Intermediate pointer moves between press and release (max 256) Default: `8`. |
| `--button` | string | no | Pointer button: "left", "right", or "middle" Default: `left`. |

### `plexi pane command`

Send an ordinary shell command to a terminal pane as if typed from the keyboard.

`--enter` submits it through the same host-confirmed settle -> Enter -> confirm sequence as `pane send --submit` (see that command for the full contract), rather than racing a raw newline against the shell's line editor. Exits 0 only once the host confirms the command was submitted; on an unconfirmed submit it exits non-zero and prints the observed input line to stderr. Terminal panes only.

This verb is scoped to ordinary shell commands. Driving an interactive TUI belongs to `plexi pane send --submit`; booting another agent belongs to `plexi pane new --agent`.

Example: plexi pane command 42 "git status" --enter

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id or name to send the command to (from `plexi pane list`) |
| `<text>` | string | yes | Text to send to the pane |
| `--enter` / `-e` | flag | no | Submit the command through the host-confirmed settle -> Enter -> confirm sequence, instead of just typing it |

### `plexi pane state`

Return the current UI state of a pane as JSON.

For app panes: returns a versioned, runtime-neutral `semantic` tree. Process apps also retain the compatible `frame` RenderCommand array. Agents can use the semantic nodes to inspect what any app runtime is currently displaying.

For terminal panes: returns a simple status object (type, title, pane_id).

Every response also carries `claimed_state` (the pane's own `status` slot — an agent-authored claim, with its step token embedded in the value by convention, or `null` if never written) and `observed_state` (host-derived: process alive, seconds since last output, child pids, current foreground command) — always together, never one without the other. Pass `--stale-after <secs>` to additionally raise a typed `stale_claim` when a claim exists but the host has observed no output within that many seconds; omit it and `stale_claim` stays `null` with no evaluation performed.

Example: plexi pane state 42 --stale-after 300

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id or name to query (from `plexi pane list`) |
| `--stale-after` | string | no | Raise `stale_claim` when the claimed state looks fresh but no output has been observed within this many seconds. Omitted by default — no stale evaluation runs unless this is set |

### `plexi pane slot`

Manage host-managed named file slots for a pane

| Subcommand | Description |
|---|---|
| `write` | Write bytes to a named pane slot. If content is omitted, stdin is read fully |
| `read` | Print raw bytes from a named pane slot |
| `wait` | Block until a slot's value matches a pattern, then print it |
| `list` | List slots for a pane as JSON |
| `delete` | Delete a named pane slot |

#### `plexi pane slot write`

Write bytes to a named pane slot. If content is omitted, stdin is read fully

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Slot name |
| `<content>` | string | no | Optional content. If omitted, stdin is read fully |
| `--pane-id` | string | no | Pane id or name. Defaults to PLEXI_PANE_ID |
| `--append` | flag | no | Append to an existing slot instead of replacing it |
| `--replace` | flag | no | Replace an existing slot |

#### `plexi pane slot read`

Print raw bytes from a named pane slot

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Slot name |
| `<pane_id>` | string | no | Pane id or name (positional form). Defaults to PLEXI_PANE_ID |
| `--pane-id` | string | no | Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID |

#### `plexi pane slot wait`

Block until a slot's value matches a pattern, then print it.

Exits 0 on a match, 2 on timeout (nothing on stdout), 1 on error. A slot whose current value already matches returns immediately.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Slot name |
| `<pane_id>` | string | no | Pane id or name (positional form). Defaults to PLEXI_PANE_ID |
| `--pane-id` | string | no | Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID |
| `--until` | string | yes | Regex the slot value must match |
| `--timeout` | string | no | Seconds to wait before giving up Default: `300`. |

#### `plexi pane slot list`

List slots for a pane as JSON

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id or name (positional form). Defaults to PLEXI_PANE_ID |
| `--pane-id` | string | no | Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID |

#### `plexi pane slot delete`

Delete a named pane slot

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Slot name |
| `<pane_id>` | string | no | Pane id or name (positional form). Defaults to PLEXI_PANE_ID |
| `--pane-id` | string | no | Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID |

### `plexi pane message`

Send and receive structured messages via a host-managed per-pane FIFO mailbox

| Subcommand | Description |
|---|---|
| `send` | Enqueue a message in a target pane's FIFO mailbox |
| `recv` | Drain all queued messages from a pane's mailbox as a JSON array |

#### `plexi pane message send`

Enqueue a message in a target pane's FIFO mailbox.

Sender identity (`from_pane_id`) is stamped host-side from the socket peer process ancestry and is never self-reported. Prints a JSON ack with `status` (`"queued"` or `"delivered"`), `message_id`, `queue_len`, and `woke`.

Pass `--submit` to wake an idle target agent with a short notice to run `plexi pane message recv` without interrupting a busy agent.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Target pane id or name (from `plexi pane list`) |
| `<payload>` | string | yes | Message payload to enqueue |
| `--submit` | flag | no | Wake the target agent if idle (never interrupts a busy agent) |

#### `plexi pane message recv`

Drain all queued messages from a pane's mailbox as a JSON array.

Defaults to `PLEXI_PANE_ID`. With `--timeout N` or `--wait`, blocks host-side with zero polling until at least one message is in the mailbox or `--timeout` expires (exits 0 with the drained JSON array, 2 on timeout, 1 on error).

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id or name (positional form). Defaults to PLEXI_PANE_ID |
| `--pane-id` | string | no | Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID |
| `--wait` | flag | no | Block host-side until at least one message arrives or `--timeout` expires |
| `--timeout` | string | no | Block for up to N seconds before exiting 2. --wait alone uses 300 seconds |

### `plexi pane result`

Read a pane's validated structured result.

Spawners declare the JSON Schema at spawn (`plexi pane new --result-schema <schema>`). The worker agent returns its structured JSON once via `plexi agent done --result <json>`: the host validates it against the declared schema synchronously and rejects malformed or schema-violating payloads back to the agent (exit 1 with the validation error) so the agent can self-correct before any consumer sees it.

Reading via `plexi pane result <id>` (or `plexi pane result read <id>`) prints the validated JSON object to stdout and exits 0, or exits 2 with `error_code: "no_result_yet"` when no result has been written yet (or on `--wait` timeout), or exits 1 with `error_code: "invalid_result"` when the last attempted write was rejected and no valid result has been committed yet.

| Subcommand | Description |
|---|---|
| `read` | Read a pane's validated structured result as JSON |
| `wait` | Block host-side until a pane commits a valid structured result, then print it |

#### `plexi pane result read`

Read a pane's validated structured result as JSON.

Exits 0 and prints the validated JSON object when committed, exits 2 (`no_result_yet` or `--wait` `timeout`) when no result has been written yet, or exits 1 (`invalid_result`) when the last write was rejected by schema validation and not yet corrected.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id or name (positional form). Defaults to PLEXI_PANE_ID |
| `--pane-id` | string | no | Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID |
| `--wait` | flag | no | Block host-side until a valid result is committed or `--timeout` expires |
| `--timeout` | string | no | Seconds to wait when `--wait` is set before exiting 2 Default: `300`. |

#### `plexi pane result wait`

Block host-side until a pane commits a valid structured result, then print it.

Equivalent to `plexi pane result <id> --wait`. Exits 0 with the validated JSON object, 2 on timeout, 1 on error.

Agent spawn returns immediately. Wait in bounded slices with `plexi pane result wait <pane_id> --timeout 300`, well under your tool harness call limit. On exit 2, report progress and re-run the same wait until done (exit 0). Inspect errors on exit 1. Never use one long wait or spawn a replacement worker just because a slice expired.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id or name (positional form). Defaults to PLEXI_PANE_ID |
| `--pane-id` | string | no | Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID |
| `--timeout` | string | no | Seconds to wait before exiting 2 Default: `300`. |

## `plexi events`

Subscribe to a Plexi app's event streams and receive brokered deliveries.

Apps declare named event streams (e.g. `probe.tick`) and emit events on them. `plexi events subscribe <app_id> <stream>` opens a long-lived connection and prints one JSON line per delivered event to stdout (NDJSON) until interrupted. Subscriptions are brokered: the host stamps your identity from the pane you run in and checks permission before any event is delivered.

| Subcommand | Description |
|---|---|
| `subscribe` | Subscribe to an app's event stream and print delivered events as NDJSON |
| `declare` | Declare an event stream so it can be emitted on and subscribed to |
| `emit` | Emit an event onto a declared stream |
| `list` | List event streams currently declared by running apps |
| `mcp-config` | Print the host MCP server config for an MCP-aware agent |

### `plexi events subscribe`

Subscribe to an app's event stream and print delivered events as NDJSON.

Opens a long-lived connection to the running Plexi instance and streams one JSON object per line to stdout: first a `subscribed` acknowledgement, then one line per delivered event. Runs until interrupted (Ctrl-C), at which point the host drops the subscription and its queued deliveries.

Example: plexi events subscribe event-probe probe.tick --payload full

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<app_id>` | string | yes | App id that publishes the stream (e.g. `event-probe`) |
| `<stream>` | string | no | Stream name to subscribe to (e.g. `probe.tick`). Omit with --all to subscribe to every stream the app declares |
| `--all` | flag | no | Subscribe to all of the app's declared streams instead of one |
| `--payload` | string | no | How much of each event to deliver: off, summary, full, or state-ref Default: `full`. |
| `--trigger` | string | no | Trigger mode recorded on the subscription: never, conversation, ambient, or ask Default: `conversation`. |
| `--resource` | string | no | Only deliver events for this resource id (document/game/pane). Omit for any |

### `plexi events declare`

Declare an event stream so it can be emitted on and subscribed to.

Registers a named stream under an app-id namespace with a JSON-Schema object describing its payload. Declaring is a prerequisite for `emit`. Re-declaring a stream replaces its previous schema. The first declare under a namespace you do not own prompts for host consent.

Example: plexi events declare my-agent task.done --schema '{"type":"object"}'

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<app_id>` | string | yes | App-id namespace to declare the stream under (e.g. `my-agent`) |
| `<stream>` | string | yes | Stream name to declare (e.g. `task.done`) |
| `--schema` | string | no | JSON-Schema object describing the event payload Default: `{"type":"object"}`. |
| `--description` | string | no | Human-readable description of when this event fires |

### `plexi events emit`

Emit an event onto a declared stream.

Records a semantic event in the host timeline and fans it out to every broker-approved subscriber. The stream must already be declared. The emitter identity (actor_id) is host-stamped from your pane and cannot be set from the CLI; `--actor` sets only the advisory semantic category.

Example: plexi events emit my-agent task.done --summary "Build finished" --resource build-42 --revision-after done

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<app_id>` | string | yes | App-id namespace the stream lives under (e.g. `my-agent`) |
| `<event>` | string | yes | Declared stream name to emit on (e.g. `task.done`) |
| `--summary` | string | yes | One-line human-readable description of what happened |
| `--resource` | string | yes | Id of the document/game/pane/resource the event is about |
| `--revision-after` | string | yes | Revision identifier after the change |
| `--actor` | string | no | Semantic actor category that caused the change Default: `agent`. |
| `--resource-scope` | string | no | Scope class of the resource id (document, game, pane…). Defaults to pane |
| `--payload` | string | no | Structured JSON payload; the declared stream schema is advisory and not enforced on the emit path |
| `--state-ref` | string | no | Stable reference subscribers can fetch full state from |
| `--revision-before` | string | no | Revision identifier before the change |
| `--rollback-token` | string | no | Opaque token that makes this event reversible (creates an undo checkpoint) |
| `--changed-resource` | string (repeatable) | no | Other resource id touched by this change (repeatable) |

### `plexi events list`

List event streams currently declared by running apps

### `plexi events mcp-config`

Print the host MCP server config for an MCP-aware agent.

Emits a `mcpServers` JSON block pointing at this instance's host MCP server (read from `PLEXI_HOST_MCP_PORT` / `PLEXI_HOST_MCP_TOKEN`), so a Claude Code or Codex agent in this pane can call workspace app tools and subscribe to app events natively over MCP. The emitted credential is valid only while the originating pane remains alive.

> **Beta-gated:** MCP client configuration is a beta surface. This reference is included for beta and worktree testing; it is not available from the stable v1 channel.

## `plexi needs-you`

List everything waiting on you.

One host record covers click approvals, agent questions, and blocked runs. The desktop badge and the phone page read that record. `resolve --approve` on a click approval is refused. `--deny` still denies.

| Subcommand | Description |
|---|---|
| `list` | List open items waiting on you as JSON |
| `show` | Show one needs-you item and its continuation record as JSON |
| `ask` | File a creative or blocking question for the human and return its id + continuation record |
| `answer` | Answer an open creative question (`Question` or `BlockedRun`). Never mints a permission grant |
| `reconcile` | Reconcile open and resolved needs-you items and continuation records for a session, pane, or context |
| `complete` | Mark a continuation record completed (or failed/canceled) with an execution receipt |
| `resolve` | Resolve one item. Approving a click approval is refused; deny still denies it |

### `plexi needs-you list`

List open items waiting on you as JSON

### `plexi needs-you show`

Show one needs-you item and its continuation record as JSON

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Needs-you item id or continuation id |

### `plexi needs-you ask`

File a creative or blocking question for the human and return its id + continuation record

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--summary` | string | yes | Question summary shown in Needs you and agent UI |
| `--resource` | string | no | Target resource identifier (e.g. clip id or file path) Default: `workspace`. |
| `--choice` | string (repeatable) | no | Allowed answer choices (repeat `--choice` for multiple options) |
| `--allow-free-text` | flag | no | Allow free-text answers in addition to `--choice` options |
| `--recommend` | string | no | Mark one `--choice` as recommended (shown starred in the decision overlay) |
| `--preview-ref` | string (repeatable) | no | Preview references (e.g. clip/frame/path references) |
| `--correlation-id` | string | no | Correlation id joining the question to an operation or continuation |
| `--operation-id` | string | no | Operation id for idempotent resume |
| `--session-id` | string | no | Agent session id for reattachment across restarts |
| `--expected-revision` | string | no | Expected resource revision when resuming after the human answers |
| `--context-id` | string | no | Target context id or name (defaults to caller pane's context) |
| `--pane-id` | string | no | Caller pane id or name (defaults to `$PLEXI_PANE_ID`) |

### `plexi needs-you answer`

Answer an open creative question (`Question` or `BlockedRun`). Never mints a permission grant

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Needs-you question id |
| `--answer` | string | yes | Selected choice or free-text answer |
| `--ui-token` | string | no | Interactive UI prompt token returned when the question was filed |

### `plexi needs-you reconcile`

Reconcile open and resolved needs-you items and continuation records for a session, pane, or context

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--session-id` | string | no | Filter by agent session id |
| `--pane-id` | string | no | Filter by pane id or name |
| `--context-id` | string | no | Filter by context id or name |
| `--id` | string | no | Filter by specific item/run/operation/correlation id |

### `plexi needs-you complete`

Mark a continuation record completed (or failed/canceled) with an execution receipt

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Continuation run id, operation id, correlation id, or needs-you id |
| `--state` | string | no | Terminal state (`completed`, `failed`, `canceled`, `outcome_unknown`) Default: `completed`. |
| `--receipt` | string | no | Execution receipt JSON or summary string |

### `plexi needs-you resolve`

Resolve one item. Approving a click approval is refused; deny still denies it

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes |  |
| `--approve` | flag | no | Approve the item. A click approval stays open |
| `--deny` | flag | no | Deny the item |
| `--from-phone` | flag | no | Accepted so a paired phone can call resolve. The host still refuses an approval. Only a desktop Allow once click grants |

## `plexi permissions`

List and change decisions stored by the permission monitor.

`list` prints the live rows. `reset` clears a stored denial so the next call asks again. `revoke` removes an allow. `allow` turns a denial into an allow. Reset and allow from a pane, a call credential, or an agent file a Needs you item and leave the decision unchanged.

| Subcommand | Description |
|---|---|
| `list` | List live permission decisions |
| `reset` | Clear a stored denial so the next call asks again |
| `revoke` | Remove an allow, or refuse a pending ask |
| `allow` | Turn a denial or a pending ask into an allow |

### `plexi permissions list`

List live permission decisions

### `plexi permissions reset`

Clear a stored denial so the next call asks again

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Decision id from `plexi permissions list` |

### `plexi permissions revoke`

Remove an allow, or refuse a pending ask

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Decision id from `plexi permissions list` |

### `plexi permissions allow`

Turn a denial or a pending ask into an allow

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | Decision id from `plexi permissions list` |

## `plexi notify`

Send a notification to the Plexi UI

| Subcommand | Description |
|---|---|
| `dismiss` | Remove a notification previously posted by this pane |

### `plexi notify dismiss`

Remove a notification previously posted by this pane

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<notify_id>` | string | yes | Notification id printed by `plexi notify` |

## `plexi ai`

AI configuration and diagnostics — scan hardware, check integrations, recommend models

| Subcommand | Description |
|---|---|
| `onboard` | Guide first-run AI setup and the next app install step |
| `doctor` | Scan hardware and report recommended AI models |
| `setup` | Interactive wizard to configure a local AI model via Ollama |

### `plexi ai onboard`

Guide first-run AI setup and the next app install step.

Runs the same checks as `plexi ai doctor`, then prints the shortest path to usable AI: local Ollama, a user-owned OpenRouter key, or skipping AI for now. Ends with the app install command to try next.

Example: plexi ai onboard

### `plexi ai doctor`

Scan hardware and report recommended AI models.

Detects your CPU, RAM/VRAM, and GPU, then recommends which local or cloud AI models are a good fit. Also checks whether Ollama is installed and running, lists any already-pulled models, and verifies OpenRouter configuration.

Example: plexi ai doctor

### `plexi ai setup`

Interactive wizard to configure a local AI model via Ollama.

Walks through Ollama installation detection, model recommendation based on your hardware, pulling the recommended model, and writing the [ai.ollama] section to your config.toml so Plexi apps can use it immediately.

Example: plexi ai setup

## `plexi ledger`

Summarize recorded AI usage from this channel's ledger.

With no subcommand, prints per-client totals for tokens, cost, runs, and wall time. Reads `ai-ledger.jsonl` in the channel profile. Does not require a running host. A token count is a positive number or the word `unknown`. Rows written before run tags existed are migrated in place to explicit null `client` and `kind`.

| Subcommand | Description |
|---|---|
| `summary` | Totals for tokens, cost, run count, and wall time |

### `plexi ledger summary`

Totals for tokens, cost, run count, and wall time

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--by` | string | no | Group rows by `client` or `kind`. Omitted means `client` |
| `--since` | string | no | Include rows at or after this YYYY-MM-DD or RFC3339 timestamp |

## `plexi debug`

Inspect what the host recorded: per-session flight traces.

Reads files in the channel profile and does not require a running host.

Example: plexi debug session list Example: plexi debug session show latest --pane 3 --join-agents

| Subcommand | Description |
|---|---|
| `session` | Inspect recorded host sessions |

### `plexi debug session`

Inspect recorded host sessions

| Subcommand | Description |
|---|---|
| `list` | List recorded host sessions with start/last time, contexts, panes, and attached external agent session ids |
| `show` | Show one session as a single chronological timeline |

#### `plexi debug session list`

List recorded host sessions with start/last time, contexts, panes, and attached external agent session ids

#### `plexi debug session show`

Show one session as a single chronological timeline.

With `--join-agents`, recorded agent session ids are resolved to their transcripts under `~/.pi/agent/sessions` and `~/.claude/projects`, and agent prompts and tool calls are interleaved with host permission and app events.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<session>` | string | no | Session id (`sess-…`) or `latest`. Omitted means `latest` |
| `--pane` | string | no | Only records for this pane (and the calls it made) |
| `--cli` | flag | no | Show only CLI argv calls and their exit codes. Secret values, known secret options, and KEY=value assignments are redacted at capture |
| `--utc` | flag | no | Render timestamps in UTC instead of local time (text output only) |
| `--join-agents` | flag | no | Interleave external agent transcripts |

## `plexi experiments`

List, enable, or disable experiments (features outside the stable v1 surface).

Every experiment is off by default on every channel. Switches live in the `[experiments]` table of the global config.toml.

Example: plexi experiments list Example: plexi experiments enable assistant

| Subcommand | Description |
|---|---|
| `list` | Show every experiment: name, on/off, whether a host restart is needed |
| `enable` | Turn an experiment on in the global config.toml |
| `disable` | Turn an experiment off by removing its switch from the global config.toml |

### `plexi experiments list`

Show every experiment: name, on/off, whether a host restart is needed

### `plexi experiments enable`

Turn an experiment on in the global config.toml

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Experiment name; see `plexi experiments list` |

### `plexi experiments disable`

Turn an experiment off by removing its switch from the global config.toml

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Experiment name; see `plexi experiments list` |

## `plexi skill`

Install the agent skill compiled into this binary.

Writes `SKILL.md` where Claude Code (`~/.claude/skills`) and Codex (`~/.codex/skills`) load user skills. The bytes are the copy embedded at build time, so the installed `plexi_version` matches this binary.

Example: plexi skill install --agent claude

| Subcommand | Description |
|---|---|
| `draft` | Draft a reusable skill or commands.toml snippet from captured CLI calls |
| `install` | Write this binary's skill for one agent CLI, or all supported agents |

### `plexi skill draft`

Draft a reusable skill or commands.toml snippet from captured CLI calls.

Prints for review; installs nothing (never edits `<repo>/.plexi-<channel>/commands.toml` or `~/.plexi-<channel>/commands.toml`, or `<repo>/.plexi/commands.toml` / `~/.plexi/commands.toml` on stable). Without --out, writes no draft files. Captured secret values and KEY=value assignments stay redacted. Review failed calls and suggested --subcontext forms before accepting the draft.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--from-session` | string | yes | Recorded host session id, or latest |
| `--pane` | string | no | Target this pane instead of PLEXI_PANE_ID (without either, includes all panes) |
| `--all-panes` | flag | no | Include calls from every pane instead of the calling pane |
| `--include-internal` | flag | no | Include agent bookkeeping and pane polling calls |
| `--include-failed` | flag | no | Include failed or incomplete calls as review-only notes |
| `--as` | string | no | Output format: SKILL.md or commands.toml snippet Default: `skill`. |
| `--out` | string | no | Write the draft to a new file instead of stdout; never installs it |

### `plexi skill install`

Write this binary's skill for one agent CLI, or all supported agents

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--agent` | string | yes | Which agent CLI loads the skill: claude, codex, pi, or all |

## `plexi completions`

Print a shell completion script to stdout.

Example: plexi completions zsh >> ~/.zshrc

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<shell>` | string | no | Shell name: zsh, bash, or fish |

## `plexi cli`

Install the Plexi CLI launcher, drop-in completions, and shell PATH registration.

Inside Plexi panes, `plexi` and tab completions are injected automatically without editing your shell dotfiles. Run `plexi cli setup` to install the `~/.local/bin` launcher and XDG/Homebrew completion drop-in files and append PATH and completion blocks to your shell startup files (`.zshrc`, `.bashrc`, `.profile`, `config.fish`). Pass `--no-modify-path` to skip dotfile edits.

Example: plexi cli setup Example: plexi cli setup --no-modify-path

| Subcommand | Description |
|---|---|
| `setup` | Install the `~/.local/bin` launcher, drop-in completions, and shell PATH registration |

### `plexi cli setup`

Install the `~/.local/bin` launcher, drop-in completions, and shell PATH registration

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--modify-path` | flag | no | Append PATH and completion blocks to ~/.zshrc, ~/.bashrc, ~/.profile, and config.fish |
| `--no-modify-path` | flag | no | Do not edit shell startup files (.zshrc, .bashrc, .profile, config.fish) |

## `plexi config`

Check your Plexi config file for errors

| Subcommand | Description |
|---|---|
| `reload` | Reload configuration in the running host |
| `check` | Validate the selected config.toml and report any errors |
| `edit` | Open the selected config.toml in your $EDITOR |
| `get` | Print a config key from the selected config.toml to stdout |
| `reset` | Overwrite the selected config.toml with the built-in default template |
| `list` | Print all known config keys with type, current value, and description |
| `set` | Set one or more keys in the selected config.toml |

### `plexi config reload`

Reload configuration in the running host

### `plexi config check`

Validate the selected config.toml and report any errors.

Reads the workspace config inside a workspace, otherwise the global config.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only (`~/.plexi-<channel>/config.toml`, or `~/.plexi/config.toml` on stable) |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only (`<repo>/.plexi-<channel>/config.toml`, or `<repo>/.plexi/config.toml` on stable) |

### `plexi config edit`

Open the selected config.toml in your $EDITOR.

Opens the workspace config inside a workspace, otherwise the global config.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only (`~/.plexi-<channel>/config.toml`, or `~/.plexi/config.toml` on stable) |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only (`<repo>/.plexi-<channel>/config.toml`, or `<repo>/.plexi/config.toml` on stable) |

### `plexi config get`

Print a config key from the selected config.toml to stdout.

Supports dotted keys: agents.low, agents.medium, agents.high. Reads the workspace config inside a workspace, otherwise the global config.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only (`~/.plexi-<channel>/config.toml`, or `~/.plexi/config.toml` on stable) |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only (`<repo>/.plexi-<channel>/config.toml`, or `<repo>/.plexi/config.toml` on stable) |
| `<key>` | string | yes | Dotted key to retrieve (e.g. agents.medium) |

### `plexi config reset`

Overwrite the selected config.toml with the built-in default template.

Writes the workspace config inside a workspace, otherwise the global config. Creates a backup at config.toml.bak before overwriting.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only (`~/.plexi-<channel>/config.toml`, or `~/.plexi/config.toml` on stable) |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only (`<repo>/.plexi-<channel>/config.toml`, or `<repo>/.plexi/config.toml` on stable) |

### `plexi config list`

Print all known config keys with type, current value, and description.

Columns: key\ttype\tvalue\tdescription. Use --json for machine-readable output. Reads the workspace config inside a workspace, otherwise the global config.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only (`~/.plexi-<channel>/config.toml`, or `~/.plexi/config.toml` on stable) |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only (`<repo>/.plexi-<channel>/config.toml`, or `<repo>/.plexi/config.toml` on stable) |

### `plexi config set`

Set one or more keys in the selected config.toml.

Each argument must be in KEY=VALUE form (e.g. theme.preset=dracula font_size=14). Writes the workspace config inside a workspace, otherwise the global config. Pass --global to read or write only the global config.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only (`~/.plexi-<channel>/config.toml`, or `~/.plexi/config.toml` on stable) |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only (`<repo>/.plexi-<channel>/config.toml`, or `<repo>/.plexi/config.toml` on stable) |
| `<pairs>` | string (repeatable) | yes | One or more KEY=VALUE pairs to write |

## `plexi note`

Capture, list, read, open, and delete notes.

`plexi note "a thought"` captures a note. `plexi note list` prints note paths, `plexi note show <path>` prints one note, `plexi note open` picks one with fzf in the focused pane, and `plexi note rm <path>` deletes one. Notes live in the tier of the current directory (its nearest ancestor holding `.plexi/`, else the global tier); see `src/host/AGENTS.md` for the tier contract. `list` also rolls up nested tiers and the global tier.

Text that is exactly `list`, `open`, `add`, `show`, or `rm` is read as that subcommand; shell quoting cannot tell them apart. Capture it with `plexi note add list`.

Example: plexi note "remember to update the docs"

| Subcommand | Description |
|---|---|
| `add` | Capture a note explicitly, even when the text is `list`, `open`, or `add` |
| `list` | Print paths of every note in every visible tier, newest first |
| `open` | Open a note picker with fzf in the focused terminal pane |
| `show` | Print one note's contents by the path `plexi note list` printed |
| `rm` | Delete one note by the path `plexi note list` printed |

### `plexi note add`

Capture a note explicitly, even when the text is `list`, `open`, or `add`.

Writes a timestamped note into the tier of the current directory.

Example: plexi note add "remember to update the docs"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<text>` | string | yes | Note text to capture |

### `plexi note list`

Print paths of every note in every visible tier, newest first.

Tiers are the current directory's context tier (its nearest ancestor holding a `.plexi/` directory), any tier nested below it, and the global tier. Resolved from the working directory, never from the environment.

### `plexi note open`

Open a note picker with fzf in the focused terminal pane.

Candidates come from `plexi note list`. Requires fzf and a running host; both are reported as errors rather than silently degrading.

### `plexi note show`

Print one note's contents by the path `plexi note list` printed.

Refuses any path outside a notes tier visible from the current directory, so a hand-typed path cannot read an arbitrary file.

Example: plexi note show ~/.plexi/notes/20260101T000000.000Z.md

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Note path from `plexi note list` |

### `plexi note rm`

Delete one note by the path `plexi note list` printed.

Irreversible: the note file is removed from disk. Refuses any path outside a notes tier visible from the current directory.

Example: plexi note rm ~/.plexi/notes/20260101T000000.000Z.md

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Note path from `plexi note list` |

## `plexi doctor`

Audit all installed apps for capability and config gaps.

Checks every installed app's declared capabilities against your current config.toml and reports what's working and what needs to be configured. Use --json for scripting.

## `plexi demo`

Interactive keybinding tutorial — learn split and navigate in real time.

Walk through macOS pane controls inside a live pane. Requires macOS and a Plexi pane (PLEXI_PANE_ID must be set).

## `plexi update`

Update this channel's managed package, or restore its previous generation

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--rollback` | flag | no | Restore the retained previous package through the installation transaction |

## `plexi uninstall`

Remove this channel's recorded package and launchers.

Files outside the installation receipt, including development binaries and other channels, are preserved.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--keep-data` | flag | no | Retain user data and its credentials (the default) |
| `--delete-data` | flag | no | Permanently delete this channel's profile and integrity credentials |
| `--yes` / `-y` | flag | no | Remove this channel without a confirmation prompt |

## `plexi host`

Launch, stop, or check a headless-friendly Plexi host from the CLI.

`host start` launches this channel's app bundle detached from the calling shell, optionally seeding panes from a `--layout` TOML file or repeated `--pane` flags, then blocks until the host confirms it's ready. Works identically on alpha, beta, main, and PR builds — the channel is resolved from the running CLI binary's own name.

| Subcommand | Description |
|---|---|
| `restart` | Restart this channel, restoring the saved workspace |
| `start` | Launch this channel's app bundle detached and wait for readiness |
| `stop` | Stop the running host and every pane agent for this channel |
| `log` | Write one info-level marker line into the running host's channel log |
| `status` | Report whether this channel's host is running, its pid, socket path, and pane count |
| `screenshot` | Capture the running host window as a PNG through the real render pipeline — the pixels the user actually sees, no OS screen capture |
| `key` | Inject a key or chord into the host-level keyboard path |

### `plexi host restart`

Restart this channel, restoring the saved workspace

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--yes` | flag | no | Confirm restarting the host containing this pane; this pane will close |

### `plexi host start`

Launch this channel's app bundle detached and wait for readiness.

Errors if a host for this channel is already running. Seeds any declared panes via the spawn-queue before the app boots, so they appear on its first frame.

Example: plexi-pr-2357 host start --pane 'cwd=/tmp,cmd=htop'

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--layout` | string | no | TOML file with `[[pane]]` tables to seed on boot |
| `--pane` | string (repeatable) | no | A pane to seed: 'cwd=<dir>[,cmd=<command>][,tab|window]'. Repeatable |
| `--timeout-secs` | string | no | Seconds to wait for the host to confirm readiness (default 15) |
| `--ephemeral` | flag | no | Boot a hermetic session: skip workspace restore on start and skip workspace save on shutdown. For automated runs (scene runners, release gates) that must never see or clobber the channel's saved session |
| `--background` | flag | no | Launch without activating Plexi or taking focus. On macOS this uses Accessory activation policy, so the host has no normal Dock or menu-bar presence and should be driven through the CLI |

### `plexi host stop`

Stop the running host and every pane agent for this channel.

Non-interactive: sends pane process groups SIGHUP then SIGTERM, then SIGKILL after a grace period (PLEXI_QUIT_GRACE_MS, default 2000 ms; closing the window uses 500 ms). Sends a clean shutdown request first, falling back to SIGTERM if the host doesn't confirm exit in time.

### `plexi host log`

Write one info-level marker line into the running host's channel log.

For automated drivers (release gates, scene runners, CI) that must leave start/finish summaries in `~/.plexi-<channel>/plexi.log` itself.

Example: plexi host log --source editor_gate "gate finished passed=9 failed=0"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<message>` | string | yes | The marker text (flattened to one line in the log) |
| `--source` | string | no | Short tool identity prefixed to the line Default: `cli`. |

### `plexi host status`

Report whether this channel's host is running, its pid, socket path, and pane count

### `plexi host screenshot`

Capture the running host window as a PNG through the real render pipeline — the pixels the user actually sees, no OS screen capture.

Example: plexi host screenshot --pane 3 --output /tmp/pane3.png

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--pane` | string | no | Crop the capture to this pane id or name's current screen rect |
| `--output` / `-o` | string | no | Where to write the PNG (default: <profile>/screenshots/<timestamp>.png) |

### `plexi host key`

Inject a key or chord into the host-level keyboard path.

This is a validation seam for host shortcuts: it does not send bytes to a focused pane. Example: `plexi-pr-2906 host key cmd+1`.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<key>` | string | yes | Single key, named key, or chord such as `cmd+1` or `cmd+shift+2` |
