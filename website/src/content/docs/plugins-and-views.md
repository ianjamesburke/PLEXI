---
title: Plugins and views
description: The 0.5.0 model. A plugin is a package, a view is what it shows in a pane, and capabilities decide what it may touch.
order: 4
---

Plexi 0.5.0 replaces the word "app" with two more exact ones.

A **plugin** is a package: a folder with a `manifest.toml` and code. You
install, update and remove plugins.

A **view** is something a plugin shows in a pane. A plugin declares its views
in the manifest. You open views.

The old word covered both. "Install the app" and "open the app" were
different operations on different things.

## The two namespaces

The CLI splits along the same line.

| You want to | Command |
|---|---|
| Create a plugin | `plexi plugin init <name>` |
| Install one | `plexi plugin install <path or source>` |
| See what is installed | `plexi plugin list` |
| Validate one | `plexi plugin check <path>` |
| See which views exist | `plexi view list` |
| Open a view in a pane | `plexi view open <view, plugin or path>` |
| Render a view without a host window | `plexi view render <plugin> --png` |

`plexi view open` accepts a view ID, a plugin ID or a path to a plugin folder.
A plugin ID opens the plugin's only view, or the one its manifest marks as
default. If a plugin has several views and none is marked default, the command
lists the choices. A plugin with no views cannot be opened.

## What a manifest declares

```toml
schema_version = 2

[plugin]
id = "counter"
name = "Counter"
version = "1.0.0"

[plugin.capabilities]
capabilities = []

[runtime]
kind = "python"
entry = "main.py"

[[contributes.views]]
id = "counter.main"
default = true
```

Identity, the capabilities the plugin requests, how it runs, and what it contributes. Views
and commands are the contributions the host accepts today. The manifest
rejects anything else by name instead of ignoring it. The
[plugin manifest reference](/docs/plugin-manifest) has every field.

## Capabilities

A plugin starts with no authority. It cannot read your files, reach the
network or see a secret unless its manifest declares the matching capability
and you grant it.

Declaring is a request. `fs.read` in a manifest means "this plugin will ask to
read files in this workspace", and grants nothing by itself. Before an install
proceeds, Plexi shows a trust sheet: what the plugin is, which runtime it uses,
and every capability it declares. `plexi plugin inspect <path>` prints the same
sheet without installing. `plexi permissions list` shows the decisions the host
has stored, and `revoke` and `reset` undo them.

Python plugins run as CPython compiled to WebAssembly, inside a sandbox. The
plugin's own folder, the SDK and the Python standard library are mounted
read-only. Everything else goes through a host API that checks the grant.
[Plugin guide](/docs/plugins#execution-and-permissions) covers the runtime.

## Where terminals and agents fit

A pane holds either a terminal or a view. [Panes](/docs/panes) covers both.

Agents are not plugins. A coding agent is a program in a terminal pane that
reports its state to the host and drives Plexi through the CLI.
[Run your first agent](/docs/first-agent) sets one up.

## If you are on an older release

Releases up to 0.4.6 use `plexi app` for both jobs, and their manifests use
`[app]` tables. The concepts on this page still apply; only the names differ.
`plexi --help` on your install shows which you have.
