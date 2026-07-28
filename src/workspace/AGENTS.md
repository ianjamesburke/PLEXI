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
- **Test keychain isolation — exact scope**: `system_store()` is the only ambient/system-backend **selector** — the one place code asks "which real store does this process use" (tests still construct `InMemoryKeychain` directly and APIs accept injected `SecretStore` handles; that is by design). Under `cfg(test)` it selects a process-local in-memory store; the real backend type (`MacKeychain`), the whole `secrets-index.json` layer, the legacy `crate::secrets` module, and the startup migration body are **not compiled at all** — so no test body, scene, harness app constructor, or lazy static can route to the login keychain or the user's real index through our code. A pre-main constructor (`keychain_prompt_guard` in `src/testing/mod.rs`) additionally disables keychain user interaction for the test process, so a would-prompt call errors instead of prompting. **What this does NOT cover**: it is scoped to the `plexi` bin test process only — not child processes, not pr binaries, not hosts, not other test executables — and it prevents ROUTING and PROMPTS, not a deliberate direct call into the `security_framework` dependency, which any same-crate test can compile and which performs a real, silent, unprompted keychain operation. Direct dependency calls outside `src/workspace/secrets.rs` and the prompt guard are banned; the genuine close (process keychain search list pointed at a throwaway keychain, plus subprocess isolation) is stint 0603. Why (2026-07-28): keychain ACLs are per-binary and every fresh test binary is a new unsigned app, so each login-keychain **value** read from a test fires its own credential dialog — an unattended gate cannot click one.

Resolution order for a canonical name with no alias route (`resolve_with_source`):

1. `plexi:<workspace-id>:<canonical_name>` (workspace Keychain entry)
2. `plexi:user:<canonical_name>` when `fallback = true`

Secret values never live in TOML — `secrets.toml` carries only routes and metadata (display label, provider, docs URL). Process-env and legacy lowercase key names are compatibility fallbacks, not a second primary resolution system — do not reintroduce a parallel injection path.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
