# Testing

How Plexi is tested, which layer owns what, and how to add coverage.

## The Rule

**If you'd assert on observable state (pane tree, app UI, pixels) → write a TOML scene. If you'd assert on a return value or internal invariant → write a Rust test.** Two layers, no overlap.

## The Gate

Select checks from the actual changed behavior and failure risk, not the stint's
S/M/L size. A change can belong to several rows; apply every relevant row.
Reassess the plan when the diff changes. The path selector proposes Rust filters,
not a complete test plan. Record the exact head, changed paths, chosen checks,
matched test counts, visual evidence and reasons for inapplicable checks. A
zero-match Rust run is a gap, not a pass. A markdown workflow or policy change
can change agent behavior even when it is not compiled; verify the affected
workflow and escalation rules rather than treating its extension as a risk tier.

| Change and example | Feature / batch checks | Scene and installed evidence |
|---|---|---|
| Unembedded documentation only, such as a prose correction | Review the text and run relevant documentation checks. `scripts/scoped-tests.sh --gate` identifies the path-specific local build/test/Clippy exemption; an embedded or test-synchronized markdown file is code. Aggregate evidence and installation requirements still apply. | No scene or screenshot unless behavior or appearance changes. Outside aggregate mode, follow the existing testing-skill install-skip rules. |
| Internal logic, such as a pure calculation | Focused invariant and failure-path tests, plus release build, strict Clippy and checks for other affected surfaces. | No scene if no observable state changes. Test observable state with a relevant scene where applicable. Independent installed affected-behavior validation applies except for the existing non-aggregate install-skip rules. |
| Visible UI, such as a spacing or overlay change | Relevant Rust/app tests, release build, strict Clippy and a scene exercising the affected state. Render realistic content and inspect its screenshot. | Run the relevant scene(s) individually; escalate to the suite when shared changes can affect it broadly. A headless image is not installed-host proof. Installed validation follows the existing rules below. |
| Runtime, packaging, channel, PTY or installed interaction, even a one-line fix | Focused positive and failure-path tests, release build, strict Clippy and any affected script/SDK checks. | Independently install the exact head and exercise affected behavior at that boundary. Add a live scene or inspected host image when needed to prove the behavior or appearance. A small stint is not an exemption. |

### Qualification through independent validation

The task author records the planned risk and evidence. The implementer reassesses
it against the actual diff and records exact-head checks, omissions and reasons;
the independent tester verifies the affected behavior and Done-When. Examples
below are feature-level; aggregate and final gates remain separate.

| Example | Author and implementer | Independent validation and PR evidence |
|---|---|---|
| Unembedded prose correction | Plan text review and relevant docs checks; prove the local exemption with `--gate`. | Review the exact text at head; record no scene or screenshot and the install-skip reason. |
| Pure non-UI calculation | Plan and run focused success and failure tests, build and lint. | Independently exercise the invariant at head; use the testing skill's skip decision if full behavior is covered and no installed boundary is affected. Record no visual evidence with reason. |
| Visible overlay spacing | Plan and run relevant scene, inspect realistic rendered pixels, and publish the inspected screenshot after PR creation. | Check the image link, origin, head and affected behavior; install and drive when required. A headless image is not live-host proof. |
| Small packaging or PTY guard | Plan failure and success cases, release build, lint and affected script tests, regardless of stint size. | Install the exact PR head and independently exercise the affected boundary; scene or screenshot only when needed to prove visible behavior. |

A repository-only script whose behavior does not execute in an installed app
may qualify for the testing skill's non-aggregate install skip if the entire
changed behavior is covered and independently exercised at the exact source head.
A script that changes an installed boundary is not repository-only for this
purpose. State the boundary and any skip explicitly; do not describe source
checks as installed behavior. Aggregate records still require installed heads.

Before a feature or batch ready push (except the draft stage below), `just build` compiles the installable release
binary with warnings denied, `scripts/scoped-tests.sh` proposes relevant Rust
filters, and `just clippy` is the lint gate. Run
relevant scene, script, SDK and specialized checks too. The sole local
build/test/Clippy exemption is the selector's proven unembedded markdown-only
case above; the aggregate record's requirements are separate. Bare `cargo build`
does not enforce the release warning gate.

Independent validation installs the exact head and exercises the affected behavior. A scoped worker pass alone is insufficient; preserve the installed-head match, independent vendor, PASS verdict, non-deferred checks, and truthful evidence. Apply the testing skill's existing install-skip rules outside aggregate mode. Aggregate batches still require an installed-head record.

