# Plexi WASM + PWA Mobile Deployment

**Status:** Phase 1 complete (scaffolding)
**Last updated:** 2026-04-14

---

## Alternate fast path — PWA over WebSocket bridge (~4 weeks)

Captured 2026-04-14. The full WASM-Plexi vision below is the multi-month endgame. There's a much faster path that gives you "interact with my desktop Plexi from my phone" in 4 focused weeks without compiling Plexi itself to WASM:

1. **Week 1 — WebSocket transport in Plexi.** Add a WebSocket server to the Plexi binary, gated behind `[network] enabled = true` in config.toml. The server mirrors the existing per-pane DrawCommand stream over WebSocket — Plexi already has the internal stream, this just exposes it on a new transport. Auth is a shared secret in the config file plus Tailscale for NAT traversal.
2. **Week 2 — PWA renderer.** A small HTML/JS/Canvas PWA that connects to `wss://your-tailnet.ts.net:port`, receives DrawCommands and renders them into a canvas (the same draw protocol you write on screen, just on a phone canvas), captures touches as mouse events and on-screen keyboard as key events, sends them back. "Add to Home Screen" installs it like a native app on iOS/Android with no app store.
3. **Week 3 — Terminal pane mirroring.** Forward the cell grid (rows × cols × {char, fg, bg, attrs}) over the same WebSocket as a typed channel. The PWA renders the grid with a monospace canvas font. Keystrokes flow back into the PTY. This is the "type into my terminal from my phone" experience.
4. **Week 4 — Multi-pane swipe UI + on-screen modifier toolbar (Tab/Esc/Ctrl/Cmd/Alt buttons, since iOS keyboards lack them).**

**Honest blockers** for this path:
- Auth — the easy answer is Tailscale; the harder answer is a real token-based system. v1 should just say "use Tailscale" and document it.
- iOS keyboard UX — no Tab/Esc/modifiers without a third-party keyboard. The PWA needs an on-screen toolbar. Real UX work.
- Performance — delta-encode the cell grid changes, otherwise mobile data eats the connection.

**What this path does NOT need:** a full WASM Plexi build, native iOS/Android apps, real-time audio/video panes on mobile, multi-user collab. All of those are the WASM endgame below.

**Trade-off vs. the WASM path:** the PWA path requires the desktop Plexi to be running — the phone is a thin client. The WASM path eventually runs Plexi-the-app entirely in the phone browser. The PWA is faster to build and validate; the WASM path is the sovereign endgame. Both end at the same place; the PWA path gets you to "I can SSH from my phone via Plexi" 3-6 months sooner.

**Status:** deferred. Not blocking any other work. Pull off the back burner when (a) someone actually wants mobile access, or (b) the desktop ecosystem is mature enough that mobile becomes a force multiplier.

---

## 1. Overview

Compile Plexi's UI layer to WebAssembly, serve it as a Progressive Web App installable from Safari and Chrome. The WASM client connects to a native Rust backend over WebSocket.

**Why this approach:**
- Skip the App Store entirely — no review process, no Apple Developer account, no Xcode/Swift
- Skip native mobile frameworks — no SwiftUI, no Kotlin, no React Native
- Works on iOS, Android, and any desktop browser from the same Rust codebase
- eframe already supports `wasm32-unknown-unknown` as a compilation target
- The app protocol is already JSON — switching from stdin/stdout to WebSocket is a transport change, not a protocol rewrite

**This replaces the native iOS companion app as the mobile strategy.** The companion app spec (`companion-app.md`) is kept as reference for the pairing ceremony and auth design, which this spec reuses. The WASM PWA covers the same use cases (agent chat, approvals, notifications) without requiring a native iOS binary.

---

## 2. Architecture

