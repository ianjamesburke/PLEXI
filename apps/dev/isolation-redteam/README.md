# isolation-redteam

Adversarial sample app for the CPython-in-WASM sandbox stress test
(`audit/wasm-isolation-redteam`, 2026-09-21).

**Threat model (honest):** each pane has its own `wasmtime::Store` + linear
memory; host reach is only via linked WIT / JSON bridge. Escape for agent trust
is usually abusing granted effects — not reading another guest's memory. This
fixture does **not** claim Docker-level jailbreak immunity.

## What it tries

1. Native WASI `open` / `listdir` / `os.environ` against host home, secrets,
   sibling apps under `~/.plexi-*/apps/*`.
2. Capability-gated effects with **empty** manifest capabilities:
   `FileRead`, `FileWrite`, `HttpFetch`, `ReadHostLog`, `OpenFilePicker`,
   `McpConnect`, `AiQuery`, `SubscribeEventStreams`.
3. Raw stdout protocol injection: `spawn_app` / `spawn_pane` /
   `show_notification` (bridge accepts these; SDK does not expose them).
4. Cross-app event subscribe to `event-probe` / `todo` without a broker grant.
5. `RequestCapability` for secrets-looking IDs (Python path only echoes
   manifest membership — no interactive prompt).

## How to drive

```bash
# From a built plexi with the repo apps on the search path, or install this
# folder into ~/.plexi-alpha/apps/isolation-redteam
plexi-alpha app open isolation-redteam
# Press r to re-run the matrix; results render in-pane.
```

Host unit-test evidence (denial paths already covered in-tree):

```bash
cargo test --bin plexi -- file_read_outside_scope fs_read_effect_is_blocked \
  http_fetch_effect_is_blocked resolve_app_fs_path
```

See also `docs/wasm-runtime.md` Security Model and the daily_log write-up
`WASM-ISOLATION-STRESS-2026-09-21.md`.
