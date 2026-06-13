---
id: "0183"
title: "Marketplace: scope-gated skill activation (design + runtime)"
status: backlog
estimate: "20h"
sprint: "s32"
blocked_by:
  - 182
gh_issue: []
area:
  - "host/permissions"
  - "sdk/pgap"
tags:
  - "marketplace"
  - "skills"
  - "agents"
  - "runtime"
---


A skill bundled with an app is active only within that app's scope — the same context-scoping the secret model already uses (a secret keyed to a workspace root is only readable by apps at that root). An agent that shares an app's context inherits that app's skills; when the app closes or the agent leaves the scope, the skills go dormant.

## Why

This is the unification Ian is after: "if an agent is open in the same pane as an app, it has that skill by default." Skills become ambient capabilities that follow app presence instead of polluting a global namespace. It mirrors secret context-scoping ([[GLOSSARY.md]] Secret entry).

## Done When

- A written design spec lands first (where activation scope is defined: workspace, context, or pane; how the assistant discovers active skills; lifecycle on app open/close).
- Bundled skills activate for an agent sharing the app's scope and deactivate when the app leaves scope/closes.
- Skill availability is observable (a way to list which skills are active for an agent and why).
- HostHarness coverage: skill active when app in scope, dormant when not.

## Gotchas

- Runtime feature, not packaging — write the spec before runtime code.
- Reuse the secret context-scoping model rather than inventing a parallel scoping mechanism.

## References

- `docs/prm/marketplace-hosted.md`
- Depends on app-bundled skills packaging from `0182`.
- Related: agent-is-a-PGAP-app model.
