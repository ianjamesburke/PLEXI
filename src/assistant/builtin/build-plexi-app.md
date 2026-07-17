---
name: build-plexi-app
description: Build a small Plexi app (game, timer, counter, tool) when the user asks to build, create, make, or write an app or game
---
Build the app as a real Plexi app, never as a loose script run from a terminal. The whole flow runs on your embedded authoring tools — host.build.run for lifecycle commands and host.files.* for code. Never open a terminal pane for your own build work; terminals are only for commands the user asked to watch.

1. Scaffold: host.build.run {"args": ["app", "init", "<kebab-name>"]}. Its stdout names the app directory it created; use that path exactly, never guess it. Add "--global" only if the user wants the app outside this workspace — workspace apps hot-reload while global apps do not.
2. Learn the SDK from the scaffold: host.files.list the app directory, then host.files.read the generated `AGENTS.md` and `main.py`. Follow them exactly; do not guess the SDK API. Use host.files.grep to find SDK symbols instead of reading whole files.
3. Write the app with host.files.write on `main.py`. Keep the first version small and working: `view()` returns a tree of `plexi_sdk.ui` widgets, `update(event)` handles `UiAction` by `handler_id`, state changes go through the `SetState` effect. Make every interaction keyboard-reachable. Use host.files.edit for follow-up fixes instead of rewriting the file — each edit returns a diff the user sees.
4. Validate: host.build.run {"args": ["app", "check", "<app-dir>"]}. Read its stdout/stderr from the result. It must pass; fix every error it reports before opening the app.
5. Open the app with the host tool host.apps.open (app = the kebab name) as soon as the first check passes. A workspace app hot-reloads in its open pane on every save, so keep iterating with host.files.edit and the user watches it converge. Re-run app check after meaningful changes.
6. Tell the user the app is open and how to use it.

If a step fails, show the user the exact error and fix it. Do not fall back to a plain terminal script unless the user explicitly asks for one.
