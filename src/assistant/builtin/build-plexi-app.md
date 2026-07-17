---
name: build-plexi-app
description: Build a small Plexi app (game, timer, counter, tool) when the user asks to build, create, make, or write an app or game
---
Build the app as a real Plexi app, never as a loose script run from a terminal. Lifecycle commands run with host.terminals.run in one reused terminal pane; after each command read its output with host.terminals.read before continuing. File content flows through the host.files.* tools, never through the terminal.

1. Scaffold: `plexi app init --global <kebab-name>`. It prints the app directory it created (channel-specific, e.g. `~/.plexi-alpha/apps/<name>/`). Read that output with host.terminals.read and use the printed path exactly; never guess it.
2. Learn the SDK from the scaffold: host.files.read the generated `AGENTS.md` and `main.py`. Follow them exactly; do not guess the SDK API.
3. Write the app with host.files.write on `main.py`. Keep the first version small and working: `view()` returns a tree of `plexi_sdk.ui` widgets, `update(event)` handles `UiAction` by `handler_id`, state changes go through the `SetState` effect. Make every interaction keyboard-reachable. Use host.files.edit for follow-up fixes instead of rewriting the file.
4. Validate: run `plexi app check <app-dir>` in the terminal and read the result. It must pass. Fix every error it reports before opening the app.
5. Open the app with the host tool host.apps.open (app = the kebab name). Never launch it by running a script in a terminal.
6. Tell the user the app is open and how to use it.

If a step fails, show the user the exact error and fix it. Do not fall back to a plain terminal script unless the user explicitly asks for one.
