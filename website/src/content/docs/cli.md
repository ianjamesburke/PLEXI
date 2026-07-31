---
title: CLI Reference
description: Complete reference for all plexi subcommands and flags.
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
| `<extra_args>` | string (repeatable) | no | Extra arguments forwarded to the command as $1, $2, … positional params |

## `plexi workspace`

Set up a .plexi/ workspace in your project folder.

Run this once inside your project directory to enable workspace-scoped secrets and commands.

| Subcommand | Description |
|---|---|
| `init` | Set up a .plexi/ workspace in the current directory |
| `clean` | Remove pane slot files for panes that are no longer open |

### `plexi workspace init`

Set up a .plexi/ workspace in the current directory.

Run this once inside your project folder. It creates a .plexi/workspace.toml so that secrets and commands are scoped to this project.

### `plexi workspace clean`

Remove pane slot files for panes that are no longer open

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--dry-run` | flag | no | Print slot directories that would be removed without deleting them |

## `plexi secret`

Store and retrieve secrets (API keys, passwords, tokens) for your project.

Secrets are saved to your system keychain and injected as environment variables when you run commands. Use `plexi workspace init` first to scope secrets to a project.

| Subcommand | Description |
|---|---|
| `set` | Save a secret to your keychain |
| `get` | Print a stored secret's value to stdout |
| `list` | Show stored secrets |
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

Show stored secrets.

Inside a workspace, shows project secrets plus user-scope secrets. Outside a workspace, falls back to user-scope secrets. Use --global to show only user-scope secrets from any directory.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` | flag | no | Show only globally-stored user-scope secrets |

### `plexi secret delete`

Delete a stored secret.

Use --global to delete a globally-stored secret (one stored with `secret set --global`). Without --global, deletes the workspace-scoped entry for the current project.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<friendly_name>` | string | yes |  |
| `--global` | flag | no | Delete from the global store instead of the project-scoped store |

## `plexi routine`

Manage workspace routines — scheduled shell commands.

Routines are declared in `routines.toml` inside the workspace channel directory — `{workspace_channel_dir}/routines.toml`, e.g. `.plexi-alpha/routines.toml` on the alpha channel — and run automatically on schedule. **Requires Plexi to be running** — there is no background daemon. Routines only fire while the host process is open.

Use `plexi routine list` to see configured routines, or `plexi routine run <name>` to fire one manually.

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

Manage workspace agent definitions.

Install agent definitions from the global registry (`~/.plexi/agents/`) into the current workspace's `.plexi/agents/` directory, each with scoped memory and logs.

| Subcommand | Description |
|---|---|
| `init` | Scaffold a new agent app with ai.query capability and a chat UI |
| `add` | Install an agent definition from the global registry into the current workspace |
| `update` | Re-install an agent definition from the global registry, preserving memory and logs |
| `list` | List agents installed in the current workspace |
| `report` | Report agent state for this pane to the host |
| `status` | Show current agent state for all panes |
| `hook` | Install or uninstall agent hook integrations |

### `plexi agent init`

Scaffold a new agent app with ai.query capability and a chat UI.

Creates the app directory, manifest.toml (with ai.query pre-configured), and main.py from the agent template. Equivalent to the former `plexi app init --agent <name>`.

Example: plexi agent init my-agent

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | App name (used as the directory name and app ID) |
| `--from` | string | no | Open the new pane relative to this pane ID. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane |

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

### `plexi agent report`

Report agent state for this pane to the host.

Called internally by hook scripts. Requires PLEXI_SOCKET and PLEXI_PANE_ID to be set in the environment.

Example: plexi agent report --state working --agent claude-code

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--state` | string | yes | State to report: working, blocked, or idle |
| `--agent` | string | no | Agent name (e.g. "claude-code") Default: `unknown`. |
| `--detail` | string | no | Active tool detail (optional, from hook event JSON) |
| `--session-id` | string | no | Session ID (optional, from hook event JSON) |

### `plexi agent status`

Show current agent state for all panes.

Queries the host for all panes that have reported agent state via hooks. Formats as a table with pane ID, agent name, state, and session ID.

