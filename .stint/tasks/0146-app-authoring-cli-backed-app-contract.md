---
id: "0146"
title: "App authoring: CLI-backed app contract"
status: done
estimate: "4h"
actual: "0m"
completed_at: "2026-06-13T16:45:46Z"
sprint: "s2"
blocked_by: []
gh_issue: []
area:
  - "cli/commands"
  - "host/terminal"
  - "sdk/pgap"
tags:
  - "v1"
  - "app-authoring"
  - "cli-renderer"
  - "terminal"
---



Formalize and document the contract for CLI-backed Plexi apps. The renderer hardening (0062) already shipped, making de facto decisions about lifecycle, caching, and descriptor invalidation. This task captures those decisions as explicit documentation and fills remaining gaps.

## Why

0062 shipped the working implementation but the lifecycle contract is implicit in code. Agents and third-party authors need an explicit reference for how `plexi app open --cli` apps launch, crash, restart, generate UI descriptors, and interact with permissions.

## Scope

- Audit `src/render/cli_renderer_app.rs` and extract the de facto lifecycle contract (launch, ready, reload, close, crash, restart) into `docs/cli-app-contract.md`.
- Document how CLI-backed apps generate UI descriptors, how the host caches them, and how stale descriptors are invalidated (already implemented in 0062).
- Document permission prompts for command execution, filesystem access, network access.
- Document logging and inspection behavior (`pane info`, `pane list`, host logs).
- Identify any gaps between the 0062 implementation and the ideal contract; file issues for gaps, do not fix inline.
- Verify channel-agnostic behavior across alpha, beta, main, and PR builds.

## Context

0062 (CLI renderer hardening) shipped without this contract being finalized first. The original blocking relationship is now inverted: this task documents what 0062 built, rather than defining what 0062 should build. Estimate reduced from 8h to 4h since this is now documentation + gap analysis, not design.
