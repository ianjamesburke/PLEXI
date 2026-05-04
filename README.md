<p align="center">
  <img src="assets/icon.svg" width="80" alt="Plexi" />
</p>

<h1 align="center">plexi</h1>

<p align="center">The last app you'll ever need.</p>

<p align="center">
  <img src="media/screenshot-3.png" width="96%" alt="Screenshot" />
</p>

One window. Tiling panes of terminals and Python apps, each isolated behind a single protocol (PGAP). Split any pane, run any app, chain apps together with typed pipes.

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

**PGAP** — every app communicates over newline-delimited JSON on stdin/stdout. No shared memory, no inherited file descriptors. Binary payloads (audio PCM, video frames) travel on typed pipes alongside the command channel.

**Python app runtime** — bundled Python 3.12, self-contained. Write a Plexi app in Python with zero environment setup. Apps declare capabilities in a manifest; the host enforces them at runtime.

**Tiling layout** — split panes horizontally or vertically, navigate with `Cmd+H/J/K/L`, zoom any pane to full-screen with `Cmd+Enter`. Press `Cmd+/` to see the full shortcut list.

**Workspace-scoped secrets** — credentials stored in macOS Keychain, keyed to a workspace root. An app at `/foo` cannot read a secret granted at `/bar` without a new prompt.

**App package manager** — install, update, uninstall, and list apps from the command palette or CLI. Ships with a bundled core pack.

**AI backend** — OpenRouter with configurable model tiers and real cost tracking. `ai.query()` is available in any app via the `llm` capability.

**Command palette** (`Cmd+P`) — jump to any context or named pane instantly, launch apps, run commands.

**Navigation stack** — `PushNav` / `PopNav` / `NavBack` for multi-screen flows inside a single app pane.

**CoreAudio + CoreMIDI** — typed pipes for audio capture and MIDI I/O. Apps request the `audio_capture` capability; the host routes PCM to the pipe.

**Agent panes** — IQ turn loop backed by Claude CLI or the Anthropic API. Runs in a pane alongside your terminals and apps.

---

## Roadmap

Near-term work tracked in GitHub Issues:

- **SpawnPane** — apps open terminal and app panes programmatically ([#527](https://github.com/ianjamesburke/PLEXI/issues/527))
- **Background apps** — apps that survive pane close and restart on demand ([#292](https://github.com/ianjamesburke/PLEXI/issues/292))
- **Notifications** — `DrawCommand::Notify` + interactive notification panel ([#291](https://github.com/ianjamesburke/PLEXI/issues/291))
- **Secret injection** — `emit.get_secret()` SDK method + shell env injection ([#296](https://github.com/ianjamesburke/PLEXI/issues/296))
- **Net capability** — brokered HTTP for apps via the `net` capability ([#412](https://github.com/ianjamesburke/PLEXI/issues/412))
- **Auto-updater** — once-a-day update check with toolbar badge ([#486](https://github.com/ianjamesburke/PLEXI/issues/486))

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
