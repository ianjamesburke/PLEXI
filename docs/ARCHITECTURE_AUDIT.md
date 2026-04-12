# Plexi Architecture & Security Audit

> Full-stack audit of signal flow, performance surfaces, and security posture.
> Generated 2026-04-11. Serves as a living reference for hardening the shell layer.

---

## Table of Contents

1. [System Overview](#1-system-overview)
2. [Signal Flow: Keyboard to Pixels](#2-signal-flow-keyboard-to-pixels)
3. [Signal Flow: Shell Output to Screen](#3-signal-flow-shell-output-to-screen)
4. [App Protocol & SDK](#4-app-protocol--sdk)
5. [Threading Model](#5-threading-model)
6. [Performance Audit](#6-performance-audit)
7. [Security Audit](#7-security-audit)
8. [Hardening Roadmap](#8-hardening-roadmap)

---

## 1. System Overview

Plexi is a **spatial terminal window manager** — a native macOS Rust binary that wraps the user's shell in a tiled, GPU-accelerated UI with an extensible app layer on top.

### Stack

| Layer | Technology | Role |
|-------|-----------|------|
| **Window/Event Loop** | eframe 0.31 (wgpu backend) | Native window, input events, GPU render |
| **UI Framework** | egui 0.31 (immediate mode) | Layout, painting, widgets |
| **Tiling** | egui_tiles 0.12 | Binary tree of panes (splits, tabs) |
| **Terminal Emulation** | alacritty_terminal 0.25 | VT100/220 state machine, PTY management |
| **Terminal Widget** | egui_term (local fork) | Bridges alacritty grid → egui painter |
| **GPU Renderer** | wgpu → Metal (macOS) | Hardware-accelerated rendering |
| **App Layer** | JSON-over-stdio protocol | External processes render into panes |

### Binary Profile

- **Target:** `aarch64-apple-darwin` (Apple Silicon native)
- **Size:** ~20 MB (unstripped, no LTO)
- **Min macOS:** 12.0 (Monterey)
- **Linking:** Static Rust deps, dynamic system frameworks (Metal, AppKit, CoreAudio)
- **Signing:** Ad-hoc only (no notarization)

---

## 2. Signal Flow: Keyboard to Pixels

```
User keystroke
  │
  ▼
eframe/winit captures OS key event
  │
  ▼
egui converts to Event::Key / Event::Text
  │
  ▼
PlexiApp::update() → poll_actions()
  │
  ├── [Plexi-level shortcut matched?] → execute action (split, tab, close, etc.)
  │
  └── [No match — forward to pane]
        │
        ▼
      TerminalView::process_input()           deps/egui_term/src/view.rs:159
        │
        ▼
      process_keyboard_event()                deps/egui_term/src/view.rs:513
        │
        ▼
      BindingsLayout::get_action()            deps/egui_term/src/bindings.rs:117
        │  Maps key+modifiers → ANSI escape sequence
        │  e.g. Ctrl+C → \x03, ArrowUp → \x1b[A
        │
        ▼
      BackendCommand::Write(bytes)
        │
        ▼
      TerminalBackend::write()                deps/egui_term/src/backend/mod.rs:549
        │  Pushes to Notifier (mpsc channel)
        │
        ▼
      alacritty EventLoop thread
        │  Writes bytes to PTY master fd
        │
        ▼
      Kernel PTY → shell process stdin
```

### Key Details

- **Login shell:** Always spawned with `-l` flag (`src/app.rs:235`)
- **Shell detection:** `$SHELL` → `/bin/zsh` → `/bin/bash` → `/bin/sh` (`src/shell.rs:8-28`)
- **TERM:** `xterm-ghostty` if Ghostty terminfo found, else `xterm-256color`
- **Zsh integration:** Custom ZDOTDIR injects `precmd` hook for OSC 7 CWD tracking (`src/shell.rs:60-192`)
- **egui caveat:** `consume_key(Modifiers::NONE, Key::Enter)` matches Enter even with Shift held — manual modifier checks required before `consume_key`

---

## 3. Signal Flow: Shell Output to Screen

```
Shell writes to stdout/stderr
  │
  ▼
PTY master fd (kernel buffer)
  │
  ▼
alacritty EventLoop thread (blocking read)
  │  Feeds bytes to vte::Parser
  │  Parser drives Term state machine
  │  Updates Grid<Cell> behind FairMutex
  │
  ▼
Event channel → pty_event_subscription thread   deps/egui_term/src/backend/mod.rs:191
  │  Filters special events (ColorRequest, PtyWrite, TextAreaSizeRequest)
  │  Forwards main events via mpsc to UI thread
  │  Calls ctx.request_repaint()
  │
  ▼
PlexiApp::update() — main UI thread
  │
  ├── drain_pty_events()                        src/app.rs:242
  │     try_recv() — non-blocking
  │     Handles: Exit (mark pane exited), Title (OSC command channel)
  │
  └── Per-pane rendering:
        │
        ▼
      TerminalView::show()                      deps/egui_term/src/view.rs:306
        │
        ├── backend.sync()                      deps/egui_term/src/backend/mod.rs:316
        │     Lock FairMutex, clone Grid<Cell> atomically
        │     Snapshot: grid + cursor + selection + terminal modes
        │
        ├── Cell-by-cell iteration
        │     For each visible cell in grid:
        │       - Compute rect from font metrics
        │       - Apply ANSI colors (fg, bg, inverse, dim, selection)
        │       - Box-drawing chars → geometric shapes
        │       - Regular chars → Shape::text()
        │       - Cursor → block/beam/underline (530ms blink)
        │
        └── painter.extend(shapes)              deps/egui_term/src/view.rs:509
              All shapes submitted to egui → wgpu → Metal → screen
```

### Key Details

- **Grid snapshot:** Atomic clone prevents tearing — EventLoop can't modify during copy
- **No texture atlas:** Glyphs rendered dynamically as `Shape::text()` per cell (egui caches internally)
- **Font:** JetBrainsMono Nerd Font (Light), bundled via `include_bytes!()`
- **Repaint:** Event-driven from PTY + 16ms timer for animations = ~60 FPS ceiling
- **CWD tracking:** `lsof -a -d cwd -Fn -p <pid>` on macOS, `/proc/<pid>/cwd` on Linux, cached 2s (`src/shell.rs:109-161`)

---

## 4. App Protocol & SDK

### Architecture

```
Plexi (parent process)
  │
  ├── stdin  → JSON events   (PlexiEvent)     → App subprocess
  ├── stdout ← JSON commands  (DrawCommand)    ← App subprocess
  └── stderr ← forwarded to plexi.log as warn  ← App subprocess
```

Apps are executables registered via `manifest.toml` in `~/.plexi/apps/<app-id>/`. Communication is **newline-delimited JSON**, one object per line.

### Events: Plexi → App

| Event | Fields | When |
|-------|--------|------|
| `Init` | `width, height, pixels_per_point` | Once on startup |
| `Render` | `width, height` | Each frame (~60fps) |
| `Resize` | `width, height` | Surface resized |
| `Key` | `key, modifiers{shift,ctrl,alt,cmd}` | Keypress |
| `Click` | `x, y, button` | Mouse click |
| `Command` | `text` | Command bar submission |
| `Shutdown` | — | Graceful close |

### Draw Commands: App → Plexi

| Command | Fields | Purpose |
|---------|--------|---------|
| `Rect` | `x, y, w, h, fill, radius` | Fill rectangle |
| `Text` | `x, y, text, size, color, monospace, bold` | Draw text |
| `Line` | `x1, y1, x2, y2, color, width` | Draw line |
| `List` | `items[], selected, item_height` | Scrollable list (Plexi handles layout) |
| `RunInTerminal` | `command` | Execute in linked terminal |
| `Cd` | `path` | Change linked terminal CWD |
| `Log` | `level, message` | Forward to plexi.log |
| `FrameDone` | — | End-of-frame marker |

### Two-Buffer Frame Design

- **`pending_frame`** accumulates commands as they arrive
- **`frame`** holds last complete frame (after FrameDone)
- Atomic swap on FrameDone prevents partial frames from rendering

### SDKs

**Python** (`sdk/python/plexi_sdk.py`): Zero-dependency, pure stdlib. Decorator-based (`@app.on_render`, `@app.on_key`).

**Rust** (`sdk/rust/src/lib.rs`): Trait-based (`impl App for MyApp`). Builder pattern for draw commands.

Both SDKs handle the JSON protocol, frame lifecycle, and provide `RenderContext` (in-frame drawing) and `Emitter` (event-handler commands).

### Permissions Model

```
src/app_permissions.rs
```

| Trust Level | Description |
|-------------|-------------|
| `Builtin` | Compiled into Plexi; all permissions pre-approved |
| `Trusted` | User-elevated third-party app |
| `Sandboxed` | Default; strict scope to launch directory |

| Permission | Default (Sandboxed) | Controls |
|------------|-------------------|----------|
| `terminal_write` | false | Can send commands to linked terminal |
| `filesystem` | None | None / ReadOnly / ReadWrite |
| `env_file_access` | false | Can read .env / credentials |
| `network` | false | Can make network requests |
| `secrets_write` | false | Can write to Keychain |

**Scope enforcement:** All file paths validated via `canonicalize()` against app's `scope_root`. Symlinks and `..` resolved before comparison.

**Config:** `~/.plexi/permissions.toml` — per-app overrides + global kill switches.

**Important:** Permissions are **advisory/UI-level**, not OS-enforced. Apps run as child processes with the user's full UID.

---

## 5. Threading Model

```
┌─────────────────────────────────────────────────┐
│  Main UI Thread (eframe event loop)             │
│    - drain_pty_events() each frame              │
│    - dispatch_app_key_events()                  │
│    - render all panes (TerminalView::show())    │
│    - egui → wgpu → Metal → screen              │
└───────────────┬───────────────┬─────────────────┘
                │               │
    ┌───────────▼──┐    ┌───────▼────────────────┐
    │ Per-PTY      │    │ Per-PTY Event          │
    │ EventLoop    │    │ Subscription Thread    │
    │ (alacritty)  │    │                        │
    │              │    │ Receives: PTY events   │
    │ Reads PTY fd │    │ Filters: ColorRequest, │
    │ Parses ANSI  │    │   PtyWrite, SizeQuery  │
    │ Updates Grid │    │ Forwards: Exit, Title  │
    │              │    │ Triggers: repaint       │
    └──────────────┘    └────────────────────────┘

    ┌──────────────┐    ┌────────────────────────┐
    │ Per-App      │    │ Per-App                │
    │ stdout       │    │ stderr                 │
    │ reader       │    │ reader                 │
    │              │    │                        │
    │ Parses JSON  │    │ Forwards lines to      │
    │ Sends via    │    │ log::warn! with        │
    │ mpsc channel │    │ target "app::<id>"     │
    └──────────────┘    └────────────────────────┘
```

**Synchronization:**
- Terminal grid: `Arc<FairMutex<Term>>` — EventLoop writes, UI thread reads via atomic clone in `sync()`
- PTY events: `mpsc::channel` — subscription thread → UI thread (non-blocking `try_recv`)
- App draw commands: `mpsc::channel` — stdout reader → UI thread
- Terminal size: `Arc<Mutex<TerminalSize>>` — shared for SIGWINCH handling

---

## 6. Performance Audit

### Current State: What's Good

| Area | Status | Notes |
|------|--------|-------|
| GPU rendering | Good | wgpu/Metal — hardware accelerated |
| Terminal emulation | Good | alacritty_terminal is battle-tested, fast |
| Grid snapshot | Good | Atomic clone prevents tearing without blocking EventLoop |
| Event-driven repaint | Good | Only repaints when PTY output arrives or animations active |
| Dependency features | Good | Image/audio codecs selectively enabled |
| Font | Good | Bundled Nerd Font — no filesystem lookup per frame |

### Surfaces to Optimize

#### P1 — Binary Size (20 MB unstripped)

**Problem:** No release profile optimization configured.

**Fix:** Add to `Cargo.toml`:
```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

**Expected impact:** ~20 MB → ~12-14 MB (30-40% reduction).

#### P2 — Per-Frame Grid Clone

**Location:** `deps/egui_term/src/backend/mod.rs:316-337`

Each frame, the entire terminal grid is cloned (`terminal.grid().clone()`). For a 200-row × 200-col terminal with scrollback, this is a non-trivial allocation.

**Current impact:** Low — grids are typically small. Becomes a concern with very large terminals or many panes.

**Future optimization:** Dirty-flag on grid; only clone when EventLoop has written new data since last sync. alacritty_terminal doesn't expose this natively, so it would require a fork modification or a generation counter.

#### P3 — No Glyph Texture Atlas

**Location:** `deps/egui_term/src/view.rs:306-510`

Every cell is rendered as a separate `Shape::text()`. egui's internal glyph cache mitigates this, but a custom texture atlas (pre-rendering the ASCII range into a spritesheet) would reduce per-frame work significantly for text-heavy content.

**Current impact:** Acceptable for typical terminal workloads. Stress-test with `cat /dev/urandom | xxd` to see ceiling.

**Future optimization:** Pre-rasterize ASCII 0x20-0x7E into a texture atlas, blit from atlas instead of per-glyph. This is the single biggest rendering optimization available.

#### P4 — Shape Vector Allocation

**Location:** `deps/egui_term/src/view.rs:329`

`Vec<Shape>` is allocated fresh each frame. For a 200×50 grid, that's 10,000+ shapes.

**Future optimization:** Pre-allocate and reuse the shape buffer across frames (`clear()` instead of `new()`).

#### P5 — App Protocol: No Payload Limits

**Location:** `src/process_app.rs:82-105`

A malicious or buggy app can send a 10 GB JSON line or spam thousands of draw commands. No rate limiting or size caps.

**Fix:** Cap line length (e.g., 1 MB), cap commands per frame (e.g., 10,000), drop excess.

#### P6 — CWD Lookup via lsof

**Location:** `src/shell.rs:136-150`

`lsof -a -d cwd -Fn -p <pid>` is spawned as a subprocess for CWD detection. This is expensive (~5ms per call) but cached with a 2s TTL.

**Current impact:** Low — only called on pane split. Would matter if called per-frame.

#### P7 — 60 FPS Request When Apps Active

**Location:** `src/process_app.rs` — `request_repaint_after(16ms)`

When any app is active, Plexi requests repaint at ~60 FPS regardless of whether the app has new content. Idle apps burn CPU.

**Future optimization:** Only request repaint when the app's stdout channel has pending data.

#### P8 — No Custom wgpu Configuration

eframe defaults are used for the wgpu backend. No explicit vsync, present mode, or power preference configuration.

**Future optimization:** Set `power_preference: LowPower` for battery life, or `HighPerformance` for plugged-in. Expose as a config option.

---

## 7. Security Audit

### Threat Model

Plexi is a **terminal multiplexer**, not a sandbox. Apps run as child processes with the user's full UID. The permission system is **advisory** — it prevents accidents, not attacks. This is the correct model for the use case (comparable to tmux plugins or shell aliases).

The primary threats are:
1. **Malicious third-party apps** installed via `plexi app install`
2. **Escape sequence injection** from untrusted terminal output
3. **Supply chain attacks** via compromised dependencies
4. **Data leakage** from environment variables or Keychain metadata

### Findings

#### S1 — CRITICAL: OSC Title Command Injection

**Location:** `src/app.rs:255-261`

```rust
PtyEvent::Title(title) => {
    if let Some(cmd) = title.strip_prefix("plexi:") {
        match cmd {
            "close" => panes_to_close.push(id),
            _ => {}
        }
    }
}
```

**Problem:** Any process running in a terminal pane can emit `\x1b]0;plexi:close\x07` to close the pane. This is the OSC 0/2 "set window title" escape sequence, which alacritty_terminal parses and forwards as `PtyEvent::Title`. A malicious script (e.g., in a curl-piped-to-bash scenario) could close panes silently.

**Blast radius today:** Limited to `close`. But if more commands are added to this channel, the surface grows.

**Fix:** Move Plexi commands to a dedicated side-channel (e.g., a custom OSC number like `\x1b]7770;...`) that normal shell output would never produce. Or require a HMAC token in the command.

#### S2 — HIGH: No Sanitization on RunInTerminal

**Location:** `src/app.rs:322-343`

```rust
AppCommand::RunInTerminal(command) => {
    let mut bytes = command.into_bytes();
    bytes.push(b'\n');
    pane.backend.process_command(BackendCommand::Write(bytes));
}
```

**Problem:** The command string is written directly to the PTY with a trailing newline — no escape sequence stripping, no validation. A sandboxed app with `terminal_write: true` can inject arbitrary escape sequences, including bracketed paste sequences.

**Fix:** Strip all bytes < 0x20 (except 0x09 tab) and all ESC sequences from RunInTerminal payloads before writing to PTY. Or route through a safer execution path (write to shell stdin via a dedicated mechanism, not raw PTY).

#### S3 — HIGH: Environment Variable Leakage

**Location:** `src/shell.rs:30-78`

Apps inherit the full process environment. This includes any `AWS_*`, `GITHUB_TOKEN`, `*_SECRET`, `*_KEY` variables the user has set.

**Fix:** Whitelist environment variables for app subprocesses. Only pass: `TERM`, `LANG`, `LC_ALL`, `HOME`, `USER`, `LOGNAME`, `PATH`, `PLEXI_RUNNING`. Explicitly exclude credential-shaped variables.

#### S4 — HIGH: TOCTOU in Path Scope Checking

**Location:** `src/app_permissions.rs:151-164`, `src/app_api.rs:208-273`

`canonicalize()` resolves symlinks at check time, but the actual file operation happens after the check. Between check and use, an attacker could swap a symlink to point outside the scope.

**Fix:** Use `O_NOFOLLOW` or `openat2(RESOLVE_NO_SYMLINKS)` for file operations. Or open the file first, then verify the fd's path matches the scope.

#### S5 — MEDIUM: No JSON Payload Size Limits

**Location:** `src/process_app.rs:82-105`

A malicious app can send arbitrarily large JSON lines, causing OOM.

**Fix:** Cap `BufReader` line length at 1 MB. Drop lines that exceed the limit.

#### S6 — MEDIUM: No App Binary Integrity Verification

**Location:** `src/app_registry.rs:118-140`

Apps are loaded from `~/.plexi/apps/` with no hash or signature verification. An attacker with write access to that directory can replace any app binary.

**Fix:** Store SHA-256 hash in `manifest.toml` and verify on load. For installed apps, record hash at install time.

#### S7 — MEDIUM: No Code Signing / Notarization

The release binary is ad-hoc signed (`codesign --sign -`). No Apple Developer ID, no notarization. Users must bypass Gatekeeper manually.

**Fix (for distribution):** Sign with Developer ID, notarize via `xcrun notarytool`. Required before any serious distribution.

#### S8 — MEDIUM: Keychain Account String Exposes Metadata

**Location:** `src/secrets.rs:15-18`

Account key format: `{app_id}/{directory}/{key}` — stored unencrypted in Keychain metadata. An attacker with Keychain read access can enumerate all app secrets and their paths.

**Fix:** Hash the account key: `sha256("{app_id}/{directory}/{key}")`.

#### S9 — LOW: Secrets Passed as CLI Arguments

**Location:** `src/secrets.rs` — `security add-generic-password -w <value>`

Secret values appear in the process argument list (visible via `ps aux`). The `zeroize` crate is used for retrieval but not for storage.

**Fix:** Pipe secrets to `security`'s stdin instead of passing as `-w` argument.

#### S10 — LOW: Custom Shell Escape Function

**Location:** `src/app.rs:380-386`

A hand-rolled `shell_escape()` function is used for `cd` commands. While it appears correct, custom escaping is a security anti-pattern.

**Fix:** Use the `shlex` crate or pass paths via `execvp` argv array instead of shell interpolation.

---

## 8. Hardening Roadmap

Priority-ordered list of improvements. Each item tagged with effort estimate (S/M/L) and whether it's a **security** or **performance** improvement.

### Phase 1 — Quick Wins (ship in next release)

| # | Item | Type | Effort | Reference |
|---|------|------|--------|-----------|
| 1 | Add `[profile.release]` with LTO + strip | Perf | S | P1 |
| 2 | Cap JSON line length at 1 MB | Security | S | S5 |
| 3 | Strip ESC sequences from RunInTerminal payloads | Security | S | S2 |
| 4 | Whitelist env vars for app subprocesses | Security | S | S3 |
| 5 | Hash Keychain account keys | Security | S | S8 |
| 6 | Pipe secrets to `security` stdin | Security | S | S9 |

### Phase 2 — Medium-Term (next few releases)

| # | Item | Type | Effort | Reference |
|---|------|------|--------|-----------|
| 7 | Move Plexi commands to dedicated OSC number | Security | M | S1 |
| 8 | Add SHA-256 hash verification for app binaries | Security | M | S6 |
| 9 | Replace custom shell_escape with `shlex` crate | Security | S | S10 |
| 10 | Dirty-flag on grid to skip unnecessary clones | Perf | M | P2 |
| 11 | Only repaint when app stdout has pending data | Perf | M | P7 |
| 12 | Pre-allocate and reuse shape buffer | Perf | S | P4 |

### Phase 3 — Long-Term (architecture)

| # | Item | Type | Effort | Reference |
|---|------|------|--------|-----------|
| 13 | Glyph texture atlas for ASCII range | Perf | L | P3 |
| 14 | Apple Developer ID signing + notarization | Security | M | S7 |
| 15 | macOS sandbox-exec or Linux seccomp for apps | Security | L | — |
| 16 | wgpu power preference config option | Perf | S | P8 |
| 17 | Fix TOCTOU with O_NOFOLLOW | Security | M | S4 |
| 18 | App manifest registry with GPG signatures | Security | L | — |

---

## Appendix A: File Reference

| File | Purpose |
|------|---------|
| `src/main.rs` | Entry point, window config, eframe setup |
| `src/app.rs` | Main app struct, update loop, event dispatch |
| `src/shell.rs` | Shell detection, env vars, zsh integration, CWD tracking |
| `src/pane.rs` | TerminalPane struct, app overlay management |
| `src/pane_ops.rs` | Split, tab, pane creation/destruction |
| `src/tiling.rs` | Pane layout rendering, egui_tiles behavior |
| `src/context.rs` | Workspace context, tile tree, pane navigation |
| `src/process_app.rs` | External app lifecycle, JSON protocol, rendering |
| `src/app_protocol.rs` | PlexiEvent / DrawCommand enum definitions |
| `src/app_permissions.rs` | Permission model, scope enforcement |
| `src/app_registry.rs` | App discovery, manifest loading, launch |
| `src/app_api.rs` | Future structured API (file/secret operations) |
| `src/app_trait.rs` | App trait, AppCommand, SurfaceMode |
| `src/secrets.rs` | Keychain read/write, zeroize |
| `src/config.rs` | Config loading, build-variant directory resolution |
| `src/theme.rs` | Color presets, font loading, terminal colors |
| `src/macos_menu.rs` | Native macOS menu customization (unsafe FFI) |
| `deps/egui_term/src/backend/mod.rs` | PTY creation, EventLoop, grid sync |
| `deps/egui_term/src/view.rs` | Terminal widget, input handling, cell rendering |
| `deps/egui_term/src/bindings.rs` | Key binding system (ANSI escape mapping) |
| `deps/egui_term/src/graphics.rs` | Box-drawing and geometric character rendering |
| `sdk/python/plexi_sdk.py` | Python SDK (zero-dependency) |
| `sdk/rust/src/lib.rs` | Rust SDK (trait-based) |

## Appendix B: Dependency Summary

**Direct dependencies:** 15 crates
**Total resolved:** 452 crates
**Unsafe blocks:** 17 (all in macOS FFI — objc2 interop, justified)
**Feature gating:** Image formats (6/10), audio codecs (4), AppKit features (5) — well-curated

## Appendix C: Key Architectural Invariants

1. **Grid is always a complete snapshot.** `sync()` locks the FairMutex and clones atomically. Partial frames never reach the painter.
2. **App frames are always complete.** Two-buffer swap on FrameDone means only fully-committed frames render.
3. **Permissions are advisory, not enforced.** Apps run as user-level processes. The permission system prevents mistakes, not attacks. This is intentional and correct for a terminal multiplexer.
4. **Login shell always.** `-l` flag ensures full user environment (PATH, aliases). Breaking this would break user expectations.
5. **Event-driven repaint.** No busy-loop — egui only repaints when triggered by PTY events, input, or timer.