```
Phone (Safari PWA)              Host Machine
┌────────────────────────┐      ┌────────────────────────┐
│  WASM Frontend         │      │  Rust Backend           │
│  • egui rendering      │◄─WS─►│  • PTY / shell         │
│  • touch input         │ JSON │  • Python apps (spawn)  │
│  • draw commands       │      │  • filesystem ops       │
│  • local state cache   │      │  • secrets (Keychain)   │
│  • service worker      │      │  • audio (rodio)        │
└────────────────────────┘      └────────────────────────┘
```

### Frontend (WASM)

egui compiled to `wasm32-unknown-unknown` via eframe's web backend. Runs in the browser's WebAssembly sandbox. Handles all rendering, layout, touch/keyboard input, and UI state. Communicates with the backend exclusively over WebSocket.

The frontend is a static bundle: one `.wasm` binary, one `.js` loader, one `index.html`, and PWA assets (manifest, service worker, icons). Served by the backend's HTTP server or any static file host.

### Backend (Native Rust)

The existing Plexi binary with an additional `serve` mode. Runs on the host machine (Mac/Linux). Manages everything that requires OS access: PTY spawning, subprocess execution, filesystem, Keychain secrets, audio playback, file watching.

Exposes two endpoints:
- **HTTP** — serves the static WASM bundle and PWA assets
- **WebSocket** — carries the app protocol (PlexiEvent / DrawCommand JSON messages)

### Bridge Protocol

The protocol is the same JSON format already defined in `app_protocol.rs` — `PlexiEvent` (host-to-client) and `DrawCommand` (client-to-host). Today these flow over stdin/stdout pipes to external app processes. For WASM deployment, they flow over WebSocket instead. The message format does not change.

This means the protocol has been validated by every external app already built. The WASM client is just another consumer of the same protocol.

---

## 3. What `#[cfg(target_arch = "wasm32")]` Means

This section explains Rust's conditional compilation for developers who will work on the WASM split.

### Compilation targets

Rust compiles to many targets. The two relevant ones:

| Target | What it produces | Where it runs |
|---|---|---|
| `aarch64-apple-darwin` | Native macOS binary | macOS directly |
| `wasm32-unknown-unknown` | WebAssembly bytecode | Browser via JS runtime |

When you run `cargo build`, Rust compiles for your host machine's native target. When you run `cargo build --target wasm32-unknown-unknown`, it cross-compiles to WebAssembly. The resulting `.wasm` file runs in any browser's WASM runtime.

### Conditional compilation attributes

Rust's `#[cfg(...)]` attribute controls which code compiles for which target. Code inside a `cfg` gate is completely absent from the binary when the condition is false — it doesn't exist at all, not even as dead code.

```rust
// This code ONLY exists in the WASM binary
#[cfg(target_arch = "wasm32")]
fn connect_websocket(url: &str) -> WebSocket { ... }

// This code ONLY exists in the native binary
#[cfg(not(target_arch = "wasm32"))]
fn spawn_pty(shell: &str) -> Pty { ... }

// This code exists in BOTH binaries
fn render_pane(ui: &mut egui::Ui, lines: &[Line]) { ... }
```

### How this splits the codebase

The goal is surgical: most code compiles everywhere. Only the I/O boundary — where Plexi touches the OS — gets gated.

```rust
// ── UI rendering (compiles everywhere) ──────────────────────────

fn render_file_browser(ui: &mut egui::Ui, entries: &[FileEntry]) {
    for entry in entries {
        ui.label(&entry.name);
    }
}

// ── Data fetching (platform-specific) ───────────────────────────

// Native: reads the actual filesystem
#[cfg(not(target_arch = "wasm32"))]
fn list_directory(path: &Path) -> io::Result<Vec<FileEntry>> {
    let entries = std::fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .map(|e| FileEntry { name: e.file_name().to_string_lossy().into() })
        .collect();
    Ok(entries)
}

// WASM: requests the file list from the backend over WebSocket
#[cfg(target_arch = "wasm32")]
async fn list_directory(ws: &WebSocket, path: &str) -> Result<Vec<FileEntry>> {
    ws.send(json!({ "ListDir": { "path": path } })).await?;
    let response = ws.recv().await?;
    Ok(serde_json::from_str(&response)?)
}
```

