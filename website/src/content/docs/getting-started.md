---
title: Installation & Setup
description: Get Plexi running on macOS, Linux, or Windows.
order: 1
---

Plexi is a tiling terminal host with a scriptable CLI for macOS, Linux, and
Windows. Its stable v1 surface includes panes, subcontexts, agent status hooks,
Quick Note, and the local app runtime.

## Install

### macOS (Apple Silicon)

```sh
curl -fsSL https://plexiapp.com/install | bash
```

This downloads the current Apple Silicon alpha release. It requires `bash`,
`curl`, `tar`, and `shasum`, but not Git, Rust, a package manager, or
administrator access. macOS Intel has no current download path; build from a
checkout instead.

### Linux (x86_64, X11)

```sh
curl -fsSL https://plexiapp.com/install | bash
```

This downloads the current Linux x64 alpha release. It requires `bash`, `curl`,
`tar`, and `sha256sum` (or `shasum`), and installs into user-owned directories.
Plexi supports X11 sessions; Wayland is not supported. This is not an apt, yay,
AUR, Flatpak, AppImage, `.deb`, or `.rpm` package.

On Linux, secrets are kept in a mode-`0600` file rather than an encrypted OS
keyring. Hardware video decoding is not implemented on Linux.

### Windows (x64)

Run this in Windows PowerShell 5.1 or later:

```powershell
irm https://raw.githubusercontent.com/ianjamesburke/PLEXI/alpha/scripts/install-windows.ps1 | iex
```

This downloads the current Windows x64 alpha release, verifies its SHA-256
checksum, and does not require Rust or Visual Studio. It does not build from
source. The Unix `curl | bash` command is not a Windows installer.

### Source builds and unsigned releases

From a macOS or Linux checkout, use `scripts/install.sh --from-source alpha` to
build and install from source; it requires Rust and Python 3. Windows has no
equivalent source-install script. All alpha builds are unsigned and not
notarized. Follow the [unsigned build opening guide](https://github.com/ianjamesburke/PLEXI/blob/alpha/docs/unsigned-install.md)
before bypassing a Gatekeeper or SmartScreen warning. Linux users who unpack an
archive manually may need `chmod +x plexi`.

Plexi does not request Accessibility permission on first launch. Its app bundle
declares camera and microphone permissions for video rooms, which macOS requests
only when a feature uses them.

## Script the Host

The CLI is designed for scripts and tools that need to control a running host.
It covers panes, contexts, status reporting, notes, and local apps. The v1
surface exposes agent status hooks; it does not ship a full Assistant product.

If you use a tool that supports [skills](https://skills.sh/), you can install
the Plexi skill to document that CLI surface:

```sh
npx -y skills@latest add ianjamesburke/plexi-skills
```

Run it in a project to install for that project, or add `-g` for a global install. To update it later, run `npx skills update`.

To pin the skill to a Plexi release, use the same current CLI:

```sh
npx -y skills@latest add "ianjamesburke/plexi-skills#v0.2.0"
```

If a pinned install reports a clone URL with the `#v…` ref embedded in it and
ends with `not valid`, a stale `skills` CLI is being used. Run the command above
instead of bare `npx skills add`.

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

Stable v1 keeps Assistant, marketplace, and MCP client surfaces gated. Use a
beta or worktree channel only when you are explicitly testing those features.

## Next Steps

- [Quick Note](/docs/quick-note) — open a persistent scratch pane from anywhere
- [Panes & Pages](/docs/panes) — understand the layout model
- [Apps](/docs/apps) — build and run Plexi apps
- [Python SDK](/docs/sdk) — author apps for the local runtime
