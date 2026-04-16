# 05 — Capability Containers

**Goal:** Give nested instances explicit, attenuated capabilities so recursive Plexi can safely host agents.

---

## Scope

- Add `CapabilityManifest` to `Init`.
- Extend permission enforcement beyond `RunInTerminal`, `Cd`, and `Notify`.
- Gate `SpawnApp`, `PipeWrite`, filesystem paths, secrets, network declarations, hardware declarations, and TTL.
- Add a root-side secret broker path for nested instances.
- Enforce capability attenuation for child spawning.

## Current State

- Added a wire-only `CapabilityManifest` type on `PlexiEvent::Init` with serde coverage.
- Still blocked: no host-side enforcement path consumes the manifest yet, so nested instances still run on the existing `AppPermissions`/manifest defaults.

## Next Patch Points

- `src/process_app.rs` — thread `capability_manifest` through the real nested-instance spawn path instead of leaving it `None`.
- `src/app_permissions.rs` — derive runtime allow/deny checks from the manifest and enforce attenuation when spawning children.
- `src/app_api.rs` — gate `SecretGet` and `SecretStore` against the new manifest allowlists.
- `src/pane_ops.rs` — enforce spawn and pipe gating at the host boundary before child launch / routing.
- `src/app_registry.rs` — decide whether manifest declarations map directly into the new capability model or stay as the current app-launch permissions layer.

---

## Relevant Files

- `src/app_protocol.rs`
- `src/app_permissions.rs`
- `src/app_api.rs`
- `src/process_app.rs`
- `src/pane_ops.rs`
- `docs/specs/releases/plexi-v2.0.md`
- `docs/specs/proposals/secrets-manager.md`
- `docs/specs/subsystems/fractal-pgap.md`

---

## Compatibility

- Existing apps without a capability manifest receive current sandbox defaults.
- Built-in apps keep explicit built-in trust behavior.
- Unknown capability fields must be ignored with a warning, not crash older hosts.

---

## Tests

- Manifest serde test.
- Path allow/deny tests for read and write scopes.
- Spawn attenuation test: a child cannot grant a grandchild a capability the child lacks.
- Secret request test: allowed secret resolves, denied secret returns no value and logs a denial.
- TTL test: expired nested instance is shut down and then killed if needed.

---

## Manual Verification

1. Launch a nested test app with read access to one fixture directory.
2. Confirm it can read inside the allowlist.
3. Confirm it cannot read outside the allowlist.
4. Request an undeclared secret and confirm the request is denied visibly or in logs.

---

## Done When

- Nested instances have no ambient authority by default.
- Capabilities can only narrow as depth increases.
- Secret access is mediated by root.
