# src/config — Agent Contract

**Read before editing anything under src/config/:** this file, plus the root AGENTS.md.

## Scope

Configuration loading, validation, channel-scoped profile dirs, and workspace config.

## Reference

- `src/config/mod.rs` — authoritative source for every config key, type, and default.
- `scripts/default-config.toml` — the template seeded on `just install`. Must stay in sync with `mod.rs`.

## Rules

- **Never hardcode profile or workspace paths.** See path rules in `src/cli/AGENTS.md`.
- **Required fields have no defaults.** Fail fast with a clear error. Optional fields are clearly marked.
- **Alpha config stays default.** `just install` refreshes it from the template. Beta config is the staging ground.
- When adding a new config key, update `scripts/default-config.toml` in the same change.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
