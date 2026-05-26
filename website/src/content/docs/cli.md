---
title: CLI Reference
description: Complete reference for all plexi subcommands and flags.
verified_version: "0.0.506"
order: 7
---

The `plexi` CLI is the primary way to interact with a running Plexi instance from the terminal, and to manage workspaces and apps from outside the UI.

All commands work identically across build channels (`plexi`, `plexi-alpha`, `plexi-beta`). When run inside a Plexi pane, `PLEXI_SOCKET` routes host commands to the correct running instance automatically.

## `plexi run`

Run a named command from your project's .plexi/commands.toml file.

Define shell commands in .plexi/commands.toml and run them by name here. Any secrets listed in the command definition are injected as environment variables automatically.

Example: plexi run dev

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<command>` | string | no | Command name to run (omit to list available commands) |

## `plexi workspace`

Set up a .plexi/ workspace in your project folder.

Run this once inside your project directory to enable workspace-scoped secrets and commands.

| Subcommand | Description |
|---|---|
| `init` | Set up a .plexi/ workspace in the current directory |

### `plexi workspace init`

Set up a .plexi/ workspace in the current directory.

Run this once inside your project folder. It creates a .plexi/workspace.toml so that secrets and commands are scoped to this project.

## `plexi secret`

Store and retrieve secrets (API keys, passwords, tokens) for your project.

Secrets are saved to your system keychain and injected as environment variables when you run commands. Use `plexi workspace init` first to scope secrets to a project.

| Subcommand | Description |
|---|---|
| `set` | Save a secret to your keychain |
| `get` | Print a stored secret's value to stdout |
| `list` | Show all secrets stored for this project |
| `delete` | Delete a stored secret |

### `plexi secret set`

Save a secret to your keychain.

Plexi will prompt you to type the value (hidden). The secret is stored in your system keychain and can be injected into commands automatically.

Use --from-env to read the value from an existing environment variable instead of typing it. Use --global to make the secret available across all projects, not just the current one.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes | Name for this secret — also the environment variable name it will be injected as |
| `--from-env` | flag | no | Read the value from the environment variable named FRIENDLY_NAME instead of prompting |
| `--global` | flag | no | Store this secret globally so it's available in all projects, not just this one |

### `plexi secret get`

Print a stored secret's value to stdout.

Looks up the secret for the current project first, then falls back to the global store. Use --global to read only from the global store.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes | Name of the secret to read |
| `--global` | flag | no | Read from the global store only, skipping the project-level lookup |

### `plexi secret list`

Show all secrets stored for this project

### `plexi secret delete`

Delete a stored secret

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes |  |

## `plexi app`

Manage your Plexi apps — scaffold, install, list, and inspect

| Subcommand | Description |
|---|---|
| `init` | Create a new app from a template |
| `uninstall` | Remove an installed app by id |
| `list` | Show all installed apps (alias for `plexi list`) |
| `render` | Render an app to a PNG image without opening the UI (useful for screenshots and testing) |
| `info` | Show details about an installed app: id, name, version, and available tools |
| `install` | Install a local app directory you are developing into Plexi |
| `run` | Run an app directly from a local directory without installing or linking |

### `plexi app init`

Create a new app from a template.

Scaffolds the folder structure and files you need to build a Plexi app. Use --lang to pick the language (default: python).

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes |  |
| `--lang` | string | no | Default: `python`. |

### `plexi app uninstall`

Remove an installed app by id.

Example: plexi app uninstall github-tree

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | App id to remove (use `plexi app list` to see installed ids) |
| `--yes` / `-y` | flag | no | Skip the confirmation prompt |

### `plexi app list`

Show all installed apps (alias for `plexi list`)

### `plexi app render`

Render an app to a PNG image without opening the UI (useful for screenshots and testing)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | App id to render (e.g. "snake") |
| `--size` | string | no | Image dimensions as WxH (e.g. 500x500) Default: `800x600`. |
| `--state` | string | no | Pre-seed the app's state from a JSON file before rendering |
| `--output` | string | no | Where to save the PNG (default: stdout) |

### `plexi app info`

Show details about an installed app: id, name, version, and available tools

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes |  |

### `plexi app install`

Install a local app directory you are developing into Plexi.

Copies your app folder into Plexi's app store so it shows up and runs like any other app. To install apps from GitHub or a pack file, use `plexi install` instead.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Path to the app folder containing manifest.toml |

### `plexi app run`

Run an app directly from a local directory without installing or linking.

Opens the app in a pane immediately. Edits to the app take effect on next launch. Replaces `plexi app link` for development workflows.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Path to the app folder containing manifest.toml |

## `plexi install`

Install an app from a remote source or a pack file.

Pass a GitHub source like `github:owner/repo`, or use `--pack` to install from a local pack file or the built-in core pack.

Example: plexi install github:owner/repo Example: plexi install --pack core

To install a local app directory you are developing, use `plexi app install <path>` instead.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<spec>` | string | no | Source to install from (e.g. github:owner/repo or a bare app id) |
| `--pack` | string | no | Install from a pack file or 'core' |

