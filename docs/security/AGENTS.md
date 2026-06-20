# docs/security — Agent Contract

**Read before editing anything under docs/security/:** this file, plus the root AGENTS.md.

## Scope

Security documentation for the Plexi host. Audit surfaces, trust boundaries, and the capability model.

## Reference

- [SECURITY_MODEL.md](SECURITY_MODEL.md) — the full security model: consent+audit for v1, capability gating, what is and isn't sandboxed, future WASM sandbox.
- [shell-execution-inventory.md](shell-execution-inventory.md) — every shell execution path classified by trust source.

## Rules

- **No new app-reachable `sh -c` path** without a capability gate and a denial test.
- When adding a shell execution path anywhere in the host, add it to the inventory in the same change.
- The security model doc is present-tense (how things work now). When the WASM sandbox ships, update it.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
