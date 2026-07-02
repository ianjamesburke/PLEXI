# Devlog

Landed-work history moved out of `WHATS_NEXT.md` so the orientation file stays
forward-looking. Newest entries first. Append a dated section when `/whats-next`
or `/merge-pr` trims the orientation file; do not rewrite old entries.

---

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