## `plexi uninstall`

Remove Plexi from your Mac — uninstalls the app, CLI, and optionally your profile data.

Removes the current channel's app bundle (/Applications/Plexi.app), CLI binary (/usr/local/bin/plexi), and shell completions. Your profile directory (~/.plexi/) holds your settings, secrets, and app configurations — you will be asked whether to keep it.

Example: plexi uninstall

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--keep-data` | flag | no | Keep your profile directory (~/.plexi/) — your settings, secrets, and app data stay on disk |
| `--yes` / `-y` | flag | no | Skip the confirmation prompt and proceed immediately (removes data unless --keep-data is set) |

## `plexi update`

Update installed apps or Plexi itself.

Run with the `apps` subcommand to update one or all installed apps. Run with no subcommand to update the Plexi binary itself.

| Subcommand | Description |
|---|---|
| `apps` | Pull the latest version of your installed apps |

### `plexi update apps`

Pull the latest version of your installed apps.

Omit the app id to update all installed apps at once.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | no | App id to update (omit to update all installed apps) |

## `plexi list`

Show all installed apps with their versions

## `plexi validate`

Check a Plexi app directory for errors before publishing or installing

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | Path to check (default: current directory) Default: `.`. |

## `plexi pack`

Package your apps for sharing or bulk installation

| Subcommand | Description |
|---|---|
| `export` | Export your currently installed apps as a single pack file for sharing or backup |

### `plexi pack export`

Export your currently installed apps as a single pack file for sharing or backup

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes |  |

## `plexi notify`

Send a notification to the Plexi UI. Run this from inside a Plexi pane (open one first with `plexi open terminal`)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--title` | string | yes | Notification title (required) |
| `--body` | string | no | Notification body text |
| `--level` | string | no | Severity level: info, warn, or error Default: `info`. |
| `--choice` | string (repeatable) | no | Add a clickable button to the notification. Format: `key:Label` (returns key when clicked) or `Label:pane_focus:<pane_id>` (switches focus to that pane when clicked). Repeatable |
| `--host-action` | string (repeatable) | no | Action to perform on the host when a button is clicked. Format: `key:action_type:action_arg`. Repeatable. The host runs this even after the process that sent the notification has exited |
| `--timeout` | string | no | How many seconds before the notification disappears (0 = stays until dismissed) Default: `0`. |
| `--scope` | string | no | Which panes see this notification: window, context, or global (default: global) |

## `plexi pane`

Control panes — list, focus, send input, capture output, and more. Run this from inside a Plexi pane (open one first with `plexi open terminal`)

| Subcommand | Description |
|---|---|
| `name` | Rename a pane |
| `list` | List all open panes as a JSON array |
| `focus` | Move the visible focus to a specific pane |
| `close` | Close a pane. Omit the pane id to close the pane you are currently in |
| `send` | Type text into another pane as if it came from the keyboard. Run this from inside a Plexi pane (open one first with `plexi open terminal`) |
| `self` | Print the id of the pane you are currently in |
| `info` | Print details about the current pane as JSON |
| `capture` | Capture the last N lines of a pane's output as a JSON array. Run this from inside a Plexi pane (open one first with `plexi open terminal`) |
| `key` | Send a key press to a pane. Run this from inside a Plexi pane (open one first with `plexi open terminal`) |

