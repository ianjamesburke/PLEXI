# docs - Agent Contract

This directory contains active PRMs (product/architecture specs). Present-tense reference docs live next to the code they describe, in the owning directory's AGENTS.md. `security/` holds threat reviews; those files are not PRMs and have no stint row.

## PRM Rules

A PRM is the destination spec for a feature. It describes what to build and why. It is not a progress tracker.

**One source of truth for progress: stint tasks.** Every PRM must have a `Stint:` line at the top naming the task ID(s) that own its execution. Stint tasks are the only place work state lives. Never track progress inside a PRM (no checklists, no strikethrough, no status tables).

**Status line:** `active` (being stinted) or `superseded` (absorbed into another doc - note which one).

**Delete rule:** delete the PRM in the same PR that closes its last stint task. The stint task is the delete trigger.

`plexi agent team run <name>` resolves a workspace team by walking up from the
current directory, then the active channel profile's `teams/` directory, then
built-ins. Global and built-in teams run outside a workspace; their default
reports are stored under that profile's `sessions/teams/` directory.

## Active PRMs

| File | Covers | Stint |
|---|---|---|
| `agent-sandbox-profile.md` | Per-harness agent sandbox profile (Claude Code, Codex, Pi): launch profile, no-native-sandbox policy, user-visible contract (v1.1) | 1273, 1378–1380 |
| `app-framework-marketplace.md` | v1 app platform + marketplace | see file |
| `assistant-authority-model.md` | Assistant threat model, reference monitor, grant binding, runtime boundary | see file |
| `specs/agents-api-and-permission-gate.md` | Agents API and the one permission gate. P1 is the monitor, exact grants, and host-approved chess | none yet |
| `prm/permissions-broker.md` | Thin core, public plugin capability API, broker consolidation and core-plugin extraction | see file |
| `wasm-runtime.md` | WASM runtime architecture | see file |
