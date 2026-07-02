# apps/ — Agent Contract

**Read before editing anything under `apps/`:** this file, plus the root `AGENTS.md`. Rules here are local to `apps/` and override nothing above them; they add app-specific contracts the root file does not repeat.

## Scope

Each app is a directory with a `manifest.toml` (schema_version + `[app]` id/type/name/entry + `[app.capabilities]`) and a Python entry file. Apps run on the Python SDK in `sdk/python`. **Canonical authoring guide: [`../sdk/python/AUTHORING.md`](../sdk/python/AUTHORING.md)** — it points on to the generated API reference and the `SDK_V3.md` design spec. Marketplace context: [`../docs/app-framework-marketplace.md`](../docs/app-framework-marketplace.md).

## Rules

- **Scaffold, never hand-write.** Create a new app with `plexi app init <name>` (or `plexi-pr-<N> app init <name>` on a PR build). Never author a `manifest.toml` by hand — the scaffold sets the correct `schema_version` and a valid manifest.
- **Prune scaffold imports.** The template imports ~6 SDK symbols; delete the unused ones before finishing. Pyright flags them immediately.
- **Every top-level app is a maintained exemplar.** The directories directly under `apps/` (not `dev/`, not `examples/`) are the curated exemplar set: `calc`, `csv_viewer`, `github-issues`, `logs`, `permissions`, `stats`, `todo`, `wikipedia`. [`packs/core.toml`](../packs/core.toml) is the subset the host auto-seeds into every profile on launch (`logs`, `stats`, `todo`, `calc`, `permissions`); the capability-gated apps (`csv_viewer`, `github-issues`, `wikipedia`) stay top-level and seed on demand. Everything under `dev/` and `examples/` is a throwaway proof-of-concept — do not maintain it. Touch a `dev/` app only when the change is itself a POC demonstrating a new SDK or host capability.
- **Exemplar bar — the gate for adding or keeping any top-level app.** Shipped apps are the de facto documentation; agents copy whatever patterns they find here. A top-level app must therefore pass the same gates a freshly scaffolded app does: `plexi app check` and `plexi app test` green on a current build, current SDK idioms only (semantic `ActionBar`/`FooterKeys`, the `log.*` module, `SelectList`/`TextEdit` over hand-rolled key handling), and no pre-v3 patterns. An app that cannot meet the bar belongs in `dev/` or is deleted — never left half-broken at the top level.
- **New capability ⇒ POC in `dev/`, not `examples/`.** Every user-visible host/SDK capability ships a small POC app under `apps/dev/`. The `alpha`/`pr-*` install flattens `dev/` to the top level (see `scripts/AGENTS.md`), so an `examples/` POC will not be picked up.
- **PGAP is L1-only.** Build declarative L1 UI trees. L0 is deprecated and its `_l0` fallbacks are gone; the `Raw` escape hatch stays.
- **Log through the frame.** Use `log.debug/info/warn/error(...)` from `plexi_sdk`. App logs forward into the host log tagged `app::<app_id>`.

## Design philosophy (apps + SDK)

See [`../sdk/python/AUTHORING.md`](../sdk/python/AUTHORING.md) § Design Philosophy — the single copy of the apps + SDK design principles.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
