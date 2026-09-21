# Plexi v1 app runtime audit

Date: 2026-09-20

## Decision

Ship the app runtime in v1. The Python SDK path works through the CPython-in-WASM adapter, core apps render, and the local package workflow is substantially present. Apps are a first-class pane type with capability declarations, tool exposure, and host event-bus integration.

The release blocker is the advertised author test command: a fresh scaffold and maintained apps cannot run `plexi app test` because the command invokes `uv run pytest tests/` without making pytest available. Fix that gate before calling the authoring loop v1-ready.

## Runtime surface map

| Surface | Implementation | Audit finding |
| --- | --- | --- |
| Discovery and lifecycle | `src/app/registry.rs`, `src/cli/app.rs`, `src/cli/open.rs` | Global/workspace registries support list, info, install, uninstall, open, update, and freeze. Opening/action delivery needs a host socket. |
| Packs | `packs/core.toml`, `packs/examples.toml`, `src/app/packs.rs` | Core/example packs are embedded; `app freeze` creates replayable TOML. |
| Python / PGAP | `sdk/python`, `src/host/wasm_python.rs` | Python apps launch in CPython-in-WASM, exchange PGAP frames, render L1 or Canvas, and tick off-screen. |
| Native WASM | `wit/plexi.wit`, `src/host/wasm_pane.rs`, `src/host/wasm_app.rs` | Separate typed WIT component runtime exists. Checked-in WASM projects are POCs, not release apps. |
| Host IPC/effects | `src/app/app_trait.rs`, `src/app/mod.rs`, `src/host/pane.rs` | Apps emit effects/actions; host services them from logic/background ticks. |
| Permissions | `src/app/permissions.rs`, runtime modules | Manifest capabilities, grants, scoped effects, and audit events are implemented; live prompts were sandbox-blocked. |
| State and SDK injection | `src/host/state_scope.rs`, `src/cli/app_state.rs`, `sdk/python/plexi_sdk` | Compiled runtime mounts the SDK; state is host scope-addressed. `app state` correctly rejects undeclared state. |
| Packaging/trust | `src/app/package.rs`, `src/cli/app.rs` | Validation is fail-closed; packages include checksums; inspect reports runtime, trust, capabilities. |

## Command evidence

Commands used installed `plexi 0.3.0`, this checkout, and temporary output paths.

| Command | Result |
| --- | --- |
| `plexi app list` | Passed; listed installed core apps and the example app. |
| `plexi app render apps/{calc,balls,logs,todo,sudoku,github-issues}` | Passed; each started a CPython WASI guest and emitted a JSON frame. |
| `plexi app check apps/{calc,balls,logs,todo} --png-dir /tmp/...` | Passed through the size matrix and wrote PNGs. Logs also exercised seeded state and semantic actions. |
| Fresh `plexi app init --lang python ...`, then `plexi app check` | Passed; SDK-v3 scaffold rendered at checked sizes and reflected fixture state. |
| `plexi app test apps/calc` and fresh-scaffold `app test` | Failed consistently: `Failed to spawn: pytest` / `No such file or directory`. Product defect, not app-specific. |
| `plexi app package apps/calc --out /tmp/plexi-audit-calc.plexipkg` | Passed; package excluded generated `.venv`. |
| `plexi app validate` for calc directory/package and `plexi app inspect` | Passed; entry, runtime, checksums, capabilities, and core trust label validated. |
| `plexi app freeze /tmp/plexi-audit-apps.toml` | Passed; wrote a replayable pack. |
| `plexi app update calc` | Expected no-op: bundled app is not a git checkout. |
| `plexi app state get calc` | Correctly rejected: calc declares no file-backed state. |
| `plexi app validate apps/wasm-poc/{counter,pong,sysmon}` | Failed because referenced built `.wasm` artifacts are absent; these are raw development POCs. |
| `plexi host status`, `plexi app open calc`, `plexi pane list` | Not product-evaluable: no host was running and sandbox policy denied Unix-socket access. |

## Confirmed breakage

### `plexi app test` does not provision pytest

`app_test_cli` in `src/cli/app.rs` starts `uv run pytest tests/` in the app directory. Neither app directories nor the workspace project declares pytest, so `uv` cannot find the executable. The scaffold, app contract, CLI help, and authoring guide all advertise this command.

Fix it by making the command self-contained (for example, explicitly requesting its supported `uv` test dependency) and add a focused command-construction regression test. Then prove it on a new scaffold and a maintained app. Do not add a dependency declaration to every exemplar as a workaround.

### Validation has acknowledged blind spots

`app check` passes while warning that type checking was skipped when mypy cannot be imported or installed. A fresh scaffold also says its semantic-chrome expectation is not implemented against the live WIT UI model. These are not launch failures, but the verification gate is weaker than its help text implies.

## V1 must-fix

- Restore a self-contained `plexi app test` path for fresh and maintained Python apps, with regression and real-scaffold evidence.
- Run an end-to-end Linux host pass outside this sandbox: open a Python app, inspect state, deliver action/key events, verify a capability decision, tool dispatch, and close.
- Decide whether skipped mypy and semantic-chrome checks are hard failures or explicit optional diagnostics.
- Keep the maintained app set green with `app check` and repaired `app test` before release.

## Nice to have / post-v1

- Build and smoke-test a native WASM component in CI/developer tooling; POC artifacts are presently absent.
- Add one Linux host smoke recipe for app pane state, actions, permissions, event-bus traffic, and tools.
- Reduce legacy-scaffold warnings where current metadata is truthful.
- Make type and semantic-chrome coverage deterministic rather than best effort.

## Issue handoff

File one grouped issue: **Fix `plexi app test` for scaffolded and maintained Python apps** (`bug`, `P1`, `area:cli/commands`, `area:sdk/python`, `load:M`, `ready`). Implementation map: `src/cli/app.rs` owns test-runner provisioning and its focused regression test; scaffolds/docs are verification consumers, not the primary fix.

GitHub API access was unavailable and `stint` is not installed in this environment, so no GitHub issue or mirrored stint task was created. File this single grouped issue when services are available; do not split it per app.

## Scope limits

The sandbox could read the installed profile but could not write it and denied Unix-socket connections. Install mutation, live IPC, permission prompts, hot reload, and tool dispatch are unverified rather than marked broken.
