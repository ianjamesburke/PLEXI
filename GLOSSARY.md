# Plexi Glossary

Shared vocabulary for Plexi. When introducing or redefining terms, update this file alongside DEV_LOG.md.

---

## Core Concepts

**Pane** — A single view in the tiling layout. Two main types: Terminal (shell/PTY) and App. App panes can run PGAP processes, WASM components, or first-party host apps. Panes are split horizontally or vertically to create the workspace grid. AI capability is app-level (`ai.query`), not a separate pane type.

**Context** — A workspace or project directory. Each context has its own state, secrets (keyed by workspace root), and app instances. Contexts are independent — a secret granted in one context never leaks to another.

**Split** — A horizontal or vertical division that creates two panes from one. Splits are reversible; closing either pane collapses back to the parent.

**Workspace** — Synonym for context. A rooted directory with its own configuration, secrets, and running state.

## Protocol & Communication

**PGAP** — Plexi Generic App Protocol. Newline-delimited JSON over a child process's stdin/stdout. Apps declare what they can do; the host sends events (input, renders, permissions); apps send draw commands and requests back. Binary payloads (audio PCM, video frames) travel on typed pipes, not stdio. PGAP is the host API boundary; Python apps are native subprocesses until the WASM runtime provides process isolation.

**WASM app** — A Plexi app implemented as a WebAssembly component. WASM apps export typed lifecycle functions and receive only the host imports Plexi links for their remembered grants. They share the app manifest, package, trust-label, and capability model with PGAP apps, but run through the sandbox/performance runtime instead of a native subprocess.

**PlexiEvent** — Host → app message. Types: init, render, input (keyboard/mouse), capability decision, secret value, run update, pipe message, path changed, suspend/resume, shutdown.

**DrawCommand** — App → host message. Three top-level variants: `Render(RenderCommand)` (frame primitives: rect, text, circle, line, etc.), `Host(AppRequest)` (capability requests, SecretGet, RunGet, Notify, PipeOpen, PipeSend, OpenVideo/SetVideoState/CloseVideo, AudioPlay, AudioCapture, StatusSummary), `Control(ControlCommand)` (clipboard, scheduling). Every frame ends with `FrameDone`.

**Typed pipes** — Binary channels for audio, video, and custom data. Created by `PipeOpen`, messages routed by `PipeSend`. Each pipe has a declared type (audio, video, custom). Host enforces type safety; apps cannot send wrong data down a pipe.

## App & Capability Model

**Manifest** — TOML file (`manifest.toml`) that declares an app: name, version, runtime type, entry point, icon, required capabilities, and background flag. PGAP and WASM apps both use manifests on the installed/packaged path.

**Capability** — A permission an app can request at runtime (e.g., `net`, `fs`, `llm`, `timer`, `audio_capture`). Undeclared capabilities queue a modal prompt on first use. Decisions persist in `permissions.jsonl`.

**App pane** — A pane hosting an app runtime instance. PGAP apps spawn an external process and speak the PGAP protocol; WASM apps run in wasmtime and speak the typed WIT runtime.

**Terminal pane** — A pane hosting a PTY (shell). Input is vt100 bytes; output is terminal escape sequences.

## State & Persistence

**Secret** — A workspace-scoped credential stored in macOS Keychain, keyed by `(workspace_root, secret_key)`. Host validates the app's actual CWD at spawn to prevent escalation. A secret granted in `/foo` is not readable by any app at any other workspace root without a new prompt.

**Event bus** — Append-only `events.jsonl` log. Records every HostEvent: app spawn/close, permission decision, secret prompt/deny, run lifecycle, notification + action invoke, agent turn, pipe open/close.

**Ledger** — `ai-ledger.jsonl` in the build-appropriate config dir (e.g. `~/.plexi-alpha/ai-ledger.jsonl`). Tracks AI turns, cost, tokens, metadata for cost accounting and debugging.

## UI & Layout

**Tiling** — The grid of panes. Built from recursive binary splits (vertical or horizontal). Navigable with directional keys (`Cmd+H/J/K/L`). Panes can be moved, resized, zoomed, or closed.

**Zoom** — Full-screen view of a single pane. All other panes hidden. `Cmd+Enter` toggles. Useful for focus when working across many splits.

**Palette** — Modal overlay for notifications, capability prompts, or structured user choices. Keyboard-navigable, stackable (multiple modals can queue).

**StatusSummary** — App-provided short status line (e.g., "Synced • 3 files") that appears in the pane header. Apps send this via DrawCommand to keep the UI informed without a full re-render.

## Infrastructure

**HostModel** — Pure state machine with zero egui dependency. Commands in, effects out. All business logic (pane lifecycle, permissions, events) lives here. Renderer (egui in prod, tiny-skia in CI) reads state and paints; never owns logic.

**Pane group** — A named group of panes. Apps opt in at spawn with `group: "name"`. `PathChanged` broadcasts within the group, allowing apps to react to CWD changes without knowing about each other.

**Run primitive** — A handle to a long-running subprocess. Apps use `RunGet` to check status or subscribe to updates. Host tracks spawned processes and surfaces completion/failure to the UI as rich notifications.

---

## How to Update This

When a DEV_LOG entry introduces or significantly changes terminology, update this file in the same commit. Keep definitions concise and cross-reference related terms.
