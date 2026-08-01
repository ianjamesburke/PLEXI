# sdk/python — Agent Contract

**Read before editing anything under `sdk/python/`:** this file, plus the root `AGENTS.md`.

## Reference

- **[AUTHORING.md](AUTHORING.md) — canonical guide for building an app.** Start here.
- [SDK_V3.md](SDK_V3.md) — design/protocol spec: adapter contract, CPython-in-WASM bridge, WIT mapping.
- [`../../website/src/content/docs/sdk.md`](../../website/src/content/docs/sdk.md) — full API reference, generated from the SDK source (gated fresh in CI).

## Traps

- **`plexi_sdk` is only importable inside Plexi's app runtime.** The CPython-in-WASM bridge adds it to `PYTHONPATH`; a terminal pane's bare `python3` does not. Do not validate imports with `python3 -c "import plexi_sdk"` — it will fail or import a stale copy.
- **`view()` must be pure.** Calling `state.set()` inside `view()` raises `RuntimeError`. All state mutations return effects from `update()`.
- **Effect return, not mutation.** `update()` returns a list of effect objects. Nothing is mutated in-place. The adapter executes effects after `update()` returns.
- **`PLEXI_SDK_PATH` does not reach a Python app.** The CPython-in-WASM runtime preopens its `/sdk` mount from `env!("CARGO_MANIFEST_DIR")/sdk/python` — the tree the *binary* was compiled from — and `sdk_path_override` never touches that path (it only builds `PYTHONPATH` for native subprocesses). So `PLEXI_SDK_PATH=… plexi-<channel> app render` on a Python app silently runs the installed binary's SDK, and a new SDK symbol fails with an `ImportError` naming `/sdk/plexi_sdk/…`. Prove SDK changes with `pytest` against the worktree; rebuild the binary before believing any headless render of them.

## Style

Document stable contracts, not history. Update traps in the same change that makes them obsolete.
