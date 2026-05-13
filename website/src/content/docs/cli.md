---
title: CLI Reference
description: Complete reference for all plexi subcommands and flags.
verified_version: "3.6.38"
order: 7
---

The `plexi` CLI is the primary way to interact with a running Plexi instance from the terminal, and to manage workspaces and apps from outside the UI.

All commands work identically across build channels (`plexi`, `plexi-alpha`, `plexi-beta`). When run inside a Plexi pane, `PLEXI_SOCKET` routes host commands to the correct running instance automatically.

## `plexi run`

Run a named command from .plexi/commands.toml

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<command>` | string | yes |  |

## `plexi workspace`

Workspace management

| Subcommand | Description |
|---|---|
| `init` | Initialise a .plexi/ workspace in the current directory |

### `plexi workspace init`

Initialise a .plexi/ workspace in the current directory

## `plexi secret`

Secret management

| Subcommand | Description |
|---|---|
| `set` | Store a secret |
| `get` | Read a secret value to stdout |
| `list` | List stored secrets |
| `delete` | Delete a secret |

### `plexi secret set`

Store a secret

Prompts for value with hidden input; walks up to nearest .plexi/ workspace. Use --from-env to read from an env var instead of prompting, or --global to store cross-workspace.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes | Name of the secret (also the env var name when using --from-env) |
| `--from-env` | flag | no | Read value from the environment variable named FRIENDLY_NAME instead of prompting |
| `--global` | flag | no | Store globally (cross-workspace) rather than scoped to the nearest .plexi/ workspace |

### `plexi secret get`

Read a secret value to stdout

Walks up to nearest .plexi/ workspace, resolves from there first, then falls back to global. Use --global to read from the global store only.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes | Name of the secret |
| `--global` | flag | no | Read from global store only (skip workspace lookup) |

### `plexi secret list`

List stored secrets

### `plexi secret delete`

Delete a secret

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes |  |

## `plexi app`

App management

| Subcommand | Description |
|---|---|
| `init` | Scaffold a new app |
| `uninstall` | Uninstall an app |
| `list` | List installed apps |
| `render` | Render an app to PNG headlessly |
| `info` | Show app info (id, name, version, MCP tools if any) |
| `install` | Install a local app directory into the channel's app store |
| `link` | Register a local app directory with the workspace (does not move files) |
| `unlink` | Remove a linked app directory from the workspace registry |

### `plexi app init`

Scaffold a new app

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes |  |
| `--lang` | string | no | Default: `python`. |

### `plexi app uninstall`

Uninstall an app

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes |  |

### `plexi app list`

List installed apps

### `plexi app render`

Render an app to PNG headlessly

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | App id to render (e.g. "snake") |
| `--size` | string | no | Dimensions as WxH (e.g. 500x500) Default: `800x600`. |
| `--state` | string | no | Pre-seed app state from a JSON file before render |
| `--output` | string | no | Output PNG path (default: stdout) |

### `plexi app info`

Show app info (id, name, version, MCP tools if any)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes |  |

### `plexi app install`

Install a local app directory into the channel's app store

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Path to the app directory containing manifest.toml |

### `plexi app link`

Register a local app directory with the workspace (does not move files)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Path to the app directory containing manifest.toml |

### `plexi app unlink`

Remove a linked app directory from the workspace registry

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Path to the app directory (same path used with `link`) |

## `plexi install`

Install an app

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<spec>` | string | no | Source spec (e.g. github:user/repo or bare app id) |
| `--pack` | string | no | Install from a pack file or 'core' |

## `plexi uninstall`

Uninstall an app

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | App id to remove |
| `--yes` / `-y` | flag | no | Skip confirmation prompt |

## `plexi update`

Update apps or self

| Subcommand | Description |
|---|---|
| `apps` | Update installed apps |

### `plexi update apps`

Update installed apps

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | no | Specific app id to update (omit to update all) |

## `plexi list`

List installed apps

## `plexi validate`

Validate a Plexi app directory

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | Path to validate (default: current directory) Default: `.`. |

## `plexi pack`

Pack management

| Subcommand | Description |
|---|---|
| `export` | Export current apps as a pack file |

### `plexi pack export`

Export current apps as a pack file

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes |  |

## `plexi notify`

