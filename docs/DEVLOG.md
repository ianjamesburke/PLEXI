# Devlog

Landed-work history moved out of `WHATS_NEXT.md` so the orientation file stays
forward-looking. Newest entries first. Append a dated section when `/whats-next`
or `/merge-pr` trims the orientation file; do not rewrite old entries.

---

## 2026-07-08 — Native pane-key driving

`0351` landed in #2367: `plexi pane key` on a native (builtin/WASM) app pane was a silent no-op — the host queued `PlexiEvent::Key`, which native apps never read (they consume `egui::InputState` via `App::handle_key`). The `KeyPane` handler now synthesizes an `egui::InputState` and drives the app's real `handle_key`, the same path a physical keystroke takes; responses report `disposition: consumed|passthrough` so drive-host validation can detect ignored keys. Unblocks validating PR #2366 (`0349`, explorer native viewers), which exposed the gap.

---

## 2026-07-02 — Epoch 1 close bundle

`0330`, `0331`, `0332`, `0335`, `0328`, `0334`, `0346`, and `0215` landed in #2360:

- App-authoring DX now has sibling-split app opens, canonical SDK authoring docs with drift gates, agent-driven E2E capture, and benchmark prompt/scorecard scaffolding.
- The shipped app set was culled into maintained exemplars, with Logs rebuilt, CSV/Wikipedia/core survivors checked, Snake/Tetris restored as examples, and stale dev apps kept out of PR/alpha installs.
- SDK v3 gained the native placeholder/component coverage slice and host renderer fixes for tables, text entry focus, button affordance, CSV scrolling, and over-eager app hung status.
- Core registry packages are generated under the website registry with bytecode/cache artifacts excluded; the remaining post-merge gap is a production install smoke after alpha deploy.

## 2026-07-02 — pane new --tab anchoring fix

- `0337` (#2357): `plexi pane new --tab --from <pane-id>` now anchors to the caller pane's window instead of the currently active window, matching the existing `--window` and split behavior. Also fixed two related cwd-fallback bugs where the tab path fell back to the ambient active window/router context instead of the target window's own context when no explicit `--cwd` was given.
- `0304` (#2332, landed earlier): confirmed still correct on inspection — the `--window` path already anchored to `from_pane_id` correctly, so this task only needed the `--tab` path fix.

## 2026-06-30 — Free v1 spine + app-builder hardening

Free v1 local/demo/distribution/trust/hosted-registry spine landed on `alpha`:

| Task | PR | Result |
|------|----|--------|
| 0313 | #2347 | SDK/scaffold self-documenting flow shipped. |
| 0314 | #2348 | ActionBar scaffold pattern and FooterKeys clipping fix shipped. |
| 0299 | #2349 | Todo rebuilt as the canonical SDK v3 demo app. |
| 0316 | #2350 | Scaffold packaging, direct GitHub/source install, update unification, ref fallback, and workspace-aware update shipped. |
| 0320 | #2351 | Reviewed-native bypass scanner and honest trust labels shipped. |
| 0321 | #2352 | Free hosted reviewed-native registry smoke path shipped. |
| 0237 | #2354 | Workspace/global secrets now flow through command runs, PTYs, and the OpenRouter broker. |

App-builder hardening also landed on `alpha`:

- `0326` shipped scaffold-local `AGENTS.md`, `.gitignore`, drift metadata, fixtures, semantic ActionBar/FooterKeys boilerplate, host probes, headless check/render coverage, and hot-reload guidance.
- SDK semantic chrome shipped in `src/render/app_chrome.rs`; app init/check now exercise host-native semantic components.
- `plexi app check` gates current scaffolds on semantic proof components and seeded render/action probes.

### Verified by validation runs

- Fresh scaffold validate -> package -> package install works; generated `.venv` artifacts are excluded.
- Direct GitHub/source installs route through the git resolver.
- Pack-file install with git cloning works.
- Core pack install works.
- `plexi app update` / `plexi update apps` use the real git update path and handle workspace installs.
- Reviewed-native package validation flags obvious subprocess/socket/path traversal bypasses.
- Free hosted reviewed-native registry smoke path is live in the website registry fixture.
- Agent app-building loop is trustworthy enough for v1: generated app instructions, drift metadata, headless check/render, JSON seed state, real host state/action/key probes, and same-pane hot reload were verified by three sequential app-build trials.
- Workspace secrets resolver now works for command-run, OpenRouter broker lookup, and GUI terminal panes after zsh startup overwrites.

### Source-of-truth correction

`0319` resolved the sequencing conflict. Free v1 marketplace work proceeds through reviewed-native Python apps with blunt trust labels, human review, and bypass-pattern checks. WASM remains the stronger sandbox/performance path and v2 trust upgrade.
