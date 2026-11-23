# CLI v1 Tree-Shake Inventory

**Status:** active
**Evidence date:** 2026-09-20; reconciled to `origin/alpha` on 2026-09-22 and 2026-11-09

## Current v1 tags (G-13 reconciliation)

This section is the current tag inventory. The source of truth for release tier is
`ReleaseFeature` in `src/release/catalog.rs`; the generated website reference is the
same clap command tree and is checked by `just check-cli-docs`.

| Surface | Current tag | Stable-v1 boundary |
|---|---|---|
| `run`, `workspace`, `secret` | v1 | Workspace and secret basics. |
| `agent report`, `agent status`, `agent hook` | v1 | Status hooks only; this does not promote the Assistant or agent mesh. |
| `agent done`, `agent ask` | v1 (worker completion) | The worker contract injected into every spawned agent session, including GUI and palette spawns, tells the harness to run them. Gating them would strand those sessions. |
| `agent list`, `agent show`, `agent call`, `agent message`, `agent handoff` | v1 (retained) | `scripts/v1-acceptance.sh` V1-01 and the pi-plexi package call them on a default config. GUI and palette role spawns are not experiment gated, so a role pane can exist with `agent_mesh` off and pi-plexi's role tools then run `agent show`, `call`, `message` and `handoff`. Gating the GUI entry points is a separate decision. |
| every other `agent` subcommand (`create`, `start`, `spawn`, `wait`, `result`, `next`, `flags`, `fleet`, `history`, `reopen`, `team`) | beta | `ReleaseFeature::AgentMesh` (`agent_mesh`); hidden from help and refused until the experiment is on. |
| `context`, `pane`, `host`, `notify`, `note`, `config`, `completions`, `demo`, `update`, `uninstall` | v1 | Terminal-host control and observation surface. `demo` refuses on platforms without a supported keymap. |
| `file open` and its shorthand `open` | v1 | Opens a path in the Plexi view that handles it. `open` is the one deliberate exception to noun-first naming. |
| `plugin` local-runtime commands | v1 | Local/remote/pack install, list/info/prune/freeze, open, uninstall, render, check/validate/inspect, state, init/test/package, action/update, plus `--cli`, `--mcp`, `cli:`, and `mcp:` wrappers. |
| `registry watch`, `descriptor probe`, `doctor`, `ai` diagnostics/setup | v1 | App-runtime and local CLI support surfaces; they are not the hosted Assistant product. |
| `events list`, `declare`, `emit`, `subscribe` | v1 | Brokered plugin-event surface. |
| `routine` and every routine subcommand | beta | `ReleaseFeature::Routines`; hidden from stable help and refused on stable execution. |
| `plugin publish`, `plugin browse`, `plugin search`, and bare marketplace-ID `plugin install` | beta | `ReleaseFeature::Marketplace`; local paths, remotes, and packs remain v1. |
| `account` and every account subcommand | beta | `ReleaseFeature::Marketplace`. |
| `events mcp-config` | beta | `ReleaseFeature::McpClient`; stable never emits an MCP endpoint or token. |
| `experiments`, `skill`, `cli`, `needs-you`, `permissions`, `lease`, `debug`, `plugin call`, `plugin tools`, `plugin trust` | public, untagged | Added after the 2026-09-22 reconciliation. Each is called by Plexi itself or by the shipped skill, so it stays public until the V1 sweep (stint 1182) decides core, core plugin or removal. Reasons are recorded per entry in `src/cli/v1_surface.txt`. |
| `pane set-title`, `update plugins` | removed | Former deceptive compatibility aliases; not part of the current command tree. |

The table is enforced. `src/cli/v1_surface.txt` is the machine-readable
allowlist of every command path visible with all experiments off, and the
`every_visible_command_is_v1_or_owned_by_an_experiment` test in
`src/cli/v1_surface.rs` fails when a visible path is neither listed there nor
owned by an experiment in `ReleaseFeature::owned_paths`. A new public command
must therefore be tagged v1 here and in the allowlist, or gated.

There are no current `dead` entries: the five historical dead/deceptive
entries were either repaired or removed. This table deliberately keeps local
plugin runtime and SDK authoring in v1 while excluding marketplace, MCP client,
routines, and the full Assistant, matching the website CLI reference.

