# New-context workspace initialization latency — Alpha measurement

**Stint:** 1008
**Status:** active
**Evidence date:** 2026-10-25

## Finding and decision

**Decision: no current regression; no bounded fix stint is warranted.**

The host-side path is measurable and completes in tens of milliseconds. The
interactive shell is a separate child-process path and was measured at roughly
0.36–0.70 seconds from `/bin/zsh` launch to the first marker written to stdout.
The shell therefore explains the perceived wait better than the host's context
bookkeeping, pane spawn, first paint, or workspace save. This is a measurement
finding, not an optimization claim.

Workspace saving is debounced, but the eventual `save_workspace_now` call is
synchronous on the UI thread. `workspace_save_due` in `src/app/render.rs`
waits for `WORKSPACE_SAVE_DEBOUNCE_MS`, then `App::logic` calls
`save_workspace_now`; it is not an off-thread save.

## Machine and build identity

The measurement machine reported:

```text
uname -srm: Darwin 23.5.0 arm64
sw_vers: ProductName macOS; ProductVersion 14.5; BuildVersion 23F79
sysctl -n machdep.cpu.brand_string: Apple M1 Pro
```

The deterministic harness measurements were emitted while validating PR #2914
after its original head `54a4932f7e182dba8a46299f02df9acb21cb41ae`, based on
`feature/alpha-cleanup`. The live measurements used the installed Alpha
channel, `plexi-alpha`, running version `v0.4.0-alpha.9` at commit
`4d2474bb4938b39351d6cca95847151855697835`.

## Measurement method

The test `measure_new_context_initialization_latency_cold_and_warm` in
`src/testing/harness_tests.rs`:

- requires a real PTY and shell; it fails if the PTY cannot be created;
- measures `id_resolve`, `pane_spawn`, and `router_register` inside both
  `new_context_at_path` and `new_context_empty`;
- measures one first-paint `HostHarness` frame and synchronous workspace save
  for every warm sample;
- prints every warm sample and aggregate average/minimum/maximum values for
  all five stages across five empty-directory and five project-directory runs;
- launches `/bin/zsh -l -i -c 'printf ...'` five times and measures launch to
  first stdout marker, independently of the host harness.

The live Alpha observation used `plexi-alpha context new --path ...` for an
empty directory and a project directory, plus `plexi-alpha context new` for
the empty-context path. Each CLI wall-clock duration includes command IPC,
host handling, and the response; it is not substituted for the harness's
per-stage timings. The live Alpha log also records synchronous workspace-save
durations.

## Harness evidence

Cold samples printed by the test:

| Path | Host total | id_resolve | pane_spawn | router_register | first_paint | save |
|---|---:|---:|---:|---:|---:|---:|
| Empty directory | 26.589 ms | 0.218 ms | 6.295 ms | 20.075 ms | 46.355 ms | 11.713 ms |
| Existing project | 17.506 ms | 0.741 ms | 6.432 ms | 10.331 ms | 10.788 ms | not sampled in cold path |
| Empty context | 9.155 ms | 0.005 ms | 6.332 ms | 2.813 ms | 7.574 ms | not sampled in cold path |

Warm aggregate values are the exact values emitted by the test (`n=5` per
path):

| Path and stage | Average | Minimum | Maximum |
|---|---:|---:|---:|
| Empty — total | 19.642 ms | 15.693 ms | 28.280 ms |
| Empty — id_resolve | 0.029 ms | 0.016 ms | 0.042 ms |
| Empty — pane_spawn | 8.457 ms | 6.545 ms | 13.711 ms |
| Empty — router_register | 10.867 ms | 8.998 ms | 13.098 ms |
| Empty — first_paint | 9.413 ms | 6.950 ms | 16.174 ms |
| Empty — save | 18.080 ms | 15.994 ms | 23.785 ms |
| Project — total | 21.632 ms | 14.615 ms | 26.152 ms |
| Project — id_resolve | 0.173 ms | 0.128 ms | 0.270 ms |
| Project — pane_spawn | 10.203 ms | 5.857 ms | 16.012 ms |
| Project — router_register | 11.254 ms | 8.628 ms | 13.249 ms |
| Project — first_paint | 6.786 ms | 4.202 ms | 8.378 ms |
| Project — save | 20.310 ms | 15.433 ms | 35.188 ms |

The independently measured login-shell-to-first-output samples averaged
544.840 ms, with a 362.009–703.874 ms range.

## Live Alpha evidence

The installed Alpha host was exercised on the same machine. CLI wall-clock
durations were:

| Operation | Samples | Observed range |
|---|---:|---:|
| `context new --path` empty directory | 51.0, 43.5, 43.8 ms | 43.5–51.0 ms |
| `context new --path` project directory | 54.4, 49.0, 51.0 ms | 49.0–54.4 ms |
| `context new` empty context | 65.4, 67.1, 56.8 ms | 56.8–67.1 ms |

The live Alpha log recorded synchronous `workspace_save` durations of 19 ms,
24 ms, and 49 ms during the same exercise. That installed Alpha predates the
new per-stage trace labels, so these live values are intentionally reported as
CLI wall time and save-log observations, while the PR harness supplies the
stage breakdown.

## Reproduction

```bash
bash scripts/cargo-with-lease.sh cargo test --bin plexi measure_new_context_initialization_latency_cold_and_warm -- --nocapture < /dev/null
```

For live Alpha wall measurements, use the channel-specific shim so ambient
beta variables cannot redirect the command:

```bash
plexi-alpha context new --path /path/to/empty-directory --json
plexi-alpha context new --path /path/to/project-directory --json
plexi-alpha context new --json
```

Inspect the corresponding installed-channel log for save observations:

```bash
rg "workspace_save|new_context" ~/.plexi-alpha/plexi.log
```
