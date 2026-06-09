---
title: CLI Reference
description: Complete reference for all plexi subcommands and flags.
verified_version: "0.0.671"
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
| `--alias` | string | no | Use a different name for the Keychain entry than the canonical env var name.  Useful when the Keychain entry already exists under a different name. Example: plexi secret set OPENAI_API_KEY --alias openai_personal |

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

Delete a stored secret.

Use --global to delete a globally-stored secret (one stored with `secret set --global`). Without --global, deletes the workspace-scoped entry for the current project.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes |  |
| `--global` | flag | no | Delete from the global store instead of the project-scoped store |

## `plexi routine`

Manage workspace routines — scheduled shell commands.

Routines are declared in `.plexi/routines.toml` and run automatically on schedule. **Requires Plexi to be running** — there is no background daemon. Routines only fire while the host process is open.

Use `plexi routine list` to see configured routines, or `plexi routine run <name>` to fire one manually.

### Routine file format (`.plexi/routines.toml`)

```toml
[[routine]]
name      = "morning-sync"
command   = "./scripts/sync.sh"
schedule  = "daily at 09:00"
context   = "work"   # optional: only fires when this context is active
ephemeral = true     # optional: close the spawned pane when the command exits
```

### Schedule formats

| Format | Example |
|---|---|
| `every N seconds` | `every 30 seconds` |
| `every N minutes` | `every 5 minutes` |
| `every N hours`   | `every 2 hours` |
| `daily at HH:MM`  | `daily at 09:00` |
| `weekly on <day> at HH:MM` | `weekly on monday at 09:00` |
| `monthly on N at HH:MM`    | `monthly on 1 at 08:00` |
| 5-field cron `m h dom mon dow` | `0 9 * * 1-5` |

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

## `plexi agent`

Manage workspace agent definitions.

Install agent definitions from the global registry (`~/.plexi/agents/`) into the current workspace's `.plexi/agents/` directory, each with scoped memory and logs.

| Subcommand | Description |
|---|---|
| `init` | Scaffold a new agent app with ai.query capability and a chat UI |
| `add` | Install an agent definition from the global registry into the current workspace |
| `update` | Re-install an agent definition from the global registry, preserving memory and logs |
| `list` | List agents installed in the current workspace |

### `plexi agent init`

Scaffold a new agent app with ai.query capability and a chat UI.

Creates the app directory, manifest.toml (with ai.query pre-configured), and main.py from the agent template. Equivalent to the former `plexi app init --agent <name>`.

Example: plexi agent init my-agent

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | App name (used as the directory name and app ID) |
| `--from-pane-id` | string | no | Open the new pane relative to this pane ID instead of the focused pane. Defaults to PLEXI_PANE_ID if set in the environment |

### `plexi agent add`

Install an agent definition from the global registry into the current workspace.

Copies `~/.plexi/agents/<name>/AGENT.md` into `.plexi/agents/<name>/AGENT.md` and creates `memory/` and `logs/` subdirectories for scoped agent state.

Example: plexi agent add project-manager

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Agent name (must exist in ~/.plexi/agents/<name>/AGENT.md) |

### `plexi agent update`

Re-install an agent definition from the global registry, preserving memory and logs.

Overwrites `.plexi/agents/<name>/AGENT.md` with the latest version from the global registry while leaving the `memory/` and `logs/` directories untouched.

Example: plexi agent update project-manager

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Agent name to update |

### `plexi agent list`

List agents installed in the current workspace

## `plexi context`

Manage the active context (the folder and project scope tied to the current pane)

| Subcommand | Description |
|---|---|
| `new` | Open a new context with an optional name |
| `open` | Switch the current pane to a context at the given path |
| `set-root` | Change the root folder for the active context |
| `current` | Print the id and name of the current pane's context as JSON |
| `describe` | Set the description for the active context |
| `zoom` | Zoom into a sub-context by its numeric context_id |
| `zoom-out` | Zoom out of the current sub-context to the parent |
| `push` | Push the focused pane into a new sub-context |
| `list` | List all open contexts as a JSON array |

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

### `plexi context push`

Push the focused pane into a new sub-context

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | no | Name for the new sub-context. Defaults to the pane name |

### `plexi context list`

List all open contexts as a JSON array

## `plexi app`

Manage your Plexi apps — open, install, list, scaffold, and inspect

