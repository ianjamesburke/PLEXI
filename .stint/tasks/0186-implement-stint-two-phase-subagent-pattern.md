---
id: "0186"
title: "implement-stint: two-phase sub-agent pattern"
status: backlog
estimate: "3h"
sprint: "s12"
blocked_by: []
area:
  - "meta/skills"
tags:
  - "dx"
  - "skills"
---

Restructure `/implement-stint` to use two sub-agents so the orchestrator context
stays clean and implementation work scales without polluting the main window.

## Why

Currently the orchestrator does all research, formulation, and code edits inline.
For larger tasks this fills context quickly. The Gemini review loop also runs in
the main context. Sub-agents let each phase own its context budget independently.

## What

**Phase 5 → Sub-agent R (Research)**
- Spawned after worktree creation
- Read-only: reads task body, issue body, greps codebase, checks CLAUDE.md constraints
- Short-circuits if the issue already contains an explicit implementation map (file+line refs) — just reformats into spec structure
- Returns a structured impl spec: files to change, what changes, invariants, test command, logging plan

**Phase 6 → Sub-agent I (Implement + Review loop)**
- Receives the spec from Sub-agent R plus worktree path
- Makes all edits, runs `cargo build` / tests
- Runs Gemini diff review (max 2 runs per validation attempt)
- Iterates on Gemini findings internally until clean
- Stages changes but does NOT commit
- Returns: summary of what changed + test results + final Gemini verdict

**Orchestrator**
- Reviews the staged diff before committing
- Owns the commit, push, and handoff to `/open-pr`
- If Sub-agent I returns unresolved findings after 2 Gemini runs, surfaces them to the user before committing

## Done When

- `/implement-stint` Phase 5 spawns Sub-agent R and uses its output as the impl spec
- `/implement-stint` Phase 6 spawns Sub-agent I which handles edits + Gemini loop internally
- Orchestrator reviews diff and commits (never Sub-agent I)
- All existing implement-stint invariants preserved (claim commit, worktree base check, pane naming, etc.)
