# Plexi Roadmap

This file is an index, not the dispatch queue.

The canonical v1 plan is [`docs/prm/app-framework-marketplace.md`](docs/prm/app-framework-marketplace.md). The WASM runtime spec is [`docs/prm/wasm-runtime.md`](docs/prm/wasm-runtime.md). Sprint tasks live in `.stint/`. If this file and a PRM disagree, the PRM wins.

## Current Focus -- Sprint s50 (v1 Landing)

All remaining v1 work is consolidated into one sprint. Sprints S1-S6 and supporting sprints (S7-S14, S31) are complete.

Remaining tasks:
1. File Explorer completion: recursive search, native actions, settings modal.
2. CLI tooling: `plexi config set` / `plexi config list`.
3. UI fixes: PIP/chip overlap on list rows.
4. Agent tooling: implement-stint two-phase sub-agent refactor.
5. v1 release gate: docs cleanup, install QA, security wording audit.

## Already Shipped

- App framework: PGAP v3, SDK v2, scaffold, dev loop, Core 9 reference apps.
- Trust and packaging: capability grants, install trust sheet, package validation, local install.
- Marketplace: hosted registry, publisher submission, browse/install, paid apps spec, AI subscription spec.
- Host UI: modal system, command palette, shortcut display, permission grants, UI gallery.
- Host agents: pane-native state, tool detail, file slots.
- WASM runtime: gates G1-G7, G11-G13 shipped. Lanes A-E complete. Zero-copy present, launch args, persistent grants.
- File Explorer: adaptive layout, columns, inspector, Quick Look, multi-select, safe operations, linked terminal.

## v2 (Post-v1)

- WASM: G8 Python compat, G9 cloud execution, G10 payment gate.
- Marketplace: scope flags, bundled skills, scope-gated activation.
- Docking layout engine.
- Collapsible subcontexts (needs keyboard navigation design).
- CLI-native PGAP apps (Board/Console primitives).

## Dispatch Rules

1. Read `docs/prm/app-framework-marketplace.md`.
2. Use `.stint/` for sprint sequencing: `stint sprint show 50`, `stint next`.
3. Use GitHub issues as implementation tickets.
4. Pick parallel lanes with non-overlapping `area:*` labels.
