---
name: build-plexi-app
description: Build a small Plexi app (game, timer, counter, tool) when the user asks to build, create, make, or write an app or game
tier: high
---
Build the app as a real Plexi app, never as a loose script run from a terminal. The whole flow runs on your embedded authoring tools — host.build.run for lifecycle commands and host.files.* for code. Never open a terminal pane for your own build work; terminals are only for commands the user asked to watch.

The complete SDK API reference is appended to this prompt. It is authoritative and current. Never read, list, or grep `plexi_sdk` sources or any SDK file to "learn the API" — every component, event, effect, and state call you need is already below. Spending tool calls on SDK discovery is a bug.

1. Scaffold: host.build.run {"args": ["app", "init", "<kebab-name>"]}. Its stdout names the app directory it created; use that path exactly, never guess it. Add "--global" only if the user wants the app outside this workspace — workspace apps hot-reload while global apps do not. If exit_code is nonzero (e.g. a name collision), never edit whatever already exists at that path — it was not scaffolded by you and may have no build tooling available. Retry `app init` with a different kebab-name, or if collisions persist, show the user the exact error text and stop.
2. Write the app with a single host.files.write of `main.py`, composed straight from the SDK reference — no exploratory list/read/grep first. Keep the first version small and working: `view()` returns a tree of `plexi_sdk.ui` widgets, `update(event)` handles `UiAction` by `handler_id`, state changes go through the `SetState` effect. Make every interaction keyboard-reachable. Use host.files.edit for follow-up fixes instead of rewriting the file — each edit returns a diff the user sees.
3. Validate: host.build.run {"args": ["app", "check", "<app-dir>"]}. Read its stdout/stderr from the result. It must pass; fix every error it reports before opening the app. Fix from the check output alone — re-read `main.py` only when an edit needs context the check output doesn't show.
4. Open the app with the host tool host.apps.open (app = the kebab name) as soon as the first check passes. A workspace app hot-reloads in its open pane on every save, so keep iterating with host.files.edit and the user watches it converge. Re-run app check after meaningful changes.
5. Tell the user the app is open and how to use it.

If a step fails, show the user the exact error and fix it. Do not fall back to a plain terminal script unless the user explicitly asks for one.
