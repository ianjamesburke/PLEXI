# src/cli — Agent Contract

**Read before editing anything under `src/cli/`:** this file, plus the root `AGENTS.md` (especially its "Channel-Agnostic CLI Rule", "CLI Namespace Design", and "CLI Tips" sections — those are binding and not repeated here).

## Traps

- **Path-based app commands must not resolve a workspace.** `app validate <path>`, `app install <path>`, `app run <path>` operate on an explicit filesystem path — never call `resolve_workspace_root` / `require_workspace` for that path argument. It returns `None` when no `.plexi/` ancestor exists, and treating `None` as an error breaks agents in a plain cloned repo. Use the path directly (`std::fs::canonicalize`). `resolve_workspace_root` is only legitimate in `AppRegistry::load` and `app init`, where `None` degrades to global rather than failing.
- **Socket-first, then channel fallback.** Any command that talks to a running instance must follow the `open_cli` pattern (`open.rs`): honor `PLEXI_SOCKET` when set; only fall back to channel-specific mechanisms (spawn-queue, `config_dir()`) when it is not. Never route around `PLEXI_SOCKET`.
- **Never hardcode a profile or workspace dir.** No literal `~/.plexi-alpha/` — use `config_dir()`. No literal `.plexi/` joined from a workspace root — use `config::workspace_channel_dir()` / `workspace_config_path()`, which return the channel-correct name (`.plexi`, `.plexi-alpha`, `.plexi-pr-N`, …).
- **Building a `-c` command string:** use `cmd_from_args` (in `src/app/mod.rs`), not `shell_join` directly. A single-arg array is already a shell expression; `shell_join(["echo hello"])` yields `'echo hello'`, which the shell tries to run as one command named `echo hello`. Only multi-arg arrays need joining.

## Style

Document stable contracts, not history. If a trap here stops being true after a refactor, update it in the same change; otherwise leave it alone.
