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

### Download

1. Grab the latest `Plexi-vX.Y.Z.zip` from [Releases](https://github.com/ianjamesburke/PLEXI/releases).
2. Unzip and move `Plexi.app` to `/Applications`.
3. First launch (unsigned app):
   - **macOS 15+:** System Settings → Privacy & Security → "Open Anyway".
   - **Or:** `xattr -cr /Applications/Plexi.app`.

### Build from source

Needs Rust ([rustup.rs](https://rustup.rs)).

```bash
curl -fsSL https://raw.githubusercontent.com/ianjamesburke/PLEXI/main/install.sh | bash
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
