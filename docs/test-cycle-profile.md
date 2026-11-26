# Test cycle profile

Stint: 1069

Measured source: `3ed3e6e6ebbb38e9069a7af58f188c8b46606e42` (alpha, clean isolated worktree); 2026-11-26 UTC. This is a measurement, not a code or test change. Raw logs, both Cargo timing HTML reports, per-test records, and the measurement scripts are at `.stint/runs/1069-profile/` in the main checkout; that directory is local and git-ignored, so it is not included in the commit.

## Conditions and commands

Apple M1 Pro, 10 logical CPUs, 16 GiB RAM, macOS, rustc 1.97.0. Fresh worktree target, but an existing 10 GiB sccache was enabled for the first builds; it is not a cold compiler-cache measurement. The lease defaults to two full permits and sets `CARGO_BUILD_JOBS=3`. Measurements ran serially in this lane. Before the first build load averages were 5.81/5.69/7.08 with no active Cargo/rustc; load rose during builds. Before test compilation load was 10.05/12.24/10.34 with no other Cargo/rustc; before serial profiling it was 7.54/9.46/10.00; before the final release-timings retry it was 6.55/8.84/8.70. We did not continuously trace other permits, so background contention cannot be excluded. Lease queue times on measured runs were below 0.001 s. Space fell from about 32 GiB free to about 12 GiB. All times are single observations, not confidence intervals.

Commands were run in the measured worktree with stdin closed for tests; `SCRUB` below means `env -u PLEXI_CHANNEL -u PLEXI_CONTEXT_ROOT -u PLEXI_CONTEXT_ID -u PLEXI_CONTEXT_NAME -u PLEXI_SOCKET -u PLEXI_RUNNING -u PLEXI_PANE_ID`. `just` supplies `RUSTFLAGS=-D warnings`. The raw test command does not. `RUST_TEST_THREADS=4` comes from Cargo config unless overridden.

| Measurement | Command | Result |
|---|---|---|
| Ordinary release, fresh target, existing sccache | `/usr/bin/time -p just build` | 379.75 s wall; Cargo 379 s; PASS. |
| Test binary, fresh target, existing sccache | `/usr/bin/time -p bash scripts/cargo-with-lease.sh cargo test --bin plexi --no-run --timings` | 213.62 s wall; Cargo 213 s; PASS with linker unwind-table warning. |
| Full test command | `/usr/bin/time -p bash scripts/cargo-with-lease.sh SCRUB cargo test --bin plexi < /dev/null` | 585.86 s wall = 28.81 s Cargo rebuild + 556.77 s reported execution, plus overhead; **FAIL**, 5,457 pass, 1 fail, 69 ignored. |
| Focused initial build | `/usr/bin/time -p just test-host app::quit::tests` | 244.99 s wall, 39 passed, 4.56 s execution. Different warning flags required separate build. |
| One-file edit to focused result | append a comment to `src/app/quit.rs`; `/usr/bin/time -p just test-host app::quit::tests` | 40.43 s wall, 39 passed, 4.59 s execution; comment removed and tree verified clean. This is a warm incremental, comment-only edit, not a semantic-change guarantee. |
| Standalone scenes | `/usr/bin/time -p env RUSTC_WRAPPER= bash scripts/cargo-with-lease.sh SCRUB cargo test --bin plexi scenes::tests::scene_suite < /dev/null` | 256.67 s wall, 123 s compile, 132.58 s execution, 1 passed. |
| Release unit timings | `/usr/bin/time -p env RUSTC_WRAPPER= bash scripts/cargo-with-lease.sh cargo build --release --timings` | 547.64 s wall, 547 s Cargo, PASS. This differs from `just build` and is without sccache. |

Substitute the full `env -u ...` command for `SCRUB`, not the literal word. The exact complete invocations and lease timestamps are in the numbered logs. Raw `cargo build --release --timings` with `RUSTC_WRAPPER=sccache` failed twice with `Operation not permitted` writing a dependency file; a direct write probe succeeded, and the retry without sccache passed. The cause of the sccache failure was not established. Its failed timing HTML is not used here. The test linker warned about an oversized `__eh_frame` compact-unwind table.