Send a notification [requires PLEXI_SOCKET — run inside a Plexi pane]

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--title` | string | yes | Notification title (required) |
| `--body` | string | no | Notification body |
| `--level` | string | no | Level: info, warn, or error Default: `info`. |
| `--choice` | string (repeatable) | no | Choice option. Format: `key:Label` (returns key when selected) or `Label:pane_focus:<pane_id>` (navigates to pane and returns label). Repeatable |
| `--host-action` | string (repeatable) | no | Host-side action for a choice key. Format: `key:action_type:action_arg`. Repeatable. The host performs this action when the user clicks the matching choice, even if the spawner has already exited |
| `--timeout` | string | no | Timeout in seconds (0 = no timeout) Default: `0`. |
| `--scope` | string | no | Notification visibility scope: window, context, or global. Default: global |

## `plexi pane`

Pane management [requires PLEXI_SOCKET — run inside a Plexi pane]

| Subcommand | Description |
|---|---|
| `name` | Set the name of a pane |
| `list` | List all open panes as a JSON array |
| `focus` | Move UI focus to a pane by ID |
| `close` | Close a pane. Omit <pane_id> to close the current pane via PLEXI_PANE_ID |
| `send` | Send text to a running pane's PTY stdin [requires PLEXI_SOCKET — run inside a Plexi pane] |
| `info` | Print JSON info for the current pane [requires PLEXI_PANE_ID] |
| `key` | Deliver a synthetic key event to a pane [requires PLEXI_SOCKET — run inside a Plexi pane] |

### `plexi pane name`

Set the name of a pane

Usage: plexi pane name <title>           — renames the current (focused) pane plexi pane name <pane-id> <title> — renames an arbitrary pane by ID

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<first>` | string | yes | Pane ID (from `plexi pane list`) or title when used alone |
| `<second>` | string | no | Title when pane-id is given as the first argument |

### `plexi pane list`

List all open panes as a JSON array

### `plexi pane focus`

Move UI focus to a pane by ID

NOTE: This moves the *user's visual focus* to the target pane — it does NOT relocate the agent's execution context. An agent calling this from pane A remains in pane A after the call; only the user sees focus shift to pane B.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane ID (from `plexi pane list`) |

### `plexi pane close`

Close a pane. Omit <pane_id> to close the current pane via PLEXI_PANE_ID

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane ID (from `plexi pane list`). Defaults to PLEXI_PANE_ID if not given |

### `plexi pane send`

Send text to a running pane's PTY stdin [requires PLEXI_SOCKET — run inside a Plexi pane]

Use `\n` in the text to send Enter (submits the command).

Example: plexi pane send 42 "git status\n"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane ID (from `plexi pane list`) |
| `<text>` | string | yes | Text to inject (use \n for Enter) |

### `plexi pane info`

Print JSON info for the current pane [requires PLEXI_PANE_ID]

### `plexi pane key`

Deliver a synthetic key event to a pane [requires PLEXI_SOCKET — run inside a Plexi pane]

For terminal panes, injects as PTY keystroke. For app panes, delivers a structured key event.

Key format: single char ("h"), named key ("enter", "escape", "space", "up", "down", "left", "right", "backspace"), or chord ("ctrl+c").

Example: plexi pane key 42 enter

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane ID (from `plexi pane list`) |
| `<key>` | string | yes | Key to inject |

## `plexi terminal`

Open a terminal pane

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<cmd>` | string | no | Optional command to run in the terminal |
| `--ephemeral` / `-e` | flag | no | Close the pane when the process exits |
| `--layout` | string | no | Layout hint (split_v, split_h, split_above) |
| `--from-pane-id` | string | no | Split relative to this pane ID instead of the focused pane |
| `--cwd` | string | no | Working directory for the new terminal pane |

## `plexi open`

Open an app pane

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<type_id>` | string | no | App or pane type id (mutually exclusive with --mcp and --cli) |
| `--mcp` | string (repeatable) | no | Wrap the given stdio MCP server command in the bundled mcp-renderer  Example: plexi open --mcp npx @modelcontextprotocol/server-filesystem /tmp |
| `--cli` | string | no | Wrap the given CLI binary in the bundled descriptor-renderer  Example: plexi open --cli git |
| `--layout` | string | no | Layout hint |
| `--from-pane-id` | string | no | Split relative to this pane ID instead of the focused pane |
| `<extra_args>` | string (repeatable) | no | Extra args passed to the app (only valid with type_id) |

## `plexi registry`

CLI registry

| Subcommand | Description |
|---|---|
| `watch` | Watch installed CLIs for descriptor drift |

### `plexi registry watch`

Watch installed CLIs for descriptor drift

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<cli>` | string | no | Only check this CLI |

## `plexi context`

Context management [requires PLEXI_SOCKET — run inside a Plexi pane]

| Subcommand | Description |
|---|---|
| `new` | Create a new context, optionally opening a path |
| `open` | Open a context at a path |
| `set-root` | Set the root directory for the active context |
| `current` | Print the context ID and name for the current pane as JSON |

### `plexi context new`

Create a new context, optionally opening a path

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no |  |

### `plexi context open`

Open a context at a path

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no |  |

### `plexi context set-root`

Set the root directory for the active context

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no |  |

### `plexi context current`

Print the context ID and name for the current pane as JSON

## `plexi completions`

Print shell completion script

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<shell>` | string | no | Shell name (zsh, bash, fish) |

