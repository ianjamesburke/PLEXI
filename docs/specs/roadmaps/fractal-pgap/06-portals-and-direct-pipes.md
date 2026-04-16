# 06 — Portals And Direct Pipe Promotion

**Goal:** Make cross-depth views and focused-depth interaction efficient after embedded instances exist.

---

## Scope

- Add a portal pane type that subscribes to a depth address.
- Route a child depth's draw output to its native location and any active portals.
- Stop sending render events to off-screen portals.
- Add focused-depth direct pipe promotion.
- Send `Suspend` to intermediate depths and `Resume` when they become active again.

---

## Relevant Files

- `src/app_protocol.rs`
- `src/process_app.rs`
- `src/pane_ops.rs`
- `src/context.rs`
- `docs/specs/subsystems/fractal-pgap.md`
- `docs/specs/subsystems/typed-pipes.md`

---

## Dependencies

- [`01-process-lifecycle.md`](01-process-lifecycle.md)
- [`03-render-summary-protocol.md`](03-render-summary-protocol.md)
- [`04-embedded-instance-spike.md`](04-embedded-instance-spike.md)

---

## Tests

- Portal routing test: one child output reaches native pane and portal pane.
- Visibility test: hidden portal receives no render events.
- Direct pipe test: focused depth receives input without intermediary processing.
- Suspend/resume test: intermediate depths stop rendering while bypassed and resume later.

---

## Manual Verification

1. Open root depth with a portal showing a child depth.
2. Navigate into the child depth.
3. Confirm interaction remains responsive.
4. Return to root and confirm the portal updates again.
5. Check logs for suspend/resume transitions.

---

## Done When

- A portal can show another depth without navigating there.
- Focused-depth input does not depend on every intermediate render loop.
- Suspended depths consume no render CPU until resumed.
