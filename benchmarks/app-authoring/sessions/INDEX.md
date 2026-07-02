# Session index

One row per captured session, newest first. Regenerate with `plexi-e2e index`
(or `just e2e-baseline` after a sweep). Every row is version-stamped so scores
stay comparable across time; `dry-run` rows are structural baselines captured
without a live host (no child agent ran).

Columns: session id, fixture, difficulty, mode, outcome, CLI/SDK versions,
wall-clock seconds, parent turns, lines of code.

| Session | Fixture | Diff | Mode | Outcome | CLI | SDK | Wall (s) | Turns | LOC |
|---------|---------|------|------|---------|-----|-----|----------|-------|-----|
| 20260702T185818Z_log-viewer_787e29 | log-viewer | medium | dry-run | plan-only | — | 0.1.16 | 0.002 | 1 | — |
| 20260702T185818Z_form_185b01 | form | medium | dry-run | plan-only | — | 0.1.16 | 0.002 | 1 | — |
| 20260702T185818Z_file-lister_42ad22 | file-lister | easy | dry-run | plan-only | — | 0.1.16 | 0.002 | 1 | — |
| 20260702T185818Z_counter_8f430e | counter | easy | dry-run | plan-only | — | 0.1.16 | 0.002 | 1 | — |
| 20260702T185818Z_assistant_de6865 | assistant | hard | dry-run | plan-only | — | 0.1.16 | 0.002 | 1 | — |
