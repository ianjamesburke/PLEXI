# sdk/python — Agent Contract

**Read before editing anything under `sdk/python/`:** this file, plus the root `AGENTS.md`.

## Reference

- **[AUTHORING.md](AUTHORING.md) — canonical guide for building an app.** Start here.
- [SDK_V3.md](SDK_V3.md) — design/protocol spec: adapter contract, CPython-in-WASM bridge, WIT mapping.
- [`../../website/src/content/docs/sdk.md`](../../website/src/content/docs/sdk.md) — full API reference, generated from the SDK source (gated fresh in CI).

## Traps

- **`plexi_sdk` is only importable inside Plexi's app runtime.** The CPython-in-WASM bridge adds it to `PYTHONPATH`; a terminal pane's bare `python3` does not. Do not validate imports with `python3 -c "import plexi_sdk"` — it will fail or import a stale copy.
- **`plexi app test` must pin the interpreter.** The SDK is `PYTHONPATH`-injected, not installed, so uv gets no `requires-python` signal and picks whatever `python3` is on PATH (3.9 on a stock Mac → `tomllib` import error). `app_test_cli` runs pytest on the app venv from `python_env::ensure_app_venv`; never let it fall back to the ambient interpreter.
- **`plexi app test` keeps the app root first on `PYTHONPATH`.** Test collection must be able to import an app's entry module before the injected SDK entries.
- **`view()` must be pure.** Calling `state.set()` inside `view()` raises `RuntimeError`. All state mutations return effects from `update()`.
- **Effect return, not mutation.** `update()` returns a list of effect objects. Nothing is mutated in-place. The adapter executes effects after `update()` returns.
- **Packaged Python apps use the package SDK.** An installed release does not read an edited checkout or accept `PLEXI_SDK_PATH` as a replacement for its package-owned SDK. Rebuild and package the SDK change before validating it with the installed host. Resource ownership and source-build discovery are defined in `scripts/DISTRIBUTION.md`.
- **`McpConnect`/`McpSend`/`McpDisconnect` are CPython-runtime-only.** They ride the raw JSON bridge in `LivePythonPane` and never appear in `wit/plexi.wit`, so Rust WASM component apps cannot use them. Do not add effects to the WIT world casually: extending it changes the component type and the host rejects every prebuilt component (CPython shim bundle, `apps/wasm-poc/*`) built against the old world — all fixtures must be rebuilt in the same change.
- **`McpConnect` carries a server id, never a command.** The host resolves argv from user-owned `mcp_servers.toml` (`src/host/mcp_client.rs`). If a command/argv field ever appears on the wire, `mcp.client` has silently become arbitrary process execution.

## Style

Document stable contracts, not history. Update traps in the same change that makes them obsolete.