Example: plexi agent status Example: plexi agent status --blocked

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--blocked` | flag | no | Show only blocked panes |
| `--working` | flag | no | Show only working panes |
| `--idle` | flag | no | Show only idle panes |

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
| `new` | Open a new context with an optional name |
| `sub` | Create a sub-context under the current one, pre-populated with panes |
| `open` | Switch the current pane to a context at the given path |
| `set-root` | Change the root folder for the active context |
| `current` | Print the id and name of the current pane's context as JSON |
| `describe` | Set the description for the active context |
| `zoom` | Zoom into a sub-context by its numeric context_id |
| `zoom-out` | Zoom out of the current sub-context to the parent |
| `push` | Push a pane into a new sub-context |
| `list` | List all open contexts as a JSON array |

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
| `--from` | string | no | Pane to anchor the portal split at (requires --parent). Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the parent context's focused pane |
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
| `--layout` | string | no | How the panes are arranged inside the sub-context's window Default: `tiled`. |
| `--focus` | flag | no | Zoom into the new sub-context after creation. Default: stay put |
| `--from` | string | no | Pane to anchor the portal split at. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the parent's focused pane |

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

Push a pane into a new sub-context.

Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | no | Name for the new sub-context. Defaults to the pane name |
| `--pane-id` | string | no | Pane to push. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane |

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
| `test` | Run an app's AppHarness tests with `uv run pytest tests/` |
| `info` | Show details about an installed app: id, name, version, and available tools |
| `state` | Read or replace a file-backed app's state document (stint 0645) |
| `init` | Create a new app from a template |
| `validate` | Check a Plexi app directory or .plexipkg package for errors before publishing or installing |
| `inspect` | Show the trust sheet for a local app directory or .plexipkg package |
| `package` | Build a distributable .plexipkg package from an app directory |
| `freeze` | Export your currently installed apps as a single TOML snapshot for sharing or backup |
| `publish` | Validate, package, and submit an app to the Plexi marketplace |
| `browse` | Browse every public app in the hosted marketplace |
| `search` | Search the public marketplace catalog |
| `update` | Pull git-backed installed apps to their latest source revision |
| `action` | Send a semantic action to a running app pane |

### `plexi app open`

Open an app or tool in a new pane.

Pass an app id (e.g. `plexi app open calc`) or a path to an app directory containing a manifest.toml. Use `--mcp` to wrap an MCP server, or `--cli` to open any CLI tool with a Plexi UI.

Default placement is a sibling split to the right — the calling pane is never taken over. Pass a direction flag (--down/--left/--up/--right), --tab, or --window to override; the app's manifest `[launch] placement` applies when no flag is given.

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
| `--from` | string | no | Open the new pane relative to this pane ID. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane |
| `<extra_args>` | string (repeatable) | no | Extra arguments passed through to the app (only valid with an app id) |

### `plexi app trust`

Pre-approve a raw `.wasm` component's host imports without a prompt.

The non-interactive form of the review `plexi app open <file.wasm>` runs at a TTY: it persists Green grants for every host interface the component imports, scoped to the wasm's parent directory — the same store and scope an installed host checks at open time. Meant for automation (release gates, scene runners) pre-approving a vetted, repo-committed fixture so an installed host opens it without a human at a terminal. It never opens a pane; production interactive review is unchanged.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | yes | Path to the `.wasm` component to trust |

### `plexi app install`

Install an app from a local path, a remote source, or a pack file.

Local path: `plexi app install ./my-app` — copies the app dir into Plexi's store. Remote source: `plexi app install github:owner/repo` — fetches and installs from GitHub. Pack file: `plexi app install --pack core` — installs from a pack file or the built-in core pack. Workspace pack: `plexi app install` (no args) — installs from .plexi/apps.toml.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<spec_or_path>` | string | no | Source to install: a local path, GitHub spec (github:owner/repo), or bare app id. Omit to install from the workspace pack (.plexi/apps.toml) |
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

Run an app's AppHarness tests with `uv run pytest tests/`.

Runs the Python tests in the app's `tests/` directory (the `tests/test_app.py` scaffolded by `plexi app init`). AppHarness spawns the app as a real subprocess and checks it renders without overlap; see `plexi_sdk/testing.py`. Exits nonzero on failure so CI can gate on it.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | App directory to test (default: current directory) Default: `.`. |
| `--snapshot` | flag | no | Update stored snapshots instead of comparing against them |

### `plexi app info`

