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

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
