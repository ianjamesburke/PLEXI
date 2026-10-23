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

## FAST and SLOW

The package registers `plexi/fast` (FAST) and `plexi/slow` (SLOW) even outside
Plexi. These are explicit selections, with no prompt classifier or fallback.
Plexi resolves the targets from its channel config at extension load and each
session start with `plexi config get --json models` (a short timeout). Configure
them globally or per workspace; each omitted field keeps its built-in default:

```toml
[models.fast]
provider = "openrouter"
model = "deepseek/deepseek-v4.1-flash"
thinking = "low"

[models.slow]
provider = "openrouter"
model = "openai/gpt-6-astra"
thinking = "high"
```

Allowed thinking levels are `off`, `minimal`, `low`, `medium`, `high`, and
`xhigh`. If config access or validation fails, pi-plexi uses these defaults and
logs one warning; each selected alias still requires its exact target and
thinking capability, so missing targets produce an error.
Inactive aliases do not resolve targets or change the selected model.

## Switching FAST/SLOW

Use `/fast` or `/slow` to switch directly, or `/models [fast|slow]` to choose.
`/model` is built in; it lists FAST and SLOW only when settings `enabledModels` is
`["plexi/fast", "plexi/slow"]`.

## Scratch-profile migration and rollback

The migration helper accepts an explicit **agent directory** (`settings.json`
lives directly inside it). It refuses the current HOME's `.pi` tree and the OS
user's real `.pi` tree, including symlink aliases. There is no personal-cutover
override. Stop processes using the scratch profile before backup or rollback.

Run these commands from `packages/pi-plexi`. Create fixtures only in the scratch
directory; do not point these commands at a personal profile.

```sh
PI_SCRATCH=$(mktemp -d)
mkdir -p "$PI_SCRATCH/agent/extensions" "$PI_SCRATCH/agent/agents"
printf '%s\n' '{"defaultProvider":"ian-router","defaultModel":"auto","packages":["npm:@alexlikevibe/pi-jev"]}' > "$PI_SCRATCH/agent/settings.json"

# Explicit backup (refuses an existing destination and verifies byte equality).
env HOME="$PI_SCRATCH" PI_CODING_AGENT_DIR="$PI_SCRATCH/agent" \
  node src/migrate-model-modes.mjs backup \
  --profile "$PI_SCRATCH/agent" --backup "$PI_SCRATCH/before-fast"

# Uses the verified backup above, or creates one if absent.
env HOME="$PI_SCRATCH" PI_CODING_AGENT_DIR="$PI_SCRATCH/agent" \
  node src/migrate-model-modes.mjs migrate \
  --profile "$PI_SCRATCH/agent" --backup "$PI_SCRATCH/before-fast"

# Restores and verifies original bytes; preserves the backup for reapplication.
env HOME="$PI_SCRATCH" PI_CODING_AGENT_DIR="$PI_SCRATCH/agent" \
  node src/migrate-model-modes.mjs rollback \
  --profile "$PI_SCRATCH/agent" --backup "$PI_SCRATCH/before-fast"
```

Backup covers settings, the subscription router, and agent Markdown files.
Migration atomically replaces settings and removes the backed-up subscription
router. It converts the old default to FAST, sets the alias cycle, and appends
Pi 1.3.0's force-exclude filter `-extensions/routing.ts` to pi-jev's package
entry. Existing package filters, compaction, `/jev`, unrelated settings and
`jev.json` remain intact. An already-disabled extension list stays disabled.
Repeated migration is a no-op; a changed source or corrupt backup fails before
migration. Rollback restores the backed-up files, so use it before making
unrelated edits to those files.

`npm test` uses a temporary HOME and `PI_CODING_AGENT_DIR` for the tests and
asserts that a read-only snapshot of the real user's `.pi` files (paths, sizes,
mtimes and SHA-256 contents) is unchanged afterward.