### Module-level gating

For modules that are entirely native-only (like `shell.rs` which is all PTY code), gate the entire module declaration in `main.rs`:

```rust
// main.rs
mod app;           // always
mod theme;         // always
mod tiling;        // always

#[cfg(not(target_arch = "wasm32"))]
mod shell;         // native only — PTY spawning

#[cfg(not(target_arch = "wasm32"))]
mod process_app;   // native only — subprocess management

#[cfg(target_arch = "wasm32")]
mod ws_bridge;     // WASM only — WebSocket transport
```

### Cargo.toml dependency gating

Dependencies that don't compile to WASM get gated in `Cargo.toml`:

```toml
[dependencies]
egui = "0.31"                    # compiles everywhere
eframe = "0.31"                  # compiles everywhere (has wasm backend)
serde = { version = "1", features = ["derive"] }  # compiles everywhere

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
rodio = "0.19"                   # native only — audio playback
alacritty_terminal = "0.24"      # native only — PTY (via egui_term)
```

### Key rule

**If it touches the OS, it gets a `cfg(not(target_arch = "wasm32"))` gate. If it's pure computation or UI rendering, it compiles everywhere.** There is no gray area. File I/O, process spawning, audio devices, Keychain — all gated. Layout, theming, input handling, state management — all universal.

---

## 4. Dependency Compatibility Matrix

| Dependency | Native | WASM | Strategy |
|---|---|---|---|
| `eframe` / `egui` | Yes | Yes | Works out of the box. eframe detects wasm32 and uses its WebGL/WebGPU backend automatically. |
| `egui_tiles` | Yes | Yes | Pure layout logic, no OS deps. |
| `egui_term` (alacritty_terminal) | Yes | No | Backend-only. The terminal grid state is computed on the backend, rendered as draw commands on the client. Gate the entire `egui_term` dep. |
| `rodio` | Yes | No | Backend-only. Audio playback stays on the host machine. WASM client sends play/pause/seek commands; if client-side audio is needed later, use Web Audio API via `wasm-bindgen`. |
| `notify` (file watcher) | Yes | No | Backend-only. Backend watches files and pushes change events over WebSocket. Client polls or subscribes. |
| `std::process::Command` | Yes | No | Backend-only. All subprocess spawning (Python apps, `plexi run`) stays native. |
| `std::fs` | Yes | No | Backend serves all file operations via WebSocket API requests (`ListDir`, `ReadFile`, `WriteFile`). |
| macOS Keychain (`security-framework`) | Yes | No | Backend-only. Secrets never leave the host. WASM client sees redacted displays only. |
| `dirs` | Yes | No | Backend-only. Home directory, config paths — all resolved server-side. |
| `image` | Yes | Yes | Works in both. Image decoding is pure computation. |
| `serde` / `serde_json` | Yes | Yes | Works everywhere. Core serialization. |
| `toml` | Yes | Yes | Works everywhere. Config parsing (though WASM client may receive config from backend). |
| `chrono` | Yes | Yes | Works with the `js` feature enabled for WASM (uses `Date.now()` instead of system clock). |
| `objc2` / AppKit | Yes | No | macOS-only already (`cfg(target_os = "macos")`). No changes needed — already gated. |
| `log` | Yes | Yes | Works everywhere. WASM uses `console_log` backend. |

---

## 5. Feature Gates Required

Every module/file in `src/` that needs conditional compilation, mapped to the reason.

### Entirely native-only modules (gate the `mod` declaration)