The final combined candidate requires a full `cargo test --bin plexi` with stdin closed and the Plexi environment scrubbed, plus release build, Clippy, independent combined installed validation, and the final manifest gate. Scoped-only evidence must fail that final gate. A subsequent change invalidates the candidate's evidence. Hosted Rust verification remains deferred to nightly after Alpha landing; record it as deferred, never passed, and retain other PR check gates.

Run the selector from the worktree against the actual PR base (`SCOPED_TEST_BASE=origin/feature/alpha-cleanup` for aggregate batches). It includes committed, staged, unstaged and untracked paths. Pass the filters together after libtest's `--`:

```bash
filters=()
while IFS= read -r filter; do
    if [[ "$filter" == :: ]]; then
        echo "Select and document relevant filters for this batch; full suite is final-candidate only." >&2
        exit 1
    fi
    filters+=("$filter")
done < <(bash scripts/scoped-tests.sh)
if (( ${#filters[@]} )); then
    just test-host "${filters[@]}"
fi
```

`just test-host <filter>...` and `just scene` share the justfile warning flags and test profile, avoiding rebuilds caused solely by mismatched warning flags; other changes or build-script invalidations can still trigger recompilation. The installer invokes its pretests under `just` with those same flags; raw Cargo can use different flags and trigger a rebuild.

No filters means no host Rust test is selected; run the affected script, SDK, documentation, or other checks instead. Markdown is code when `include_str!` reaches it: use the path-specific `scripts/scoped-tests.sh --gate` decision. A broad `::` fallback requires a reviewed relevant selection, not an automatic full-suite run. State why the selected tests cover the diff.

For observable changes, name the affected scene files and run each with `just scene <file>`; a changed `tests/scenes/*.toml` is itself a required selection. If no scene covers the behavior, add one as required by the testing contract. The selector does not infer the relevant scenes from UI paths, so an empty scene selection must be justified rather than silently treated as coverage. Shared scene-runner changes select `scene_suite`; expand to the suite for other cross-cutting changes when individual scenes cannot establish coverage. Preserve screenshot steps and inspect images for visual changes; state-only runs are not visual proof. The final full host suite includes `scene_suite`.

### Draft PR and candidate qualification

After focused checks and source review, commit and push a clean feature head and
open a **draft** PR to acquire its channel number. This is the sole exception to
the release/Clippy-before-push rule; draft status is not qualification or `pr:done`.
Run `just pr-candidate <PR>` at the final head. It resolves that exact source,
runs installer pretests, builds the release with compiled `pr-<PR>` identity and
warnings denied, runs strict Clippy, and records the artifact digest. Run all
other applicable scoped, script, SDK, docs and visual checks before ready status.
A changed head/base/configuration requires reassessment and fresh matching evidence.
Only then mark the PR ready and hand it to the independent tester.

`just pr-candidate` and `just pr-install` use `scripts/build-cache.py`: two
persistent, exclusively locked Cargo slots per repository under
`~/.cache/plexi-build`. The lock covers the entire candidate/install operation;
the global Cargo memory lease remains active. Slots survive feature cleanup.
Before reuse, an owned slot exceeding the default retained-data threshold is
reset; `PLEXI_BUILD_CACHE_MAX_BYTES` configures that threshold. This bounds retained
slots, not peak temporary build space. `PLEXI_BUILD_CACHE_ROOT` relocates the pool.
Never copy or share another lane's mutable target. For additional cache-backed
checks use `python3 scripts/build-cache.py pr-<PR> just <check>`.

The tester owns installation. `build-evidence.py` permits pretest reuse only for
successful matching clean-source execution, including exact head/base/selection,
source path, toolchain, Cargo configuration, relevant environment and receipt
schema, with an intact execution log. Missing, dirty or mismatched evidence runs
the checks. A candidate's verified head/channel/configuration and binary digest
permit installation without another host release compilation. SDK selection,
installed SDK digest verification and independent affected-behavior checks remain
separate. An artifact stamp alone never proves test success.

Each slot retains queue/acquire/finish events and Cargo fresh/compiled artifact
counts in `telemetry.jsonl`. These distinguish waiting from execution; a warm
cache is not a promise that changed source paths or build scripts avoid rebuilding.

### Aggregate evidence

`local.head`, passing `build`/`test`/`clippy`, and `local.test_scope` identify the current validation. Use `"scoped"` for a batch selection, and `"full"` only when the full host suite actually passed at that head. Historical full batch evidence remains valid; it never creates a new batch requirement.

For scoped evidence, provide:

