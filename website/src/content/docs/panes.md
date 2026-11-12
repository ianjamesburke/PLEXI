---
title: Panes
description: Terminal panes and view panes, splitting, focus, and working-directory inheritance.
order: 4
---

A Plexi window is a grid of **panes**. Each pane holds a terminal or a view.

## Pane Types

There are two kinds:

- **Terminal** is a full PTY session. Inherits your shell, dotfiles, and CWD.
- **View** shows something a plugin draws. A view runs sandboxed and reaches your files, network and secrets only through capabilities you grant. See [Plugins and views](/docs/plugins-and-views).

## Splitting

| Action | Shortcut |
|--------|----------|
| Split focused pane horizontally | `⌘D` |
| Split focused pane vertically | `⌘⇧D` |
| Split right, mirroring type | `⌘\` |
| Split down, mirroring type | `⌘⇧\` |

"Mirroring type" means the new pane matches the focused pane's type: a terminal produces a terminal, and a view produces a copy of the same view.

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

## Panes from the CLI

Everything above is also a command, which is how agents arrange a workspace:

```sh
plexi pane new "npm run dev" -n "dev"   # terminal running a command, named
plexi pane new -d                       # empty terminal, split below
plexi view open calc                    # a view pane
plexi pane list                         # ids and names
plexi pane focus dev
plexi pane close dev
```

The [CLI reference](/docs/cli#plexi-pane) has the whole `plexi pane` namespace.
