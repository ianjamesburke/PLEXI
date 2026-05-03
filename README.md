<p align="center">
  <img src="assets/icon.svg" width="80" alt="Plexi" />
</p>

<h1 align="center">plexi</h1>

<p align="center">The last app you'll ever need.</p>

<p align="center">
  <img src="media/screenshot-3.png" width="96%" alt="Screenshot" />
</p>

**Mac only** — Linux untested.

One window. Terminals, apps, and agents all install into it — each isolated behind a single protocol (PGAP). Split any pane, run any app, talk to any agent. Nothing leaks between workspaces.

---

## Install

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

## Keyboard Shortcuts

| Action | Shortcut |
|---|---|
| Command palette | `Cmd+P` |
| New terminal | `Cmd+N` |
| Split right | `Cmd+D` |
| Split below | `Cmd+Shift+D` |
| Navigate panes | `Cmd+H/J/K/L` |
| Move pane | `Cmd+Shift+H/J/K/L` |
| Close pane | `Cmd+W` |
| New context | `Cmd+T` |
| Cycle contexts | `Cmd+]` / `Cmd+[` |
| Zoom pane | `Cmd+Enter` |
| Open config | `Cmd+,` |
| Reload config | `Cmd+Shift+,` |
| Show shortcuts | `Cmd+/` |
| Quit | `Cmd+Q` |

---

## What's in v3

- **PGAP** — clean protocol over stdin/stdout. Every pane is a `Terminal`, `App`, or `Agent`. Binary side channel via typed pipes for audio, MIDI, and video.
- **Bundled Python 3.12** — self-contained runtime; no system Python dependency. Write apps in Python with zero setup.
- **Workspace-scoped secrets** — a secret granted in one workspace never leaks to a sibling without a brokered prompt.
- **App package manager** — install, uninstall, update, list with a bundled core pack.
- **OpenRouter AI backend** — configurable model tiers, real cost tracking. `ai.query()` available in any app.
- **Command palette** (`Cmd+P`) — jump to any context or named pane instantly.
- **Navigation stack** — `PushNav` / `PopNav` / `NavBack` for multi-screen app flows.
- **CoreAudio + CoreMIDI** — typed pipes for audio capture and MIDI I/O on macOS.
- **Agent Workspace** — spawn Claude Code agents with repo context from inside Plexi.

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
