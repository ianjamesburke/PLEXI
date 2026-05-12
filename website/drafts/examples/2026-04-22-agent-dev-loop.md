---
title: "The agent-dev-loop: how Plexi makes apps that AIs can actually build"
date: 2026-04-22
description: "PGAP plus a headless test harness. The agent writes the app, spawns it, gets a PNG back, and asserts on the next frame — no GUI required."
---

Most apps assume a human in front of a screen. That's why agents are bad at building them. The feedback loop runs through your eyeballs.

Plexi flips this. Apps speak PGAP — the Plexi Generic App Protocol. It's newline-delimited JSON over stdin and stdout. Events flow host-to-app, draw commands flow app-to-host. No GUI framework to learn. No window system to mock. The boundary is two pipes.

That boundary is the unlock. Because every app is a process that reads JSON and writes JSON, you can drop one into a headless test harness and drive it from a script. The harness spawns the app, sends it a synthesized keypress, captures the next draw command, renders it to a PNG, and hands the PNG back to whatever's calling. An agent can:

1. Write the app.
2. Spawn it under the harness.
3. Send the keypress that should trigger the change.
4. Receive the rendered frame as a PNG.
5. Assert that the right thing happened on screen.

That's a loop an agent can actually close. Not "did the code compile." Not "do the unit tests pass." Did the user-facing pixel change in the way the spec asked for. The agent can iterate until the answer is yes, then commit.

This is the technical moat. Cursor is great at writing the code that goes inside an existing app. It cannot write a new app and verify, end-to-end, that the new app behaves correctly — because there's no protocol that lets it. Warp has a beautiful terminal but no app surface for an agent to build into. Raycast has the surface but a closed protocol.

PGAP is open. The harness is open. The Python SDK is fifteen lines of imports plus a render function. Every commission I ship goes through this loop, and most of the work the agent does is invisible by the time the video is recorded — because by the time I hit record, the loop has already converged.

The thing nobody is building is the protocol that lets agents build small, useful, sandboxed software. So I'm building it.
