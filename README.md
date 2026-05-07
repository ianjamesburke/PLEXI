<h1 align="center"> under construction </h1>



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

## Install

> **macOS only.** Linux is untested.

### One-liner

```bash
curl -fsSL https://plexiapp.com/install | sh
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

## Features

**PGAP** — every pane communicates over a single protocol (newline-delimited JSON on stdin/stdout). No shared memory, no inherited file descriptors. Binary payloads travel on typed pipes alongside the command channel. The protocol is the isolation boundary.

**Notification bus** — any terminal process can emit a notification; any app or pane can receive it. Route events across the workspace to tie independent processes together.

**AI backend** — OpenRouter with configurable model tiers and real cost tracking. `ai.query()` is available in any app; agent panes run a full LLM turn loop backed by Claude or the Anthropic API.

**App runtime** — write apps in Python (bundled 3.12, zero setup) that render native UI, play audio, capture MIDI, and communicate over typed pipes. Apps declare capabilities in a manifest; the host enforces them.

**Workspace-scoped secrets** — credentials stored in macOS Keychain, keyed to a workspace root. An app at `/foo` cannot read a secret granted at `/bar` without a new prompt.

**App package manager** — install, update, uninstall, and list apps from the command palette or CLI.

**Tiling layout** — split panes horizontally or vertically, navigate with `Cmd+H/J/K/L`, zoom any pane full-screen with `Cmd+Enter`. Press `Cmd+/` for the full shortcut list.

**Command palette** (`Cmd+P`) — jump to any context or named pane, launch apps, run commands.

---

## Apps

A fresh install seeds a core set of apps automatically. Browse them with `Cmd+P` or manage them from the terminal.

### Install an app

```bash
plexi install <id>                        # from the registry
plexi install github:owner/repo           # any public git repo
plexi install git+https://example.com/repo.git   # explicit git URL
plexi install --pack path/to/pack.toml    # apply a whole pack at once
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

### Build an app

Every Plexi app is a git repo with a `manifest.toml` at the root and a Python entry point. Scaffold one with:

```bash
plexi app init my-app
```

This creates `.plexi/apps/my-app/` in the current directory with `manifest.toml`, `main.py`, and a bundled `plexi_sdk.py`. Edit `main.py`, launch Plexi, and the app appears in the command palette immediately — no restart needed.

The minimal `manifest.toml`:

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
capabilities = []   # e.g. ["fs.read", "ai.query"]

[launch]
layout_hint = { side = "right", split = 0.5 }
```

To share your app: push the repo to GitHub, then anyone can install it with `plexi install github:you/your-app`. To add it to the public registry, open a PR against [plexi-registry](https://github.com/ianjamesburke/plexi-registry).

---

## Roadmap

- Background apps — persist across pane close, restart on demand
- Apps can open terminal and app panes programmatically
- Brokered HTTP for apps via the `net` capability
- Secret injection into shell environment
- Auto-updater with toolbar badge

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
