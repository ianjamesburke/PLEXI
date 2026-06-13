# apps/ — Agent Contract

**Read before editing anything under `apps/`:** this file, plus the root `AGENTS.md`. Rules here are local to `apps/` and override nothing above them; they add app-specific contracts the root file does not repeat.

## Scope

Each app is a directory with a `manifest.toml` (schema_version + `[app]` id/type/name/entry + `[app.capabilities]` + `[launch]`) and a Python entry file. Apps run on the Python SDK in `sdk/python`. Authoring path: `docs/SDK_QUICKSTART.md`, then `docs/sdk-v2.md`, then `docs/prm/app-framework-marketplace.md`.

## Rules

- **Scaffold, never hand-write.** Create a new app with `plexi app init <name>` (or `plexi-pr-<N> app init <name>` on a PR build). Never author a `manifest.toml` by hand — the scaffold sets the correct `schema_version` and a valid manifest.
- **Prune scaffold imports.** The template imports ~6 SDK symbols; delete the unused ones before finishing. Pyright flags them immediately.
- **Core 9 only.** Only fix or improve the maintained Core 9 apps. Everything in `dev/` and `examples/` is a throwaway proof-of-concept — do not maintain it. Touch a `dev/` app only when the change is itself a POC demonstrating a new SDK or host capability.
- **New capability ⇒ POC in `dev/`, not `examples/`.** Every user-visible host/SDK capability ships a small POC app under `apps/dev/`. The install script only flattens `dev/`, so an `examples/` POC will not be picked up.
- **PGAP is L1-only.** Build declarative L1 UI trees. L0 is deprecated and its `_l0` fallbacks are gone; the `Raw` escape hatch stays.
- **Log through the frame.** Use `ctx.info/warn/error/debug(...)` inside a frame and `emit.info(...)` outside one. App logs forward into the host log tagged `app::<app_id>`.

## Design philosophy (apps + SDK)

- Obvious over clever — fight for the solution an agent would naturally assume.
- Simulate affordances, never lie about contracts — isolation, durability, persistence, and security boundaries stay explicit.
- Build primitives, not features — omit anything a developer's agent can trivially build atop the platform.
- Design for agents, not humans browsing docs — if it needs a README to be usable, the API is wrong.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
