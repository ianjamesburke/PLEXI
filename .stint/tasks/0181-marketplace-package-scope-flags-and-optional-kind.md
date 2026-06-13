---
id: "0181"
title: "Marketplace: package install scope flags + optional kind"
status: backlog
estimate: "8h"
sprint: "s32"
blocked_by:
  - 20
gh_issue: []
area:
  - "cli/commands"
  - "host/permissions"
tags:
  - "marketplace"
  - "scope"
  - "kind"
---


Make every installable package (app/agent/skill) installable either workspace-local or global, chosen by the installer with `-ws`/`--workspace` and `-g`/`--global` flags. Add an optional `kind` manifest field (`app | agent | skill`, default `app`) as metadata only.

## Why

Scope must be a universal install-time choice, never forced by the manifest or derived from kind. Ian: "I can't think of a situation where an app would NEED to be global... I want them to work like agent skills, they can be either just fine." This enables nix-style declarative workspace provisioning — a setup script is a list of `plexi app install <id> -ws` lines, reproducible per project. `kind` is optional metadata for filtering/display; do not force the field.

## Done When

- `plexi app install <id>` and `plexi install <id>` accept `-g/--global` and `-ws/--workspace`.
- `--workspace` installs into `<workspace_root>/<workspace_channel_dir()>/apps/`; `--global` into `apps_dir()`. Default preserves current behavior (global) and is documented.
- Workspace install walks up from CWD to find the workspace (reuse the `app init` resolution); errors clearly when `--workspace` is used outside a workspace.
- Optional `[app].kind` (or `kind`) field parses, defaults to `app`, surfaces in the trust sheet and registry entry, and never gates behavior.
- Tests: install dest resolution for both scopes; kind defaulting; `--workspace` outside a workspace fails closed.

## References

- `docs/prm/marketplace-hosted.md`
- Memory: package scope is a universal install flag, kind optional.
