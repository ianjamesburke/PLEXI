# sdk/python — Agent Contract

**Read before editing anything under `sdk/python/`:** this file, plus the root `AGENTS.md`.

## Reference

- [SDK_V3.md](SDK_V3.md) — complete design spec: all types, adapter protocol, native ProcessApp bridge, scaffold template, deletion list.

## Traps

- **`plexi_sdk` is only importable in Plexi-spawned processes.** It is on PYTHONPATH only for apps Plexi launches through native `ProcessApp`. A terminal pane's bare `python3` never sees it. Do not validate import changes with `python3 -c "import plexi_sdk"` — it will fail or import a stale copy.
- **`view()` must be pure.** Calling `state.set()` inside `view()` raises `RuntimeError`. All state mutations return effects from `update()`.
- **Effect return, not mutation.** `update()` returns a list of effect objects. Nothing is mutated in-place. The adapter executes effects after `update()` returns.

## Style

Document stable contracts, not history. Update traps in the same change that makes them obsolete.