### `plexi pane name`

Rename a pane.

With one argument, renames the current pane: plexi pane name "My Project" With two arguments, renames any pane by id: plexi pane name 42 "My Project"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<first>` | string | yes | Pane id (from `plexi pane list`) or the new name if renaming the current pane |
| `<second>` | string | no | New name when a pane id is given as the first argument |

### `plexi pane list`

List all open panes as a JSON array.

Filter by context with --context <id> or --current (reads PLEXI_CONTEXT_ID).

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--context` | string | no | Only return panes belonging to this context ID |
| `--current` | flag | no | Only return panes in the caller's context (reads PLEXI_CONTEXT_ID from env) |

### `plexi pane focus`

Move the visible focus to a specific pane.

This moves what the user sees on screen — it does not change which pane an agent is running in. An agent calling this from pane A remains in pane A; the user just sees pane B highlighted.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to focus (from `plexi pane list`) |

### `plexi pane close`

Close a pane. Omit the pane id to close the pane you are currently in

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id to close (from `plexi pane list`). Defaults to the current pane if not given |

### `plexi pane send`

Type text into another pane as if it came from the keyboard. Run this from inside a Plexi pane (open one first with `plexi open terminal`).

Use `\n` in the text to press Enter (which submits a command).

Example: plexi pane send 42 "git status\n"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to send text to (from `plexi pane list`) |
| `<text>` | string | yes | Text to type into the pane (use `\n` for Enter) |

### `plexi pane self`

Print the id of the pane you are currently in.

Useful in scripts: MY_PANE=$(plexi pane self)

### `plexi pane info`

Print details about the current pane as JSON

### `plexi pane capture`

Capture the last N lines of a pane's output as a JSON array. Run this from inside a Plexi pane (open one first with `plexi open terminal`).

Defaults to the current pane when no pane id is given.

Example: plexi pane capture --lines 50 42

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id to capture output from. Defaults to the current pane |
| `--lines` | string | no | How many lines to read from the end of the output Default: `50`. |
| `--full-output` | flag | no | Preserve trailing empty lines (by default they are stripped) |
| `--from-cursor` | string | no | Read only lines written after this cursor value. Get the cursor from a previous capture response. When set, the response is always JSON object format |

### `plexi pane key`

Send a key press to a pane. Run this from inside a Plexi pane (open one first with `plexi open terminal`).

For terminal panes, injects the keystroke into the terminal. For app panes, delivers a structured key event.

Key formats: single character ("h"), named key ("enter", "escape", "space", "up", "down", "left", "right", "backspace"), or chord ("ctrl+c").

Example: plexi pane key 42 enter

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to send the key to (from `plexi pane list`) |
| `<key>` | string | yes | Key to press |

## `plexi terminal`

Open a plain terminal pane

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<cmd>` | string | no | Optional shell command to run inside the new terminal |
| `--ephemeral` / `-e` | flag | no | Close the pane automatically when the command finishes |
| `--layout` | string | no | Where to place the new pane: split_h (right), split_left (left), split_v (below), split_right, split_below, split_above, tab, or new_window |
| `--from-pane-id` | string | no | Open the new pane relative to this pane ID instead of the focused pane |
| `--cwd` | string | no | Directory to open the terminal in |
| `--no-focus` | flag | no | Keep focus on the current pane instead of jumping to the new one |

## `plexi open`

Open an app or tool in a new pane.

Pass an app id (e.g. `plexi open snake`) to open an installed app. Use `--mcp` to wrap an MCP server, or `--cli` to open any CLI tool with a Plexi UI.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<type_id>` | string | no | App id to open (mutually exclusive with --mcp and --cli) |
| `--mcp` | string (repeatable) | no | Wrap a stdio MCP server in a Plexi pane.  Example: plexi open --mcp npx @modelcontextprotocol/server-filesystem /tmp |
| `--cli` | string | no | Wrap a CLI tool in a Plexi pane with a visual UI.  Example: plexi open --cli git |
| `--layout` | string | no | Where to place the new pane: split_h (right), split_left (left), split_v (below), split_right, split_below, split_above, tab, or new_window |
| `--from-pane-id` | string | no | Open the new pane relative to this pane ID instead of the focused pane |
| `<extra_args>` | string (repeatable) | no | Extra arguments passed through to the app (only valid with an app id) |

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

