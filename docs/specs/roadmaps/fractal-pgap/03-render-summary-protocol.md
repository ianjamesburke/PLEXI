# 03 — Render Summary Protocol

**Goal:** Add lightweight status and preview data so parent depths can summarize children without rendering a full nested instance.

---

## Scope

- Add a `RenderMode` enum with `full` and `preview`.
- Add `mode` to `PlexiEvent::Render` with a default of `full`.
- Add a `DrawCommand::StatusSummary` variant.
- Define a serializable `PaneSummary` and `Health` type.
- Update the Python SDK after the Rust protocol lands.

---

## Relevant Files

- `src/app_protocol.rs`
- `src/process_app.rs`
- `sdk/` or bundled Python SDK locations
- `docs/specs/subsystems/app-infrastructure.md`
- `docs/specs/subsystems/fractal-pgap.md`

---

## Compatibility

- Existing apps must continue to deserialize `Render` as full renders.
- Apps that never emit `StatusSummary` remain valid.
- The host should keep rendering the previous full frame if a preview response is absent.

---

## Tests

- Serde test for `Render { mode: "preview" }`.
- Serde default test for old `Render` JSON with no mode.
- Serde test for `StatusSummary`.
- Process app test proving `StatusSummary` can be received without replacing the visual frame.

---

## Manual Verification

1. Run an example app updated to emit `StatusSummary`.
2. Trigger a preview render.
3. Confirm the host receives summary metadata and keeps normal rendering intact.

---

## Done When

- Parent depths have a protocol-native way to ask for cheap summaries.
- The summary protocol is additive and safe for older apps.
