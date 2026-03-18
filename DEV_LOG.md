<!-- DEV_LOG.md — decision journal for the Plexi project. Newest entries at the top. Records non-obvious choices, abandoned approaches, and root causes so future sessions don't repeat mistakes. -->

## 2026-03-18 — Future enhancement: Claude Code notification routing + conversation cycling

**Feature idea:** Surface Claude Code conversations/notifications in the Plexi UI so you can cycle through multiple sessions waiting for input (e.g., "5 chats need responses, hop between them").

**How cmux does it:** Uses a hook injection system. It wraps Claude Code with environment variables pointing to hook commands (`CMUX_ON_NOTIFICATION`, `CMUX_ON_WAITING_FOR_INPUT`, etc.). When Claude Code hits lifecycle events, it executes the hooks, which fire back to cmux via socket API with structured metadata (status, notification text, waiting_for_input flag).

**Options for Plexi (in priority order):**

1. **Request hook support from Anthropic** (preferred, Option A): File a feature request with Claude Code team to support `PLEXI_ON_*` environment variables. If Claude Code adopts this, Plexi can inject them when spawning sessions and get structured notifications via IPC callback.

2. **Parse OSC sequences Claude Code already emits** (Option B): Check if Claude Code emits OSC 777 (desktop notification) or OSC 9/99 (status). If so, parse them from PTY output like OSC 7 (cwd tracking). Less structured than hooks but works today.

3. **Implement hook system yourself** (Option C, medium effort): Patch or wrap Claude Code to inject Plexi's own hook environment variables. Hooks call back to Tauri backend via IPC. Full control but requires maintaining a Claude Code wrapper.

**MVP approach:** Defer until users ask for it. If this becomes a priority, start with Option A (upstream request) or Option B (parse existing sequences). Option C is a fallback.

