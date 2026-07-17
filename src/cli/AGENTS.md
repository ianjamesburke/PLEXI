# src/cli — Agent Contract

**Read before editing anything under `src/cli/`:** this file, plus the root `AGENTS.md`.

## Reference

- [`src/render/CLI_APP_CONTRACT.md`](../render/CLI_APP_CONTRACT.md) — CLI-backed app runtime contract (launch, lifecycle, caching, permissions).
- [`registry/CLI_DESCRIPTOR_GUIDE.md`](../../registry/CLI_DESCRIPTOR_GUIDE.md) — CLI descriptor authoring guide (field reference, `ui_hint`, verification).

## Channel-Agnostic CLI Rule

Every CLI command and feature must work identically on alpha, beta, main, and PR builds. The release channel is an implementation detail. See `scripts/RELEASE_CHANNELS.md` for the channel table and shim behavior.

**Path rules:** Never hardcode a profile directory path — always use `config_dir()`. Never hardcode `.plexi/` as a workspace dir — always use `workspace_channel_dir()` or `workspace_config_path()`.

**Socket rule:** A channel-suffixed binary always sends commands to its own profile's `notify.sock`, even when it inherits a different `PLEXI_SOCKET`. The bare `plexi` binary honors `PLEXI_SOCKET`. Route new command dispatch through `resolve_command_socket()`.

**Completion testing on PR builds:** `just pr-install` skips completion install. To test, manually run `plexi-pr-<N> completions zsh > <path>` and restore after.

## CLI Design Rules

- **Namespace design:** verify a new command belongs in the right namespace. Place it where the noun already lives, not at top level.
- **Pane naming:** always name panes after spawning them. Every `plexi pane new`, `plexi app open`, split, or new window should be followed by `plexi pane name <id> "descriptive name"`.
- **Tips:** use `print_tip()` from `src/cli/mod.rs`. Never raw `eprintln!`. Respects `config.cli.tips` and `NO_COLOR`.

## Traps

- **Path-based app commands must not resolve a workspace.** `app validate <path>`, `app install <path>`, `app run <path>` operate on an explicit filesystem path. Never call `resolve_workspace_root` for that argument. Use `std::fs::canonicalize` directly. `resolve_workspace_root` is only legitimate in `AppRegistry::load` and `app init`.
- **Building a `-c` command string:** use `cmd_from_args` (in `src/app/mod.rs`), not `shell_join` directly. A single-arg array is already a shell expression; `shell_join(["echo hello"])` yields `'echo hello'`.
- **Shell suffix construction:** when appending a stay-alive or exec suffix to a user command string, use the absolute shell path from `settings.shell` (already resolved), not `$SHELL`. `trim_end_matches([';', ' '])` the user command before appending to prevent `;;` syntax errors.

## Style

Document stable contracts, not history. Update in the same change that makes a rule obsolete.
