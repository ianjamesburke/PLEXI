# 04 — Embedded Instance Spike

**Goal:** Prove whether `plexi --embedded` can render a Plexi instance through PGAP without an OS window.

---

## Scope

- Add a CLI flag skeleton for `--embedded`.
- Build the smallest possible egui + wgpu loop that does not depend on eframe windowing.
- Emit PGAP draw output or a documented intermediate representation.
- Accept basic input events over stdin.
- Register the embedded binary as a launchable app only after the spike proves output.

---

## Relevant Files

- `src/main.rs`
- `src/app_protocol.rs`
- `src/process_app.rs`
- `Cargo.toml`
- `docs/specs/subsystems/fractal-pgap.md`

---

## Research Notes

`eframe` is the windowing/application shell. Embedded rendering should bypass it and use `egui` plus `egui_wgpu` directly. `egui_wgpu::Renderer` renders egui output into a caller-provided wgpu render pass, which is the right layer for offscreen rendering experiments.

---

## Spike Rules

- Keep the spike isolated behind the `--embedded` flag.
- Do not rewrite the main eframe app.
- Prefer proving a single static frame over chasing full interaction immediately.
- If the renderer path is blocked, write the blocker into this spec and stop before broad refactors.

---

## Tests

- CLI test: `plexi --embedded --help` or equivalent argument parsing recognizes the flag.
- Protocol smoke test: start the binary with `--embedded`, send `Init` and `Render`, receive valid JSON lines.
- Timeout test: embedded mode exits cleanly on `Shutdown`.

---

## Manual Verification

1. Run `plexi --embedded` from a terminal.
2. Send a minimal `Init` and `Render` JSON line.
3. Confirm stdout emits valid PGAP JSON.
4. Launch it through Plexi as an app and confirm the pane receives output.

---

## Done When

- There is a clear yes/no answer on embedded rendering feasibility.
- If yes, there is a minimal interactive nested Plexi frame.
- If no, the blocker is specific enough to choose a different rendering strategy.