- `local.test_base`: the record's `base_head`.
- `local.test_paths`: the exact changed paths from `git diff --name-only --no-renames -z <base_head>...<head>`. Policy checks this against Git during live gates, merge recovery, and prerequisite verification.
- `local.test_evidence`: passing entries with `command`, `covers` (literal changed paths), and `passed: true`. Their coverage must equal `test_paths`. Rust commands include explicit libtest filters and a positive integer `matched`; unfiltered, broad, or zero-match Rust evidence is rejected. Non-Rust checks identify their actual command and covered paths without inventing Rust test counts.
- `validation.affected_behavior`: what the independent tester verified at the matching installed head, alongside the existing head/install/vendor/PASS evidence.

The final phase requires `local.test_scope = "full"` regardless of hosted CI policy. Preserved batch prerequisite records use batch scope rules, so a properly validated scoped integration can satisfy a later batch dependency without marking its task done.

### Installer pretests

`just pr-install <PR>` defaults to `scoped`; `just pr-install <PR> scoped` is the explicit batch path. It resolves the PR's head and base, selects source SDK smoke by affected boundary and runs selected host tests from that exact worktree, and records scope/base in installation provenance. It never falls back to the full suite for a batch.

If automatic selection returns `::`, set `PLEXI_PR_TEST_FILTERS` to reviewed newline-separated relevant filters and record the choice in the batch evidence. Empty overrides and zero-match selections fail. Audited native toolbar-only changes can skip source SDK smoke; other shared UI/context paths remain conservative; SDK, protocol, Python runtime/app, build and unknown boundaries retain it. `PLEXI_PR_REQUIRE_SDK=1` forces it. Full candidates always run it. Selection and reasons are printed. The installed SDK digest guard always runs independently. Source SDK smoke uses uv; the separate native runtime download is unnecessary for these tests and the WASI package. The legacy fetch recipe remains available for explicit compatibility work. For a script/documentation-only diff, automatic selection runs no host Rust tests; the tester separately verifies the affected scripts at the installed source head.

`just pr-install <PR> full` is supported only for the configured aggregate branch targeting its final base, when collecting final combined full-suite evidence. Do not use it for a feature or batch. Independent testing verifies the installed affected behavior rather than rerunning the worker's unrelated tests.

## Wall-clock budgets

Default `cargo test --bin plexi` proves correctness, not idle-machine speed.
Any test that waits for a real subprocess, filesystem watcher, GPU readback, or
host response must route its bound through `crate::testing::load_aware_timeout`.
It scales the one-minute load average against available CPUs and caps the
multiplier, so fleet contention does not look like a product failure while a
genuine hang remains bounded. This does not apply to a deliberately tiny timeout
that is an input to a pure unit test; assert its returned error, not how quickly
the scheduler ran it. Tests that assert a frame-time or throughput ceiling are
performance gates: name them `perf_gate_*`, mark them
`#[ignore = "perf-gate: ..."]`, and run only those gates on a quiet machine:

```bash
cargo test --bin plexi perf_gate_ -- --ignored
```

Keep a non-ignored correctness counterpart that proves the operation eventually
completes.

| Layer | Tool | Lives in | Runs |
|---|---|---|---|
| Pure logic | `#[test]` unit tests | next to the code | `cargo test --bin plexi` |
| Host logic (AppRequest/HostEffect dispatch) | `HostHarness` (`src/testing/mod.rs`) | `src/*/tests/` | `cargo test --bin plexi` |
| Observable behavior — host UI, host apps, portals, real PGAP apps | **TOML scenes** (`tests/scenes/*.toml`) | `tests/scenes/` | targeted `just scene`; `scene_suite` for broad/final coverage |

`PlexiUiHarness` (`src/ui_tests.rs`) is the engine under scenes: PlexiApp inside egui_kittest with fully headless wgpu Metal rendering (no display needed). Direct PlexiUiHarness tests in `src/ui_tests.rs` are for egui-input edge cases scenes can't express (e.g. rename-commit-on-Enter inside a draw pass).

## Scenes

A scene file is a list of steps: setup, actions, structured assertions, optional screenshots. **The scene file IS the test** — `scene_suite` globs `tests/scenes/*.toml`, so adding a file adds a regression test. Runner: `src/scenes.rs`.

Use `clear_seed = true` as the first step only when a scene needs an empty
initial topology. Headless runs require an already-empty harness; live runs
close only the sole seed terminal of a fresh runner-owned host and refuse
attached hosts or scenes that have already opened panes.