Show details about an installed app: id, name, version, and available tools

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<id>` | string | yes |  |

### `plexi app state`

Read or replace a file-backed app's state document (stint 0645).

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

Create a new app from a template.

Scaffolds the folder structure and files you need to build a Plexi app: manifest.toml, main.py, tests/test_app.py, AGENTS.md, .gitignore, and plexi.scaffold.toml drift metadata.

By default, the app is placed in your workspace's app directory. If no workspace is detected, pass --global to scaffold into the global registry.

Use --open to launch it in a split-right pane after scaffolding.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | no |  |
| `--wasm` | string | no | Create a Rust WASM component app with the Plexi WASM SDK |
| `--lang` | string | no | App language/template: `python` (declarative UI) or `python_agent` (agent-loop app). Use `--wasm <name>` for a Rust WASM component Default: `python`. |
| `--global` | flag | no | Scaffold into the global app registry instead of the workspace |
| `--open` | flag | no | Open the app in a split-right pane after scaffolding |
| `--no-open` | flag | no | Deprecated compatibility flag. App init no longer opens by default |
| `--from` | string | no | Open the new pane relative to this pane ID. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane |

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

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<path>` | string | no | App directory to publish (default: current directory) Default: `.`. |

### `plexi app browse`

Browse every public app in the hosted marketplace

### `plexi app search`

Search the public marketplace catalog

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
| `<pane_id>` | string | yes | Pane id of the target app pane (from `plexi pane list`) |
| `<action>` | string | yes | Action name to invoke (e.g. "refresh", "navigate-to", "add-item") |
| `<args>` | string (repeatable) | no | Optional arguments forwarded to the action handler |

## `plexi account`

Manage your Plexi marketplace account (only needed to publish or buy paid apps).

Free apps install without an account. Login requires the accounts backend enabled (`[marketplace].account_backend = "plexi"`); otherwise it fails closed with a clear message.

| Subcommand | Description |
|---|---|
| `status` | Show whether you are logged in |
| `login` | Log in to your marketplace account via emailed sign-in link |
| `logout` | Log out and clear the local session |

### `plexi account status`

Show whether you are logged in

### `plexi account login`

Log in to your marketplace account via emailed sign-in link.

Runs the device-code flow: plexiapp.com emails a link, you click it in any browser, and the CLI stores the session. Magic-link login creates the account on first use — there is no separate signup.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--email` | string | no | Account email (falls back to [marketplace].account_email in config) |

### `plexi account logout`

Log out and clear the local session

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
| `heartbeat` | Configure a host-owned recurring prompt for a terminal pane |
| `close` | Close a pane. Omit the pane id to close the pane you are currently in |
| `send` | Type text into another pane as if it came from the keyboard |
| `self` | Print the id of the pane you are currently in |
| `info` | Print details about the current pane (or the previously focused pane) as JSON |
| `capture` | Capture the last N lines of a pane's output as a JSON array |
| `status` | Report one composite working, idle, blocked, or unknown verdict as JSON |
| `key` | Send a key press to a pane |
| `drop` | Drop a local file or image URL onto a pane |
| `click` | Inject a synthetic pointer click into an app pane, for driving canvas interaction without OS-level automation |
| `drag` | Drag the pointer across an app pane through the production input path: press, N intermediate moves, release — delivered one frame at a time |
| `command` | Send an ordinary shell command to a terminal pane as if typed from the keyboard |
| `state` | Return the current UI state of a pane as JSON |
| `slot` | Manage host-managed named file slots for a pane |

### `plexi pane new`

Open a new terminal pane.

Examples: plexi pane new                          # empty terminal, split right plexi pane new "npm run dev" -n "dev"   # terminal with command, named plexi pane new -d                       # split below plexi pane new --agent c-large          # agent pane, id printed once it is ready

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
| `--from` | string | no | Pane ID to split relative to. Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the focused pane |
| `--ephemeral` / `-e` | flag | no | Close the pane when the command finishes |
| `--no-focus` | flag | no | Keep focus on the current pane |
| `--cwd` | string | no | Working directory |
| `--agent` | string | no | Launch an agent in the new pane and block until it reports ready.  The value is a shell command — normally a size-tier alias such as `c-large` or `codex-small` — run in the new pane verbatim. The pane id is printed only once the agent has booted, so a successful id on stdout means the pane is ready to receive a brief. On boot timeout the reason and the created pane id go to stderr, stdout stays empty, and the exit code is 2 so the caller can close the pane.  Requires a running Plexi host (PLEXI_SOCKET). |
| `--boot-timeout` | string | no | How long to wait for the agent to report ready, in seconds. Defaults to 60 when omitted (the host owns the default) |

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

### `plexi pane heartbeat`

Configure a host-owned recurring prompt for a terminal pane.

Example: plexi pane heartbeat 42 --every 5m --text "cycle"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to prompt (from `plexi pane list`) |
| `--every` | string | no | Interval such as 30s, 5m, or 1h |
| `--text` | string | no | Prompt text submitted on each beat |
| `--while-idle-only` | flag | no | Only submit when the shared agent detector reports idle (default) |
| `--off` | flag | no | Disable the pane heartbeat |

### `plexi pane close`

Close a pane. Omit the pane id to close the pane you are currently in

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id to close (from `plexi pane list`). Defaults to the current pane if not given |

### `plexi pane send`

Type text into another pane as if it came from the keyboard.

Terminal panes receive PTY bytes. App panes receive one real egui text input event after the host focuses the target pane. Use `\n` in terminal text to press Enter (which submits a command).

Example: plexi pane send 42 "git status\n"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to send text to (from `plexi pane list`) |
| `<text>` | string | yes | Text to type into the pane (use `\n` for Enter) |
| `--submit` / `-s` | flag | no | Press Enter for you and wait until the host confirms the prompt left the pane's input line.  The host settles the pane, submits, and re-sends Enter once if the text is still parked as a collapsed paste. Exits 0 only when the submission is confirmed; on an unconfirmed submit it exits non-zero and prints the observed input line to stderr. Terminal panes only. |

### `plexi pane self`

Print the id of the pane you are currently in.

Useful in scripts: MY_PANE=$(plexi pane self)

### `plexi pane info`

Print details about the current pane (or the previously focused pane) as JSON

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--previous` | string | no | Return info for a previously focused pane. With no value, returns the immediately previous pane (step 1). Provide an integer N to walk back N steps in focus history.  Examples: plexi pane info --previous      # pane focused 1 step ago plexi pane info --previous 3    # pane focused 3 steps ago |

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
| `--plain` | flag | no | Print only captured lines on stdout. The next cursor is printed to stderr |