The parallel full-suite failure was `overlays::command_palette::tests::single_line_user_command_runs_in_focused_idle_shell_and_splits_when_busy_agent_view_or_split_true`: expected 2 panes, observed 3. The serial full test execution subsequently passed, so this run does not prove a stable regression or a fix. It does not satisfy a requirement that the ordinary full suite pass.

The audit-plan full gate was also attempted with `/usr/bin/time -p env RUSTC_WRAPPER= SCRUB bash scripts/release-host-gate.sh < /dev/null`: 946.59 s to failure. Its release stage passed in 212.16 s, all-target compile passed in 132.83 s, and host execution ran 566.53 s with 5,457 passed, 1 failed and 69 ignored; Clippy was not reached. The `cli_closed_stdout_uses_sigpipe` failure detected different embedded source identities. The report was created and staged during the gate, after its release build, so this is an invalidated attempt caused by the measurement workflow, not evidence of a product defect. Rerun on a stable committed tree; its raw result belongs beside this log. The audit plan's PR-channel CLI hyperfine benchmark was not run: this measurement does not create or install a PR channel.

## Runtime distribution

The single-thread pass ran the original compiled binary under the lease with `--test-threads=1` and timestamped the start and completion of each test using `.stint/runs/1069-profile/profile-tests.py`; command: `/usr/bin/time -p bash scripts/cargo-with-lease.sh python3 /Users/ianburke/Documents/GitHub/PLEXI/.stint/runs/1069-profile/profile-tests.py target/debug/deps/plexi-431e9dde5a41103f /Users/ianburke/Documents/GitHub/PLEXI/.stint/runs/1069-profile/06-serial-tests.jsonl`. It passed in 1,833.00 s. Direct child-process output disrupted the parser for 226 tests; `.stint/runs/1069-profile/recover-tests.py` reran the missing non-ignored tests individually under the lease, using `target/debug/deps/plexi-431e9dde5a41103f --exact <name>`, measuring process wall time. Command: `/usr/bin/time -p bash scripts/cargo-with-lease.sh python3 /Users/ianburke/Documents/GitHub/PLEXI/.stint/runs/1069-profile/recover-tests.py`. Names were reconciled against the full suite: 5,458 non-ignored tests observed, 69 ignored, no missing names. Recovery includes process startup; two recovered observations above 1 s are within 0.15 s of the threshold, so 324 above 1 s is a process-inclusive upper observation, not an exact in-process count. Above 5 s: 41; above 30 s: 5. None in these measured sets was named `perf_gate_`. Four ignored tests are named `perf_gate_`, and one non-ignored `perf_gate_` ran below the thresholds. Ignored tests were not timed, so their distribution is unknown.

Individual serial durations are not the ordinary four-thread critical path, and cannot simply be subtracted from the parallel suite time. The scene suite is one libtest test containing multiple scenes; its number is not a per-scene ranking.

