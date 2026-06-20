# scripts — Agent Contract

**Read before editing anything under scripts/:** this file, plus the root AGENTS.md.

## Scope

Build, install, release, and channel management scripts. Called from `justfile` recipes.

## Reference

- [RELEASE_CHANNELS.md](RELEASE_CHANNELS.md) — channel table, feature gates, RC flow, bare CLI shim, stable release flow.

## Rules

- **Channel-agnostic.** Every script must work identically on alpha, beta, main, RC, and PR builds.
- **Never hardcode profile paths** (e.g. `~/.plexi-alpha/`). Derive from the binary name or `config_dir()`.
- Scripts are the only place `just` recipes call into. Do not duplicate script logic in the justfile.
- `default-config.toml` is the config template seeded on install. Keep it in sync with `src/config/CONFIG.md`.

## Child DOX Index

- `default-scripts/` — default app scripts bundled into new user profiles.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
