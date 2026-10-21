# `@plexi/pi-plexi`

First-party [Pi coding agent](https://github.com/earendil-works/pi) package for Plexi (`@earendil-works/pi-coding-agent` `1.3.0`).

`@plexi/pi-plexi` turns Pi inside a Plexi terminal pane into a workspace-native participant with:

- **Channel-safe host attachment** (`PLEXI_SOCKET`, `PLEXI_PANE_ID`, `PLEXI_CONTEXT_ID`, `PLEXI_CHANNEL`) and graceful degradation outside Plexi or when pointed at a dead/mismatched host socket.
- **Live workspace widget & status bar** (`src/ui.ts`) showing current context, pane identity, open apps/tools, recent event stream summaries, and pending `Needs you` items.
- **Typed Plexi tools** (`src/tools.ts`):
  - `plexi_workspace_overview` — inspect current context, panes, open apps, and pending `Needs you` items.
  - `plexi_pane_inspect` — inspect terminal or app pane state, slots, or terminal text capture (`plexi pane state` / `plexi pane capture`).
  - `plexi_app_tools` — discover schemas for live app tools in the current context (`plexi app tools --json`).
  - `plexi_app_call` — invoke an app tool (`plexi app call <app> <tool> --args <json>`), with automatic `--wait-approved` correlation for ask-tier mutations, exact resource scoping, and revision-conflict reporting.
  - `plexi_events_poll` — inspect buffered events from subscribed app event streams (`plexi events subscribe`).
  - `plexi_needs_you_ask` — file a structured creative or blocking question (`choices`, `allow_free_text`, `preview_refs`, `correlation_id`, `expected_revision`) in both Pi's UI and the Plexi host's `Needs you` surface, and wait for or reconcile the human's answer without minting a permission grant.
  - `plexi_needs_you_reconcile` — reconcile open/resolved `Needs you` items and durable continuation records across Pi or host restarts, preventing duplicate mutations.
  - `plexi_host_screenshot` — capture a PNG screenshot of the Plexi host window or a specific pane (`plexi host screenshot`).
  - `plexi_run` — list merged workspace and global commands (`plexi run --json`) or execute a named command (`plexi run <name> [-- <args>...]`).
- **Lifecycle bridge & legacy hook migration** (`src/lifecycle.ts`): reports `working`, `blocked`, and `idle` states to the host and automatically migrates legacy `~/.pi/agent/extensions/plexi-agent-state.ts` hooks to `.plexi-hook.bak` so lifecycle events are never reported twice.

## Loading from a checkout

```bash
pi -ne \
  -e /path/to/PLEXI/packages/pi-plexi/extensions/index.ts \
  --skill /path/to/PLEXI/packages/pi-plexi/skills
```

## Slash commands

- `/plexi-status` — refresh the live workspace snapshot and widget.
- `/plexi-reconcile` — reconcile pending/resolved `Needs you` items and continuations for this session/pane.
- `/plexi-watch <app> <stream>` — subscribe to an app event stream in the current context.
