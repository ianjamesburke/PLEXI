# docs - Agent Contract

This directory contains active PRMs (product/architecture specs). Present-tense reference docs live next to the code they describe, in the owning directory's AGENTS.md.

## PRM Rules

A PRM is the destination spec for a feature. It describes what to build and why. It is not a progress tracker.

**One source of truth for progress: stint tasks.** Every PRM must have a `Stint:` line at the top naming the task ID(s) that own its execution. Stint tasks are the only place work state lives. Never track progress inside a PRM (no checklists, no strikethrough, no status tables).

**Status line:** `active` (being stinted) or `superseded` (absorbed into another doc - note which one).

**Delete rule:** delete the PRM in the same PR that closes its last stint task. The stint task is the delete trigger.

## Active PRMs

| File | Covers | Stint |
|---|---|---|
| `app-framework-marketplace.md` | v1 app platform + marketplace | see file |
| `assistant-host-app.md` | Host assistant app spec | see file |
| `browser-surface.md` | Native browser App pane, profiles, context binding, automation, and live validation | see file |
| `marketplace-hosted.md` | Hosted marketplace (Sprint S4) | see file |
| `marketplace-monetization.md` | Accounts, payments, no-license commercial model | 0338–0341, 0322 |
| `notes-editor.md` | Native Notes editor, Live Preview, links, attachments, and agent validation | see file |
| `wasm-runtime.md` | WASM runtime architecture | see file |
| `wasm-runtime-impl-plan.md` | WASM runtime build sequence (G1-G7, G11-G13) | see file |
