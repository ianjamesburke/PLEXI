# Plexi Roadmap

> Single source of truth for what's shipping and when. Update this alongside DEV_LOG.md.
> GitHub issues are the task tracker — this is the narrative.

---

## Vision

One window. Everything installs into it. Terminals, agents, file explorers, AI pipelines — all isolated behind PGAP, all navigable without switching windows. The agent is not a chatbot bolted on — it's a first-class pane that knows your project, calls tools, dispatches subprocesses, and surfaces results as rich notifications you can respond to without context-switching.

---

## Phase 0 — Foundation ✅ (complete, 2026-04-18)

The 12-step v3 refactor. HostModel state machine, PGAP v3 protocol, capability broker, HTTP broker, directory-scoped secrets, FileEventSink, CI gate, 74+ tests green, zero warnings.

---

## Phase 1 — v3.0: Daily Driver 🚧 (target: this week)

**What "daily driver" means:** open Plexi instead of iTerm2. Use it for terminals, the file browser, and agent conversations every day.

### Blocking

| Issue | What | Notes |
|---|---|---|
| [#279](https://github.com/ianjamesburke/PLEXI/issues/279) | Decompose tiling.rs + pane_ops.rs | Pure refactor — unlocks clean agent pane wiring. tiling.rs → ~200 lines, pane_ops → submodules, AgentPane stub added. |
| [#288](https://github.com/ianjamesburke/PLEXI/issues/288) | Pane::Agent — claude -p --resume backend | The missing third pane type. Session IDs persist per workspace. Transcript UI, ledger.jsonl cost tracking, CWD context injection. |

### Do after v3.0 is tagged, before v3.1 starts

| Issue | What |
|---|---|
| [#192](https://github.com/ianjamesburke/PLEXI/issues/192) | Terminal scrollback — PgUp/PgDn, copy-mode |
| [#200](https://github.com/ianjamesburke/PLEXI/issues/200) | Clipboard / paste / mouse events across apps |
| [#258](https://github.com/ianjamesburke/PLEXI/issues/258) | File browser UX polish |
| [#146](https://github.com/ianjamesburke/PLEXI/issues/146) | DrawCommand::CopyToClipboard |

---

## Phase 2 — v3.1: Intelligence Layer (target: 2–3 weeks post-v3.0)

**What this unlocks:** apps can call the LLM. Agents are installable PGAP apps. Agents know about each other. Per-project agents live alongside the code.

### Core issues (ordered)

| Issue | What | Depends on |
|---|---|---|
| [#284](https://github.com/ianjamesburke/PLEXI/issues/284) | `iq.query` — brokered LLM capability (low/medium/high tiers) | #288 |
| [#285](https://github.com/ianjamesburke/PLEXI/issues/285) | Agent-as-app — PGAP subprocess agents with manifests | #284 |
| [#286](https://github.com/ianjamesburke/PLEXI/issues/286) | Agent roster + inter-agent pipes | #285 |
| [#287](https://github.com/ianjamesburke/PLEXI/issues/287) | Directory-scoped app + agent registry (.plexi/apps/, .plexi/agents/) | #285 |
| [#74](https://github.com/ianjamesburke/PLEXI/issues/74) | Rich notification panel — images, structured choices, keyboard nav | #288 |
| [#83](https://github.com/ianjamesburke/PLEXI/issues/83) | Hot reload for app development | — |

### Supporting v3.1 issues

| Issue | What |
|---|---|
| [#78](https://github.com/ianjamesburke/PLEXI/issues/78) | Canvas terminal binding primitives (RunInLinkedTerminal, StreamProcess) |
| [#132](https://github.com/ianjamesburke/PLEXI/issues/132) | Advanced UI SDK: mouse events, delta_time |
| [#255](https://github.com/ianjamesburke/PLEXI/issues/255) | SDK button primitive |
| [#115](https://github.com/ianjamesburke/PLEXI/issues/115) | Split command palette: Cmd+P (find pane) vs Cmd+Shift+P (launch app) |
| [#169](https://github.com/ianjamesburke/PLEXI/issues/169) | Publish plexi-sdk to PyPI with type stubs |

---

## Phase 3 — v3.2: Platform (target: 1–2 months post-v3.1)

**What this unlocks:** community app distribution. Real media. Mobile/web target.

| Issue | What |
|---|---|
| [#277](https://github.com/ianjamesburke/PLEXI/issues/277) | CoreAudio capture + device enumeration |
| [#278](https://github.com/ianjamesburke/PLEXI/issues/278) | AVFoundation video decode |
| [#79](https://github.com/ianjamesburke/PLEXI/issues/79) | GUI↔Terminal media bridge |
| [#99](https://github.com/ianjamesburke/PLEXI/issues/99) | App Store — discover, install, update apps |
| [#105](https://github.com/ianjamesburke/PLEXI/issues/105) | WASM + PWA mobile deployment |
| [#217](https://github.com/ianjamesburke/PLEXI/issues/217) | Rust SDK parity with Python SDK |

---

## Backlog (v3.1+ no fixed phase)

Good ideas, not yet scheduled. Promote to a phase when the dependencies above are clear.

[#49](https://github.com/ianjamesburke/PLEXI/issues/49) Auto-update •
[#52](https://github.com/ianjamesburke/PLEXI/issues/52) Light/dark mode awareness •
[#58](https://github.com/ianjamesburke/PLEXI/issues/58) Agent Workspace + git worktrees •
[#59](https://github.com/ianjamesburke/PLEXI/issues/59) Session management + per-project config •
[#62](https://github.com/ianjamesburke/PLEXI/issues/62) Modifier key: split into new tab vs in-place •
[#65](https://github.com/ianjamesburke/PLEXI/issues/65) Directional nav retrace on reverse •
[#66](https://github.com/ianjamesburke/PLEXI/issues/66) Chained terminal execution (task pipeline) •
[#68](https://github.com/ianjamesburke/PLEXI/issues/68) Auto-close pane on process exit •
[#70](https://github.com/ianjamesburke/PLEXI/issues/70) Hover tokens for file explorer •
[#76](https://github.com/ianjamesburke/PLEXI/issues/76) File browser depth/breadcrumb indicator •
[#81](https://github.com/ianjamesburke/PLEXI/issues/81) Quick Note pane •
[#89](https://github.com/ianjamesburke/PLEXI/issues/89) Navigator — Harpoon-style directory hotlist •
[#94](https://github.com/ianjamesburke/PLEXI/issues/94) Backlog triage app •
[#116](https://github.com/ianjamesburke/PLEXI/issues/116) First-run UX / Starship detection •
[#122](https://github.com/ianjamesburke/PLEXI/issues/122) Finder Service "Open in Plexi" •
[#124](https://github.com/ianjamesburke/PLEXI/issues/124) Better process monitor app •
[#127](https://github.com/ianjamesburke/PLEXI/issues/127) github-issues: comment authoring •
[#129](https://github.com/ianjamesburke/PLEXI/issues/129) github-issues: state mutation keys •
[#130](https://github.com/ianjamesburke/PLEXI/issues/130) github-issues: pane label + Focus Manager •
[#138](https://github.com/ianjamesburke/PLEXI/issues/138) File explorer: Enter opens file in editor •
[#141](https://github.com/ianjamesburke/PLEXI/issues/141) Hold-Cmd modal keyboard overlay •
[#182](https://github.com/ianjamesburke/PLEXI/issues/182) Pane resize handle visual bugs •
[#185](https://github.com/ianjamesburke/PLEXI/issues/185) SDK startup message into linked terminal •
[#188](https://github.com/ianjamesburke/PLEXI/issues/188) --plexi standard for CLIs •
[#190](https://github.com/ianjamesburke/PLEXI/issues/190) Animated pane transitions •
[#201](https://github.com/ianjamesburke/PLEXI/issues/201) ctx.list() position params •
[#223](https://github.com/ianjamesburke/PLEXI/issues/223) Notification undo •
[#242](https://github.com/ianjamesburke/PLEXI/issues/242) SDK docs: Enter/Escape convention •
[#256](https://github.com/ianjamesburke/PLEXI/issues/256) CAPABILITIES.md •
[#257](https://github.com/ianjamesburke/PLEXI/issues/257) Justfile dev standard for app authors •
[#283](https://github.com/ianjamesburke/PLEXI/issues/283) TextInput primitive

---

## How to update this

- When an issue moves from Blocking → done, strike it and add ✅.
- When a phase completes, mark it ✅ with the date.
- When new issues are filed, add them to the right phase or Backlog.
- DEV_LOG.md captures the *why* behind decisions. This file captures *what* and *when*.
