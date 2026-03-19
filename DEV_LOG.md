<!-- DEV_LOG.md — decision journal for the Plexi project. Newest entries at the top. Records non-obvious choices, abandoned approaches, and root causes so future sessions don't repeat mistakes. -->

## 2026-03-19 — Fix: `clear` content reappearing after zoom/navigate

Root cause: alacritty's `grow_lines()` explicitly pulls scrollback content into the visible area whenever the terminal gains rows. This happens during zoom/navigate — a pane shrinks (tile tree placeholder size), then grows again (zoom overlay size), and old cleared content from scrollback fills the new rows.

Fix in `deps/egui_term/src/backend/mod.rs::resize()`: capture `old_lines` before resize, then call `terminal.grid_mut().clear_history()` if lines grew. Also added `scroll_display(Scroll::Bottom)` after resize to snap viewport on any reflow.

**Known tradeoff:** `clear_history()` nukes ALL scrollback when the terminal grows — not just the lines pulled in. Legitimate scrollback is lost on zoom-in. A future improvement would be to only trim the N lines that `grow_lines` pulled from history, rather than wiping everything.

---

## 2026-03-19 — Repo cleanup: promote egui crate to root, remove legacy code

Removed all legacy codebases (Tauri, Electrobun/Node.js, Playwright tests, npm configs) and promoted `plexi-egui/` to root level. Binary renamed from `plexi-egui` to `plexi`. The `deps/egui_term` path dependency updated accordingly. Icon copied from `src-tauri/icons/icon.png` to `assets/app-icon.png` before deleting `src-tauri/`. Now installable via `cargo install --git`. README rewritten for pure Rust egui architecture.

---

## 2026-03-19 — Remove sidebar minimap

Removed the non-functional minimap section (Map label, node count, visual minimap widget) from the sidebar. It was visual-only clutter with no interactivity. Candidate for future re-implementation as a real feature once pane navigation warrants it.

---

## 2026-03-19 — [FIX] Zoom + tab cycling desync

Tab cycling (Cmd+]/[) while zoomed updated `focused_pane` to the new tab's TileId but left `zoomed_pane` pointing at the old TileId. Result: the dot indicator switched correctly but the overlay kept rendering the old terminal. Unzoom (Cmd+Enter) also failed on first press because the toggle's equality check (`zoomed_pane == Some(focused)`) was comparing two different TileIds. Fix: one conditional in `cycle_tab` — if `zoomed_pane.is_some()`, update it to match the new `focused_pane`. Reinforces the pattern: any code that changes `focused_pane` needs to check whether `zoomed_pane` should follow.

---

## 2026-03-19 — Zoom/maximize pane (Cmd+Enter)

**What:** Cmd+Enter toggles a "zoom" mode that expands the focused pane to fill the central panel with a slight inset (10px), similar to tmux's zoom feature.

**Rendering approach:** Instead of hiding other panes or reparenting, the zoomed pane's slot in the tile tree renders as a dark placeholder (no terminal). After `tree.ui()`, a semi-transparent scrim (black @ 63% opacity) is painted over the entire central panel, then the zoomed terminal is rendered in an inset overlay rect on top. This avoids double-rendering the terminal (which would cause double-input) and keeps the background layout visible but dimmed as a visual cue.

**Auto-unzoom:** Split (Cmd+D/Shift+D), navigate (Cmd+HJKL), and close (Cmd+W) all clear zoom first. Tab cycling (Cmd+]/[) works while zoomed. Context switch inherently changes the active context which has its own `zoomed_pane` field.

**State:** `zoomed_pane: Option<TileId>` on `Context`. Ephemeral — not persisted to workspace file.

---

## 2026-03-19 — [FIX] Focus landing on invisible tab after closing pane

`find_first_pane_in` iterated all children for every container type, including `Tabs`. For a Tabs container, only the active tab is visible, but the function returned whichever child was first in the Vec — often an inactive/hidden tab. This meant after closing the last tab in a pane group, focus could land on a terminal hidden behind another tab. Fixed by checking for `Container::Tabs` and descending only into `tabs.active` instead of iterating all children. One function, ~3 lines added.

---

## 2026-03-19 — Functional contexts (workspaces) with disk persistence

