# Tauri Migration Guide

## Status
- ✅ Tauri project initialized
- ✅ Cargo.toml configured with PTY dependencies
- ✅ Session manager module scaffolded
- ✅ Tauri commands registered
- 🔄 Frontend IPC refactor needed
- 🔄 PTY spawning implementation needed
- 🔄 Build + test verification needed

## What Changed

### Backend (src-tauri/)
- **Session management:** Moved from `src/bun/session-manager.ts` → `src-tauri/src/session.rs`
- **Tauri commands:** Defined in `src-tauri/src/lib.rs`
- **RPC layer:** Electrobun RPC → Tauri `invoke()`

### Frontend (src/mainview/)
No structure changes, but IPC calls need updating:

**Old (Electrobun):**
```javascript
await sessionRpc.openSession({ panelId, cwd, cols, rows });
```

**New (Tauri):**
```javascript
await window.__TAURI__.invoke('open_session', { panelId, cwd, cols, rows });
```

## Migration Steps

### 1. Update Frontend IPC (src/mainview/app.js)
Find all Electrobun RPC calls and convert to Tauri `invoke()`:

```javascript
// Import Tauri at top
const { invoke } = window.__TAURI__.core;

// Replace RPC calls
async function openSession(params) {
  return invoke('open_session', params);
}

async function writeSession(panelId, data) {
  return invoke('write_session', { panel_id: panelId, data });
}

async function resizeSession(panelId, cols, rows) {
  return invoke('resize_session', { panel_id: panelId, cols, rows });
}

async function closeSession(panelId) {
  return invoke('close_session', { panel_id: panelId });
}
```

### 2. Handle Listen Events (Tauri doesn't have RPC push)
Electrobun had `sessionRpc.sendProxy.*()` to push from main to renderer.
Tauri uses the **WebView API** with `window.__TAURI__.event`:

**Old (Electrobun):**
```typescript
// main process
sessionRpc.sendProxy.sessionOutput({ panelId, data, seq });

// renderer
sessionRpc.on.sessionOutput((msg) => { ... });
```

**New (Tauri):**
```rust
// main process - emit from Rust
tauri::api::ipc::InvokeResponse::Ok(/* ... */);
// OR: send via custom event
```

For now, **poll approach:**
- Renderer calls `get_sessions()` periodically
- Or: listen for filesystem changes (workspace save)
- TODO: implement proper Tauri event system

### 3. Remove Electrobun Files
```bash
rm electrobun.config.ts
rm -rf src/bun/  # (keep for reference, not used)
rm -rf node_modules/@electrobun
```

### 4. Update package.json Scripts
```json
{
  "start": "tauri dev",
  "dev": "tauri dev",
  "build": "tauri build",
  "test": "bun test ./tests/unit/*.test.ts",
  "test:e2e": "bun run build && playwright test"
}
```

### 5. Update Playwright Config
Tauri runs on port `1415` (configurable) by default:

```typescript
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  webServer: {
    command: 'npm run build && npm run tauri dev',
    port: 1415,
    reuseExistingServer: false,
  },
  use: {
    baseURL: 'http://localhost:1415',
  },
});
```

## Build & Test

```bash
# Install Rust (if not already)
# macOS: xcode-select --install

# Build
npm run build

# Dev mode with hot reload
npm run dev

# Run tests
npm run test:e2e
```

## Known Issues

### 1. Session Output Events
Tauri doesn't have a built-in push mechanism like Electrobun's `sendProxy`.
Options:
- **WebSocket:** Use `tauri-plugin-websocket` for real-time updates
- **Polling:** Renderer polls `get_sessions()` every 100ms
- **File watching:** Listen for workspace file changes

**TODO:** Implement proper event system (likely WebSocket for terminal output).

### 2. PTY Process Spawning
`session.rs` has placeholder code. Real implementation needs:
- Use `pty-process` crate to spawn shells
- Handle stdout/stderr streaming to frontend
- Implement resize, input, cleanup

**TODO:** Port `src/bun/session-manager.ts` logic to `session.rs`.

### 3. Window Configuration
The window size is set in `src-tauri/tauri.conf.json`. Update as needed:
```json
{
  "width": 1200,
  "height": 800,
  "resizable": true
}
```

## Testing

### Unit Tests
```bash
bun test ./tests/unit/*.test.ts
```

### E2E Tests (Playwright)
```bash
# Runs tauri dev + playwright
npm run test:e2e
```

Example test:
```typescript
import { test, expect } from '@playwright/test';

test('can open a session', async ({ page }) => {
  await page.goto('http://localhost:1415');
  
  // Wait for app to load
  await page.waitForSelector('#app');
  
  // Simulate opening a session
  const result = await page.evaluate(() => {
    return window.__TAURI__.invoke('open_session', {
      panelId: 'test-panel',
      cwd: '/tmp',
      cols: 80,
      rows: 24,
    });
  });
  
  expect(result.backend).toBe('pty-process');
});
```

## Next Steps

1. **Implement real PTY spawning** in `session.rs`
   - Use `pty-process::fork()` to spawn shells
   - Stream output to frontend
2. **Build frontend IPC layer** for Tauri
   - Update all Electrobun RPC calls
   - Implement event/output mechanism
3. **Test locally**
   - `npm run dev` → verify terminal opens
   - `npm run test:e2e` → verify Playwright tests pass
4. **Compare with Electrobun**
   - Check for regressions (double-input bug should be gone)
   - Measure bundle size, startup time
5. **Deploy & monitor**

## Rollback

If Tauri doesn't work out:
```bash
git checkout main
git branch -D feature/tauri-rebuild
npm install  # Restore Electrobun deps
```

The Electrobun code is still in `src/bun/` if needed.