```toml
size = [1280.0, 800.0]      # optional, default 1280x800
sizes = [[520.0, 400.0], [1280.0, 800.0]]
                            # optional responsive matrix, mutually exclusive
                            # with `size`: the whole step list runs once per
                            # size on a fresh harness, and every `shot` stem
                            # gains a `-WxH` suffix so the passes don't
                            # overwrite each other. One file, one report, one
                            # pass/fail — never clone a scene per breakpoint.
suite = false               # optional, default true. false = excluded from
                            # scene_suite (use for scenes spawning real app
                            # processes); run them via `just scene`.

[[steps]]
open = { kind = "process", path = "apps/dev/balls", as = "balls", args = ["--state", '{"balls": 3}'] }

[[steps]]
wait_app_frame = { target = "balls", timeout_s = 15.0 }

[[steps]]
assert = { target = "balls", pane_count = 1, lifecycle = "running", tree_contains = "balls" }

[[steps]]
shot = "balls.png"                          # optional; written to the out dir
```

### Verbs

| Verb | Effect |
|---|---|
| `open = { kind = "process", path = "<dir>", as = "<handle>", args = [...] }` | Spawn a real PGAP app process from an app dir. Relative paths resolve from the repo root. Args surface as `ctx.args`. |
| `open = { kind = "wasm", path = "<file.wasm>", as = "<handle>", args = [...] }` | Open a reviewed raw WASM component. |
| `open = { kind = "builtin", id = "<id>", as = "<handle>", cwd = "<dir>" }` | Open a compiled-in host app by id. `cwd` is optional and defaults to a fresh harness-owned directory. |
| `text = { target = "<handle>", value = "hello" }` | Focus a pane and insert text through egui's normal text-input path. The report records the character count, not the text. |
| `paste = { target = "<handle>", value = "first\nsecond" }` | Focus an app pane and deliver literal text as one paste event (`plexi pane send --paste` live). Preserves newlines and literal backslashes without changing the system clipboard. |
| `key = { target = "<handle>", value = "enter" }` | Focus a pane and press a key combo (`cmd`/`ctrl`/`alt`/`shift` plus a key name). Use `target = "host"` for a whole-host shortcut. |
| `drop_file = { target = "<handle>", value = "<path-or-url>" }` | Deliver a file or image URL through the same pane drop handler used by native host drops. Rejection fails the scene. |
| `drag = { target = "<handle>", from = [x, y], to = [x, y], steps = 8, button = "left" }` | Pointer drag through the production input path: press, `steps` intermediate moves, release — one frame each (`AppRequest::DragPane`; `plexi pane drag` live). Endpoints are pane-pixel coordinates, or semantic node ids via `from_node`/`to_node` (the drag targets the node's rendered bounds center; nodes without recorded bounds fail loudly). |
| `focus = "<handle>"` | Focus an opened pane through the production pane-navigation path (`plexi pane focus` live). |
| `close = "<handle>"` | Close an opened pane through the production pane lifecycle path (`plexi pane close` live). |
| `sidebar = true` | Show/hide the host sidebar. |
| `switch_context = 0` | Switch to the context at this router index. |
| `push_to_subcontext = "Name"` | Push the focused pane into a new subcontext (creates a portal). |
| `wait_app_frame = { target = "<handle>", timeout_s = N }` | Wait for a process app's first committed frame. Fails with the app's stderr on crash or timeout. |
| `run_steps = N` | Advance N harness frames. |
| `assert = { ... }` | Structured assertions — see below. |
| `expect = { target = "<handle>", after_key = "left", node_changes = "paddle", timeout_s = 2.0 }` | Deliver input normally and poll semantic state. On input timeout the stable failure code is `input_no_effect`; without input it is a normal eventual assertion. |
| `expect = { target = "<handle>", event_stream = "probe.tick", event_payload_contains = '"count":1', timeout_s = 5.0 }` | Event-bus expectation: bounded-poll the recorded event bus (the same `AppTimeline` production `EmitEvent` records into) for an emitted event on the stream, optionally requiring a payload substring. Composable with the other expect predicates. Headless-only — the live backend returns `unsupported_live_verb` (no sanctioned live event-query seam). Rust-level counterpart: `HostHarness::wait_for_app_event`. |
| `assert_label = { target = "<handle>", label = "Settings" }` | Focus a pane and require an exact semantic label inside that pane's rendered rectangle. Use `target = "host"` for the whole accessibility tree. |
| `shot = "name.png"` | Screenshot to the out dir: headless framebuffer in the headless backend; host pixels in the live backend. |

Every `open` step binds one unique symbolic handle. Later pane steps reject missing handles, duplicate handles fail before an app opens, and `host` is reserved for explicit whole-host input. New verbs require a scene that needs them. Assertions are structured keys, never expression strings. `tests/scenes/assistant-settings.toml` exercises `open`, `text`, pane-targeted `key`, and `assert_label` against the native Assistant without starting an AI turn.

### Assertions

`pane_count`, `window_count`, `context_count`, `portal_count` (across all windows), and `sidebar` assert host state. `context_pane_count` counts all owned panes across the target pane's context pages on both backends. `context_pane_count`, `exists`, `focused`, `lifecycle`, and `tree_contains` require `target = "<handle>"`; `fit = "fill"|"contain"` and `aspect = [width, height]` inspect committed canvas semantic bounds. `exists` and `focused` check pane lifecycle/focus while the latter inspect a process or WASM app's serialized L1 state. Native builtins use `assert_label`. Every present key must match. Prefer handle-scoped assertions in scenes meant for both backends: an installed profile may restore unrelated panes, so absolute global counts are intentionally profile-dependent.

### Running

```bash
just scene tests/scenes/balls.toml           # one scene → /tmp/plexi-scenes
just scene <file> /tmp/out 0                 # state-only: skip shot steps (no GPU)
just scene-live <file> pr-123                # same schema against installed host
cargo test --bin plexi scene_suite           # all committed suite scenes
```

`just scene` accepts absolute paths, so one-off scenes can live in `/tmp` and never touch the repo.

### SceneReport

Every run writes `<out>/<scene>.json` (`schema_version: 4`). It includes the selected backend and channel, pass/fail per step, resolved real pane ids, host state, teardown result, and the last-opened app state. Successful action details and failures are structured objects; failures carry stable `code` and `message` fields. Failed steps include a `failure_bundle` path with the semantic dump, log tail, manifest (including eventual-poll history), and the headless screenshot when available. Prefer state and semantic-label assertions over pixel comparisons. Screenshots remain a human review artifact.

`just scene-live` requires an explicit installed channel, boots that channel's host, drives generic app open/text/key/context switch/context push operations through the public CLI/IPC surface, observes `pane state` and context metadata, and always tears down a runner-owned host. Set `PLEXI_SCENE_ATTACH=1` only to attach deliberately; attached hosts are never stopped by the runner. The outer script writes a runner-only ownership marker, so SIGINT/SIGTERM/HUP cleanup can stop an owned host without touching an attached one. Unsupported live verbs fail with `unsupported_live_verb` rather than silently diverging from headless semantics. Live assertions use bounded eventual polling with a small poll interval and a stable-state barrier, never fixed workflow sleeps.

Changes to `LiveBackend` require a `just scene-live` run against the installed PR
channel. A passing headless scene does not exercise CLI command construction,
host startup/ownership, or eventual polling.

Whole-host `text`, `key`, and `assert_label` targets are headless-only because the installed host exposes no sanctioned generic host-input or host-semantic CLI/IPC seam. The live backend returns `unsupported_live_target` for `target = "host"`; pane targets retain the shared semantics.

## Host Surface Coverage

Use this map before extending the DSL. “Scene” means observable coverage through
the shared TOML schema; “logic” names the lower layer for invariants that are not
pixels. A blank scene cell is a deliberate gap, not permission to add an ad hoc
driver.

| Host surface | Scene coverage | Logic coverage | Live CLI surface |
|---|---|---|---|
| Open process, builtin, and WASM panes | `open`, app-specific scenes | launch and lifecycle tests | `plexi app open` |
| Pane focus, close, text, and native/process/WASM keys | `focus`, `close`, `text`, `key`; `pane-lifecycle.toml`, `assistant-settings.toml`, `wasm-sysmon.toml` | `HostHarness` IPC and native key tests | `plexi pane focus/close/send/key` |
| Pane file and URL drops | `drop_file`; `notes-agent-drive.toml` | Production dispatch, persistence, and observable acceptance or rejection tests | `plexi pane drop <id> <path-or-url>` |
| Canvas/pointer clicks on process and WASM app panes (stint 0398) | no scene verb yet — use `HostHarness::inject_click`/`AppRequest::ClickPane` for headless coverage | `HostHarness` `click_pane_delivers_canvas_space_coordinate_through_fit_contain_transform` (end-to-end, real process app, `fit="contain"`) + `wasm_render.rs` `canvas_click_inverts_fit_transform_to_canvas_space` (unit, transform math only) | `plexi pane click <id> <x> <y> [--button left]` — pane-pixel coordinates, delivered through the pane's live `canvas_transform` inversion, never OS automation |
| Node-targeted clicks on Button/TextInput/ListView nodes in WASM (incl. CPython-in-WASM) app panes (stint 0414) | no scene verb yet — use `HostHarness::inject_node_click`/`AppRequest::ClickPaneNode` for headless coverage | `HostHarness` `click_pane_node_activates_button_and_mutates_guest_view` (end-to-end, real process app, node resolved from `plexi pane state`, covers the fail-loud missing/non-interactive paths) + `pane.rs` `resolve_interactive_node_*` (unit, role validation) | `plexi pane click <id> --node <node_id> [--button left]` — targets a node by the id `plexi pane state` reports, never pixel geometry; fails loudly (named error, nonzero exit) when the id is absent from the current tree or not an interactive role |
| Pointer drags (press → moves → release) on app panes (stint 0510) | `drag` verb; `canvas-drag.toml` | `HostHarness` `drag_pane_delivers_press_moves_release_through_canvas_transform` (end-to-end, real process app, full trajectory in canvas space) + `drag_pane_node_endpoints_fail_loudly_without_bounds` (node-endpoint contract) | `plexi pane drag <id> --from x,y --to x,y [--steps N] [--button left]` (or `--from-node`/`--to-node`) — delivered one frame per sample through the production input paths, never OS automation |
| Emitted event-bus events (`DeclareEventStreams`/`EmitEvent`, stint 0511) | `expect` `event_stream`/`event_payload_contains`; `event-probe.toml` | `HostHarness::wait_for_app_event` over the recorded `AppTimeline`; `emitted_app_event_is_recorded_and_awaitable` (end-to-end, real process app) | `plexi events subscribe <app_id> <stream>` (NDJSON) — live scenes have no event-query seam yet, so event expectations are headless-only |
| Split directions, tabs, overlays, and multi-window placement | host shortcuts can be driven with whole-host `key`; placement state is covered in focused Rust tests | pane layout, window cleanup, and spawn tests | `plexi pane new` / `plexi app open` placement flags |
| Context switch, subcontext push, and portals | `switch_context`, `push_to_subcontext`; `subcontext-portal.toml` | context and portal lifecycle tests | `plexi context zoom/push/list` |
| Sidebar and context drag/drop | `sidebar`; file-browser sidebar scene. Host-chrome pointer drag (sidebar reorder) remains a direct `PlexiUiHarness` edge-case test — `plexi pane drag` targets app panes, not host chrome. | sidebar reorder/drop resolver tests | app-pane drags use `plexi pane drag`; single clicks use `plexi pane click` |
| Command palette and host overlays | whole-host `key` plus semantic/pixel assertions; direct harness tests cover draw-pass focus edge cases | palette, notification, quick-note, and focus-stack tests | no generic whole-host key command; invoke the underlying pane/context/app CLI command |
| Agent panes and terminal PTY behavior | host state and pane chrome can be captured; PTY behavior is not simulated | agent-state and `#[ignore = "requires-pty"]` tests | `plexi pane new`, `plexi agent report`, `plexi pane capture` |
| Native, process, WASM, portal, sidebar, and overlay screenshots | `shot` captures the entire headless host framebuffer, so pane type does not require a separate capture path | renderer and semantic-state tests | live scenes capture `shot` through the channel host screenshot command; installed validation can also use `drive-host` capture |
| Experiments (stint 0785) | top-level `experiments = ["wasm_apps", ...]` scene key lists `ReleaseFeature` keys switched on for the scene (all are off by default). Headless-only. | `set_test_experiments` in Rust tests | a live host reads its own `[experiments]` config; use `plexi experiments enable` |
| App file picker + picker-granted fs scopes (stint 0508) | top-level `picker_script` scene key scripts every `OpenFilePicker` outcome in order (`{ paths = [...] }` / `{ cancel = true }`, `{out}` prefix → scene out dir); `file-picker.toml` runs the pick → write → read → cancel round trip. Headless-only. | `wasm_pane` picker tests (capability denial, grant scoping, save-as, folder subtree, cancel) + `resolve_app_fs_path` jail/grant tests | no scene-live support; launch the host with `PLEXI_PICKER_SCRIPT=<json>` — same entry format, consumed per pane at launch — so tester agents drive picks without a dialog |

Do not require scene and CLI syntax to be identical. They must converge on the
same production host operation. Pointer-only UI affordances and whole-host
overlays have no sanctioned live input seam; test their observable UI headlessly
and test the underlying mutation through its public CLI command.

## Real App Processes in Tests

`open = { kind = "process", ... }` uses the production path: manifest load, launch through the CPython-in-WASM adapter (`src/host/wasm_python.rs`, stint 0285), and the L1 render pipeline. Outside an installed bundle the repo SDK is exported through `PLEXI_SDK_PATH=sdk/python` automatically. Python resolution comes from `src/app/python_env.rs`: per-app `.venv`, bundled python, then system `python3`, with Python 3.11 or newer required.

## Pre-Push Evidence

This file is the self-validation contract for every coding agent. The `/testing`
skill (`.agents/skills/testing/SKILL.md`) is its executable workflow: classify the
diff, run every touched layer, inspect every generated screenshot, and write the
`**Test evidence:**` block into the issue Ship Log or PR body. validate-pr reads
that block; `install skippable — full coverage` means diff-review-only validation.

**Visual review is mandatory for any host UI change, not optional.** A change
touching layout, alignment, spacing, color, or any `egui::Ui`/render code is not
done when the assertions pass — a passing geometry or galley-halign test proves
the numbers are right, not that a human/agent looking at the pixels would agree.
Before calling a UI fix complete:

1. Write or reuse a `PlexiUiHarness::save_screenshot` test that renders the
   actual changed surface with realistic seeded content (not an empty state) —
   `screenshot_assistant_conversation_bubbles` in `src/ui_tests.rs` is the
   pattern: build real model state, open through the real pane path, screenshot.
   Name the output with `evidence_png!()`, never a hand-typed path: it derives
   `/tmp/plexi_<test fn>.png` from the enclosing function, so a renamed test
   cannot leave a stale filename and `rm /tmp/plexi_*.png` reaches every
   artifact. Pass a suffix for several shots from one test, `evidence_png!("_ppp2")`.
2. Run it and **open the PNG with the Read tool and look at it** — confirm the
   change looks right, not just that the test exited 0.
3. Keep the inspected screenshot until it is published as review evidence.
   After opening the PR, run the command below and add its output to the PR
   body. Do not commit the PNG to the feature branch. The retained
   `evidence/pr-<number>` branch holds the image independently of feature-branch
   cleanup. Its immutable commit link remains usable while that branch is kept.
4. For a nonvisual change, record `Visual evidence: not applicable` with a
   reason. When a required screenshot or publication fails, record the failure
   and do not claim visual evidence or complete the required gate. A headless
   render is not live-host proof; capture real host pixels when that boundary is
   required. Independent validation is separate from the implementer's image.

```bash
python3 scripts/publish-pr-image.py --pr <number> --head <full-pr-head-sha> \
  --image /tmp/<inspected>.png --source headless \
  --context "<feature and scene or capture details>" --inspected
```

Use `--source live-host` only for actual `plexi host screenshot` pixels and
include the channel/build identity in `--context`. The source and head are
assertions by the operator, not inferred from PNG bytes: verify both before
publishing. Inspect the image for private information first; publishing uses the
repository's visibility. Public repository images are publicly accessible.
The command verifies the saved blob and immutable URL; private repository links
require reviewer access. Never describe a local `/tmp` path as durable evidence.
Keep the evidence branch for as long as the PR's visual proof is needed; deleting
it can eventually make commit-based links unavailable. Delete temporary local
PNGs after successful publication.

This applies whenever the diff includes host UI rendering — not only when the
task is explicitly "visual." A logic-only PR that happens to touch `render.rs`
still gets this pass before push.

## Editor Release Gate

Any diff touching `src/editor/` or `src/app/text_editor_app.rs` runs the editor
release gate before push, in this order:

1. **Core matrix + fuzz** — `cargo test --bin plexi editor::gate`. Runs the
   table-driven command matrix (`gate_cases` in `src/editor/gate.rs`), a long
   deterministic stress sequence with full undo/redo round trips, and seeded
   randomized command fuzzing with per-command invariant checks (grapheme-valid
   selections, semantic/text agreement, history consistency, monotonic
   revisions).
2. **Harness layer** — `cargo test --bin plexi editor_gate`. Drives a
   representative subset of the same matrix through the installed-host input
   paths (`SendToPane`/`KeyPane`/click/drop against a real builtin Notes pane)
   plus host-only surfaces: pointer caret placement, Live Preview toggling,
   save success/failure reporting, and drop accept/reject.
3. **Scenes headless** — the `editor-gate-*.toml` and `notes-*.toml` scenes run
   in `scene_suite`; iterate on one with `just scene tests/scenes/<file>`.
4. **Installed host** — `just editor-gate pr-<N>` re-runs the core
   qualification, boots one hermetic host (`plexi host start --ephemeral`:
   no workspace restore, no workspace save), leaves start/finish markers in
   the channel's `plexi.log` via `plexi host log --source editor_gate`, and
   runs every editor scene against that host in attach mode. It collects
   everything into `/tmp/plexi-editor-gate/pr-<N>/`: per-scene SceneReports
   and failure bundles, a best-effort bounded `host screenshot` per scene
   (never a gate condition), the core qualification artifact
   (`editor-gate-core.json`: per-case pass/duration/final semantic state,
   per-seed randomized results, totals), a channel `log-tail.txt`, and
   `summary.json`. Non-attached live scene runs (`just scene-live`) are
   hermetic too: the runner-owned host always boots `--ephemeral`.

Randomized failures are replayable: the panic names a seed and writes a
minimized replay bundle to `$TMPDIR/plexi-editor-gate-failure-<seed>.json`;
rerun exactly that seed with `PLEXI_EDITOR_GATE_SEED=<seed>`.

## DAW Release Gate

Any diff touching `crates/daw-*` or `apps/wasm-poc/daw-engine/` runs the DAW
release gate before push. The DAW's pure model is a sibling crate, so its gate
(`gate_cases` in `crates/daw-model/src/gate.rs`, exposed to the host via that
crate's `gate` feature) is reused across every tier — same shape as the editor
gate above:

1. **Core matrix + fuzz** — `cargo test --bin plexi daw_gate`. The host bridge
   (`src/testing/daw_gate.rs`) drives the pure-model matrix, deterministic long
   sequence, and seeded command fuzzer (per-command `validate` + revision
   monotonicity + undo consistency, rejected/no-op no-mutation), and asserts
   offline-mixdown determinism: fixture project → equal PCM hash, block-size
   independence, byte-identical `.wav` export.
2. **Harness layer** — `daw_gate_engine_matrix` replays every named case
   through the real `plexi-daw-engine` render path; `daw_gate_pane_drive` drives
   the committed DAW WASM fixture in-process through named-key input, asserting
   the semantic tree reacts.
3. **Scenes headless** — the suite `daw-*.toml` scenes (`daw-timeline`,
   `daw-bundle`) run in `scene_suite`; iterate on one with `just scene
   tests/scenes/<file>`. The live scene runner pre-approves a fixture's
   raw-WASM imports so an attach run never stalls on capability review:
   headless seeds the in-process test profile directly, live shells `plexi app
   trust <wasm>` through the real channel binary (the test process is walled
   off from real profiles).
4. **Installed host** — `just daw-gate pr-<N>` re-runs the core qualification,
   boots one hermetic host, leaves markers via `plexi host log --source
   daw_gate`, and runs the attach-eligible scene set (`daw-timeline` plus
   `daw-gate-bundle`, the attach counterpart of the headless `daw-bundle` whose
   in-scene `picker_script` is headless-only) against that host. Save/open/
   export picks come from the host's `PLEXI_PICKER_SCRIPT`, set before `host
   start`. The gate refuses any selected scene that declares a headless-only
   feature. Everything lands in `/tmp/plexi-daw-gate/pr-<N>/` (per-scene
   SceneReports, best-effort host screenshots, the `daw-gate-core.json`
   artifact, a channel `log-tail.txt`, and `summary.json`).

Randomized failures are replayable: the panic names a seed and writes a
minimized replay bundle to `$TMPDIR/plexi-daw-gate-failure-<seed>.json`; rerun
exactly that seed with `PLEXI_DAW_GATE_SEED=<seed>`.

## Conventions

- Follow The Gate above for the push path and integration. The justfile exports `RUSTFLAGS="-D warnings"`, so warnings are build failures under `just test`.
- PTY-dependent tests are tagged `#[ignore = "requires-pty"]`.
- Test-first for host logic: a new `AppRequest`/`HostEffect` gets a failing `HostHarness` test before implementation.
- New host UI component or overlay → a scene in `tests/scenes/` (open → act → assert → shot).
- Coverage: `cargo llvm-cov --bin plexi` (cargo-llvm-cov installed at `~/.cargo/bin/`).

## Per-Test Profile Isolation

Every `HostHarness::new()` call creates a fresh `tempfile::TempDir` and installs a thread-local override so `config_dir()` and `config_path()` resolve inside that tempdir for the lifetime of the harness. The tempdir is deleted when the harness drops.

**What this means for test authors:**
- Tests that go through `HostHarness` never touch `$HOME` — no stray `~/.plexi-<hash>/` dirs.
- The override is thread-local, so worker threads spawned by the host (e.g. IPC handlers) won't see it. This is acceptable for dispatch tests; note it when writing tests that assert on paths written by background threads.
- For unit tests that call `config_dir()` directly (without HostHarness), use `set_test_profile_dir(tempdir.path().to_path_buf())` and hold the returned `TestProfileDirGuard` for the test duration.
- `set_test_channel()` remains for tests that specifically exercise channel-name derivation logic. Prefer `set_test_profile_dir` for everything else.
