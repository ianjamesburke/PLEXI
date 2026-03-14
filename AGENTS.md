# AGENTS.md - Plexi Development

## Project Overview

**Plexi** is an infinite 2D canvas terminal multiplexer built with Electrobun (Bun-native Electron alternative).

## Product Specs

Plexi uses two product requirement documents:

- [`PRD-mvp.md`](./PRD-mvp.md) for the current shippable target
- [`PRD-future.md`](./PRD-future.md) for deferred enhancements and longer-term vision

When planning or implementing features, treat the MVP PRD as the default scope boundary unless the user explicitly asks to work from the future PRD.

## Future Enhancements Docs

Deferred architecture work and longer-horizon refactors can be documented in [`docs/future-enhancements/`](./docs/future-enhancements/).

When the user explicitly asks about future work, deep refactors, or deferred improvements, check that directory for relevant markdown specs before planning or implementing. Treat those docs as supporting guidance, not default MVP scope.

## Electrobun Reference

When working with Electrobun APIs or platform behavior, consult [`llms.txt`](./llms.txt) alongside this file. It contains the project's framework-specific guidance for main-process imports, `views://` URLs, and other Electrobun constraints.

## Self-Verification Loop (WORKING ✅)

The verification loop uses **Playwright headless browser only** to test UI without needing a display.
Agents must not launch the native Electrobun window for routine verification unless the user explicitly asks for a native-window check.

### How It Works

1. Build the app views with `bun run build`
2. Serve the repo root over local HTTP in the background
3. Playwright loads the built `build/.../views/mainview/index.html`
4. Runs assertions against DOM elements and terminal behavior
5. Captures screenshot for visual verification
6. No X11/Wayland display required

### Running Tests

```bash
# Run Playwright e2e tests
bun run test:e2e

# View test results
cat test-results/.last-run.json

# Screenshot saved to
ls tests/e2e/screenshot.png
```

### Test Coverage

- [x] Page loads with correct title ("Plexi")
- [x] Main heading renders ("Plexi")
- [x] Subtitle renders ("Infinite 2D Canvas Terminal Multiplexer")
- [x] Canvas placeholder visible
- [x] Screenshot captured
- [x] Keyboard navigation
- [x] xterm assets load from built view output
- [x] Terminal accepts a simple command (`help`) and renders output

### Adding New Tests

Add tests to `tests/e2e/`. Example:

```typescript
import { test, expect } from '@playwright/test';

test('new feature works', async ({ page }) => {
  await page.goto('http://127.0.0.1:4173/build/dev-macos-arm64/plexi-dev.app/Contents/Resources/app/views/mainview/index.html');
  // assertions here
});
```

## Development Phases

### Phase 0: Hello World (COMPLETE ✅)
- [x] Initialize Electrobun project
- [x] UI renders with "Plexi" title
- [x] Playwright test verifies UI
- [x] Screenshot captured and verified

### Phase 1: Core Canvas MVP
- [ ] Infinite 2D viewport (pan/zoom)
- [ ] Basic terminal panel (xterm.js)
- [ ] JSON config save/load
- [ ] Context system (sessions)
- [ ] Keyboard navigation

### Phase 2: SSH & Multiplexing
- [ ] SSH auto-connect
- [ ] Connection pooling
- [ ] Visual links between panels

### Phase 3: Advanced (Future)
- [ ] libghostty terminal integration
- [ ] Browser panels
- [ ] Notifications
- [ ] Excalidraw panels

## Architecture

```
plexi/
├── src/
│   ├── bun/            # Bun main process
│   │   └── index.ts    # Entry point
│   ├── mainview/       # WebView UI
│   │   ├── index.html
│   │   ├── index.css
│   │   └── index.ts
│   └── shared/         # Shared types (future)
├── tests/
│   └── e2e/            # Playwright tests
├── package.json
├── electrobun.config.ts
└── AGENTS.md           # This file
```

## Key Constraints

1. **Windows-first** - MVP targets Windows 11+ (dev on Linux ok)
2. **Electrobun** - NOT Electron (smaller, faster)
3. **Self-testing** - All features must have e2e tests
4. **Spec-driven** - MVP PRD before implementation, future ideas go in the future PRD

## Tech Stack

- **Runtime:** Bun (via Electrobun)
- **UI:** TypeScript + HTML/CSS (no framework initially)
- **Terminal:** xterm.js → libghostty (later)
- **Testing:** Playwright
- **Config:** JSON

## Reference Projects

- [cmux](https://github.com/manaflow-ai/cmux) - Ghostty-based terminal (macOS)
- [t3code](https://github.com/pingdotgg/t3code) - Minimal coding agent GUI
- [Niri](https://github.com/niri-wm/niri) - Scrollable-tiling compositor

## Commands for Agents

```bash
# Development
bun run dev         # Start dev server
bun run build       # Build for production

# Testing (ALWAYS RUN BEFORE COMMITS)
bun test            # Unit tests
bun run test:e2e    # Playwright e2e

# Verification
bun run verify      # Full verification loop
```

## libghostty Integration (Future)

Ghostty's terminal rendering library. Will replace xterm.js for:
- GPU-accelerated rendering
- Better performance
- Ghostty config compatibility

Repository: https://github.com/ghostty-org/ghostty

Integration requires native bindings (Zig or Rust FFI).
