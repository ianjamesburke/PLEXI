# Plexi App Infrastructure

**Status:** Phase 2 in progress  
**Last updated:** 2026-04-09

---

## Overview

Plexi apps are visual surfaces that live inside terminal panes. They coexist with the terminal — the same pane that previously showed a shell prompt can host a full app UI. The terminal stays alive underneath; Escape closes the app and returns to it. Tab toggles focus between app and terminal.

Two kinds of apps:

- **In-process apps** — Rust trait objects compiled into Plexi. Zero IPC overhead. Used for built-ins (file browser, audio player, permissions manager).
- **Out-of-process apps** — External binaries that communicate over stdin/stdout using a JSON newline-delimited protocol. These are third-party apps and are subject to the capability system.

Both implement the same `App` trait from Plexi's perspective. The surface rendering model is identical.

---

## App Protocol

Communication between Plexi and an out-of-process app is line-delimited JSON over stdin/stdout.

### Plexi → App (PlexiEvent)

| Event | Payload | Description |
|---|---|---|
| `Init` | `{ width, height, launch_dir, capabilities[] }` | Sent once on startup |
| `Render` | `{ width, height }` | Request a frame |
| `Key` | `{ key, modifiers }` | Forwarded key event (when app has focus) |
| `Command` | `{ name, args{} }` | Named command invocation |
| `Shutdown` | — | Graceful teardown signal |

### App → Plexi (DrawCommand)

Used to render custom UI. Plexi composites these into the pane surface.

| Command | Fields |
|---|---|
| `Rect` | `x, y, w, h, color` |
| `Text` | `x, y, content, color, bold` |
| `ProgressBar` | `x, y, w, value (0.0–1.0), color` |
| `FrameDone` | — (signals end of frame) |

### App → Plexi (API Requests)

Structured requests for host capabilities. Every request is capability-checked before execution.

| Request | Description |
|---|---|
| `ListDir { path }` | List directory contents |
| `ReadFile { path }` | Read file contents |
| `WriteFile { path, content }` | Write file |
| `RunCommand { command, args[] }` | Execute subprocess |
| `SecretGet { key }` | Retrieve named secret |
| `SecretStore { key, value }` | Store named secret |
| `SecureInput { prompt }` | Plexi renders masked input, returns final value |

Paths in all filesystem requests are resolved relative to the app's launch directory. Any path that escapes this root (e.g., `../etc/passwd`) is rejected before execution.

---

## Capability System

### App Manifest

Each app ships a `manifest.toml` that declares what it needs:

```toml
[app]
id = "com.example.myapp"
name = "My App"
version = "1.0.0"

[capabilities]
filesystem = "read_write"   # none | read_only | read_write
terminal_write = false
network = false
env_file_access = false
```

### User Permissions

`~/.plexi/permissions.toml` stores per-app overrides and global kill switches:

```toml
[apps."com.example.myapp"]
filesystem = "read_only"   # downgrade from declared

[global]
network = false            # kill switch — blocks all apps regardless of manifest
```

### Trust Levels

| Level | Who | Behavior |
|---|---|---|
| `builtin` | Compiled-in apps | Pre-approved, bypass capability checks |
| `trusted` | User-elevated third-party apps | Granted declared capabilities |
| `sandboxed` | Default for third-party | Scoped to launch directory, minimal permissions |

New third-party apps start as `sandboxed`. The user can elevate via the Permissions Manager app.

---

## Secrets Management

No .env files. No plain text secrets on disk.

### Storage

Secrets are stored in the macOS Keychain (libsecret on Linux). Each secret is namespaced by `app_id + launch_directory`, so the same app running in different project directories gets different secrets.

### App API

Apps request secrets through the structured protocol:

- `SecretGet { key: "ANTHROPIC_API_KEY" }` — Plexi resolves the right secret for this app + directory context
- `SecretStore { key: "ANTHROPIC_API_KEY" }` — triggers a `SecureInput` prompt, stores result in Keychain