| Subcommand | Description |
|---|---|
| `open` | Open an app or tool in a new pane |
| `install` | Install an app from a local path, a remote source, or a pack file |
| `uninstall` | Remove an installed app by id |
| `list` | Show all installed apps with their versions |
| `render` | Render an app headlessly (JSON frame tree by default, or PNG with --png) |
| `check` | Check a local app with manifest, SDK, and render-size checks |
| `info` | Show details about an installed app: id, name, version, and available tools |
| `init` | Create a new app from a template |
| `validate` | Check a Plexi app directory for errors before publishing or installing |
| `freeze` | Export your currently installed apps as a single TOML snapshot for sharing or backup |
| `publish` | Publish an app to the Plexi marketplace |
| `update` | Check installed apps for available updates |
| `action` | Send a semantic action to a running app pane |

### `plexi app open`

Open an app or tool in a new pane.

Pass an app id (e.g. `plexi app open snake`) or a path to an app directory containing a manifest.toml. Use `--mcp` to wrap an MCP server, or `--cli` to open any CLI tool with a Plexi UI.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<type_id>` | string | no | App id or path to open (mutually exclusive with --mcp and --cli) |
| `--mcp` | string (repeatable) | no | Wrap a stdio MCP server in a Plexi pane.  Example: plexi app open --mcp npx @modelcontextprotocol/server-filesystem /tmp |
| `--cli` | string | no | Wrap a CLI tool in a Plexi pane with a visual UI.  Example: plexi app open --cli git |
| `--down` / `-d` | flag | no | Split below |
| `--left` / `-l` | flag | no | Split left |
| `--up` / `-u` | flag | no | Split up |
| `--right` / `-r` | flag | no | Split right |
| `--tab` | flag | no | New tab |
| `--window` | flag | no | New window |
| `--from-pane-id` | string | no | Open the new pane relative to this pane ID instead of the focused pane |
| `<extra_args>` | string (repeatable) | no | Extra arguments passed through to the app (only valid with an app id) |

### `plexi app install`

Install an app from a local path, a remote source, or a pack file.

Local path: `plexi app install ./my-app` — copies the app dir into Plexi's store. Remote source: `plexi app install github:owner/repo` — fetches and installs from GitHub. Pack file: `plexi app install --pack core` — installs from a pack file or the built-in core pack. Workspace pack: `plexi app install` (no args) — installs from .plexi/apps.toml.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<spec_or_path>` | string | no | Source to install: a local path, GitHub spec (github:owner/repo), or bare app id. Omit to install from the workspace pack (.plexi/apps.toml) |
| `--pack` | string | no | Install from a pack file or 'core' |

### `plexi app uninstall`

Remove an installed app by id.

Example: plexi app uninstall github-tree

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes | App id to remove (use `plexi app list` to see installed ids) |
| `--yes` / `-y` | flag | no | Skip the confirmation prompt |

### `plexi app list`

Show all installed apps with their versions

### `plexi app render`

