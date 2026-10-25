# New-Context Workspace Initialization Latency — Alpha Measurement Findings

**Stint:** 1008 (this measurement and decision)
**Status:** active
**Date of evidence:** 2026-10-25, Alpha branch point (includes stint 0993)
**Target:** macOS / aarch64 (Apple Silicon)

An empirical measurement of new-context workspace initialization latency on Alpha, investigating the tentative report from Quick Note `note-20261024-201849.662.md` regarding new-context creation performance.

## The Finding and Explicit Decision

**Decision:** **No current regression.** No separate fix stint or speculative optimization is warranted.

Host workspace initialization is responsive and well within the frame budget across all stages:
- **Host context creation:** 8 ms to 20 ms total blocking time on the host.
- **First UI frame paint:** 7.5 ms to 29.4 ms to build and paint the new context window.
- **Combined input-to-first-usable-pane:** 15 ms to 45 ms (1 to 3 frames at 60 Hz).
- **Workspace disk save:** ~12 ms, executed asynchronously/debounced off the critical interaction path.

The perceived latency reported in tentative notes is driven by **external shell startup** (interactive `zsh` loading `.zshrc` environments, plugin managers, and toolchains), which operates in the spawned child process after the host has already presented the pane. Host UI blocking is negligible. Furthermore, stint 0993 recently addressed visual hesitation during that shell startup window by painting stable placeholder chrome before shell integration reports a title.

## Methodology

### Machine and Build Environment
- **Platform:** macOS 24.x / Darwin (Apple Silicon aarch64)
- **Host Channel:** Alpha (`plexi-alpha`)
- **Measurement Harness:** `HostHarness` deterministic headless host test suite

### Permanent Host Instrumentation
Permanent `info`-level traces instrument the stage boundaries in `src/pane_ops/workspace.rs`:
- `seed_window_root_pane`: measures PTY allocation and child process spawn time (`seed_window_root_pane: spawned root pane ... in ...`).
- `new_context_empty`: measures identity resolution, pane spawn, router registration, and total context creation time (`new_context_stage_timing: context_id=... kind=empty ...`).
- `new_context_at_path`: measures anchor configuration detection, identity resolution, pane spawn, router registration, and total time (`new_context_stage_timing: context_id=... kind=at_path ...`).
- `save_workspace_now`: measures workspace serialization and file persistence (`save_workspace_now: saved workspace in ...`).

### Deterministic Test Verification
The test `measure_new_context_initialization_latency_cold_and_warm` in `src/testing/harness_tests.rs` validates:
1. Cold new context creation at an empty directory.
2. Cold new context creation at an existing project directory (parsing `.plexi/workspace.toml` anchor defaults).
3. Cold new standalone empty context creation at home directory.
4. Repeated warm context creation across multiple iterations.
5. First frame paint duration following each context transition.
6. Non-vacuity assertions ensuring all timing records are emitted, match the active context identity, and reflect actual stage execution.

## Measured Stage Breakdown

All timings are recorded on Alpha on Apple Silicon hardware:

| Stage | Operation | Cold Empty | Cold Project | Cold Home Empty | Warm Avg (n=5) |
|---|---|---|---|---|---|
| **Stage 1: Identity & Config** | Anchor detection, auto-name or `workspace.toml` parse, `PaneContextEnv` setup | 0.06 ms | 0.33 ms | 0.003 ms | 0.05 - 0.25 ms |
| **Stage 2: Pane Spawn** | `create_single_pane_tree` -> `TerminalPane::new` (PTY allocation & fork/exec) | 4.47 ms | 5.51 ms | 5.28 ms | 4.5 - 6.0 ms |
| **Stage 3: Router Register** | `ensure_context_state_ignore`, router insertion, window activation, minimap update | 9.01 ms | 9.15 ms | 2.77 ms | 8.0 - 15.0 ms |
| **Total Context Creation** | Host-side synchronous creation pass | **13.55 ms** | **14.99 ms** | **8.05 ms** | **18.35 ms (empty) / 20.60 ms (project)** |
| **Stage 4: First Paint** | Host logic + UI layout pass painting the new context window | 29.44 ms | 10.65 ms | 7.51 ms | 6.5 - 12.0 ms |
| **Stage 5: Workspace Save** | `save_workspace_now` disk serialization (debounced) | 12.23 ms | — | — | 10.0 - 14.0 ms |

### Stage Observations
1. **Config & Identity Resolution (`id_resolve`):** Reading anchor metadata from `.plexi/workspace.toml` takes ~0.33 ms, and empty directory name resolution takes ~0.06 ms. Neither presents any bottleneck.
2. **Pane Spawn (`pane_spawn`):** Allocating the pseudoterminal via `portable_pty` and spawning the shell process takes ~4.5 ms to 5.5 ms on macOS. This is standard kernel process-spawn overhead.
3. **Router Registration (`router_register`):** Context bookkeeping, gitignore verification, and transition effects take ~2.8 ms to 9.1 ms.
4. **First Paint (`first_paint`):** The initial frame rendering the new terminal tile takes ~7.5 ms to 29.4 ms depending on cache state. Subsequent frames render at normal 60 Hz frame intervals (16.6 ms budget).
5. **Disk Save (`save_workspace_now`):** Serializing windows, contexts, and pane sessions to `workspaces/default.json` takes ~12 ms and is debounced so it does not block user input.

## Host vs. Shell Separation

The total host duration from user command to the first rendered frame of the new context is **15 ms to 45 ms**.

In contrast, interactive login shells on macOS (`/bin/zsh`) typically spend hundreds of milliseconds evaluating startup files (`/etc/zshenv`, `~/.zshenv`, `~/.zprofile`, `~/.zshrc`, `~/.zlogin`) before presenting a shell prompt. Common developer configurations including NVM, Pyenv, Homebrew shellenv, and Starship prompts add 200 ms to 1500 ms of child-process initialization.

Because the host creates the context and paints the initial pane within 45 ms, any delay before command readiness is attributed to user-shell execution rather than host UI latency.

## How to Reproduce

To run the deterministic measurement test:
```bash
bash scripts/cargo-with-lease.sh cargo test --bin plexi measure_new_context_initialization_latency_cold_and_warm -- --nocapture
```

To monitor live stage timings in a running Alpha host (`~/.plexi-alpha/plexi.log`):
```bash
grep -E "(new_context_stage_timing|seed_window_root_pane|save_workspace_now)" ~/.plexi-alpha/plexi.log
```
