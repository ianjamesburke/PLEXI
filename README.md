<p align="center">
  <img src="assets/app-icon.png" width="100" alt="Plexi icon" />
</p>

<h1 align="center">Plexi</h1>

<p align="center">A terminal multiplexer that hosts apps.</p>

<p align="center">
  <img src="media/screenshot-3.png" width="96%" alt="Screenshot" />
</p>

**Mac only** — Linux untested.

One window. Everything else installs into it — terminals, apps, agents — all isolated behind a single protocol (PGAP).

---

## Status

Plexi is in active development on a clean v3.0 rewrite. The v2.x tree (on `alpha`) is frozen and being retired. See [`STATE_OF_PLEXI.md`](STATE_OF_PLEXI.md) for the current architecture and [`docs/specs/releases/plexi-v3.0.md`](docs/specs/releases/plexi-v3.0.md) for the v3 spec.

Stable v1 releases continue to work. If you just want tiling terminals today, use a tagged release.

---

## Quick Start

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

## Keyboard Shortcuts

| Action | Shortcut |
|---|---|
| Split right | `Cmd+D` |
| Split below | `Cmd+Shift+D` |
| Navigate panes | `Cmd+H/J/K/L` |
| Close pane | `Cmd+W` |
| New tab | `Cmd+T` |
| Cycle tabs | `Cmd+]` / `Cmd+[` |
| Zoom pane | `Cmd+Enter` |
| Show shortcuts | `Cmd+/` |
| Quit | `Cmd+Q` |

---

## What v3 adds

- **Pane ADT** — every pane is a `Terminal`, an `App`, or an `Agent`. One clear model.
- **PGAP v3** — clean protocol over stdin/stdout. Binary side channel via typed pipes for audio PCM, video frames, arbitrary media.
- **Directory-scoped secrets** — hard invariant: a secret granted in one workspace never leaks to a sibling or child without a brokered prompt.
- **Host-owned media** — audio record/playback and video playback as first-class protocol commands. Mock devices for headless testing.
- **Plexi IQ** — agent panes wired from day one.
- **Five example apps:** `snake`, `wikipedia`, `todo`, `audio-recorder`, `video-player`. Plus `quick-note` as a first-party productivity app.

What v3 explicitly drops: recursion / fractal PGAP, `Pane::Embedded`, portals, OpenIntent-as-v2-spec'd. See [`docs/specs/releases/plexi-v3.0.md`](docs/specs/releases/plexi-v3.0.md) §12.

---

## Development

```bash
just dev          # cargo run
just build        # cargo build --release
just install      # build + install to /Applications and /usr/local/bin
```

Built with Rust, [egui](https://github.com/emilk/egui), and [egui_term](https://github.com/niceda/egui_term).