Render an app headlessly (JSON frame tree by default, or PNG with --png)

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<app>` | string | yes | App id or local path to render (e.g. "snake" or "./my-app") |
| `--size` | string | no | Image dimensions as WxH (e.g. 500x500) Default: `800x600`. |
| `--state` | string | no | Pre-seed the app's state from a JSON file before rendering |
| `--output` | string | no | Where to save the output (default: stdout) |
| `--png` | flag | no | Render to a PNG image instead of JSON (default: JSON) |

### `plexi app check`

Check a local app with manifest, SDK, and render-size checks.

This is the compiler-like gate for generated Plexi apps. It checks the manifest, inspects Python SDK usage without importing app code, and renders the app at small and normal pane sizes.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | Local app directory to check (default: current directory) Default: `.`. |
| `--size` | string (repeatable) | no | Render size to check as WxH. Repeat to override the default matrix |
| `--png-dir` | string | no | Write PNG snapshots for each checked size into this directory |

### `plexi app info`

Show details about an installed app: id, name, version, and available tools

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes |  |

### `plexi app init`

Create a new app from a template.

Scaffolds the folder structure and files you need to build a Plexi app, then opens it in a split-right pane so you can edit code alongside the running app.

By default, the app is placed in your workspace's app directory. If no workspace is detected, pass --global to scaffold into the global registry.

Use --no-open to scaffold without opening.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes |  |
| `--lang` | string | no | Default: `python`. |
| `--global` | flag | no | Scaffold into the global app registry instead of the workspace |
| `--no-open` | flag | no | Scaffold the app without opening it in a pane |
| `--from-pane-id` | string | no | Open the new pane relative to this pane ID instead of the focused pane. Defaults to PLEXI_PANE_ID if set in the environment |

### `plexi app validate`

Check a Plexi app directory for errors before publishing or installing

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | Path to check (default: current directory) Default: `.`. |

### `plexi app freeze`

Export your currently installed apps as a single TOML snapshot for sharing or backup.

Like `pip freeze` — captures exactly what's installed so you can replay it later with `plexi app install`.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Destination path for the TOML snapshot file |

### `plexi app publish`

Publish an app to the Plexi marketplace.

The Plexi app marketplace is under development. This command will be available in a future release.

### `plexi app update`

Check installed apps for available updates.

Compares each app's recorded installed version against the version in its manifest. In v1 this is a local check only — no network calls are made. Use `plexi update apps` for git-checkout apps.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | no | App id to check (omit to check all installed apps) |

### `plexi app action`

Send a semantic action to a running app pane.

Unlike `pane command` (which sends raw text), `app action` delivers a structured semantic event directly to the app's event handler — no keystroke simulation.

Example: plexi app action 42 refresh Example: plexi app action 42 navigate-to /some/path

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id of the target app pane (from `plexi pane list`) |
| `<action>` | string | yes | Action name to invoke (e.g. "refresh", "navigate-to", "add-item") |
| `<args>` | string (repeatable) | no | Optional arguments forwarded to the action handler |

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

Control panes — list, focus, send input, capture output, and more

| Subcommand | Description |
|---|---|
| `new` | Open a new terminal pane |
| `name` | Rename a pane |
| `list` | List all open panes as a JSON array |
| `focus` | Move the visible focus to a specific pane |
| `close` | Close a pane. Omit the pane id to close the pane you are currently in |
| `send` | Type text into another pane as if it came from the keyboard |
| `self` | Print the id of the pane you are currently in |
| `info` | Print details about the current pane (or the previously focused pane) as JSON |
| `capture` | Capture the last N lines of a pane's output as a JSON array |
| `key` | Send a key press to a pane |
| `command` | Send a shell command to a terminal pane as if typed from the keyboard |
| `state` | Return the current UI state of a pane as JSON |

### `plexi pane new`

Open a new terminal pane.

Examples: plexi pane new                          # empty terminal, split right plexi pane new "npm run dev" -n "dev"   # terminal with command, named plexi pane new -d                       # split below

For apps use `plexi app open`. For MCP servers use `plexi app open --mcp`.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<cmd>` | string | no | Shell command to run in the new terminal |
| `--name` / `-n` | string | no | Name the pane |
| `--down` / `-d` | flag | no | Split below instead of right |
| `--left` / `-l` | flag | no | Split left |
| `--up` / `-u` | flag | no | Split up |
| `--right` / `-r` | flag | no | Split right (explicit, same as default) |
| `--tab` | flag | no | New tab |
| `--window` | flag | no | New window |
| `--overlay` | flag | no | Overlay pane |
| `--from` | string | no | Pane ID to split relative to (default: focused pane) |
| `--ephemeral` / `-e` | flag | no | Close the pane when the command finishes |
| `--no-focus` | flag | no | Keep focus on the current pane |
| `--cwd` | string | no | Working directory |

### `plexi pane name`

Rename a pane.

