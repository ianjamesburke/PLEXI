<h1 align="center"> under construction </h1>

> **Pre-release:** Plexi is pre-v1 and under active development. APIs, config format, and behavior may change without notice. The version will be reverted to v0 to reflect this.

<p align="center">
  <img src="assets/icon.svg" width="80" alt="Plexi" />
</p>

<h1 align="center">plexi</h1>

<p align="center">The last app you'll ever need.</p>

<p align="center">
  <img src="media/screenshot-3.png" width="96%" alt="Screenshot" />
</p>

One binary. A tiling shell that brings Unix composability to the desktop — terminals, apps, and AI agents all speak the same protocol. Pipe output between processes, route notifications across panes, query any model from any context.



---

## Contact

If you run into any issues, don't hesitate to reach out directly: adhdisntreal@gmail.com

---

## Install

> **macOS only.** Linux is untested.

### One-liner

```bash
curl -fsSL https://raw.githubusercontent.com/ianjamesburke/PLEXI/main/scripts/user-install.sh | sh
```

Downloads the latest release, installs to `/Applications`, sets up the `plexi` CLI, and wires ZSH integration. Restart your terminal when done.

**First launch (unsigned app):** macOS may block it on first open.
- **macOS 15+:** System Settings → Privacy & Security → "Open Anyway".
- **Or:** `xattr -cr /Applications/Plexi.app && open /Applications/Plexi.app`

### Manual

