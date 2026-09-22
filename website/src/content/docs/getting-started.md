---
title: Installation & Setup
description: Get Plexi running on macOS, Linux, or Windows.
order: 1
---

Plexi is a tiling terminal host with a scriptable CLI for macOS, Linux, and
Windows. Its stable v1 surface includes panes, subcontexts, agent status hooks,
Quick Note, and the local app runtime. The public installer currently has
prebuilt alpha assets for Linux x86_64 and macOS Apple Silicon; macOS Intel and
Windows are not yet available through this curl installer.

Linux is currently supported on x86_64 X11 sessions. Wayland is not supported.
The Linux installer is a user-directory install, not a distro package: there is
no apt, yay, AUR, Flatpak, AppImage, `.deb`, or `.rpm` install path.

## Install

Open Terminal and run:

```sh
curl -fsSL https://plexiapp.com/install | sh
```

For now, the public installer defaults to the alpha channel, detects your
platform, downloads the matching available release asset, and installs the CLI
into `~/.local/bin` (or your configured Plexi user directory). It does not need
Git, Rust, a package manager, or administrator access. Open a new shell after
installation if it asks you to refresh `PATH`. Contributors can build from a
checkout with `bash -s -- --from-source`.

On Linux, secrets are kept in a mode-`0600` file rather than an encrypted OS
keyring. Hardware video decoding is not implemented on Linux.

Plexi alpha builds are unsigned on purpose. If Gatekeeper or SmartScreen blocks
your download, follow the [unsigned build opening guide](https://github.com/ianjamesburke/PLEXI/blob/alpha/docs/unsigned-install.md): it covers Finder's
**Open** flow and optional `xattr -cr` command on macOS, plus **More info** →
**Run anyway** on Windows after you confirm the download is from the official
Plexi GitHub release. Linux users who unpack an archive manually may need
`chmod +x plexi`.

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