The app never sees the raw Keychain call. Plexi mediates everything.

### SecureInput

When an app requests sensitive input, Plexi renders the masked input field itself. The app only receives the final value after the user confirms. The app cannot intercept keystrokes during entry.

### `plexi run` CLI

Reads `.plexi/commands.toml`, pulls declared secrets from Keychain, injects them as environment variables into the subprocess. No secrets touch the filesystem at any point.

```toml
# .plexi/commands.toml
[[commands]]
name = "deploy"
run = "scripts/deploy.sh"
secrets = ["VERCEL_TOKEN", "DATABASE_URL"]
```

`plexi run deploy` → fetches secrets → injects as env vars → runs `scripts/deploy.sh`.

---

## .plexi/ Directory Convention

A `.plexi/` directory at a project root declares Plexi configuration for that context.

| File | Purpose |
|---|---|
| `.plexi/commands.toml` | Named runnable commands with their required secrets |
| `.plexi/apps.toml` | Project-scoped app registrations (launch shortcuts, default args) |
| `.plexi/agents/` | (Future) Project-scoped agent definitions |

This is the analog of `.vscode/` or `.cursor/` — project-local tooling config that can be checked into version control. Secrets are never stored here.

---

## UI Paths

Apps can render UI two ways. Both go through the same capability system.

### Declarative (auto-generated form UI)

The `manifest.toml` can include a `[cli]` section with a JSON Schema–like definition of inputs. Plexi generates a form UI from this automatically: text fields, selects, secret inputs, buttons.

Good for: CLI wrappers, Firebase tools, deploy scripts — anything that maps cleanly to a set of inputs and a run action.

### Custom (draw protocol)

Full `DrawCommand` protocol for apps that need pixel-level control over layout. The app drives the render loop, Plexi composites the output.

Good for: file browsers, audio players, agent dashboards, anything with dynamic state.

---

## Permissions Manager

A built-in Plexi app for managing app capabilities and secrets. Accessible from the command palette.

**Apps tab:**
- Lists all installed apps (built-in + third-party)
- Shows declared capabilities vs. granted permissions
- Per-app override controls
- Revoke access button

**Secrets tab:**
- Shows which apps have which secret keys set, and in which directories
- Delete individual secrets
- No secret values are ever displayed

**Global tab:**
- Kill switches for `terminal_write`, `network`, `env_file_access`
- These override all per-app permissions

---

## Phase Plan

| Phase | Status | Scope |
|---|---|---|
| 1 | Done | `App` trait, `ProcessApp`, `AppRegistry`, `SurfaceMode`/`SurfaceLayer`, file browser, audio player PoC |
| 2 | In progress | Capability system + permission gate on `execute_app_command` |
| 3 | Planned | Structured filesystem API (`ListDir`, `ReadFile`, `WriteFile`) replacing raw terminal commands |
| 4 | Planned | Secrets management — Keychain integration, `SecureInput` API, `plexi run` CLI |
| 5 | Planned | Declarative CLI wrapper schema (`manifest.toml` `[cli]` section) |
| 6 | Planned | Permissions Manager app |
| 7 | Future | Process sandboxing (`sandbox_init` on macOS, `seccomp-bpf` on Linux) |
| 8 | Future | WASM runtime (`wasmtime`, browser deployment, hosted service) |
| 9 | Future | Agent infrastructure (`.plexi/agents/`, agent dashboard, scoped AI) |

---

## App Backlog

These are not scheduled — just tracked so they don't get lost:

- **Claude Code wrapper** — separated input/output panes, scrollable history
- **Audio recorder** — records to launch directory, respects `read_write` capability
- **Firebase / CLI tool wrappers** — auto-generated from manifest `[cli]` section
- **Permissions Manager** — see above; Phase 6
- **Agent dashboard** — monitor, inspect, and kill running agents
- **REPL loop runner** — submits prompt, re-prompts until a satisfaction criteria is met (useful for iterative codegen)
