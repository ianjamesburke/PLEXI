---
id: "0274"
title: "feat: bundled agent definitions and install-time discovery"
status: in-progress
priority: p2
started_at: "2026-06-21T19:50:07Z"
blocked_by: []
gh_issue: []
area:
  - "agents"
  - "infra/build"
tags:
  - "v2"
---






Define where agent definitions that ship with Plexi live in the repo and wire them into the install path so they land in `~/.plexi-<channel>/agents/` alongside apps.

The host already discovers agents from `~/.plexi/agents/` (global) and `<workspace>/.plexi/agents/` (local), with `plexi agent install` and `plexi agent update` CLI commands. But there is no repo-side convention for bundled agents and `scripts/install.sh` does not copy them.

## Scope

- Choose a repo directory for bundled agent definitions (likely `agents/` at repo root, parallel to `apps/`)
- Add agent install step to `scripts/install.sh` that copies bundled agents into `~/.plexi-<channel>/agents/`
- Move `apps/examples/chess-opponent/` to the new location as the first bundled agent
- Ensure `just install` picks up agents the same way it picks up apps

## Non-Scope

- Agent marketplace or remote distribution
- Changes to the host's runtime agent discovery (`src/agent/mod.rs`)
- Assistant agent registry (stint 0225)
- Agent permission broker or grant storage

## Why

chess-opponent is the only agent definition in the repo and it's buried in `apps/examples/` with no install path. Without a convention, every new agent definition will land in a random spot.

## References

- `apps/examples/chess-opponent/` — only existing bundled agent
- `src/agent/mod.rs:200` — `load_agents()` scans workspace channel dir
- `src/cli/agent.rs` — install/update commands copy from global to workspace
- `src/app/registry.rs:376` — registry scans `agents/` subdirs
- `scripts/install.sh` — installs apps and skills but not agents
