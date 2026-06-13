---
id: "0185"
title: "plexi run: dynamic shell completions from workspace commands.toml"
status: in-progress
estimate: "2h"
started_at: "2026-06-13T20:03:39Z"
sprint: "s11"
blocked_by: []
gh_issue: []
area:
  - "cli/completions"
tags:
  - "v1"
  - "cleanup"
  - "cli"
---


Add dynamic zsh (and bash) completions for `plexi run <TAB>` that list available command names from the active channel's workspace `commands.toml`.

## Why

`plexi run` accepts a command name defined in `.plexi/commands.toml` (or the channel-scoped equivalent), but completions currently offer nothing for that position. Users have to remember command names by heart. This should be as discoverable as any other CLI subcommand.

## Scope

1. **Trailing-args forwarding in `plexi run`** (prerequisite, ~30 min): Add `extra_args: Vec<String>` to the `Run` variant in `src/cli/args.rs`. Append them to the shell command when spawning in `src/cli/run.rs`. This enables workspace commands to receive `$1`, `$2`, etc., which unblocks the `plexi run implement-stint 0045` pattern.

2. **Dynamic completions** (~90 min): Update the generated zsh completion function for `plexi run` to call a helper that:
   - Resolves the workspace commands file via `workspace_channel_dir()` — never hardcode `.plexi/`. The channel-scoped path is `.plexi/`, `.plexi-alpha/`, `.plexi-beta/`, or `.plexi-pr-N/` depending on the running binary.
   - Parses `commands.toml` in the current working directory under that path.
   - Emits `name:description` pairs for `compadd` / `_describe`.
   - Falls back to listing global scripts from `config_dir()/scripts/` when no workspace commands file exists.

## Implementation Notes

- Completion function lives in `src/completions/` (wherever the current `_plexi` zsh file is generated). Find with `rg 'plexi run' src/completions/` or check `just completions`.
- The resolution logic for the channel-scoped workspace dir is already in `crate::config::workspace_channel_dir()` — use it, do not re-derive the path from the binary name in the completion handler.
- Dynamic completion in zsh: use `_arguments` with a `->state` transition, then in `case $state` call `compadd $(plexi run 2>/dev/null | awk '/^  / {print $1}')`. Alternatively, add a hidden `plexi run --complete` flag that emits newline-separated names for easy shell consumption — this is more robust than parsing the human-readable list output.
- Bash: add the same logic to the `_plexi` bash completion function in the `run` case branch.

## Gotchas

- `workspace_channel_dir()` requires a `cwd` — completions run in whatever directory the shell is in, so this is correct by default.
- If the user is not in a workspace (no `commands.toml`), fall back gracefully to global scripts; do not error.
- The `plexi run --complete` hidden flag approach is strongly preferred over parsing `plexi run` stdout — stdout format can change, a dedicated flag cannot accidentally break.
