---
title: Panes & Pages
description: The Plexi layout model — panes, pages, and how to navigate them.
verified_version: "3.6.19"
order: 4
---

Plexi's UI is built from **panes** arranged into **pages**.

## Panes

A pane is a single rectangular region. There are two kinds:

- **Terminal** — a full PTY session. Inherits your shell, dotfiles, and CWD.
- **App** — runs a sandboxed Plexi app via PGAP. No shell; the app controls the render.

## Pages

A page is a tabbed collection of panes. Think of it like a workspace: you can switch between pages instantly, and each page has its own pane layout.

## Splitting

| Action | Shortcut |
|--------|----------|
| Split focused pane horizontally | `⌘D` |
| Split focused pane vertically | `⌘⇧D` |
| Split right, mirroring type | `⌘\` |
| Split down, mirroring type | `⌘⇧\` |

"Mirroring type" means the new pane matches the focused pane's type — terminal produces terminal, app produces a copy of the same app.

## Navigation

| Action | Shortcut |
|--------|----------|
| Move focus left / down / up / right | `⌘H` / `⌘J` / `⌘K` / `⌘L` |
| Swap pane with neighbor | `⌘⌃H` / `⌘⌃J` / `⌘⌃K` / `⌘⌃L` |
| Close focused pane | `⌘W` |
| Rename focused pane | `⌘R` |

## CWD Inheritance

When you split a terminal pane, the new pane inherits the current working directory of the focused pane. Plexi tracks CWD via OSC 7 sequences emitted by your shell's integration layer.

## Shell Integration

Plexi injects a shell integration shim via `ZDOTDIR` (for zsh) or equivalent. This shim sources your real dotfiles, so your prompt, aliases, and environment load normally. The shim also emits the OSC 7 sequences required for CWD tracking.