| Module | Reason | Gate |
|---|---|---|
| `src/shell.rs` | PTY spawning, shell detection, `std::process::Command`, `lsof` calls | `#[cfg(not(target_arch = "wasm32"))]` |
| `src/process_app.rs` | Spawns external app binaries as subprocesses, manages stdin/stdout pipes | `#[cfg(not(target_arch = "wasm32"))]` |
| `src/secrets.rs` | macOS Keychain access via `security-framework` | `#[cfg(not(target_arch = "wasm32"))]` |
| `src/secrets_app.rs` | UI for secrets management — calls into `secrets.rs` for Keychain ops | `#[cfg(not(target_arch = "wasm32"))]` |
| `src/cli.rs` | CLI subcommands (`plexi run`, `plexi secret`, `plexi app`) — all use subprocess/filesystem | `#[cfg(not(target_arch = "wasm32"))]` |
| `src/macos_menu.rs` | Already gated with `#[cfg(target_os = "macos")]` — no changes needed | Already gated |
| `src/logging.rs` | File-based log rotation — WASM uses `console_log` instead | `#[cfg(not(target_arch = "wasm32"))]` |

### Mixed modules (gate specific functions/blocks within the file)

| Module | What stays universal | What gets gated |
|---|---|---|
| `src/file_browser/mod.rs` | All UI rendering (`render_*` functions), navigation state, icon logic | `read_dir` calls, `std::fs::metadata`, `std::fs::canonicalize`, path resolution |
| `src/file_browser/audio.rs` | Audio metadata display | File reading for metadata extraction |
| `src/file_browser/helpers.rs` | String/path utilities | Any `std::fs` calls |
| `src/audio_app.rs` | UI rendering (progress bar, controls, playlist display) | `rodio` thread spawning, `std::fs::read_dir` for file scanning, `mpsc` audio channel |
| `src/config.rs` | `PlexiConfig` struct, deserialization, defaults | `load()` function (reads `config.toml` from filesystem via `dirs::config_dir()`) |
| `src/app_registry.rs` | Registry data structures, app lookup | Filesystem scanning for installed apps, manifest loading from disk |
| `src/app_api.rs` | API request/response types | Filesystem operations (`ListDir`, `ReadFile`, `WriteFile` execution) |
| `src/context.rs` | Context struct, state management | CWD detection, `lsof` calls |
| `src/main.rs` | (see below) | CLI handling, `NativeOptions`, `eframe::run_native` |

### New WASM-only modules to create

| Module | Purpose |
|---|---|
| `src/ws_bridge.rs` | WebSocket connection management, message send/recv, reconnection logic |
| `src/wasm_main.rs` | WASM entry point — `#[wasm_bindgen(start)]`, eframe web runner setup |

### `main.rs` split

`main.rs` currently handles CLI parsing and launches `eframe::run_native`. For WASM:

- The CLI block (`plexi run`, `plexi secret`, etc.) gets `#[cfg(not(target_arch = "wasm32"))]`
- `eframe::run_native` gets `#[cfg(not(target_arch = "wasm32"))]`
- A new `#[cfg(target_arch = "wasm32")]` block uses `eframe::WebRunner` instead
- The module declarations for native-only modules get gated as shown in Section 3

---

## 6. PWA Setup

### Required files

**`manifest.json`** — declares the app as installable:

```json
{
  "name": "Plexi",
  "short_name": "Plexi",
  "description": "Terminal multiplexer and agent interface",
  "start_url": "/",
  "display": "standalone",
  "orientation": "any",
  "background_color": "#1e1e2e",
  "theme_color": "#1e1e2e",
  "icons": [
    { "src": "/icons/icon-192.png", "sizes": "192x192", "type": "image/png" },
    { "src": "/icons/icon-512.png", "sizes": "512x512", "type": "image/png" },
    { "src": "/icons/icon-maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
  ]
}
```

**`index.html`** — Apple-specific meta tags for iOS PWA behavior:

```html
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
<meta name="apple-mobile-web-app-title" content="Plexi">
<link rel="apple-touch-icon" href="/icons/apple-touch-icon-180.png">
<meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover">
<link rel="manifest" href="/manifest.json">
```

The `viewport-fit=cover` and `user-scalable=no` are important — egui handles its own scaling, and iOS Safari's pinch-to-zoom and elastic scroll interfere with the canvas.

