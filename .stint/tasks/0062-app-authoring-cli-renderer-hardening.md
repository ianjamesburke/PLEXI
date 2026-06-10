---
id: "0062"
title: "App authoring: CLI renderer hardening"
status: backlog
sprint: "s2"
estimate: 8h
blocked_by:
  - 146
gh_issue: ["1947"]
area: ["cli/commands"]
tags: ["v1", "app-authoring", "cli-renderer"]
---

Finish the native CLI renderer path by hardening cache, recursive crawl, Plexi-native descriptor detection, and stale Python-renderer assumptions.

## Current State

`src/render/cli_renderer_app.rs` exists and `plexi app open --cli` routes through `cli-renderer`, so the original "replace Python renderer" headline is partially complete. The remaining issue scope is the production hardening described in the action plan.

This task hardens the implementation after the CLI-backed app contract (`0146`) is settled. Do not use it to make new lifecycle or permission decisions ad hoc.