1. Download the latest `Plexi-vX.Y.Z.zip` from [Releases](https://github.com/ianjamesburke/PLEXI/releases).
2. Unzip and move `Plexi.app` to `/Applications`.
3. Run `xattr -cr /Applications/Plexi.app` if macOS blocks it.
4. On first launch, click **Install CLI** in the setup prompt to add `plexi` to your PATH.

### Build from source

Needs Rust ([rustup.rs](https://rustup.rs)).

```bash
just install
```

---

## Quick Note (`Cmd+0`)

Quick Note is a global capture modal for routing text to any destination without breaking your flow.

**Opening it:** `Cmd+0` from anywhere in Plexi opens the compose modal. Type your note, press `Enter` to advance to the destination picker, then press a digit key to route instantly — no second `Enter` needed.

**Destination 0** (global backlog) is always available regardless of config. It saves notes to `~/.plexi/backlog/` as timestamped Markdown files.

**Custom destinations** are configured in `[[quick_note.destinations]]` in your `config.toml`. Each destination gets a digit key (`1`–`9`) and a label shown in the picker.

### Destination types

**`backlog`** — writes the note as a Markdown file into a directory.

```toml
[[quick_note.destinations]]
key   = 1
label = "Backlog"
type  = "backlog"
path  = "~/.plexi/backlog"
```

**`pane`** — runs a shell command template in a new pane. Use `{note}` and `{cwd}` as tokens; both are shell-escaped before substitution.

```toml
[[quick_note.destinations]]
key      = 2
label    = "Ask Claude"
type     = "pane"
command  = "claude -p {note}"
position = "context-end"
```

The `position` field controls where the new pane opens:
- `context-end` — appended at the end of the current context
- `context-start` — inserted at the start of the current context
- *(omitted)* — splits the focused pane

**Submenus** — omit `type` and set `options = [...]` to expand into a sub-picker on keypress:

```toml
[[quick_note.destinations]]
key   = 3
label = "GitHub issue"
options = [
  { key = 1, label = "Bug",         command = "cd {cwd} && gh issue create --label bug --title {note} --body {note}" },
  { key = 2, label = "Enhancement", command = "cd {cwd} && gh issue create --label enhancement --title {note} --body {note}" },
  { key = 3, label = "No label",    command = "cd {cwd} && gh issue create --title {note} --body {note}" },
]
```

**Security:** Only `{note}` and `{cwd}` are shell-escaped. Do not interpolate other user-supplied values into command templates — they will not be escaped and are a shell injection risk.

---

## Notifications

Any process running inside Plexi — a terminal command, an app, or the CLI — can emit a notification. The notification bus routes events across panes so independent processes can communicate.

### Emitting a notification

**From the CLI:**

```bash
plexi notify --title "Job done" --body "Output is ready in ~/out.txt"
```

**With action choices:**

```bash
plexi notify --title "Deploy ready" --body "Review before promoting." \
  --choice "a:Open PR" \
  --choice "b:Skip"
```

**From an app (Python SDK):**

```python
ctx.notify("Job done", body="Output is ready in ~/out.txt", priority=ctx.PRIORITY_HIGH)
```

### Priority tiers

| Priority | Value | Behavior |
|----------|-------|----------|
| `PRIORITY_LOW` | 0 | Queues silently; badge ticks on toolbar |
| `PRIORITY_NORMAL` | 50 | Queues silently by default |
| `PRIORITY_HIGH` | 100 | Auto-opens the modal (default threshold) |
| `PRIORITY_CRITICAL` | 200 | Always interrupts |

The `interrupt_threshold` in `config.toml` sets the minimum priority that auto-opens the modal. Notifications below it queue silently — open `Cmd+Shift+A` to review.

### The notification modal (`Cmd+Shift+A`)

Keyboard-first navigation: `Enter` confirms / acknowledges, `j`/`k` or `↑`/`↓` cycles choices, `1`–`9` for direct selection, `Esc` defers (modal closes, notification stays in queue). Notifications with `required = true` cannot be deferred with `Esc`.

A toolbar badge shows the count of pending notifications. `Cmd+Shift+A` always gives feedback — if the queue is empty, a brief empty-state card appears.

---

## Apps

Apps are Python processes that render native UI and communicate with the host over PGAP. They declare capabilities in a manifest; the host enforces them at runtime.

A fresh install seeds a core set of apps automatically. Browse them with `Cmd+P` or manage them from the terminal.

### Install an app

```bash
plexi install <id>                         # from the registry
plexi install github:owner/repo            # any public git repo
plexi install git+https://example.com/repo.git    # explicit git URL
plexi install --pack path/to/pack.toml     # apply a whole pack at once
```

Registry IDs resolve against the [Plexi app registry](https://github.com/ianjamesburke/plexi-registry). Git URLs clone the repo directly — no registry needed.

### Manage installed apps

```bash
plexi list                    # show all installed apps and versions
plexi uninstall <id>          # remove an app
plexi update apps             # update all installed apps
plexi update apps <id>        # update a specific app
plexi validate <path>         # validate a manifest before publishing
```

### Scaffold an app

```bash
plexi app init my-app
```

Creates `.plexi/apps/my-app/` in the current directory with `manifest.toml`, `main.py`, and a bundled `plexi_sdk.py`. Edit `main.py`, open Plexi, and the app appears in the command palette immediately — no restart needed.

**Minimal `manifest.toml`:**

```toml
schema_version = 1
type = "app"

[app]
id = "my-app"
name = "My App"
entry = "main.py"
version = "0.1.0"
description = "What this app does"

[app.capabilities]
capabilities = []   # e.g. ["fs.read", "ai.query", "secrets.get"]

[launch]
layout_hint = { side = "right", split = 0.5 }
```

### App interaction model

Apps launch side-by-side with an agent pane (or alone, depending on `layout_hint`). The host spawns the app process and communicates over stdin/stdout using PGAP.

**Inside a frame** (`on_render`): call `ctx.rect(...)`, `ctx.text(...)`, or pass a UI tree to `ctx.render(...)`. The SDK handles layout, padding, and truncation.

**Outside frames**: use `emit.*` for out-of-frame actions — `emit.notify(...)`, `emit.info(...)`, `emit.error(...)`.

**Logging from an app:**

```python
ctx.info("State initialized")   # inside on_render
ctx.warn("Retrying connection")
ctx.error("Auth failed", detail=str(e))
emit.info("App starting up")    # outside frames / at module level
```

App logs forward into the host log tagged `app::<app_id>`. Check `~/.plexi/plexi.log` (or `~/.plexi-alpha/plexi.log` on alpha) when debugging.

To share your app: push the repo to GitHub, then anyone can install it with `plexi install github:you/your-app`. To add it to the public registry, open a PR against [plexi-registry](https://github.com/ianjamesburke/plexi-registry).

---

## PGAP — Plexi General App Protocol

PGAP is the wire protocol that every Plexi app speaks — built-in or third-party. It is the isolation boundary: no shared memory, no inherited file descriptors.

**Transport:** newline-delimited JSON on **stdin** (host → app) and **stdout** (app → host). One JSON object per line; no framing, no length prefix.

**Binary data** (audio PCM, video frames, raw bytes) travels on **typed pipes** — Unix sockets opened by the host on demand. The JSON wire carries only control and draw messages.

**Handshake:**
1. Host spawns the app process.
2. Host sends one `init` event (fields: `protocol`, `app_id`, `workspace_root`, `capabilities`, `feature_flags`).
3. App replies with `{"type": "ready", "sdk": "...", "features_used": [...]}`.
4. Each frame: host sends `render`; app replies with draw commands terminated by `frame_done`.
5. Input events (`key`, `click`, `command`, mouse) arrive between frames as they occur.
6. Out-of-frame commands (`notify`, `secret_get`, `capability_request`, etc.) arrive at any time; host processes them immediately.
7. On close: host sends `shutdown`; app must exit cleanly within a short timeout.

Current protocol version: **pgap/3**. Full reference: [PGAP.md](PGAP.md).

---

## Secrets management *(in development)*

Workspace-scoped secrets store credentials in the macOS Keychain without exposing them to the shell environment or other apps.

Secrets are keyed by a `(app_id, workspace_root, capability)` triple. An app initialized at `/foo` cannot read a secret granted at `/bar` — a new prompt appears for each workspace root. Secret injection into the shell environment is on the roadmap.

**CLI:**

```bash
plexi secret set <key>          # prompt for value, store in Keychain
plexi secret get <key>          # retrieve (requires workspace context)
plexi secret list               # list keys scoped to current workspace
plexi secret delete <key>       # remove from Keychain
```

**From an app**, request the `secrets.get` capability in the manifest and call:

```python
value = await ctx.secret_get("MY_API_KEY")
```

The host presents a permission prompt on first access; subsequent calls within the same session use the cached grant.

---

## CLI reference

All `plexi` subcommands work identically on alpha, beta, stable, and PR builds. `PLEXI_SOCKET` (set inside a Plexi pane) routes host commands to the correct running instance automatically.

| Command | Description |
|---------|-------------|
| `plexi install <id>` | Install an app from the registry or a git URL |
| `plexi uninstall <id>` | Remove an installed app |
| `plexi update apps [id]` | Update all apps, or a specific one |
| `plexi list` | List all installed apps and their versions |
| `plexi validate <path>` | Validate an app directory's manifest |
| `plexi pack` | Pack management (apply, list packs) |
| `plexi app init <name>` | Scaffold a new app in the current directory |
| `plexi open <app-id>` | Open an app pane |
| `plexi terminal [cmd]` | Open a terminal pane, optionally running a command |
| `plexi pane <subcommand>` | Pane management (name, close, list, focus) |
| `plexi notify` | Emit a notification (see Notifications section) |
| `plexi context` | Query or set the active workspace context |
| `plexi workspace` | Workspace management (init, status) |
| `plexi secret` | Secret management (set, get, list, delete) |
| `plexi run <name>` | Run a named command from the registry |
| `plexi completions <shell>` | Output shell completions (zsh, bash, fish) |

---

## Tiling layout

Split panes, navigate, and zoom with keyboard shortcuts. Press `Cmd+/` for the full shortcut list at any time.

| Shortcut | Action |
|----------|--------|
| `Cmd+D` | Split horizontally |
| `Cmd+Shift+D` | Split vertically |
| `Cmd+H/J/K/L` | Navigate between panes |
| `Cmd+Ctrl+K` | Send pane to the front |
| `Cmd+Ctrl+J` | Send pane to the back |
| `Cmd+Enter` | Zoom focused pane full-screen |
| `Cmd+W` | Close focused pane |
| `Cmd+P` | Command palette |
| `Cmd+T` | New tab |
| `Cmd+N` | New page (right) |
| `Cmd+R` | Rename pane |
| `Cmd+[` / `Cmd+]` | Focus history — back / forward |

---

## Development

```bash
just dev          # cargo run
just build        # cargo build --release
just install      # build + install to /Applications and /usr/local/bin
```

Built with Rust, [egui](https://github.com/emilk/egui), and [egui_term](https://github.com/niceda/egui_term).

---

## Contact

ADHDISNTREAL@GMAIL.COM
