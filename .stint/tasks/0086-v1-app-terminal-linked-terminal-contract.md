---
id: "0086"
title: "v1 app-terminal: linked terminal contract"
status: done
estimate: "10h"
actual: "6m"
started_at: "2026-06-13T08:23:54Z"
completed_at: "2026-06-13T08:28:56Z"
sprint: "s3"
blocked_by: []
gh_issue:
  - "599"
area:
  - "host/pane-ops"
  - "host/terminal"
  - "host/permissions"
  - "sdk/pgap"
tags:
  - "v1"
  - "app-terminal"
  - "permissions"
  - "terminal"
---




Formalize and harden the linked app-terminal contract before marketplace-style apps can drive terminals.

## Already Landed

Significant infrastructure shipped in stints 0013/0014 and the File Explorer work:
- `Capability::TerminalBindings` gates all five terminal operations (`RequestLinkedTerminal`, `RunInLinkedTerminal`, `InsertPathToken`, `RequestCommandPreview`, `OpenArtifact`) in `src/app/permissions.rs`
- `linked_pane_id` and `pane_group` fields on `PaneEntry` (`src/host/pane.rs`)
- Dispatch + denial tests in `src/app/dispatch.rs` and `src/process_app/tests/canvas_bindings_tests.rs`
- Directory handoff shipped as stint 0113 / GH #2145 (closed)

## Remaining Work

- Audit all five terminal bindings operations for consistent permission error surfaces (some emit sentinel events, some drop silently)
- Add HostHarness tests for lifecycle/visual grouping behavior (close app = close linked terminal?)
- Document the contract in a developer-facing reference so marketplace app authors know how to declare and use terminal bindings
- Verify `pane_group` is wired into visual chrome (tab grouping, paired close)

## Why

This is broader than paired pane close behavior. It needs to settle how apps obtain terminals, how `terminal.bindings` maps to permission prompts, how command preview and arbitrary command execution work, how lifecycle/visual grouping behaves, and how directory handoff flows avoid invisible PTY writes.

## Scope

- `src/app/canvas_bindings.rs` (dispatch implementations)
- `src/app/dispatch.rs` (routing + tests)
- `src/app/permissions.rs` (capability enum)
- `src/host/pane.rs` (PaneEntry fields)
- `src/process_app/routing.rs` (PGAP routing)
- `src/process_app/tests/canvas_bindings_tests.rs` (tests)

## Follow-ups

File Explorer directory handoff (#2145 / stint `0113`) already shipped and consumed this contract.

## Audit outcome

The five operations and their permission checks were already shipped (0062). The
audit found the "inconsistent error surfaces" are in fact **principled**:
request-response ops (`RequestLinkedTerminal`, `RequestCommandPreview`) carry a
`request_id` and emit a sentinel on denial so the SDK helper unblocks;
fire-and-forget ops (`RunInLinkedTerminal`, `InsertPathToken`, `OpenArtifact`)
have no `request_id` and drop silently. This is a coherent contract — **not**
changed. Completed the denial-test coverage with silent-drop tests for
`InsertPathToken` and `OpenArtifact` (the two that were missing).

Genuine gaps were filed as issues rather than changed inline:
- Lifecycle (paired close, stale-link clearing, visual grouping from
  `pane_group`) → **#2241**. No close path touches `linked_pane_id` today.
- `OpenArtifact` lexical path check does not resolve symlinks → **#2242**.

Primary deliverable: `docs/TERMINAL_BINDINGS_CONTRACT.md` documenting the
capability, the five ops, the principled denial contract, current lifecycle,
`pane_group` routing (no visual grouping yet), and path safety. This fulfills the
"define the contract" goal of the linked GitHub issue **#599**, which is closed
with the doc as evidence; the open lifecycle decisions live in #2241.

## Variance

Estimate 10h. The execution + permissions were already shipped, so the work was
the audit (confirming the denial contract is principled), the contract doc, two
denial-coverage tests, and filing the two real gaps as issues — not a rewrite of
security-sensitive code.
