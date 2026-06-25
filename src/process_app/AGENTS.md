# src/process_app — Agent Contract

**Read before editing anything under src/process_app/:** this file, plus the root AGENTS.md.

## Scope

PGAP process app lifecycle: launch, IPC routing, capability gating, linked terminal bindings, and the security model.

## Reference

- [TERMINAL_BINDINGS_CONTRACT.md](TERMINAL_BINDINGS_CONTRACT.md) — the `terminal.bindings` capability contract: five operations, permission error contract, lifecycle, known gaps.
- [SECURITY_MODEL.md](SECURITY_MODEL.md) — full security model: consent+audit for v1, capability gating table, what is and isn't sandboxed, future WASM sandbox.
- [shell-execution-inventory.md](shell-execution-inventory.md) — every shell execution path in the host classified by trust source.

## Security invariants

- **No new app-reachable `sh -c` path** without a capability gate and a denial test. Update the shell execution inventory in the same change.
- v1 apps are native Python subprocesses. The trust boundary is consent+audit, not process isolation.
- App subprocesses do NOT inherit `PLEXI_SOCKET`. Only terminal PTY panes get it.

## Traps

- **Capability checks happen at the routing boundary** (`routing.rs`). A request from an app without the grant is denied before any host effect.
- **Request-response denials emit sentinel values** (e.g. `terminal_pane_id: 0`) so the SDK unblocks instead of hanging. Fire-and-forget denials are dropped silently.
- **`linked_pane_id` goes stale** when the linked terminal is closed. A `RunInLinkedTerminal` to a dead pane id is a graceful no-op.
- **`OpenArtifact` path validation is lexical** (no symlink resolution). A symlink inside the workspace that points outside passes the check.
- **Python apps and WASM apps speak different key dialects.** Python/process apps (`mod.rs::handle_key`) send the raw egui debug name (`"ArrowUp"`, `"Escape"`); the Python SDK normalizes it. WASM apps (`src/host/wasm_pane.rs::handle_key`) have no normalization layer — the guest matches the wire string literally (`"up"`, `"down"`, `"space"`, `"enter"`, `"escape"`, lowercase letters). `canonical_key_name()` is the single source of truth. For WASM: forward both press AND release edges; collapse OS auto-repeat; test via `translate_key_event()` unit tests, not just direct `push_input`.
- **Escape is host-managed for ProcessApp.** Bare Escape is never forwarded to the Python app via `send_event`. Instead it stays in InputState for `poll_actions` → `ClosePane`, which checks `try_nav_back_focused()`: if `nav_stack` is non-empty, the host sends `NavBack` to the app; if empty, the pane closes. Apps that need layered Escape use `PushNav`/`PopNav` to manage the stack. Native Rust apps (file browser, text editor) handle Escape directly in their own `handle_key` and return `Consumed` to suppress `ClosePane`.
- **Subprocess env for processes spawned outside `ProcessApp::launch`.** Any code that launches a Plexi app subprocess must replicate `ProcessApp::launch` env setup: `ENV_WHITELIST` (`HOME/PATH/LANG/LC_ALL/TERM/USER/SHELL`), `PLEXI_*` passthrough, Python runtime resolution via `src/app/python_env.rs`, and `PYTHONPATH` → `config_dir/sdk` + bundle SDK path.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
