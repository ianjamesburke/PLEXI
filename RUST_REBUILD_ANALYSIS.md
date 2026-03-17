# Plexi Rust Rebuild Analysis

## Current Architecture (Electrobun)

**Problem:** Double-input issue in dev mode. Electrobun is early-stage and has build quirks.

**Current stack:**
- **Main process:** Bun + TypeScript (src/bun/*) — handles PTY, RPC, app lifecycle
- **Renderer:** Native/CEF webview → HTML/CSS/JS (src/mainview/) + xterm.js
- **PTY backend:** bun-pty (Rust-compiled dylib)
- **IPC:** Electrobun's RPC layer (session-rpc.ts)
- **Terminal emulation:** xterm.js (cross-platform, well-tested)
- **Tests:** Bun unit tests + Playwright e2e (headless)

**Why Electrobun fails here:** Early-stage framework with renderer/IPC bugs. CEF option too heavy. Switching to Electron would fix most issues but doesn't leverage Rust.

---

## Rust GUI Framework Options

### Option 1: **Tauri** ⭐ RECOMMENDED
**What it is:** Lightweight Electron alternative. Rust backend + Webview (native or WRY).

**Pros:**
- ✅ Small bundle (~10MB vs Electron's 150MB)
- ✅ IPC layer built-in (strongly typed with serde)
- ✅ Playwright/Webdriver automation works natively (webview is real browser)
- ✅ PTY spawning via std::process or crate like `pty-process`
- ✅ Cross-platform (Mac, Linux, Windows)
- ✅ Large ecosystem (pty-process, tauri-fs, tauri-shell plugins)
- ✅ Mature testing story (Playwright headless, WDA for Mac)

**Cons:**
- Still uses a webview (not "pure Rust UI"), but xterm.js still works
- Slightly different IPC model than Electrobun (minor refactor needed)

**Testing strategy:**
- Keep xterm.js frontend (works in webview)
- Use Playwright with `@tauri-apps/plugin-webdriver` or raw WebDriver to interact with terminals
- Spawn test sessions via Tauri IPC, send input, validate output
- E2E: headless browser automation, no manual interaction

**Estimated refactor effort:** 2-3 days (main process refactor + build config)

---

### Option 2: **Druid**
**What it is:** Immediate-mode GUI framework. Native widgets, no webview.

**Pros:**
- ✅ Pure Rust, no JS/webview complexity
- ✅ Fast, small binary
- ✅ Built for keyboard/accessibility

**Cons:**
- ❌ xterm.js doesn't work (no webview)
- ❌ Would need to rewrite terminal renderer in Rust (6+ weeks)
- ❌ No built-in automation testing support
- ❌ Much slower development timeline

**Verdict:** Too much UI rewrite. Not viable.

---

### Option 3: **EGUI (Immediate Mode)**
**What it is:** Lightweight egui rendering + windowing (winit).

**Pros:**
- ✅ Small, fast, pure Rust
- ✅ Can embed as web canvas (egui-wgpu renderer)
- ✅ Good for data-heavy UIs

**Cons:**
- ❌ xterm.js still doesn't work natively
- ❌ Terminal rendering would need custom work
- ❌ Testing is manual/screenshot-based
- ❌ Complex state management

**Verdict:** Same terminal-rewrite problem as Druid.

---

### Option 4: **Leptos / Yew (Rust + WASM)**
**What it is:** Rust → WebAssembly frontend framework + Rust backend.

**Pros:**
- ✅ Type-safe full-stack (Rust frontend + backend)
- ✅ xterm.js can integrate into Leptos components
- ✅ Playwright testing works (full browser)
- ✅ Can use existing PTY logic

**Cons:**
- ❌ WASM bundle overhead
- ❌ Steeper learning curve (Leptos has complex reactivity)
- ❌ Not as battle-tested for desktop apps as Tauri

**Verdict:** Viable but more complex than Tauri for this use case.

---

### Option 5: **Electron (Not Rust, But Viable)**
**What it is:** Node.js + Chromium. Industry standard.

**Pros:**
- ✅ Mature ecosystem
- ✅ Known solutions for all problems
- ✅ Existing Playwright tests likely work with minimal changes
- ✅ Large community

**Cons:**
- ❌ Not Rust (but pragmatically, it's the "safest" choice)
- ❌ ~150MB bundle (vs Tauri's 10MB)
- ❌ More memory usage

**Verdict:** Fallback if Rust complexity becomes blocking.

---

## Recommendation: **Tauri**

**Why:**
1. Addresses Electrobun instability immediately
2. Keeps xterm.js (no terminal rewrite)
3. Strong testing story via Playwright + WebDriver
4. Small bundle, real Rust backend
5. Production-ready ecosystem
6. 2-3 day refactor (fastest path to working)

**Testing approach:**
```rust
// Tauri main.rs
#[tauri::command]
async fn open_session(panel_id: String, cwd: String) -> Result<SessionStartedMessage, String> {
    // PTY spawning via pty-process crate
    // Return to frontend
}
```

```typescript
// Playwright test
await page.goto('http://localhost:1420');
// IPC to spawn session
await page.evaluate(() => window.__TAURI__.invoke('open_session', {...}));
// Wait for output
await page.locator('.xterm-screen').waitFor();
// Send input via simulated keyboard
await page.keyboard.type('ls\n');
// Validate output appeared
await expect(page.locator('.xterm-screen')).toContainText('...');
```

---

## Migration Path

1. **Create new branch:** `feature/tauri-rebuild`
2. **Initialize Tauri project:** `cargo new plexi-tauri`
3. **Port main process logic:**
   - SessionManager → Tauri command
   - RPC calls → IPC invokes
   - File I/O → Tauri fs plugin
4. **Keep frontend mostly as-is:**
   - Copy src/mainview/* → src-tauri/tauri.conf.json#build.frontendDist
   - Update IPC calls (Electrobun → Tauri invoke)
   - No UI rewrite needed
5. **Set up tests:**
   - Port Playwright tests (mostly unchanged)
   - Add Tauri-specific fixtures
6. **Build & verify locally**
7. **Test deployment**

---

## First Steps

```bash
# Create branch
git checkout -b feature/tauri-rebuild

# Initialize Tauri
npm install -D @tauri-apps/cli@latest
npm run tauri init

# Install PTY crate
cargo add pty-process

# Build & test
npm run tauri dev
npm run test:e2e
```

---

## Cross-Platform Notes

- **Mac:** Uses native WebKit (faster than WRY fallback). Good.
- **Linux:** WRY + GTK/Qt. Tested by many Tauri apps.
- **Windows:** Native WebView2 (OS-bundled). Smallest footprint.

---

## Risk Assessment

| Issue | Current (Electrobun) | Tauri |
|-------|:--:|:--:|
| Double-input bug | 🔴 Active | 🟢 Solved |
| Testing | 🟡 Playwright works but flaky | 🟢 Mature |
| Bundle size | 🟡 ~40MB | 🟢 ~10MB |
| Docs | 🟡 Sparse | 🟢 Excellent |
| Community | 🔴 Tiny | 🟢 Large |

---

## Decision

**Go with Tauri.** It's the pragmatic choice: fixes the immediate problem, keeps the terminal working, has a proven testing story, and delivers a better user experience (smaller, faster, more stable).

If you want pure Rust UI later, that's a different project (6+ month timeline). For now, Tauri gives you production-ready, testable, maintainable code.
