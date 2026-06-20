# src/cli — Agent Contract

**Read before editing anything under `src/cli/`:** this file, plus the root `AGENTS.md`.

## Reference

- [`src/render/CLI_APP_CONTRACT.md`](../render/CLI_APP_CONTRACT.md) — CLI-backed app runtime contract (launch, lifecycle, caching, permissions).
- [`registry/CLI_DESCRIPTOR_GUIDE.md`](../../registry/CLI_DESCRIPTOR_GUIDE.md) — CLI descriptor authoring guide (field reference, `ui_hint`, verification).

## Channel-Agnostic CLI Rule

Every CLI command and feature must work identically on alpha, beta, main, and PR builds. The release channel is an implementation detail.

- `/usr/local/bin/plexi` is a contextual macOS shim. If `PLEXI_CHANNEL` is set and the matching binary exists, bare `plexi` delegates to that channel binary. Otherwise falls back to stable.
- `PLEXI_SOCKET` routes host commands to the correct running instance. Binary-local behavior (config paths, release gates) comes from the binary the shim executes.
- When `PLEXI_SOCKET` is not set, commands fall back to channel-specific mechanisms derived from the binary name.

**Enforcement:** Never hardcode a profile directory path. Always use `config_dir()`. Never route around `PLEXI_SOCKET` when set. New host commands must follow the socket-first pattern in `open_cli()`.

**Channel-aware workspace paths:** Never hardcode `.plexi/` when joining from a workspace root. Always use `workspace_channel_dir()` or `workspace_config_path()`.

**Completion testing on PR builds:** `just pr-install` skips completion install. To test, manually run `plexi-pr-<N> completions zsh > <path>` and restore after.

## CLI Design Rules

- **Namespace design:** verify a new command belongs in the right namespace. Place it where the noun already lives, not at top level.
- **Pane naming:** always name panes after spawning them. Every `plexi pane new`, `plexi app open`, split, or new window should be followed by `plexi pane name <id> "descriptive name"`.
- **Tips:** use `print_tip()` from `src/cli/mod.rs`. Never raw `eprintln!`. Respects `config.cli.tips` and `NO_COLOR`.

## Traps

- **Path-based app commands must not resolve a workspace.** `app validate <path>`, `app install <path>`, `app run <path>` operate on an explicit filesystem path. Never call `resolve_workspace_root` for that argument. Use `std::fs::canonicalize` directly. `resolve_workspace_root` is only legitimate in `AppRegistry::load` and `app init`.
- **Socket-first, then channel fallback.** Follow the `open_cli` pattern (`open.rs`): honor `PLEXI_SOCKET` when set; only fall back to channel-specific mechanisms when it is not.
- **Never hardcode a profile or workspace dir.** No literal `~/.plexi-alpha/` or `.plexi/`. Use `config_dir()` and `workspace_channel_dir()`.
- **Building a `-c` command string:** use `cmd_from_args` (in `src/app/mod.rs`), not `shell_join` directly. A single-arg array is already a shell expression; `shell_join(["echo hello"])` yields `'echo hello'`.
- **Shell suffix construction:** when appending a stay-alive or exec suffix to a user command string, use the absolute shell path from `settings.shell` (already resolved), not `$SHELL`. `trim_end_matches([';', ' '])` the user command before appending to prevent `;;` syntax errors.
- **Note token shell injection surface.** `substitute_note_tokens_static` applies `shell_quote` (POSIX single-quote wrapping) to both `{note}` and `{cwd}` before substituting into the command template. Do not add new substitution tokens that expand arbitrary strings without routing them through `shell_quote`. The command template itself comes from `config.toml` (user-controlled, trusted) — only substituted values are escaped.

## Style

Document stable contracts, not history. Update in the same change that makes a rule obsolete.
