---
id: "0161"
title: "v2 secrets: workspace-scoped PTY env injection"
status: in-progress
estimate: "8h"
started_at: "2026-06-11T22:10:16Z"
sprint: "s30"
blocked_by:
  - 41
  - 42
gh_issue: []
area:
  - "host/secrets"
  - "host/terminal"
  - "host/config"
tags:
  - "v2"
  - "secrets"
  - "terminal"
  - "config"
---


Implement the [`workspace environment secrets PRM`](../../docs/prm/workspace-env-secrets.md): canonical env-var secret names, workspace-vs-global resolution, and allowlisted PTY environment injection.

## Scope

- Add a single workspace-aware secret resolver shared by terminal PTY env construction, `plexi run`, PGAP `secrets.get`, and host AI integrations.
- Treat all-caps canonical names such as `OPENROUTER_API_KEY`, `OPENAI_API_KEY`, `NVIDIA_API_KEY`, and `STRIPE_API_KEY` as the primary UX.
- Add a workspace `terminal.env.inject` allowlist so new PTY panes receive only selected secrets.
- Preserve aliases as an advanced compatibility layer, not the main user model.
- Migrate OpenRouter toward canonical `OPENROUTER_API_KEY`, with temporary compatibility for `openrouter-api-key`.

## Foot-Gun Constraints

- Do not inject every stored secret into every terminal.
- Do not keep legacy terminal env injection and workspace secret routing as separate sources of truth.
- Do not write secret values into TOML. Only routes, metadata, and policy belong on disk.
- Workspace values must override global values with the same canonical name.

## Done

- Two workspaces can store different values for `OPENROUTER_API_KEY`; terminals opened in each workspace receive their own value when the name is allowlisted.
- A global `OPENAI_API_KEY` can fall back into any workspace that has `fallback = true` and no workspace value.
- `plexi run`, PGAP apps, PTY env, and host AI broker resolve through the same code path.
- Tests cover workspace override, global fallback, no-inject-by-default, and OpenRouter canonical-key migration.