### `plexi pane status`

Report one composite working, idle, blocked, or unknown verdict as JSON

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to inspect |

### `plexi pane key`

Send a key press to a pane.

For terminal panes, injects the keystroke into the terminal. For app panes, delivers a structured key event.

Key formats: single character ("h"), named key ("enter", "escape", "space", "up", "down", "left", "right", "backspace", "plus", "minus", "equals"), or chord ("ctrl+c").

Example: plexi pane key 42 enter

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to send the key to (from `plexi pane list`) |
| `<key>` | string | yes | Key to press |

### `plexi pane drop`

Drop a local file or image URL onto a pane

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes |  |
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
| `<pane_id>` | string | yes | Pane id to click (from `plexi pane list`) |
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
| `<pane_id>` | string | yes | Pane id to drag in (from `plexi pane list`) |
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
| `<pane_id>` | string | yes | Pane id to send the command to (from `plexi pane list`) |
| `<text>` | string | yes | Text to send to the pane |
| `--enter` / `-e` | flag | no | Submit the command through the host-confirmed settle -> Enter -> confirm sequence, instead of just typing it |

### `plexi pane state`

Return the current UI state of a pane as JSON.

For app panes: returns a versioned, runtime-neutral `semantic` tree. Process apps also retain the compatible `frame` RenderCommand array. Agents can use the semantic nodes to inspect what any app runtime is currently displaying.

For terminal panes: returns a simple status object (type, title, pane_id).

Example: plexi pane state 42

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | yes | Pane id to query (from `plexi pane list`) |

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
| `--pane-id` | string | no | Pane id. Defaults to PLEXI_PANE_ID |
| `--append` | flag | no | Append to an existing slot instead of replacing it |
| `--replace` | flag | no | Replace an existing slot |

#### `plexi pane slot read`

Print raw bytes from a named pane slot

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Slot name |
| `<pane_id>` | string | no | Pane id. Defaults to PLEXI_PANE_ID |

#### `plexi pane slot wait`

Block until a slot's value matches a pattern, then print it.

Exits 0 on a match, 2 on timeout (nothing on stdout), 1 on error. A slot whose current value already matches returns immediately.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Slot name |
| `<pane_id>` | string | no | Pane id. Defaults to PLEXI_PANE_ID |
| `--until` | string | yes | Regex the slot value must match |
| `--timeout` | string | no | Seconds to wait before giving up Default: `300`. |

#### `plexi pane slot list`