## `plexi context`

Manage the active context (the folder and project scope tied to the current pane). Run this from inside a Plexi pane (open one first with `plexi open terminal`)

| Subcommand | Description |
|---|---|
| `new` | Open a new context with an optional name |
| `open` | Switch the current pane to a context at the given path |
| `set-root` | Change the root folder for the active context |
| `current` | Print the id and name of the current pane's context as JSON |
| `describe` | Set the description for the active context |
| `zoom` | Zoom into a sub-context by its numeric context_id |
| `zoom-out` | Zoom out of the current sub-context to the parent |

### `plexi context new`

Open a new context with an optional name

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | no | Name for the new context. Defaults to the directory basename |
| `--path` | string | no | Root path for the new context. Defaults to current working directory |
| `--parent` | string | no | Create as a child of the named context. Defaults to current context if inside one |

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

Zoom into a sub-context by its numeric context_id

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<context_id>` | string | yes |  |

### `plexi context zoom-out`

Zoom out of the current sub-context to the parent

## `plexi completions`

Print a shell completion script to stdout.

Example: plexi completions zsh >> ~/.zshrc

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<shell>` | string | no | Shell name: zsh, bash, or fish |

## `plexi config`

Check your Plexi config file for errors

| Subcommand | Description |
|---|---|
| `check` | Validate your config.toml and report any errors |
| `edit` | Open config.toml in your $EDITOR |
| `get` | Print the resolved value of a config key to stdout |
| `reset` | Overwrite config.toml with the built-in default template |

### `plexi config check`

Validate your config.toml and report any errors

### `plexi config edit`

Open config.toml in your $EDITOR

### `plexi config get`

Print the resolved value of a config key to stdout.

Supports dotted keys: agents.low, agents.medium, agents.high. Returns the effective value (user setting or built-in default).

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<key>` | string | yes | Dotted key to retrieve (e.g. agents.medium) |

### `plexi config reset`

Overwrite config.toml with the built-in default template.

Creates a backup at config.toml.bak before overwriting.

## `plexi routine`

Manage workspace routines — scheduled shell commands.

Routines are declared in `.plexi/routines.toml` and run automatically on schedule. Use `plexi routine list` to see configured routines, or `plexi routine run <name>` to fire one manually.

| Subcommand | Description |
|---|---|
| `list` | List routines defined in .plexi/routines.toml with their schedule and next fire time |
| `run` | Manually trigger a named routine from .plexi/routines.toml |

### `plexi routine list`

List routines defined in .plexi/routines.toml with their schedule and next fire time

### `plexi routine run`

Manually trigger a named routine from .plexi/routines.toml

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Name of the routine to run |

## `plexi notes`

Browse and open scratchpad notes created with Cmd+Shift+Space.

Each scratchpad session writes a timestamped file to `<config_dir>/notes/`. Use `plexi notes list` to print note paths, or `plexi notes open` to pick one with fzf.

| Subcommand | Description |
|---|---|
| `list` | Print paths of all scratchpad notes, newest first |
| `open` | Open a note picker with fzf in the focused terminal pane |

### `plexi notes list`

Print paths of all scratchpad notes, newest first

### `plexi notes open`

Open a note picker with fzf in the focused terminal pane.

Requires fzf to be installed. Falls back to printing the notes directory when fzf is not available or PLEXI_SOCKET is not set.

## `plexi demo`

Interactive keybinding tutorial — learn split and navigate in real time.

Walk through two fundamental Plexi interactions inside a live pane: split a pane (⌘D) and navigate between panes (⌘L / ⌘H). Must be run inside a Plexi pane (PLEXI_PANE_ID must be set).

