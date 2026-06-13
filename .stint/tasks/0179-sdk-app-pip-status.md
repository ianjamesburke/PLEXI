---
id: "0179"
title: "feat(sdk): apps optionally report their own pip status (red/yellow/green)"
status: done
estimate: "3h"
actual: "19m"
started_at: "2026-06-13T08:05:05Z"
completed_at: "2026-06-13T08:23:18Z"
sprint: "s2"
blocked_by: []
gh_issue:
  - "2230"
area:
  - "sdk/python"
  - "host/pane-ops"
  - "ui/overlays"
tags:
  - "v1"
  - "app-authoring"
---




## What

Add an optional SDK + host protocol surface so apps can declare their own pip
state (green/yellow/red). Host falls back to today's derived activity when an
app hasn't set a status. No behavior change for existing apps.

## Scope

- Add `SetPipStatus { status: PipStatus }` variant to `AppRequest` enum, modeled on the existing `SetAgentState` handler.
- Add `PipStatus` enum (green/yellow/red) to `src/protocol/commands.rs`.
- Store on `AppPane` struct. `effective_activity()` checks pip status first, falls back to derived activity.
- Add `App.set_pip_status(status)` method to the Python SDK.
- No overlay changes needed (command palette reads `effective_activity` which will pick up the new signal).

## References (verified line numbers as of v0.0.768)

- GitHub issue #2230
- `src/protocol/commands.rs:464` (AgentState enum), `:485` (AppRequest enum)
- `src/host/pane.rs:408` (AppPane struct), `:114` (effective_activity)
- `src/app/lifecycle.rs:1290` (SetAgentState handler, model for new handler)
- `sdk/python/plexi_sdk/_app.py:139` (App class, add set_pip_status method)

## Design decisions (verified, not guessed)

- **Color-faithful mapping.** The pip palette (`src/ui/theme.rs`) is *inverted*
  from a naive traffic-light: `pip_working`=green (pulsing), `pip_idle`=yellow,
  `pip_blocked`=red. So `PipStatus::as_agent_state()` maps green→Working,
  yellow→Idle, red→Blocked — the dot renders the app's intended color. A naive
  green→Idle would have shown a green app as yellow.
- **Host stamps pane_id; app never supplies it.** The SDK has no way for an app
  to learn its own pane id, and `ForwardPaneRequest` does not rewrite it, so the
  wire `pane_id` defaults to 0 and `ProcessApp::route_command` stamps the
  sending app's real `self.pane_id`. This makes the API `App.set_pip_status("green")`
  (no pane id) and makes spoofing another pane's pip impossible. (The original
  spec's `self.emit.set_pip_status(self.app_id, ...)` was wrong — app_id is a
  string manifest id, not a u64 pane id.)
- **Ungated.** Self-status is not pane *control*, so SetPipStatus forwards
  without a capability gate (unlike SetPaneTitle), honoring "works for every app".

## Variance

Estimate 3h. Touched commands.rs (enum + variant + mapping), host/pane.rs
(field + `pip_status()` accessor + `effective_activity` priority + `set_pip_status`),
lifecycle.rs (handler), routing.rs (ungated forward + pane_id stamping),
_emitter.py + _app.py (SDK), plus a scripted `pip_status: None` into 24 AppPane
literals. Tests: mapping unit, wire-format/default unit, and a HostHarness
integration test proving the full path + pip-overrides-agent. Follow-up (optional):
a tiny `apps/dev/pip-status` POC to demo the dot color cycling manually.
