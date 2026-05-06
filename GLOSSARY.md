# Plexi Glossary

Shared vocabulary for Plexi. When introducing or redefining terms, update this file alongside DEV_LOG.md.

---

## Core Concepts

**Pane** — A single view in the tiling layout. Three types: Terminal (shell/PTY), App (external PGAP process), Agent (LLM turn loop). Panes are split horizontally or vertically to create the workspace grid.

**Context** — A workspace or project directory. Each context has its own state, secrets (keyed by workspace root), and app instances. Contexts are independent — a secret granted in one context never leaks to another.

**Split** — A horizontal or vertical division that creates two panes from one. Splits are reversible; closing either pane collapses back to the parent.

**Workspace** — Synonym for context. A rooted directory with its own configuration, secrets, and running state.

## Protocol & Communication

**PGAP** — Plexi Generic App Protocol. Newline-delimited JSON over a child process's stdin/stdout. Apps declare what they can do; the host sends events (input, renders, permissions); apps send draw commands and requests back. Binary payloads (audio PCM, video frames) travel on typed pipes, not stdio. The isolation boundary — no shared memory, no inherited file descriptors.

**PlexiEvent** — Host → app message. Types: init, render, input (keyboard/mouse), capability decision, secret value, run update, pipe message, path changed, suspend/resume, shutdown.

**DrawCommand** — App → host message. Types: frame primitives (rect, text, circle, line), VideoPlayer, AudioPlay, AudioCapture, log, capability request, SecretGet, RunGet, Notify, PipeOpen, PipeSend, StatusSummary, FrameDone.

**Typed pipes** — Binary channels for audio, video, and custom data. Created by `PipeOpen`, messages routed by `PipeSend`. Each pipe has a declared type (audio, video, custom). Host enforces type safety; apps cannot send wrong data down a pipe.

## App & Capability Model

**Manifest** — TOML file (`manifest.toml`) that declares an app: name, version, entry point, icon, required capabilities, background flag. Every PGAP app must have one.

**Capability** — A permission an app can request at runtime (e.g., `net`, `fs`, `llm`, `timer`, `audio_capture`). Undeclared capabilities queue a modal prompt on first use. Decisions persist in `permissions.jsonl`.

**App pane** — A pane hosting an external PGAP process. Spawned from a manifest, communicates via PGAP protocol.

**Terminal pane** — A pane hosting a PTY (shell). Input is vt100 bytes; output is terminal escape sequences.

**Agent pane** — A pane hosting an LLM turn loop via IQ query. Requests are passed to the backend (OpenRouter now; Ollama next); results stream to the UI. Persists session ID and transcript. Cost and token usage logged to ledger per turn.

## State & Persistence

**Secret** — A workspace-scoped credential stored in macOS Keychain, keyed by `(workspace_root, secret_key)`. Host validates the app's actual CWD at spawn to prevent escalation. A secret granted in `/foo` is not readable by any app at any other workspace root without a new prompt.

**Event bus** — Append-only `events.jsonl` log. Records every HostEvent: app spawn/close, permission decision, secret prompt/deny, run lifecycle, notification + action invoke, agent turn, pipe open/close.

**Ledger** — `ledger.jsonl` in the agent's directory. Tracks agent turns, cost, tokens, metadata for cost accounting and debugging.

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
