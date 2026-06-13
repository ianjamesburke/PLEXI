---
id: "0146"
title: "App authoring: CLI-backed app contract"
status: done
estimate: "4h"
actual: "6m"
started_at: "2026-06-13T08:41:12Z"
completed_at: "2026-06-13T08:46:29Z"
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

## Outcome

Wrote `docs/cli-app-contract.md`: the runtime contract for `plexi app open --cli`
apps — launch sequence, ready/run/(no-)reload lifecycle, the two-layer descriptor
cache + version-keyed invalidation, the trust boundary (native builtin, no
permission prompts; the app is as trusted as the wrapped binary), channel-agnostic
behavior, and logging/inspection. Cross-links to `cli-descriptor-guide.md` for the
schema rather than duplicating it. Every claim anchored to verified
`cli_renderer_app.rs` / `open.rs` / `crawl.rs` / `canvas_bindings.rs` line refs.

Seven gaps found and filed (grouped): **#2244** descriptor cache invalidation,
**#2245** command-execution trust gating (writes/reads fields inert), **#2246**
silent degraded-ready + temp-file cleanup.

## Variance

Estimate 4h. Docs-only; the work was reading `cli_renderer_app.rs` end-to-end,
verifying the de-facto contract against the code, and the gap analysis. Delegated
the draft to a subagent and reviewed/verified the anchors before committing.
