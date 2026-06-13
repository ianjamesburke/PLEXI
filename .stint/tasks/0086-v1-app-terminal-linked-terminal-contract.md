---
id: "0086"
title: "v1 app-terminal: linked terminal contract"
status: todo
estimate: "10h"
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
