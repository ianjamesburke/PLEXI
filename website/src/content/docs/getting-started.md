---
title: Installation & Setup
description: Get Plexi running on your Mac.
order: 1
---

Plexi runs on macOS (Apple Silicon or Intel, macOS 12 Monterey or later).

## Install

Plexi needs Git and a Rust toolchain. If Rust is missing, the installer first
asks whether to install it with rustup. A cold build takes several minutes.

Open Terminal and run:

```sh
curl -fsSL https://plexiapp.com/install | sh
```

The installer clones Plexi into `~/.plexi-src`, builds it on your Mac, copies
Plexi.app to `/Applications`, installs the `plexi` CLI in `/usr/local/bin`, and
adds shell completions. It may ask for your password to write the CLI. Launch
Plexi from Applications or Spotlight.

Plexi does not request Accessibility permission on first launch. Its app bundle
declares camera and microphone permissions for video rooms, which macOS requests
only when a feature uses them.

## Teach Your Agent

Plexi is built to be driven by coding agents. If you use Claude Code, Cursor, Codex, or any agent that supports [skills](https://skills.sh/), install the Plexi skill so your agent knows the CLI:

```sh
npx -y skills@latest add ianjamesburke/plexi-skills
```

Run it in a project to install for that project, or add `-g` for a global install. The skill documents the `plexi` CLI surface — panes, apps, contexts, notifications — so your agent can drive Plexi without trial and error. To update it later, run `npx skills update`.

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

## Next Steps

- [Quick Note](/docs/quick-note) — open a persistent scratch pane from anywhere
- [Panes & Pages](/docs/panes) — understand the layout model
- [Apps](/docs/apps) — build and run Plexi apps