**`sw.js`** — service worker for offline caching:

```javascript
const CACHE_NAME = 'plexi-v1';
const ASSETS = [
  '/',
  '/index.html',
  '/plexi.js',
  '/plexi_bg.wasm',
  '/manifest.json',
  '/icons/icon-192.png',
  '/icons/icon-512.png'
];

self.addEventListener('install', e => {
  e.waitUntil(caches.open(CACHE_NAME).then(c => c.addAll(ASSETS)));
});

self.addEventListener('fetch', e => {
  e.respondWith(caches.match(e.request).then(r => r || fetch(e.request)));
});
```

The service worker caches the WASM binary and static assets so the PWA shell loads instantly. The WebSocket connection to the backend is not cached — if the backend is unreachable, the app shows a "connecting..." state.

### HTTPS requirement

PWAs require HTTPS (service workers won't register over HTTP, except on `localhost`).

- **Local dev:** `localhost` works without TLS. Use `127.0.0.1:<port>` for same-machine testing.
- **Local network:** Self-signed certificate with the machine's local IP as SAN. Mobile device must trust the certificate.
- **Production / remote:** Let's Encrypt via ACME, or Tailscale's automatic TLS certificates (ideal for Plexi's use case since Tailscale is already a natural fit for remote access).

---

## 7. iOS / Android PWA Capabilities

| Capability | iOS Safari | Android Chrome | Notes |
|---|---|---|---|
| Add to Home Screen | Manual (Share > Add to Home Screen) | Auto-prompt via `beforeinstallprompt` | iOS requires user to know the flow |
| Standalone mode (no browser chrome) | Yes | Yes | `display: standalone` in manifest |
| WebSocket (WSS) | Yes | Yes | Full duplex, no message size issues |
| Canvas / WebGL | Yes | Yes | egui renders to canvas |
| Push notifications | Yes (iOS 16.4+, requires user opt-in) | Yes | Requires service worker + VAPID keys |
| Camera / microphone | Yes (HTTPS only) | Yes | For future voice chat features |
| Background execution | No (suspended when not visible) | Limited (service worker only) | WebSocket disconnects on iOS background; reconnect on resume |
| IndexedDB | Yes (capped ~1GB, evictable under storage pressure) | Yes (generous limits) | For local state cache, preferences |
| Clipboard access | Yes (user gesture required) | Yes | For copy/paste in terminal |
| Haptic feedback | Yes (`navigator.vibrate` not supported, but CSS `touch-action` works) | Yes (`navigator.vibrate`) | Subtle feedback on actions |
| Orientation lock | No (manifest `orientation` ignored) | Yes | Not critical for Plexi |
| Splash screen | Yes (auto-generated from manifest + apple-touch-startup-image) | Yes (from manifest) | Covers WASM load time |

### iOS-specific constraints

- **No background WebSocket:** When the user switches away, iOS suspends the PWA within seconds. The WebSocket will disconnect. The client must detect this and reconnect when the app returns to foreground. This is the same constraint a native iOS app would face (without background modes entitlement).
- **IndexedDB eviction:** iOS can evict IndexedDB data under storage pressure. Don't rely on it for anything that can't be re-fetched from the backend.
- **No install prompt:** Unlike Android, iOS has no programmatic "Add to Home Screen" prompt. The app should show a one-time banner explaining how to install.

---

## 8. Implementation Phases

### Phase 1: Feature-gate native dependencies

**Goal:** `cargo build --target wasm32-unknown-unknown` compiles the UI layer without errors.

- Add `#[cfg(not(target_arch = "wasm32"))]` gates to all modules listed in Section 5
- Gate native-only dependencies in `Cargo.toml` under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`
- Create stub types/traits behind `#[cfg(target_arch = "wasm32")]` where the UI layer expects them (e.g., a no-op `AudioPlayer` stub)
- Verify: `cargo build --target wasm32-unknown-unknown` succeeds
- Verify: `cargo build` (native) still works identically

This phase changes zero runtime behavior. It only restructures compilation boundaries.

### Phase 2: WebSocket server mode

**Goal:** The native Plexi binary can serve the WASM client.

- Add `axum` + `tokio` + `tokio-tungstenite` as native-only dependencies
- Implement `plexi serve` CLI subcommand — starts HTTP server + WebSocket endpoint
- HTTP serves the static WASM bundle from an embedded or adjacent directory
- WebSocket endpoint accepts connections and bridges them to the existing app protocol
- The server manages PTY sessions, app spawning, and file ops on behalf of connected clients

### Phase 3: WASM client with WebSocket transport

**Goal:** Open Plexi in a browser, see the UI, interact with it.

- Create `ws_bridge.rs` — WebSocket connection, send/recv, automatic reconnect
- Create WASM entry point using `eframe::WebRunner`
- Wire egui rendering to draw commands received over WebSocket
- Forward keyboard/touch input as `PlexiEvent` messages
- Basic functional loop: type in browser, see output from PTY on host

### Phase 4: PWA manifest and touch input

**Goal:** Installable PWA with mobile-friendly interaction.

- Add `manifest.json`, service worker, icons, Apple meta tags
- Handle touch events: tap, long-press, swipe gestures mapped to Plexi actions
- Add virtual keyboard management (show/hide, viewport resize handling)
- Add reconnection UI (offline/connecting/connected states)
- iOS "Add to Home Screen" instruction banner

### Phase 5: Auth, TLS, deployment

**Goal:** Secure, deployable to real devices.

- Reuse Ed25519 keypair pairing ceremony from `companion-app.md` (Section: Pairing & Authentication)
- WSS (TLS) for all non-localhost connections
- Tailscale integration for zero-config remote access with automatic TLS
- Session management: paired device list, revocation, multi-device support

---

## 9. Security

### Transport encryption

All non-localhost WebSocket connections must use WSS (WebSocket over TLS). The backend rejects plain WS connections from non-loopback addresses. This is enforced at the server level, not left to configuration.

### Authentication

Reuse the Ed25519 keypair pairing ceremony defined in `companion-app.md`:

1. User opens Plexi on Mac, navigates to Settings > Remote Access
2. Plexi generates a one-time 6-digit pairing code + Ed25519 keypair
3. User opens the WASM PWA on their phone, enters the code (or scans QR)
4. Key exchange completes — both sides hold each other's public keys
5. Subsequent connections authenticate by signing a challenge with the paired key

The pairing code is single-use and expires after 60 seconds. Paired devices are stored in `~/.plexi-alpha/paired-devices.json`.

### Secrets isolation

Secrets (API keys, tokens) never leave the backend. The WASM client can:
- See that a secret exists (key name, which app uses it)
- Trigger a "set secret" flow (the value is typed in the browser but sent directly to the backend for Keychain storage)
- See redacted displays (e.g., `ANTHROPIC_API_KEY: ••••••••`)

The client never receives, caches, or stores secret values. IndexedDB on the client stores only UI preferences and non-sensitive state.

### CORS policy

The backend's HTTP server sets CORS headers to restrict which origins can connect:
- `localhost` origins: allowed (development)
- The PWA's own origin (if hosted externally): allowed
- All other origins: rejected

### Filesystem sandboxing

The existing path sandboxing (reject paths that escape the app's launch directory) is enforced on the backend. The WASM client cannot bypass it because it has no direct filesystem access — every file operation is a request to the backend, which validates the path before executing.

---

## 10. Relation to Other Specs

### Replaces: `companion-app.md`

The WASM PWA replaces the native iOS companion app as Plexi's mobile strategy. The companion app spec is retained as reference because:
- The Ed25519 pairing ceremony design is reused verbatim (Section 9)
- The agent chat and approval management UX concepts carry forward
- The Bonjour/mDNS and Tailscale discovery patterns apply to the WebSocket server

The companion app spec should be marked as `Status: Superseded by wasm-pwa-deployment.md`.

### Extends: `app-infrastructure.md`

The app protocol (`PlexiEvent` / `DrawCommand`) defined in `app-infrastructure.md` is the same protocol used over WebSocket. No protocol changes required. The WASM client is architecturally equivalent to an external app — it sends events, receives draw commands, and requests host capabilities through the same API.

### Enables: `sync-architecture.md`

The WebSocket server built for WASM deployment is also the foundation for multi-machine sync. Once Plexi has a WebSocket server that bridges the app protocol, extending it to peer-to-peer sync (machine A's Plexi connecting to machine B's Plexi) requires only adding routing and conflict resolution on top of the same transport.

---

## 11. Phase 1 Status (2026-04-11)

Issue #105 / `feature/wasm-phase1-feature-gates`.

### What works

- **Both targets compile cleanly.** `cargo build` (native) and `cargo build --target wasm32-unknown-unknown` both succeed from a clean state.
- **Cargo.toml dependency split.** Native-only crates (`egui_term`, `rodio`, `notify`, `dirs`, `fern`) moved into `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`. WASM-only crates (`wasm-bindgen`, `web-sys`, `getrandom` with `js` feature, `console_error_panic_hook`) added under `[target.'cfg(target_arch = "wasm32")'.dependencies]`. `chrono` now enables its `wasmbind` feature on all targets.
- **Module-level gating in `main.rs`.** Every `mod` declaration for a native-only file is wrapped in `#[cfg(not(target_arch = "wasm32"))]`. The native `main()` function is also cfg-gated, and a no-op WASM `main()` stub exists so the binary crate has an entry symbol on `wasm32-unknown-unknown`.
- **Wasm subset compiled.** Currently only two modules are universal: `app_protocol` (JSON protocol types) and `agent_mode` (pure state machine). These are the seed of the shared UI/domain layer the WASM client will be built on in Phase 3.

### What's still blocked

Phase 1 was scoped as "dependency + module gating, no behaviour changes." Splitting the actual rendering core to compile for WASM requires a refactor that's deliberately deferred.

- **`app.rs`, `pane.rs`, `pane_ops.rs`, `tiling.rs`, `theme.rs`, `command_palette.rs`** all import from `egui_term` (alacritty_terminal wrapper) directly. The PTY terminal view type appears in struct fields and render methods. Decoupling them requires introducing a render-only terminal grid type that `egui_term::TerminalView` and a future WASM draw-command consumer can both satisfy. That's Phase 2/3 work.
- **`app_trait.rs`** depends on `theme::Colors` and `tiling::PaneId`, so every app (text_editor, quick_note, file_browser, secrets, audio, process_app) transitively drags in the native render stack.
- **`config.rs`, `logging.rs`, `context.rs`, `workspace.rs`** all use `dirs`, `fern`, or `std::process`. They need client/server splits — the client-side config arrives over WebSocket, logging goes to `console_log`.
- **`app_permissions.rs` and `features.rs`** only touch `config` and `app_trait` — they're the lowest-hanging fruit for the next round of universal-izing, once `config` is split.
- **No WASM UI yet.** The wasm binary currently has an empty `main()`. Wiring up `eframe::WebRunner` is Phase 3.

### Follow-up work for Phase 2+

1. Introduce a client/server split for `config.rs` — pure `PlexiConfig` struct on both sides, `load()` native-only.
2. Introduce a render abstraction for terminal grids so `tiling.rs` and `app.rs` don't import `egui_term` directly.
3. Start `ws_bridge.rs` — WebSocket transport module for WASM.
4. Port `file_browser/mod.rs` rendering into a universal module, with `read_dir` calls behind a trait that has a native (`std::fs`) and a WASM (WS request) implementation.

### Files touched in Phase 1

- `Cargo.toml` — dependency reorganisation
- `src/main.rs` — module gating, cfg-split entry point
- `docs/specs/wasm-pwa-deployment.md` — this section