**What:** Contexts in the sidebar are now functional workspaces (like tmux sessions). Each context owns its own tile tree, panes HashMap, and focused pane. Switching contexts swaps the entire view; background terminals keep running. Workspace state persists to `~/.plexi/workspaces/default.json`.

**Architecture decisions:**
- Tree-walking methods (`find_ancestor_tabs`, `find_logical_parent`, `find_pane_in_direction_from`, etc.) moved from `PlexiApp` to `Context` to keep the borrow checker happy — `PlexiApp` methods that need both `self.next_pane_id` and `self.contexts[i].tree` can now call context methods without conflicting borrows.
- `next_pane_id` stays global (on `PlexiApp`) because the PTY event channel is shared across all contexts — pane IDs must be unique globally.
- `close_focused` was restructured into read-only / mutable / cleanup phases to satisfy the borrow checker when accessing `Context` fields.
- Closing the last pane in a context deletes that context (unless it's the only one, then quit). This avoids empty zombie contexts.
- Workspace save uses `egui_tiles::Tree<u64>` serialization directly (serde feature on egui_tiles). On restore, terminals are re-spawned at their saved cwds; stale cwds fall back to context path → home dir.
- Corrupt workspace JSON is renamed to `.backup-{timestamp}.json` and a fresh workspace starts.

**New features:** `+` button creates contexts, double-click renames, hover `x` deletes (2+ contexts), Cmd+1-9 switches contexts, Cmd+Q/exit saves workspace.

**Explicitly deferred:** process persistence (needs daemon), auto-save timer (save-on-exit sufficient for MVP), drag-to-reorder, right-click menus.

---

## 2026-03-19 — [GOTCHA] 60% CPU in debug mode is expected — it's wgpu, not a bug

Investigated high idle CPU usage (~60% in btop). Traced the full repaint chain: eframe 0.31 is already reactive (only repaints on `request_repaint()` / `request_repaint_after()`). The only idle repaint source is cursor blink at ~2 FPS via `request_repaint_after(530ms)`. The 60% is unoptimized wgpu rendering in debug builds — confirmed by running `cargo run --release` which dropped CPU to near-zero. No code fix needed. If debug perf becomes annoying, add `[profile.dev.package."*"] opt-level = 2` to Cargo.toml to optimize deps while keeping app code debuggable.

Also removed a redundant `ctx.send_viewport_cmd(ViewportCommand::Title("Plexi"))` that ran every frame in `update()` — the title was already set once via `ViewportBuilder::with_title("Plexi")` in main.rs.

---

## 2026-03-19 — [FUTURE] Rename binary from plexi-egui to plexi

btop shows the process as "plexi-egui" because `Cargo.toml` has `name = "plexi-egui"`. Defer renaming until the Tauri codebase is removed and `plexi-egui/` becomes the sole binary. Trivial one-liner when the time comes.

---

## 2026-03-19 — [FIX] Cursor rendering: visibility, shape, and unfocused style

Fixed three cursor issues in the forked `egui_term`:

1. **Cursor always visible** — `RenderableContent` never exposed `TermMode::SHOW_CURSOR`, so apps sending `\e[?25l` (hide cursor — used by Claude Code, vim, fzf) still showed a blinking block. Added `cursor_visible` field populated from `terminal.mode().contains(TermMode::SHOW_CURSOR)`.

2. **Unfocused panes drew solid block** — standard terminal behavior (Ghostty, iTerm2, Alacritty) is a hollow 1px outline for unfocused panes. Changed from `RectShape::filled` to `RectShape::stroke` with `StrokeKind::Inside`.

3. **No cursor shape support** — alacritty_terminal tracks `CursorShape` (Block/Beam/Underline/HollowBlock/Hidden) via `term.cursor_style().shape`, but the view always drew a filled block. Added `cursor_shape` field to `RenderableContent` and a `match` in the renderer for Beam (2px vertical line), Underline (2px horizontal line at bottom), and Block (filled rect).

Also fixed text color inversion — was gated on `APP_CURSOR` mode (wrong), now gated on focused + block cursor + cursor visible (correct).

---

## 2026-03-19 — Flat tile tree for equal splits + share equalization on close

**What:** Splitting in the same direction as the parent Linear container now inserts the new pane as a sibling instead of creating a nested container. This keeps the tree flat: three horizontal splits produce three equal thirds, not 50/25/25.

**Key detail — shares on close:** The initial implementation only changed `split_focused` but missed that `close_focused` was manually transferring the closing pane's share to its neighbor (preserving uneven ratios from drag-resizing). Fixed by resetting all sibling shares to `1.0` on close, so remaining panes always redistribute equally.

**Lesson:** Create and destroy paths are coupled. When changing how something is created (split), always read the corresponding teardown (close) in the same pass. The existing share-transfer logic in `close_focused` was the clue that egui_tiles doesn't auto-equalize.

---

## 2026-03-19 — Tab stacking via egui_tiles Tabs containers

**What:** Cmd+T creates a new terminal tab stacked behind the focused pane. Cmd+]/[ cycles between tabs. Replaces Cmd+N (which created a new split alongside root).

**How it works:** `egui_tiles` has a native `Container::Tabs` type. Cmd+T wraps the focused pane + new pane in a Tabs container (or appends to an existing one if focused pane is already in a Tabs container). The tab bar (24px) only appears when a Tabs container has 2+ children — the default `SimplificationOptions::prune_single_child_tabs` auto-removes single-child Tabs containers each frame, so lone panes never show a tab bar.

**Tab bar styling:** Active tab gets terminal bg color (`0x1e1e2e`), inactive tabs get `BG_DARKEST`. Tab titles show "Terminal N" in dim text.

**New tabs inherit cwd** from the focused pane (same as splits).

**Keybindings changed:** Cmd+N removed, Cmd+T added, Cmd+]/[ added for tab cycling.

---

## 2026-03-19 — Post-MVP: tmux-style session persistence

**Deferred until after MVP ships.** Background daemon that owns PTY sessions, GUI connects as a client. Sessions survive GUI restart, processes keep running. This is the #1 differentiator from the UX research but requires an architectural shift (daemon/client split) that touches everything. Validate that people want Plexi first.

---

## 2026-03-19 — TODO: Tauri codebase cleanup / removal

**Deferred.** Once the egui rewrite is feature-complete, remove `src-tauri/`, the Node/npm toolchain, xterm.js, and all Tauri-related config. `plexi-egui/` becomes the sole binary. Benefit is operational: one Rust binary, no webview, no IPC serialization, faster startup, smaller binary.

---

## 2026-03-19 — Keybindings overhaul + app icon + macOS menu FFI (plexi-egui/)

**New keybindings:** Cmd+N (new pane), Cmd+Q (force quit — bypasses close-pane guard via `quitting` flag), Cmd+/ (shortcuts overlay, was Shift+/).

**Cmd+H fix via Cocoa FFI:** macOS intercepts Cmd+H as "Hide Application" before egui/winit see it. Tried three alternatives first:
1. `with_default_menu(false)` — removes entire menu bar, losing Edit (copy/paste) and Window menus. Too aggressive.
2. Alt+HJKL — macOS Option key produces special Unicode chars (∆, ˚, etc.) instead of the base letter, so winit reports the wrong logical key. egui docs explicitly warn against Alt-based shortcuts for this reason.
3. Cmd+[ for left + Cmd+J/K/L for rest — asymmetric and awkward.

**Solution:** Keep default menu, surgically remove "Hide" and "Hide Others" menu items via `objc2-app-kit` FFI in `macos_menu.rs`. Called from `PlexiApp::new()` (after eframe creates the window). Uses `NSApplication::mainMenu()` → first submenu → iterate items → remove those with `hide:` and `hideOtherApplications:` selectors. ~40 lines of safe-ish Rust wrapping unsafe AppKit calls. This is the same approach Ghostty uses.

**App icon:** Embedded via `include_bytes!("../../src-tauri/icons/icon.png")` + `eframe::icon_data::from_png_bytes()` + `ViewportBuilder::with_icon()`. Shows in Dock.

**New pane (Cmd+N):** Creates a fresh terminal (no inherited cwd) and inserts it alongside the root as a horizontal split.

**Dependencies added:** `objc2`, `objc2-app-kit`, `objc2-foundation` (macOS-only, behind `cfg(target_os = "macos")`). These are already transitive deps of winit so no new downloads.

---

## 2026-03-19 — Pane padding color + sizing (plexi-egui/)

Added `TERMINAL_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2e)` color constant to match the Catppuccin Mocha terminal background. Updated the pane frame in `tiling.rs` to fill with this color instead of leaving it transparent, so the inner padding inside each pane blends seamlessly with the terminal text area. Increased pane `inner_margin` from 4 to 8 for more breathing room. The outer window margin remains `BG_DARKEST` (darker black) at 4px to match the inter-pane `gap_width`, creating visual consistency around the border.

---

## 2026-03-19 — UX research: competitive patterns + opportunities

**What's working well in the space (patterns worth adopting):**
- cmux's vertical sidebar with per-workspace metadata (branch, ports, notification badges) is the breakout UX pattern — gives spatial context at a glance
- Zellij's stacked panes (collapsed title bars showing what's behind) is the cleanest "tabs behind a pane" visual — avoids the tab-bar clutter problem
- Emerging keybinding consensus: Alt+hjkl or Cmd+hjkl for splits, Cmd+[/] for tab cycling, Cmd+1-9 for workspace jumping
- Fixed sidebar ordering is a must — users cite reordering-by-activity as a top cmux frustration; muscle memory depends on stability
- Activity indicators (dot, badge, color change) on hidden/background tabs are considered essential, not nice-to-have

**cmux pain points = our opportunities:**
1. No process persistence across restart — sessions die on quit; the hardest problem but highest-value differentiator
2. Keybindings not customizable enough — low effort to fix, high user satisfaction payoff
3. Sidebar reorders by activity — actively breaks muscle memory; fixed ordering is a one-liner policy decision

**For MVP:** Don't act on any of this yet. Priority is getting a working multiplexer in front of users. Revisit sidebar metadata and activity indicators once the core split/navigate/close loop is solid.

---

## 2026-03-19 — Uniform spacing + terminal text padding (plexi-egui/)

Changed `gap_width` from `6.0` to `4.0` in `tiling.rs` so inter-pane gaps match the outer `inner_margin: Margin::same(4)` set in `app.rs`. Wrapped both the live terminal and the exited-pane message in `egui::Frame::new().inner_margin(Margin::same(4))` to give text 4px breathing room from pane edges. The focus border in `paint_on_top_of_tile()` operates on the full tile rect (before the frame inset), so it still sits flush at the tile boundary.

---

## 2026-03-18 — Phases 3–4: shell integration + polish (plexi-egui/)

**Shell integration (Phase 3):**
- Forked `egui_term` into `deps/egui_term/` as a path dependency — added `env: HashMap<String, String>` field to `BackendSettings` and wired it into `tty::Options`. Only 3 lines changed in the upstream crate.
- `shell::build_env()` sets TERM, COLORTERM, LANG, LC_ALL, prepends Homebrew PATH on macOS, and injects ZDOTDIR for zsh shell integration.
- `shell::ensure_shell_integration()` writes `.zprofile`/`.zshrc` to `~/.plexi/shell-integration/zsh/` — these source the user's real dotfiles then add a precmd hook emitting OSC 7 (cwd tracking for future split-inherits-cwd).

**Why fork instead of upstream PR:** The egui_term crate is young (v0.1.0) and the maintainer may not want env passthrough in the public API. A local path dep is the lowest-risk approach for MVP. If upstream accepts, we switch back to a version dep.

**Polish (Phase 4):**
- Exited panes show "[process exited]" centered, auto-close on any keypress.
- Window title set to "Plexi" via `ViewportCommand::Title`.
- Removed all `log::info!` debug spam from keys.rs and split_focused.
- Zeroed CentralPanel margins to eliminate padding around terminals.
- Renamed `TerminalPane.id` → `_id` to suppress unused warning.

---

## 2026-03-18 — egui rewrite: pure Rust terminal multiplexer (plexi-egui/)

**Why:** The Tauri + xterm.js architecture has fundamental TUI rendering artifacts (column mismatch, missing glyphs, no synchronized rendering). Native egui rendering via `egui_term` (which wraps `alacritty_terminal`) eliminates all of these. The `egui-poc` branch proved the approach works.

**Architecture:**
- `plexi-egui/` is a standalone Rust crate (sibling to `src-tauri/`, doesn't replace it yet)
- `egui_tiles 0.12.0` for tiled layout with drag-to-resize dividers
- `egui_term 0.1.0` wraps `alacritty_terminal` for PTY + rendering
- No Tokio — egui_term handles PTY I/O on background `std::thread` with `std::sync::mpsc`
- `Tree<PaneId>` stores only u64 IDs; actual `TerminalPane` data lives in a `HashMap`

**Key design decisions:**
- egui_tiles over egui_dock: maintained by Rerun, supports `Linear` containers with H/V splits, `Behavior` trait gives full control (hide tab bars, custom gaps, focus painting)
- Pane type is `u64` (not the full struct) — avoids borrow checker issues since Behavior receives `&mut Pane` but we need separate mutable access to the panes HashMap
- Focus border via `paint_on_top_of_tile()` with `StrokeKind::Inside` to stay within tile bounds
- Window close (`Cmd+W`) intercepted via `close_requested()` + `CancelClose` when multiple panes exist
- Keyboard shortcuts consumed via `ctx.input_mut(|i| i.consume_key(...))` BEFORE `tree.ui()` so terminals don't see them
- Split creates a new Linear container wrapping `[focused, new_tile]`, then replaces focused in its parent — egui_tiles `join_nested_linear_containers` simplification auto-flattens same-direction nesting

**Deferred to Phase 3 (requires egui_term fork):**
- `BackendSettings` has no `env` field — can't inject ZDOTDIR, LANG, COLORTERM, PATH/Homebrew. Need 3-line fork to wire env HashMap into alacritty_terminal's `tty::Options`.

---

## 2026-03-18 — E2E binary testing with tauri-plugin-webdriver

**Problem:** The official `tauri-driver` does not work on macOS — it prints "not supported on this platform" because Apple provides no WKWebView WebDriver tool. The existing Playwright tests run against a static HTTP server (mock backend, no real PTY sessions).

**Solution:** Community crate `tauri-plugin-webdriver` (Choochmeque) embeds a W3C WebDriver server inside debug builds. A companion `tauri-webdriver` CLI on port 4444 launches the `.app` binary and proxies WebDriver commands. WebdriverIO connects as the test client.

**Setup:**
- `tauri-plugin-webdriver` added as optional dep behind `webdriver` Cargo feature
- Plugin registered in `lib.rs` with `#[cfg(feature = "webdriver")]`
- Build: `cargo build --features webdriver` (or `npm run test:e2e:binary:build`)
- Run: `npm run test:e2e:binary`

**Key gotchas discovered:**
1. `browser.execute()` serializes `undefined` args as `null`, which bypasses JS default parameter values. Workaround: branch on whether the arg is defined before calling execute.
2. xterm.js with WebGL addon renders to `<canvas>`, not `.xterm-rows` divs — DOM text queries on `.xterm-rows` return empty. Use `__PLEXI_DEBUG__.getPanelBuffer()` instead.
3. `Cmd+N` / `Cmd+W` are native menu accelerators handled by macOS, not DOM key events. WebDriver can't trigger them. Use `__PLEXI_DEBUG__.runCommand()` to invoke app commands.
4. PTY sessions need ~1s after `openSession` before the shell prompt arrives. Tests must `waitForPtyReady()` before sending input.
5. No headless mode on macOS — WKWebView requires a window server. On Linux CI, Xvfb provides a virtual display.

**Alternatives evaluated and rejected:**
- `tauri-driver` (official): macOS not supported
- Appium mac2: can't access WKWebView DOM
- Playwright WebKit: can't connect to WKWebView in native apps
- Computer Use / AI vision: non-deterministic, expensive, no DOM assertions
- `danielraffel/tauri-webdriver`: similar approach but macOS-only, 3 open bugs, stale

**Test coverage (17 tests, ~25s):**
- App shell: title, sidebar/workspace render, context list, clean state
- Terminal lifecycle: open with real PTY, execute command + verify output
- Splits: split-right, close-keeps-original, split-down
- Top-level nodes: new-node-right, new-node-down
- Ephemeral directory: creates temp dir under `~/.plexi/`, cd's into it, splits pane and verifies cwd propagation via OSC 7, creates a file in one pane and reads it from the sibling, tears down temp dir (with `after()` safety net for failed runs)
- Cleanup: close all panels

**TODO:**
- Add keyboard shortcut for `new-context` (currently only accessible via sidebar button / modal)
- Context creation test needs modal automation or a programmatic API

---

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
