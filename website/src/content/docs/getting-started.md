---
title: Install
description: Install Plexi on an Apple Silicon Mac, open an unsigned build, and learn the first shortcuts.
order: 1
---

Plexi runs on Apple Silicon Macs today. Windows and Linux builds are coming soon.

## Install

Paste this into a terminal:

```sh
curl -fsSL https://plexiapp.com/install | bash -s -- --channel beta
```

The installer downloads the current beta release for your Mac, checks the
archive's SHA-256 checksum, and installs into your home directory. It needs
`bash`, `curl`, `tar` and `shasum`. It does not need Rust, Git, a package
manager or administrator access.

The beta's launcher is `plexi-beta`. Inside a Plexi pane, plain `plexi` runs
the build that pane belongs to, so the commands in these docs work as written.
The [Download page](/download) lists which release each channel currently
installs.

## Open an unsigned build

Plexi releases are signed with a self-signed identity and are not notarized by
Apple, so macOS warns the first time you open one.

1. Open `Plexi.app` from the `Applications` folder in your home directory, then dismiss the warning.
2. Open **System Settings > Privacy & Security** and choose **Open Anyway** for Plexi.
3. Confirm, and authenticate if macOS asks.

Only do this for an archive that came from an
[official Plexi GitHub release](https://github.com/ianjamesburke/PLEXI/releases).
The [unsigned build guide](https://github.com/ianjamesburke/PLEXI/blob/alpha/docs/unsigned-install.md)
has the full steps.

Plexi does not ask for Accessibility permission on first launch. Its app bundle
declares camera and microphone permissions, which macOS requests only when a
feature uses them.

## First five shortcuts

Plexi opens with one terminal pane running your shell.

| Action | Shortcut |
|--------|----------|
| Split horizontally | `⌘D` |
| Split vertically | `⌘⇧D` |
| Move focus | `⌘H` / `⌘J` / `⌘K` / `⌘L` |
| Close pane | `⌘W` |
| Command palette | `⌘P` |

The command palette lists every action, so you can find the rest from there.
`plexi demo` runs a short interactive tutorial for splitting and moving
between panes. [Panes](/docs/panes) has the full set, and
[Configuration](/docs/config) shows how to rebind any of them.

## Set up AI (optional)

```sh
plexi ai onboard
```

This checks your hardware and existing setup, then walks you to a working
model: local Ollama, your own OpenRouter key, or skip it for now. Plexi does not
need a model to host your terminals and agents. Plugins that call a model do.

## Channels

Plexi ships three channels. Each one has its own binary, profile directory and
app bundle, so you can run them side by side.

| Channel | Binary | Profile | Workspace directory |
|---------|--------|---------|---------------------|
| Stable | `plexi` | `~/.plexi/` | `<repo>/.plexi/` |
| Beta | `plexi-beta` | `~/.plexi-beta/` | `<repo>/.plexi-beta/` |
| Alpha | `plexi-alpha` | `~/.plexi-alpha/` | `<repo>/.plexi-alpha/` |

The install command above passes `--channel beta`. Pass `--channel alpha` for
the alpha channel instead:

```sh
curl -fsSL https://plexiapp.com/install | bash -s -- --channel alpha
```

Features outside the stable surface are experiments, off by default on every
channel. `plexi experiments list` shows them.

## Build from source

From a checkout of the repository on macOS:

```sh
scripts/install.sh --from-source alpha
```

This needs Rust, just, uv and Python 3.11.

## Update or remove

`plexi update` updates the channel you run it from, and `plexi update --rollback`
restores the previous build. `plexi uninstall` removes that channel's package
and launchers and keeps your data unless you pass `--delete-data`.

## Next

[Run your first agent](/docs/first-agent) connects a coding agent to Plexi.
