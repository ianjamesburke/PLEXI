# src/workspace — Agent Contract

**Read before editing anything under src/workspace/:** this file, plus the root AGENTS.md.

## Scope

Workspace state (contexts, panes, layout — `src/workspace/mod.rs`, `router.rs`) and workspace-scoped environment secrets (`src/workspace/secrets.rs`).

## Reference — Environment Secrets

One resolver serves every consumer that needs a credential: terminal PTY env construction, `plexi run` env injection, PGAP `secrets.get`, and host integrations (e.g. the AI broker). See `src/workspace/secrets.rs`.

- **Canonical name**: the env var name a tool expects (e.g. `OPENAI_API_KEY`) — the primary identity and default Keychain suffix.
- **Scope**: `workspace` or `global`. Workspace values win over global values; global is a cross-workspace fallback, never a workspace-local override.
- **Alias**: an optional route from a canonical name to a different storage name, for compatibility/teams — not the default UX.
- **Injection policy**: a workspace-controlled allowlist (`[terminal.env].inject` in `secrets.toml`) of canonical names Plexi may place into PTY environments. Never inject every stored secret into a PTY by default.

Resolution order for a canonical name with no alias route (`resolve_with_source`):

1. `plexi:<workspace-id>:<canonical_name>` (workspace Keychain entry)
2. `plexi:user:<canonical_name>` when `fallback = true`

Secret values never live in TOML — `secrets.toml` carries only routes and metadata (display label, provider, docs URL). Process-env and legacy lowercase key names are compatibility fallbacks, not a second primary resolution system — do not reintroduce a parallel injection path.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
