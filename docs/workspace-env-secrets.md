# Workspace Environment Secrets PRM

Status: product and architecture spec.
Parent: [`assistant-host-app.md`](assistant-host-app.md).
Stint: [`0161`](../../.stint/tasks/0161-v2-workspace-env-secret-injection.md).
Last updated: 2026-06-11.

This PRM replaces ad hoc shell-secret setup with a Plexi-owned, workspace-aware secret system that can inject selected credentials into terminal panes, `plexi run` commands, PGAP apps, and host integrations.

The goal is simple: a non-technical user can paste an API key into Plexi once, choose where it applies, and tools launched inside Plexi receive the right environment variable without editing `.zshrc`, `.zprofile`, or a private shell include file.

## Product Goal

Plexi should support canonical environment-style secret names as the primary user model:

```text
OPENROUTER_API_KEY
OPENAI_API_KEY
NVIDIA_API_KEY
STRIPE_API_KEY
ANTHROPIC_API_KEY
```

Users can set a key globally or for one workspace. A terminal pane opened in that workspace receives the workspace value when configured for injection. If the workspace has no value and fallback is allowed, the pane receives the global value.

Two workspaces may both expose `OPENROUTER_API_KEY` while holding different underlying values.

## North Star Fit

This belongs in Phase 1 and Phase 3:

- Phase 1 because it removes shell setup friction and consolidates overlapping secret paths.
- Phase 3 because AI onboarding depends on user-owned provider keys for OpenRouter, OpenAI, Anthropic, NVIDIA, and similar services.

It serves the North Star by making local, owned provider credentials usable by agents, apps, and terminals without forcing users to learn shell startup files.

## Non-Goals

- Do not inject every stored secret into every terminal.
- Do not create OS-wide environment variables outside Plexi.
- Do not require cloud sync, hosted accounts, or Plexi-managed provider accounts.
- Do not keep lowercase provider-specific names such as `openrouter-api-key` as the primary API.
- Do not preserve two independent secret resolution systems.

## Concepts

**Canonical name**: The env var name a tool expects, such as `OPENAI_API_KEY`. This is the primary identity and default Keychain suffix.

**Scope**: `workspace` or `global`. Workspace values win over global values.

**Metadata**: Optional user-facing details stored outside the secret value: display label, provider, description, docs URL, created time, last-used time, and injection policy.

**Alias**: An advanced route from a canonical name to a different storage name. Aliases exist for compatibility and teams, not as the default UX.

**Injection policy**: A workspace-controlled allowlist of canonical names that Plexi may place into PTY environments.

## Target Behavior

Workspace A:

```text
OPENROUTER_API_KEY = key-a
```

Workspace B:

```text
OPENROUTER_API_KEY = key-b
```

Global:

```text
OPENAI_API_KEY = fallback-key
```

A terminal opened in Workspace A with `OPENROUTER_API_KEY` in its injection allowlist receives:

```sh
OPENROUTER_API_KEY=key-a
OPENAI_API_KEY=fallback-key
```

A terminal opened in Workspace B receives:

```sh
OPENROUTER_API_KEY=key-b
OPENAI_API_KEY=fallback-key
```

Existing panes do not need live env mutation. A settings change can apply to newly opened panes first; a later phase can add user-visible restart/new-pane affordances.

## Configuration Shape

The workspace secret router remains the source of scope and fallback policy, but canonical env names become the base case.

Example:

```toml
fallback = true

[terminal.env]
inject = [
  "OPENROUTER_API_KEY",
  "OPENAI_API_KEY",
  "NVIDIA_API_KEY",
]

[metadata.OPENROUTER_API_KEY]
label = "OpenRouter API key"
provider = "OpenRouter"
docs_url = "https://openrouter.ai/keys"
description = "Used by Plexi Assistant and AI-enabled terminal tools."
```

Aliases remain optional:

```toml
[default]
OPENAI_API_KEY = "openai_personal"
```

If no alias exists, `OPENAI_API_KEY` resolves from:

1. `plexi:<workspace-id>:OPENAI_API_KEY`
2. `plexi:user:OPENAI_API_KEY` when `fallback = true`

## Resolver Contract

One resolver should serve every consumer:

- terminal PTY env construction
- `plexi run` env injection
- PGAP `secrets.get`
- host integrations such as the AI broker
- future Secrets UI validation and previews

The resolver takes:

- workspace root or workspace id
- app/actor id when relevant
- canonical secret names
- consumer kind: `terminal`, `plexi_run`, `pgap_app`, `host`

It returns:

- resolved values, held in zeroizing wrappers where possible
- missing names
- source metadata: workspace, global, alias, or env fallback
- diagnostic text suitable for UI and CLI

## Migration

1. Add canonical global lookup for `OPENROUTER_API_KEY` and keep lowercase `openrouter-api-key` as a temporary legacy fallback.
2. Add canonical-name storage and metadata support to `plexi secret set`.
3. Add `terminal.env.inject` to workspace `secrets.toml` and resolve those names when building `BackendSettings`.
4. Route `plexi run`, PGAP secret reads, and host AI broker through the same resolver.
5. Deprecate legacy injected secrets from `crate::secrets::list_inject_secrets()`.
6. Add a Secrets UI flow for setting global or workspace keys with optional provider docs links.

## Foot-Gun Constraints

- There must be one source of truth for secret resolution. Do not keep legacy terminal env injection and workspace routing as separate systems.
- Canonical all-caps env names are the main user-facing API.
- Injection must be allowlisted. Never inject every stored secret into a PTY by default.
- Workspace value wins over global value.
- Global values are cross-workspace fallback, not workspace-local overrides.
- Secret values stay out of TOML files. TOML may contain metadata and routes only.

## Done When

- A workspace can define which canonical secret names are injected into new PTY panes.
- Workspace and global values with the same canonical name resolve correctly.
- `plexi run`, PGAP apps, PTY env, and the AI broker share the same resolver.
- OpenRouter uses `OPENROUTER_API_KEY` as the canonical key, with a migration path from `openrouter-api-key`.
- The docs explain canonical names, workspace scope, global fallback, and injection allowlists.
- Tests cover two workspaces with different `OPENROUTER_API_KEY` values producing different PTY environments.
