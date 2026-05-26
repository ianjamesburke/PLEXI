---
title: Installation & Setup
description: Get Plexi running on your Mac.
verified_version: "0.0.505"
order: 1
---

Plexi runs on macOS (Apple Silicon or Intel, macOS 12 Monterey or later).

## Install

Open Terminal and run:

```sh
curl -fsSL https://plexiapp.com/install | sh
```

You'll be asked for your password. The script installs Plexi.app to /Applications, sets up the `plexi` CLI in /usr/local/bin, and adds shell completions. Launch Plexi from Applications or Spotlight.

<details>
<summary>Prefer to install manually?</summary>

Download the `.dmg` from the [GitHub releases page](https://github.com/ianjamesburke/PLEXI/releases/latest), open it, and drag Plexi to your Applications folder.

Note: the install script also sets up the CLI and shell completions automatically. If you install manually, open Plexi after dragging it to Applications — it will prompt you to install the CLI on first launch.
</details>

On first launch, Plexi will request Accessibility permission. This is required for context-aware features.

## Your First Session

Plexi opens with a single terminal pane. From there:

| Action | Shortcut |
|--------|----------|
| Split horizontally | `⌘D` |
| Split vertically | `⌘⇧D` |
| Navigate panes | `⌘H` / `⌘J` / `⌘K` / `⌘L` |
| Close pane | `⌘W` |
| Rename pane | `⌘R` |

## Build Channels

Plexi ships three channels. Each is a fully isolated instance with its own binary, profile directory, and app bundle — you can run all three simultaneously.

| Channel | Binary | Profile |
|---------|--------|---------|
| Stable | `plexi` | `~/.plexi/` |
| Beta | `plexi-beta` | `~/.plexi-beta/` |
| Alpha | `plexi-alpha` | `~/.plexi-alpha/` |

## Next Steps

- [Quick Note](/docs/quick-note) — open a persistent scratch pane from anywhere
- [Panes & Pages](/docs/panes) — understand the layout model
- [Apps](/docs/apps) — run sandboxed apps inside Plexi
