---
title: Installation & Setup
description: Get Plexi running on your Mac.
verified_version: "3.6.19"
order: 1
---

Plexi runs on macOS. Download the latest release from the [download page](/download).

## Requirements

- macOS 12 Monterey or later
- Apple Silicon or Intel

## Install

Download the `.dmg` from the [download page](/download), open it, and drag Plexi to your Applications folder. Launch it from Applications or Spotlight.

On first launch, Plexi will request Accessibility permission. This is required — Plexi uses it to track your focused application for context-aware features.

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
