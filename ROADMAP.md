# Plexi Roadmap

Reference document linking layers of work to specs and issues. This file tracks sequencing and dependencies — the specs have the details.

---

## Layer 0: Unblocked Now

| Task | Type | Reference | Status |
|------|------|-----------|--------|
| Self-closing panes via OSC title | Code (~35 LOC) | [#90](https://github.com/ianjamesburke/PLEXI/issues/90) | Ready |
| Hot reload for app development | Code | [#83](https://github.com/ianjamesburke/PLEXI/issues/83) | Ready |
| App protocol test harness (`plexi_test.py`) | Code (~200 LOC) | See handoff doc | Ready |
| Issue triage cleanup (close 13, update 7) | Ops | Triage report in DEV_LOG 2026-04-11 | Ready |
| Finder Service "Open in Plexi" | Code | North star ship order #6 | Ready |
| Theme: surface-specific hover tokens | Code (small) | [#70](https://github.com/ianjamesburke/PLEXI/issues/70) | Ready |

## Layer 1: App Protocol Testing

Depends on: Layer 0 test harness

| Task | Type | Reference |
|------|------|-----------|
| `plexi_test.py` — spawn app, send events, assert on draw commands | Code | Handoff doc |
| Test cases for existing apps (Wikipedia, Plexi Browser) | Code | — |
| CI integration (run tests on PR) | Ops | — |

**Unlocks:** All app development iteration, Parallax app, app store

## Layer 2: Agent Mode in Terminal

Depends on: Nothing (Plexi core, independent track)

| Task | Type | Reference |
|------|------|-----------|
| `/` mode switching in terminal pane | Rust | [Agent Mode spec](docs/specs/agent-mode.md) |
| Agent context loading (lazy index) | Rust | Agent Mode spec §5 |
| LLM call backend (async, SecretGet for API key) | Rust | Agent Mode spec §6 |
| Slash commands (/status, /cost, /jobs) | Rust | Agent Mode spec §4 |
| Background job tracker | Rust | Agent Mode spec §7 |
| Trust/risk scoring | Rust | [Agent Orchestration spec](docs/specs/agent-orchestration.md) §4, §10 |

**Unlocks:** Agent-driven app interaction, companion app, remote access

## Layer 3: Parallax Refactor

Depends on: Layer 1 (for testing the app)

| Task | Type | Reference |
|------|------|-----------|
| Manifest-first refactor (editors write YAML, not ffmpeg calls) | Python | [Parallax Packaging spec](../parallax/docs/parallax-plexi-packaging.md) §3 |
| Agent extraction (system prompts → .md files) | Python | Packaging spec §4 |
| Tool packaging (pipeline scripts + tool.yaml descriptors) | Python | Packaging spec §5 |
| SecretGet integration for API keys | Python | Packaging spec §6 |
| Cost reporting via cost_report events | Python | Packaging spec §7 |

**Unlocks:** Parallax app, agent orchestration in practice

## Layer 4: Apps That Prove the Protocol

Depends on: Layers 1-3

| Task | Type | Reference |
|------|------|-----------|
| Parallax chat MVP | Python app | [Parallax App spec](../parallax/docs/parallax-plexi-app-spec.md) |
| App Store (built-in) | Rust app | [#99](https://github.com/ianjamesburke/PLEXI/issues/99) |
| `get_state`/`set_state` protocol implementation | Rust | Parallax App spec §9 |
| `cost_report` protocol implementation | Rust | Parallax App spec §10 |
| SDK `configure()` for undo/save/standard keys | Rust + Python | Parallax App spec §11 |

**Unlocks:** Real users, app ecosystem, marketplace

## Layer 5: Multiplayer + Companion

Depends on: Stable Layers 2-4

| Task | Type | Reference |
|------|------|-----------|
| Companion App Phase 1 (text + approvals + biometric) | Swift | [Companion App spec](docs/specs/companion-app.md) |
| Companion App Phase 2 (voice via Gemini Live) | Swift | Companion App spec §4 |
| Directory sync (Tailscale + file sync) | Rust | [Sync Architecture spec](docs/specs/sync-architecture.md) |
| Presence (who's in this directory) | Rust | Sync spec §Layer 3 |
| Agent orchestration trust system | Rust + Python | [Orchestration spec](docs/specs/agent-orchestration.md) |

---

## Spec Index

| Spec | Location | Status |
|------|----------|--------|
| App Infrastructure | `docs/specs/app-infrastructure.md` | Active — Phase 2 in progress |
| Agent Mode | `docs/specs/agent-mode.md` | Draft — ready for implementation |
| Agent Orchestration | `docs/specs/agent-orchestration.md` | Draft — core logic ready, improvement officer deferred |
| Companion App | `docs/specs/companion-app.md` | Draft — Phase 1 scoped |
| Intelligence Protocol | `docs/specs/intelligence-protocol.md` | **Deferred** — apps manage own LLM calls |
| Sync Architecture | `docs/specs/sync-architecture.md` | Draft — Phase 2+ |
| Telegram Integration | `docs/specs/telegram-integration.md` | Reference — companion app preferred |
| Parallax App | `parallax/docs/parallax-plexi-app-spec.md` | Draft — depends on manifest-first refactor |
| Parallax Packaging | `parallax/docs/parallax-plexi-packaging.md` | Draft — migration path defined |
| North Star | `~/.agents/skills/plexi-north-star/SKILL.md` | Active — ship order updated 2026-04-11 |
