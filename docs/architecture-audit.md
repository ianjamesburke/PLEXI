# Plexi Architecture Audit

## Current Debt Hotspots

### 1. Command routing is duplicated and stringly typed
- The same workspace commands are hard-coded in the menu, Bun process, and renderer.
- Impact: adding or renaming a command requires coordinated edits across multiple files and makes regressions easy.

### 2. Main-process wiring mixes boot, menu dispatch, window creation, and RPC setup
- `src/bun/index.ts` owns unrelated responsibilities in one file.
- Impact: Electrobun lifecycle changes and future windows/views will be harder to add safely.

### 3. PTY infrastructure is packed into one session manager
- Native library resolution, shell bootstrap workarounds, and session lifecycle live together.
- Impact: terminal backend improvements and future renderer swaps become harder to test independently.

### 4. Renderer app orchestration is monolithic
- `src/mainview/app.js` currently owns persistence, command handling, DOM querying, xterm bootstrapping, terminal lifecycle, and render-time event binding.
- Impact: behavior is harder to reason about, and the repeated listener registration pattern is an avoidable leak risk.

### 5. Persistence is renderer-local instead of a durable app service
- Workspace state is stored in `localStorage`, which is good enough for headless verification but not the intended local-disk ownership model from the PRD.
- Impact: restore semantics stay tied to one view and do not establish a future settings/workspace service boundary.

## Refactor Target

For the MVP, keep the current behavior but align the codebase around four explicit layers:

1. Shared contracts
- Command names
- RPC schema
- workspace-state helpers

2. Bun process
- window boot
- menu wiring
- PTY/session infrastructure

3. Webview renderer
- UI controller
- terminal runtime
- persistence adapter

4. Future extension seams
- workspace persistence service
- alternative terminal backend/renderer
- multi-window or multi-view app shell

## Decisions For This Refactor

- Keep `xterm.js` and the mock bridge because they support the current verification loop.
- Do not attempt a full persistence migration in this pass; instead, isolate storage behind a renderer adapter.
- Move duplicated command definitions into a shared module used by both Electrobun and the webview.
- Split PTY bootstrapping from session orchestration so shell/backend work stays testable.
- Replace render-time click listener attachment with delegated handlers.

## Follow-Up Work After This Pass

- Move workspace persistence behind Bun-side file-backed RPC.
- Add explicit app lifecycle cleanup tests for session teardown on window close.
- Introduce typed shared models for panels/contexts instead of open-ended objects.