With one argument, renames the current pane: plexi pane name "My Project" With two arguments, renames any pane by id: plexi pane name 42 "My Project"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<first>` | string | yes | Pane id (from `plexi pane list`) or the new name if renaming the current pane |
| `<second>` | string | no | New name when a pane id is given as the first argument |

### `plexi pane list`

List all open panes as a JSON array.

Filter by context: `--context` (no value) returns panes in the caller's context (reads PLEXI_CONTEXT_ID from env). `--context <id>` filters to a specific context ID.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--context` | string | no | Filter by context. With no argument, reads PLEXI_CONTEXT_ID from env (caller's context). With a numeric argument, returns panes in that specific context |

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

Type text into another pane as if it came from the keyboard.

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

Print details about the current pane (or the previously focused pane) as JSON

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--previous` | flag | no | Return info for the previously focused pane instead of the current one |

### `plexi pane capture`

Capture the last N lines of a pane's output as a JSON array.

Defaults to the current pane when no pane id is given.

Example: plexi pane capture --lines 50 42

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id to capture output from. Defaults to the current pane |
| `--lines` | string | no | How many lines to read from the end of the output Default: `50`. |
| `--full-output` | flag | no | Preserve trailing empty lines (by default they are stripped) |
| `--from-cursor` | string | no | Read only lines written after this cursor value. Get the cursor from a previous capture response. When set, the response is always JSON object format |

### `plexi pane key`

Send a key press to a pane.

For terminal panes, injects the keystroke into the terminal. For app panes, delivers a structured key event.

Key formats: single character ("h"), named key ("enter", "escape", "space", "up", "down", "left", "right", "backspace"), or chord ("ctrl+c").

Example: plexi pane key 42 enter

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to send the key to (from `plexi pane list`) |
| `<key>` | string | yes | Key to press |

### `plexi pane command`

Send a shell command to a terminal pane as if typed from the keyboard.

Use `--enter` to append a newline so the command is submitted immediately.

Example: plexi pane command 42 "git status" --enter

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to send the command to (from `plexi pane list`) |
| `<text>` | string | yes | Text to send to the pane |
| `--enter` / `-e` | flag | no | Append a newline after the text, submitting it as a command |

### `plexi pane state`

Return the current UI state of a pane as JSON.

For app panes: returns a JSON object with a `frame` array of RenderCommands representing the last-rendered L1 UiNode tree. Agents can use this to inspect what an app is currently displaying.

For terminal panes: returns a simple status object (type, title, pane_id).

Example: plexi pane state 42

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to query (from `plexi pane list`) |

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

## `plexi notify`

Send a notification to the Plexi UI

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--title` | string | yes | Notification title (required) |
| `--body` | string | no | Notification body text |
| `--level` | string | no | Severity level: info, warn, or error Default: `info`. |
| `--choice` | string (repeatable) | no | Add a clickable button to the notification. Format: `key:Label` (returns key when clicked) or `Label:pane_focus:<pane_id>` (switches focus to that pane when clicked). Repeatable |
| `--host-action` | string (repeatable) | no | Action to perform on the host when a button is clicked. Format: `key:action_type:action_arg`. Repeatable. The host runs this even after the process that sent the notification has exited |
| `--timeout` | string | no | How many seconds before the notification disappears (0 = stays until dismissed) Default: `0`. |
| `--scope` | string | no | Which panes see this notification: window, context, or global (default: global) Default: `global`. |

## `plexi ai`

AI configuration and diagnostics — scan hardware, check integrations, recommend models

| Subcommand | Description |
|---|---|
| `doctor` | Scan hardware and report recommended AI models |
| `setup` | Interactive wizard to configure a local AI model via Ollama |

### `plexi ai doctor`

Scan hardware and report recommended AI models.

Detects your CPU, RAM/VRAM, and GPU, then recommends which local or cloud AI models are a good fit. Also checks whether Ollama is installed and running, lists any already-pulled models, and verifies OpenRouter configuration.

Example: plexi ai doctor

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--json` | flag | no | Output results as JSON (for scripting or agent use) |

### `plexi ai setup`

Interactive wizard to configure a local AI model via Ollama.

Walks through Ollama installation detection, model recommendation based on your hardware, pulling the recommended model, and writing the [ai.ollama] section to your config.toml so Plexi apps can use it immediately.

Example: plexi ai setup

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

## `plexi doctor`

Audit all installed apps for capability and config gaps.

Checks every installed app's declared capabilities against your current config.toml and reports what's working and what needs to be configured. Use --json for scripting.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--json` | flag | no | Output results as JSON (for scripting or agent use) |

## `plexi demo`

Interactive keybinding tutorial — learn split and navigate in real time.

Walk through two fundamental Plexi interactions inside a live pane: split a pane (⌘D) and navigate between panes (⌘L / ⌘H). Must be run inside a Plexi pane (PLEXI_PANE_ID must be set).

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

## `plexi uninstall`

Uninstalls the app, CLI, and optionally your profile data.

Removes the current channel's app bundle (/Applications/Plexi.app), CLI binary (/usr/local/bin/plexi), and shell completions. Your profile directory (~/.plexi/) holds your settings, secrets, and app configurations — you will be asked whether to keep it.

Example: plexi uninstall

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--keep-data` | flag | no | Keep your profile directory (~/.plexi/) — your settings, secrets, and app data stay on disk |
| `--yes` / `-y` | flag | no | Skip the confirmation prompt and proceed immediately (removes data unless --keep-data is set) |

