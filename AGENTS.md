# AGENTS.md - Plexi Development

## Project Overview

**Plexi** is an infinite 2D canvas terminal multiplexer built with Electrobun (Bun-native Electron alternative).

## Self-Verification Loop (CRITICAL)

Before implementing ANY features, verify:

1. **App launches** - `bun run dev` opens a window
2. **Tests pass** - Playwright e2e tests verify UI
3. **Screenshot captured** - Visual verification

### Running Tests

```bash
# Run all tests
bun test

# Run e2e tests with Playwright
bun run test:e2e

# Capture screenshot for verification
bun run test:screenshot
```

### Test Coverage Requirements

- [ ] App window opens
- [ ] Window has correct title ("Plexi")
- [ ] Basic keyboard navigation works
- [ ] Terminal panel renders (Phase 2)

## Development Phases

### Phase 0: Hello World (CURRENT)
- [x] Initialize Electrobun project
- [ ] App window opens with "Plexi" title
- [ ] Playwright test verifies window
- [ ] Screenshot captured and sent

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
4. **Spec-driven** - PRD before implementation

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
