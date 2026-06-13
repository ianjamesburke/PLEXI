# sdk/python — Agent Contract

**Read before editing anything under `sdk/python/`:** this file, plus the root `AGENTS.md`. These are the silent-failure traps specific to this package — none are visible from the code alone.

## Traps

- **Emitter proxies are hand-synced, not generated.** `_render_context.py` holds proxy wrappers for every `Emitter` method (`notify`, `notify_choice`, `notify_input`, `notify_and_wait`, …). When you add or change a parameter on a method in `_emitter.py`, update the matching proxy in `_render_context.py` in the same edit — nothing generates them, and a stale proxy silently drops the new param.
- **`plexi_sdk` is only importable in Plexi-spawned processes.** It is on PYTHONPATH only for processes Plexi launches; a terminal pane's bare `python3` never sees it. Do not validate import changes with `python3 -c "import plexi_sdk"` — it will fail or import a stale copy. Verify by opening a canvas app and checking it renders.

## Style

Document stable contracts, not history. If a trap here stops being true after a refactor, update it in the same change; otherwise leave it alone.