List slots for a pane as JSON

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<pane_id>` | string | no | Pane id. Defaults to PLEXI_PANE_ID |

#### `plexi pane slot delete`

Delete a named pane slot

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<name>` | string | yes | Slot name |
| `<pane_id>` | string | no | Pane id. Defaults to PLEXI_PANE_ID |

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

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--json` | flag | no | Output as JSON instead of a human-readable table |

### `plexi events mcp-config`

Print the host MCP server config for an MCP-aware agent.

Emits a `mcpServers` JSON block pointing at this instance's host MCP server (read from `PLEXI_HOST_MCP_PORT` / `PLEXI_HOST_MCP_TOKEN`), so a Claude Code or Codex agent in this pane can call workspace app tools and subscribe to app events natively over MCP. The emitted credential is valid only while the originating pane remains alive.

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
| `list` | Print all known config keys with type, current value, and description |
| `set` | Set one or more config keys in-place |

### `plexi config check`

Validate your config.toml and report any errors

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only |

### `plexi config edit`

Open config.toml in your $EDITOR

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only |

### `plexi config get`

Print the resolved value of a config key to stdout.

Supports dotted keys: agents.low, agents.medium, agents.high. Returns the effective value (user setting or built-in default).

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only |
| `<key>` | string | yes | Dotted key to retrieve (e.g. agents.medium) |

### `plexi config reset`

Overwrite config.toml with the built-in default template.

Creates a backup at config.toml.bak before overwriting.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only |

### `plexi config list`

Print all known config keys with type, current value, and description.

Columns: key\ttype\tvalue\tdescription. Use --json for machine-readable output.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only |
| `--json` | flag | no | Output as a JSON array instead of tab-separated lines |

### `plexi config set`

Set one or more config keys in-place.

Each argument must be in KEY=VALUE form (e.g. theme.preset=dracula font_size=14). Scope defaults to workspace when inside a workspace, global otherwise.

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--global` / `-g` | flag | no | Use the global channel config.toml only |
| `--workspace` / `-w` | flag | no | Use the active workspace's channel-scoped config.toml only |
| `<pairs>` | string (repeatable) | yes | One or more KEY=VALUE pairs to write |

## `plexi notes`

Browse and open scratchpad notes created with Cmd+Shift+Space.

Each scratchpad session writes a timestamped file to `<config_dir>/notes/`. Use `plexi notes list` to print note paths, or `plexi notes open` to pick one with fzf.

| Subcommand | Description |
|---|---|
| `list` | Print paths of all scratchpad notes, newest first |
| `open` | Open a note picker with fzf in the focused terminal pane |
| `inbox` | List notes in the inbox with frontmatter context |
| `process` | Print inbox notes in agent-legible format with configured triage actions |

### `plexi notes list`

Print paths of all scratchpad notes, newest first

### `plexi notes open`

Open a note picker with fzf in the focused terminal pane.

Requires fzf to be installed. Falls back to printing the notes directory when fzf is not available or PLEXI_SOCKET is not set.

### `plexi notes inbox`

List notes in the inbox with frontmatter context

### `plexi notes process`

Print inbox notes in agent-legible format with configured triage actions

## `plexi note`

Capture a quick note to the inbox.

Writes a timestamped note to `<config_dir>/notes/inbox/` with frontmatter capturing cwd, workspace, and context root. Triage later via Cmd+O, then t.

Example: plexi note "remember to update the docs"

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `<text>` | string | yes | Note text to capture |

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
| `apps` | Compatibility alias for `plexi app update` |

### `plexi update apps`

Compatibility alias for `plexi app update`.

Omit the app id to update all installed apps visible from the current workspace.

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

## `plexi host`

Launch, stop, or check a headless-friendly Plexi host from the CLI.

`host start` launches this channel's app bundle detached from the calling shell, optionally seeding panes from a `--layout` TOML file or repeated `--pane` flags, then blocks until the host confirms it's ready. Works identically on alpha, beta, main, and PR builds — the channel is resolved from the running CLI binary's own name.

| Subcommand | Description |
|---|---|
| `start` | Launch this channel's app bundle detached and wait for readiness |
| `stop` | Stop the running host for this channel |
| `log` | Write one info-level marker line into the running host's channel log |
| `status` | Report whether this channel's host is running, its pid, socket path, and pane count |
| `screenshot` | Capture the running host window as a PNG through the real render pipeline — the pixels the user actually sees, no OS screen capture |

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

Stop the running host for this channel.

Sends a clean shutdown request first, falling back to SIGTERM if the host doesn't confirm exit in time.

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

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--json` | flag | no | Output as JSON |

### `plexi host screenshot`

Capture the running host window as a PNG through the real render pipeline — the pixels the user actually sees, no OS screen capture.

Example: plexi host screenshot --pane 3 --output /tmp/pane3.png

| Flag / Arg | Type | Required | Description |
|---|---|---|---|
| `--pane` | string | no | Crop the capture to this pane's current screen rect |
| `--output` / `-o` | string | no | Where to write the PNG (default: <profile>/screenshots/<timestamp>.png) |

