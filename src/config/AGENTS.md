# src/config — Agent Contract

**Read before editing anything under src/config/:** this file, plus the root AGENTS.md.

## Scope

Configuration loading, validation, channel-scoped profile dirs, and workspace config.

## Reference

- `src/config/mod.rs` — authoritative source for every config key, type, and default.
- `scripts/default-config.toml` — the template seeded on `just install`. Must stay in sync with `mod.rs`.
- `mcp_servers.toml` (next to `config.toml` in the channel profile) is a separate, user-owned file — Plexi reads it and never writes it. Format and process contract: `src/host/mcp_client.rs`; user docs: `docs/CONFIG.md`.

## Rules

- **Never hardcode profile or workspace paths.** See path rules in `src/cli/AGENTS.md`.
- **Required fields have no defaults.** Fail fast with a clear error. Optional fields are clearly marked.
- **Alpha config stays default.** `just install` refreshes it from the template. Beta config is the staging ground.
- When adding a new config key, update `scripts/default-config.toml` in the same change.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.

## Traps

- **Shared Cargo targets can reuse a config-doc generator from another worktree.** `gen_config_docs` embeds `CARGO_MANIFEST_DIR` and reads that checkout's config sources at runtime. If its output contains keys absent from the current tree, force the generator to recompile (touch `tools/gen_config_docs/src/main.rs` before `cargo run -p gen_config_docs`) and compare again. Do not copy another checkout's generated config into this one.