**References:** cmux architecture at [manaflow-ai/cmux](https://github.com/manaflow-ai/cmux) PR #1306.

---

## 2026-03-18 — TUI rendering: root cause analysis + libghostty evaluated (deferred)

**Why Plexi is janky with TUIs (Claude Code, htop, lazygit, etc.):**

xterm.js measures cell size *backward*: render HTML → measure DOM element → derive cell dimensions → set PTY size. Native terminals (Ghostty, iTerm2) go the other way: read OS font metrics → derive cell dimensions → render. Any browser rounding or CSS approximation in the xterm.js path compounds into a PTY col count that doesn't match what's actually displayed. TUI apps query `TIOCGWINSZ`, get the wrong number, and wrap/overlap content.

Specific xterm.js failure modes:
- **FitAddon col math**: documented upstream; approximates scrollbar width rather than measuring it
- **Unicode width tables**: shipped tables are ~2019 vintage — newer emoji are 1-cell in xterm.js but 2-cell in the PTY. This was the immediate autocomplete bug (emoji in completion entries pushed cursor wrong)
- **No synchronized rendering** (ANSI 2026): Ghostty supports batched frame commits to eliminate partial-render flicker; xterm.js doesn't
- **No Kitty Keyboard Protocol**: modern TUIs increasingly rely on this for reliable modifier+key combos

**libghostty evaluated and rejected for now:**

libghostty would fix the rendering accuracy (it uses OS font metrics → Metal on macOS), but it cannot be embedded in a Tauri app:
- Its rendering layer expects direct Metal/OpenGL GPU surface access — it renders into a native AppKit/GTK view, not an offscreen buffer you can composite into a WebView
- The apps that have embedded it (cmux, mdnb, pynb) are all native Swift/AppKit — cmux's creators explicitly rejected Tauri/Electron for this reason
- Unstable C API (officially marked in-progress; stable release targeted sometime 2026), requires Zig toolchain, no pre-built binaries

**Decision:** Accept xterm.js limitations for the MVP. Simple shell usage works fine; TUI-heavy apps suffer. If TUI quality becomes a core differentiator (e.g., "the terminal for Claude Code users"), the right long-term path is a native rendering layer — either a native AppKit view overlay in Tauri, or rebuilding the terminal component entirely outside the WebView. Defer until there are real users to justify the effort.

**Deferred fixes to revisit when needed:**
1. Patch the acute emoji width bug: force double-width emoji in xterm.js via a custom `unicodeService` override
2. Replace fitAddon column calc: measure cell size from canvas `measureText()` on the actual font instead of the DOM probe span
3. Monitor libghostty C API stability (aimed for late 2026 stable) — revisit embedding feasibility then

---

## 2026-03-18 — TUI rendering artifacts: UNSOLVED — known limitation

**Status:** Reverted all attempted fixes. The column-count safety margin, CSS specificity fix, and timing fix were all insufficient — Claude Code's Ink-based TUI still renders with garbled re-renders, missing icons (◆ rendered as `???`), and text overlap.

**What we know:**
- The issue is a column-count mismatch between what xterm.js fitAddon reports to the PTY and what the WebGL renderer actually displays
- Native terminals (Ghostty, iTerm2) don't have this because their renderer and column math are the same code path — xterm.js has an inherent measurement gap between fitAddon (CSS pixels) and the WebGL renderer
- The missing diamond icons (`◆` → `???`) are a separate issue — likely a font/glyph coverage problem in the WebGL renderer's texture atlas
- Multiple fix attempts (safety margin subtraction, CSS scrollbar specificity, fit timing) failed to fully resolve it

**Attempted fixes (all reverted):**
1. Subtracting 1 column after fitAddon.fit() — still garbled
2. Fixing CSS specificity on scrollbar width (6px override) — no visible improvement
3. Synchronous fit + rAF re-fit after WebGL addon load — no visible improvement

**This is a known class of xterm.js issues.** TUI-heavy apps (Claude Code, htop, etc.) are affected. Simple shell usage works fine. Needs deeper investigation — possibly a custom fitAddon that reads dimensions directly from the active renderer, or disabling WebGL for affected sessions.

---

## 2026-03-17 — TUI rendering artifacts in xterm.js (Claude Code, Ink apps) — OPEN

**Symptom:** Claude Code (and likely other Ink/TUI apps) renders with column-alignment artifacts inside Plexi. Specific issues observed:
- Two `◆◆` glyphs in the separator line appear and disappear as the window is resized — confirmed to be a wrapping/column-width issue, not a missing font glyph issue
- Right-panel header content shows a `m]` prefix (truncated label, visible as wrap artifact)
- Bottom status bar sections overlap or concatenate without proper spacing
- Text content from one logical row bleeds onto the next visual row

**Key observation:** The `◆` glyphs in the separator line become MORE numerous when the window is narrower and FEWER when wider — they are real rendered glyphs, but wrapping causes them to spill onto adjacent lines, implying the PTY is reporting MORE columns than xterm.js is actually displaying.

**Root cause hypothesis (unconfirmed):** The PTY col count and xterm.js display col count are mismatched. Likely causes:
1. The fitAddon subtracts scrollbar width incorrectly (see CSS below)
2. The `overviewRuler: { width: 1 }` option may not map correctly in some xterm.js 6 paths
3. CSS specificity conflict: `.scrollbar.scrollbar.vertical { width: 6px !important }` overrides `.scrollbar.vertical { width: 0 !important }` due to higher specificity — the scrollbar may be taking 6px of layout space while fitAddon only subtracts 1px (the ruler width), creating a ~5px discrepancy

**What was tried and ruled out:**
- Adding `"Apple Color Emoji"` and `"Apple Symbols"` to the font-family fallback → made column alignment WORSE (emoji font metrics interfere with xterm.js char-width calculations). Reverted.
- Adding `@xterm/addon-unicode11` and activating it before `terminal.open()` → PARTIAL FIX. Eliminated the garbled full-layout issues (misaligned text across the whole terminal). The major rendering is now correct. The remaining `◆◆` and alignment issues persist. **This fix is in place and correct — do not revert.**
- Moving `ensurePanelSessions()` to after `syncVisiblePaneRuntimes()` + synchronous `fitAddon.fit()` before `terminal.open()` → no visible improvement. Reverted. The PTY size mismatch hypothesis (PTY spawning at 80×24) was not the primary cause since Claude Code receives SIGWINCH and redraws.

**Current state (after unicode11 fix, emoji fonts reverted, timing revert):** Most of the layout is correct. The remaining issue is a consistent column-count discrepancy between PTY and xterm.js display, causing TUI apps that use the full terminal width to overflow by ~2–5 cols and wrap content onto the next line.

**Next steps to investigate:**
- Audit CSS scrollbar rules for specificity conflicts — the 6px `.scrollbar.scrollbar.vertical` override may be the culprit
- Add a diagnostic: run `tput cols` in a Plexi session and compare to `window.innerWidth` / observed char count to confirm the actual discrepancy
- Consider whether `overviewRuler: { width: 1 }` in TERMINAL_PROFILE is correctly recognized by xterm.js 6 (vs the older `overviewRulerWidth` flat option)
- The fitAddon source reads: `t = scrollback === 0 ? 0 : overviewRuler?.width || 14` — if `overviewRuler` is not stored in `terminal.options`, t defaults to 14, causing fitAddon to under-report cols by ~1

---

## 2026-03-17 — Switch xterm.js to WebGL renderer for better color fidelity

Added `@xterm/addon-webgl` and activated it after `terminal.open()` in `xterm-runtime.js`. Fixes wrong colorization in TUI apps (Claude Code, etc.) vs Ghostty. The default Canvas 2D renderer was the culprit — it's less accurate than a GPU-composited path.

Includes an `onContextLoss` handler that disposes the WebGL addon if the GPU context is lost (can happen when window backgrounds on macOS), falling back to canvas automatically. Without this handler, a context loss leaves the terminal blank permanently.

Vendor script added at `vendor/xterm/addon-webgl.js`; `copy-vendor` script updated to include it.

---

## 2026-03-17 — Fix Cmd+V paste showing permission popup instead of pasting

Pressing Cmd+V in the terminal showed a WebView permission popup ("Paste from clipboard?") at the cursor instead of cleanly pasting text.

**Root cause:** The `paste_from_clipboard` keybind handler in `app.js` was intercepting Cmd+V, calling `event.preventDefault()`, then manually reading the clipboard via `navigator.clipboard.readText()`. In Tauri's WKWebView on macOS, that API triggers a native clipboard permission dialog.

**Fix:** Removed the manual clipboard read. The keybind handler now returns `true` to let the keypress pass through to xterm.js, which has its own built-in `paste` event listener. The browser fires the native `paste` event (no permission needed), xterm.js picks it up, and routes the text through `onData` into the PTY session.

**Dead end:** Tried using `tauri-plugin-clipboard-manager` to bypass the WebView permission system via native OS clipboard access. Plugin compiled and registered fine, but the invoke calls silently failed — paste did nothing at all. Reverted. The xterm.js native paste path is simpler and requires zero Rust changes.

**Lesson:** Don't fight the browser's clipboard security model — use the native `paste` event flow instead of `navigator.clipboard.readText()`. xterm.js already handles this correctly if you let the key event through.

---

## 2026-03-17 — Custom title bar and window dragging with titleBarStyle Overlay

Switched from default macOS gray title bar to a transparent overlay bar (`"titleBarStyle": "Overlay"`, `"hiddenTitle": true` in `tauri.conf.json`) so the app background color extends into the title bar area. Bumped `--window-top-inset` from `6px` to `28px` for macOS so content clears the traffic light buttons.

**Window dragging:** `data-tauri-drag-region` on the toolbar elements wasn't enough — the attribute only applies to the exact element it's on, not children, so child `div`s and `span`s inside the toolbar swallow the mousedown before it reaches the drag region. Fixed by adding a `mousedown` listener that calls `getCurrentWindow().startDragging()` when the click target isn't an interactive element.

**Critical:** `startDragging()` requires the capability permission `core:window:allow-start-dragging` in `src-tauri/capabilities/default.json`. Without it, the call silently fails — no error, no drag. This is a Tauri 2.x security sandbox requirement.

## Future: Single-instance enforcement

By default Tauri does not prevent multiple app instances from running simultaneously. A second launch opens a second process with its own config read/write cycle — potential for concurrent writes to `~/.plexi/`. Not an issue now (no users, macOS Dock typically re-focuses the existing window anyway). When it matters, add the official [`tauri-plugin-single-instance`](https://v2.tauri.app/plugin/single-instance/).

---

## Future: E2E test suite with tauri-driver

Full end-to-end tests using `tauri-driver` + WebdriverIO against a compiled binary. Spin up a clean, unconfigured app (no `~/.plexi` state) and exercise every major user flow:

- Create a new terminal session, run a command, verify output appears
- Split panes horizontally and vertically
- Close a pane, verify others are unaffected
- Workspace save + restore (relaunch app, verify layout and sessions recover)
- Resize terminal, verify PTY SIGWINCH propagates correctly

This is the right long-term confidence net before releases. Not MVP — defer until the core feature set stabilizes and there are real users to break things. When implementing, start with the official Tauri guide: https://tauri.app/develop/tests/webdriver/

---

## 2026-03-17 — Shell integration via ZDOTDIR injection for cwd tracking

Split terminals and workspace saves were always showing the initial session directory (e.g. `~`) instead of the user's current directory. `panel.cwd` was only set once at session spawn and never updated because the shell wasn't emitting any cwd signal.

**Fix:** ZDOTDIR injection — the same approach used by Ghostty, iTerm2, and WezTerm.
- `shell_integration.rs` writes `~/.plexi/shell-integration/zsh/{.zshrc,.zprofile}` at startup (idempotent)
- The `.zshrc` sources the user's real `~/.zshrc` (via `PLEXI_ORIG_ZDOTDIR`), then appends a `precmd` hook
- The hook emits **OSC 7** (`\e]7;file://hostname/path\a`) — the standard cwd protocol
- Replaced the custom `PlexiCwd` OSC 633 sequence with OSC 7 in `session-output.js`, mock bridge, and tests

**Why OSC 7 over the custom PlexiCwd sequence:** OSC 7 is already supported by fish (built-in), and shell integration scripts for bash/fish are widely available. fish users already get cwd tracking for free. Bash support just needs an additional `shell_integration.rs` script later.

**Also fixed:** `home_dir` is now returned from `SessionStartedMessage` so the frontend initializes `homeDirectory` immediately (fixes `cwdLabel` showing full paths instead of `~` in workspace saves).

**Zsh only for now** — bash/fish integration scripts are the next step when needed.

## 2026-03-17 — Double input bug in production Tauri builds (RESOLVED)

**Status: fixed**

First keystroke after each prompt appeared doubled in production builds (`tauri build`), but worked perfectly in dev mode (`tauri dev`). Same bug existed in the earlier Electrobun version. Typing "echo hi" rendered as "ececho hi".

**Root cause:** Missing locale environment + non-login shell. When a macOS app launches from `/Applications` (via Finder/launchd), it gets a barebones environment — no `LANG`, no `LC_ALL`. In dev mode, `tauri dev` inherits the full terminal environment, so everything works. Without `LANG=en_US.UTF-8`, zsh's ZLE and plugins (autosuggestions, syntax highlighting, Starship) miscalculate character widths on the first keystroke, position the cursor wrong, and the first character renders with ghost artifacts.

**Fix (pty.rs):**
```rust
Command::new(shell_path)
    .arg("-l")  // login shell — sources ~/.zprofile, /etc/zprofile
    .env("LANG", "en_US.UTF-8")
    .env("LC_ALL", "en_US.UTF-8")
```

**What we ruled out first (all dead ends):**
- Custom native menu / `Menu::default` — no effect
- Menu event listener in JS — no effect
- Ghost processes — none found
- Doubled IPC calls — debug logs showed input fires once, output seq numbers are clean
- xterm.js `attachCustomKeyEventHandler` workaround intercepting all printable chars — no effect
- Recent code regression — bug existed in older commits too (`b190e64`, `c761d23`)

**Lesson:** When spawning PTY shells from a GUI app on macOS, ALWAYS set locale env vars and spawn as a login shell. The launchd environment is not the same as a terminal environment. This applies to any framework (Tauri, Electrobun, Electron).

## 2026-03-17 — Implement ~/.plexi directory: workspace persistence + config file

Added filesystem persistence for workspaces and a global config file. Structure:

```
~/.plexi/
  config.json          # global settings (terminal, shell, keyboard)
  workspaces/
    default.json       # workspace layout + contexts + panel metadata
    <name>.json        # future: multiple named workspaces
```

**Key decisions:**

1. **Workspaces are named files, not a single workspace.json.** Each workspace is `~/.plexi/workspaces/<name>.json`. Currently only "default" is used, but the API supports multiple named workspaces for future workspace switching.

2. **Config overrides in workspace files.** Workspace documents already serialize `terminal` and `keyboard` keys. These can override the global config via `resolveConfig()` in `plexi-config.js`. No new format needed.

3. **Config file written on first launch.** If `~/.plexi/config.json` doesn't exist, defaults are written from `plexi-config.js`. Values come from the existing hardcoded constants in `app-constants.js`. Comments in the code note which settings aren't actually wired up yet (theme, fonts, keybinds).

4. **localStorage kept as fallback.** Every save still writes to localStorage in addition to disk. This means the app degrades gracefully if the disk write fails.

5. **Skipped "profiles" concept.** Profiles would bundle config + workspace together — unnecessary complexity until users ask for it.

6. **Rust side uses `dirs` crate** for `home_dir()`. Workspace names are sanitized to prevent path traversal.

**New files:** `src-tauri/src/config.rs`, `src/mainview/plexi-config.js`
**Modified:** `lib.rs` (6 new commands), `tauri-session-bridge.js` (bridge stubs → real IPC), `workspace-storage.js` (tauri mode support), `app.js` (config loading + mode checks)

## 2026-03-17 — Future enhancement: scriptable workspace layouts
Like tmuxinator/tmuxp — user-defined named layouts that open split panes with specific commands pre-launched (e.g. "dev stack" = frontend + backend side-by-side). First-class differentiator for Plexi. Not MVP — shelved until there are users.

## 2026-03-17 — Real PTY sessions fixed on macOS Tauri

**Status: resolved**

The actual root cause of `[session failed] undefined` was not the frontend retry loop. It was the PTY backend.

- `pty-process 0.4` was being used with the older borrowed-PTS spawn API. On macOS this fails during controlling-terminal setup with `Inappropriate ioctl for device (os error 25)`, so `open_session` rejected before the shell ever started.
- The frontend then rendered `error.message`, but Tauri invoke errors can arrive as plain strings/objects, so the user-facing result became `undefined` instead of the real backend error.

**Fixes applied:**

1. Upgraded `pty-process` from `0.4` to `0.5.3`.
2. Switched PTY creation to `blocking::open()` and moved the slave PTY into `Command::spawn(...)` using the current API, which works on macOS.
3. `spawn_shell()` now returns the resolved working directory so the frontend gets a real `cwd` immediately.
4. Tauri bridge errors are normalized to real `Error` objects before surfacing to the UI.
5. Added a Rust session test that opens a real shell, sends `printf '__PLEXI_OK__\n'`, and verifies the output round-trip.
6. Native `npm run dev` smoke check now shows successful session creation in logs:
   - `Spawned shell: /bin/zsh (80x24)`
   - `Opened session panel-1 with shell zsh (80x24)`

**Additional Tauri architecture issue found:**

- `beforeDevCommand` used `npx serve src -l 1415`, and if port `1415` was busy it silently picked a random port while Tauri still loaded `http://localhost:1415/mainview/`. That creates stale-frontend debugging traps. Replaced it with `python3 -m http.server 1415 --bind 127.0.0.1 --directory src` so port conflicts fail loudly instead of drifting.

## 2026-03-17 — Real PTY sessions not opening: current blocker

**Status: unresolved — handing off**

The Tauri IPC bridge is now wired up and `window.__TAURI_INTERNALS__` is detected correctly, so the app is no longer falling back to the mock shell. However, real zsh sessions are still not starting successfully. Symptoms:

- UI shows `[session failed] <error>` in the terminal panel
- `poll_session_output` floods the console with "Session not found" (hundreds of times before stopping)

**What was fixed in this session:**

1. **`window.__TAURI__` not injected**: `withGlobalTauri: true` added to `tauri.conf.json` under `app`. Without it, `window.__TAURI__` is undefined and the bridge falls back to mock every time.

2. **Wrong detection check**: `hasTauriRuntime()` was checking `window.__TAURI__.invoke` (Tauri 1.x location) but Tauri 2.x puts it at `window.__TAURI__.core.invoke`. Fixed to use `window.__TAURI_INTERNALS__` for detection (always injected by Tauri regardless of `withGlobalTauri`) and `getInvoke()` helper that tries `__TAURI__.core.invoke` then falls back to `__TAURI_INTERNALS__.invoke`.

3. **PTY spawn with bad CWD**: Workspace restored from localStorage had `cwd: "/mock/project"` (from old mock sessions). `pty.spawn_shell()` with a non-existent CWD fails. Fixed in `pty.rs` to silently fall back to `$HOME` if the saved CWD path doesn't exist.

4. **Infinite retry loop on session failure**: `ensurePanelSession` was called on every `render()`. When `openSession` threw, it called `panelSessions.delete(panel.id)`, which allowed the next render to retry immediately — infinite loop. Also called `render()` from inside the catch block, making it worse. Fixed by adding a `panelSessionFailed` Set; failed sessions are not retried until explicitly closed.

5. **Polling loop on session not found**: `_startPolling` caught errors with `console.error` but never stopped the interval. 1000+ "Session not found" errors per run. Fixed: stop polling after 3 consecutive errors.

6. **`just dev-fresh`**: Added `justfile` with `dev` and `dev-fresh` recipes. `dev-fresh` uses `tauri dev --config` to override `devUrl` to `src/fresh.html`, which clears `localStorage["plexi.workspace.v2"]` before redirecting to `/mainview/`. Eliminates stale mock-era workspace state on startup.

**Current state / what the next agent should investigate:**

After all the above fixes, `just dev-fresh` + `Cmd+N` still shows `[session failed]` and "Session not found" errors (though now only ~13 instead of 1000+). The root cause is not yet confirmed. Key things to check:

- **What is the actual error message from `open_session`?** Add `console.error("openSession failed:", error)` to the catch block in `ensurePanelSession` in `app.js` and check DevTools console. The error string from Rust will say whether it's "Failed to spawn PTY: ..." or "Session already exists" or something else.
- **Is `open_session` even being called?** Add a `console.log` before the `invoke("open_session", ...)` call in `tauri-session-bridge.js` to confirm IPC is reaching Rust.
- **Is the Tauri app being fully rebuilt?** Changes to `pty.rs` require a full Rust rebuild. `npm run dev` triggers this, but `just dev` may not if the Tauri watcher doesn't detect the change. Confirm with `cargo build` directly.
- **Check Tauri logs**: Run `RUST_LOG=debug npm run dev` or look at `~/Library/Logs/dev.plexi/` for PTY spawn errors.
- **The remaining 13 "Session not found" errors**: These come AFTER the polling stop-on-3-errors fix. 13 / 3 = ~4 separate polling intervals were started, meaning `open_session` succeeded for ~4 sessions before they disappeared. This suggests sessions ARE being opened (Rust side OK) but then something calls `close_session` or removes them. Possible culprit: `syncVisiblePaneRuntimes` disposes runtimes on re-render, but does NOT call `closePanelSession` — check whether `disposePaneRuntime` is inadvertently triggering session cleanup.

## 2026-03-17 — Fix Tauri app initialization and IPC bridge

Multiple issues prevented the Tauri rebuild from being functional:

1. **Electrobun bare import crash**: `session-bridge.js` had `import { Electroview } from "electrobun/view"` — a bare specifier that crashes in any non-Electrobun environment (Tauri, browser). `app.js` imported both bridges unconditionally, so this killed the entire module graph. Fix: removed Electrobun bridge import from `app.js`; `tauri-session-bridge.js` now falls back to mock bridge directly.

2. **Double log plugin registration**: `lib.rs` had `.plugin(tauri_plugin_log::...)` on the builder AND again inside `.setup()`. Also had two `.setup()` blocks. Consolidated to one empty `.setup()`.

3. **IPC parameter naming**: Tauri 2.x auto-converts camelCase JS args → snake_case Rust params. Original bridge used `panel_id` (snake_case) in JS which wouldn't match. Fixed all IPC calls to use camelCase (`panelId`, `lastSeq`, etc.).

4. **Blocking PTY reads under mutex**: `poll_session_output` locked the SessionManager mutex then did a blocking `read()` on the PTY fd. If no data, this blocked all other IPC commands. Fix: set PTY fd to `O_NONBLOCK` via `libc::fcntl` after spawn.

5. **Polling never started after openSession**: `openSession()` fired `onStarted` but never called `_startPolling()`. Terminal output never arrived. Fix: start polling immediately after successful open.

6. **Dev server for Playwright**: Added `beforeDevCommand` with `npx serve src` to `tauri.conf.json` so Tauri dev mode serves frontend over HTTP. Playwright tests now point to `/mainview/` path. All 10 e2e tests pass.

## 2026-03-15 — Fix 14px black gap on right side of xterm terminal

xterm's FitAddon (v6) subtracts a scrollbar width when `scrollback > 0`: `overviewRuler?.width || 14`. With no `overviewRuler` option set, it always subtracts 14px, leaving a black gap where the canvas doesn't reach the terminal frame edge.

Fix: set `overviewRuler: { width: 1 }` in Terminal options so FitAddon subtracts 1px instead of 14px. Then hide the resulting 1px ruler canvas (`.xterm-decoration-overview-ruler`) and the native scrollbar element (`.scrollbar.vertical`) with CSS `display: none / width: 0`. Also suppress the native viewport scrollbar with `scrollbar-width: none`.

Setting `overviewRuler: { width: 0 }` doesn't work because `0 || 14 = 14` — needs a truthy value to bypass the fallback.

## 2026-03-14 — Remove overview mode entirely

Deleted the overview feature: `#overview-shell` HTML, all `.overview-*` CSS, `mode`/`camera` state, `toggleMode`/`panCamera`/`adjustZoom`/`resetViewport` from workspace-state.js, `toggleOverview`/`zoomIn`/`zoomOut` commands, all keyboard handlers, and `renderOverview`/`renderOverviewHud` functions.

Why: Overview was decorative at this stage — no dragging, no meaningful spatial navigation beyond what the minimap already provides. The mode boundary was leaky (zoom changed terminal font size even in overview mode). An empty overview state duplicated the empty landing screen. Cut it until there's a real use case.

Also fixed two pre-existing gaps exposed by the test suite: `#focus-title` was showing directory name instead of panel title, and context rename was using a custom modal instead of `window.prompt()`. Simplified rename to native prompt. Added `#toolbar-context` and `#focus-position` to the toolbar (were already tested, just missing from HTML).