| Test | Seconds, serial |
|---|---:|
| `scenes::tests::scene_suite` | 126.22 |
| `cli::front::tests::ownership_equals_clap_at_every_insertion_position` | 117.01 |
| `cli::front::tests::every_command_accepts_help` | 43.78 |
| `app::tests::command_palette_oracle_tests::command_palette_production_path_matches_sequential_oracle` | 42.44 |
| `cli::plugin_check::layout_finding_tests::core_pack_plugins_have_only_known_layout_findings` | 37.76 |
| `app::plugin_migration::tests::plugin_rename_migrates_with_launchd_path` | 19.39 |
| `ui_tests::tests::pane_chrome::screenshot_pane_chrome_state_matrix` | 16.85 |
| `cli::front::tests::help_matches_clap_native_rendering` | 16.79 |
| `cli::host::tests::a_host_not_seen_to_end_is_not_reported_terminated` | 16.32 |
| `host::plugin_admission::tests::an_embedded_seed_without_bytecode_opens_every_maintained_plugin_without_asking` | 16.05 |
| `host::plugin_admission::tests::a_packaged_profile_opens_every_maintained_plugin_without_asking` | 16.05 |
| `cli::host::tests::a_host_that_never_replies_is_terminated_and_reported` | 15.06 |
| `cli::plugin_check::gauntlet_probe_tests::scaffolded_python_plugin_has_no_layout_or_probe_findings` | 14.49 |
| `ui_tests::tests::titlebar::titlebar_title_never_overlaps_toolbar_at_any_scale_or_width` | 14.44 |
| `ui::modal_kit_guard_tests::every_dialog_uses_the_dialog_kit` | 13.29 |
| `app::startup_error::tests::no_text_in_the_window_has_backticks_or_internal_wording` | 12.86 |
| `testing::permission_gate_tests::permission_gate_real_chess_all_ingresses` | 12.74 |
| `broker::custody_guard_tests::only_host_start_installs_custody` | 12.62 |
| `app::plugin_migration::tests::plugin_rename_every_manifest_fault_class_has_its_own_fix` | 11.58 |
| `cli::registry_watch::registry_watch_tests::a_hung_cli_is_stopped_at_the_inspection_limit` | 10.02 |

## Build units

Cargo timing reports measure overlapping units, not additive time savings. The test report's top units were host test binary 94.5 s, `objc2-app-kit` 25.2 s, `cranelift-codegen` 13.9 s, `objc2-foundation` 13.0 s, `wasmtime` 12.9 s, and `wasmtime-wasi` 11.1 s. The release report without sccache showed host binary 196.1 s, `cranelift-codegen` 68.9 s, `wasmtime` 55.8 s, `objc2-app-kit` 27.2 s, `egui` 26.0 s and `wgpu-core` 25.9 s. Reports include all units and concurrency diagrams.

## Ranked recommendations

The savings below are hypotheses per affected run, not measured improvements. Compare repeated before/after runs on the same machine, revision, flags, cache state, workload and contention; verify the full required gate and test counts remain sound.

1. **Investigate the two longest tests first:** scene suite (126 s serial; 133 s standalone) and CLI ownership coverage (117 s serial). Avoid blindly removing coverage; profile individual scenes and redundant CLI cases, move suitable exhaustive cases to a scheduled tier only if the per-push gate retains meaningful coverage, or replace fixed sleeps with event-driven waits where semantics allow. Tentative 0–2 min per gate, depending on parallel critical path; verify matched tests, failure detection, and repeated parallel wall time. The parallel-only palette failure needs its own investigation, not removal.
2. **Measure `debug = 0` against current `[profile.test] debug = "line-tables-only"`:** tentative 0–1 min per rebuilt host test binary. Compare identical fresh and warm edits, compile timings, binary size and failure backtraces; keep line tables if debugging cost outweighs savings.
3. **Protocol extraction, then decide editor extraction:** a protocol-only focused test could avoid rebuilding the monolithic host, bounded by the observed 40.43 s edit-to-result and 94.5 s host test unit; tentative 0–0.7 min on a warm focused loop, not an actual measured crate build. Include the six duplicated wire types, preserve schema and protocol parity, compare the exact focused command after extraction. Editor savings cannot be inferred from Cargo unit timings because it is not a separate unit; measure the actual extracted crate and its consumers. Extract protocol first only after 0.5.0 security work, in stint 1319; do not extract here.
4. **Tune codegen units experimentally:** tentative 0–1 min per rebuilt host, direction unknown. Sweep representative values with identical jobs, flags, cache and clean/warm states; verify build, binary performance and full tests. Parallelism and linking may offset gains.
5. **CI caching:** the current macOS test job already uses `Swatinem/rust-cache` through `.github/actions/plexi-ci-env`; the old proposal to add it is stale, and this workflow currently has no Linux job. Tentative 0 min for adding the existing cache; measure restore/save time, hit rates and fresh versus restored build stages on the actual CI workflow before changing cache policy.

Splitting Rust test modules (now stint 1309) improves navigation but by itself does not reduce build time: modules remain within the same crate compilation unit. It is not a speed recommendation.
