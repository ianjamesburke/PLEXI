# Tauri Rebuild — Next Steps

## ✅ Done
- [x] Branch created: `feature/tauri-rebuild`
- [x] Tauri project initialized
- [x] Cargo.toml configured (pty-process, tokio, uuid)
- [x] Session manager Rust module created (src-tauri/src/session.rs)
- [x] Tauri commands registered (open_session, write_session, etc.)
- [x] Migration guide written (TAURI_MIGRATION.md)
- [x] Initial commit made

## 🔄 Next: Frontend IPC Refactor (2-3 hours)

The biggest remaining work: updating `src/mainview/app.js` to use Tauri's IPC instead of Electrobun's.

### Files to modify:
1. **src/mainview/app.js** — Replace all Electrobun RPC calls with `window.__TAURI__.invoke()`
2. **src/shared/workspace-state.js** — Check if it references any Electrobun APIs
3. **Tests** — Update Playwright config to target Tauri dev server (port 1415)

### The pattern change:
```javascript
// OLD (Electrobun)
await sessionRpc.openSession(params);
await sessionRpc.on.sessionOutput(msg => { ... });

// NEW (Tauri)
await window.__TAURI__.invoke('open_session', params);
// For events: either poll or use WebSocket (TBD)
```

### Checklist:
- [ ] Search app.js for all `sessionRpc` references
- [ ] Replace with `window.__TAURI__.invoke()` calls
- [ ] Handle the output/event streaming (see TAURI_MIGRATION.md)
- [ ] Test locally: `npm run dev`
- [ ] Run: `npm run test:e2e`

## 🔧 Then: Real PTY Implementation (4-6 hours)

Currently `session.rs` has placeholder code. You need to:

1. **Use pty-process crate** to actually spawn shells
   ```rust
   let child = pty_process::Command::new("/bin/zsh")
     .spawn_pty(Some(&pty_process::PTY::new()?))
     .expect("failed to spawn");
   ```

2. **Stream terminal output** to the frontend
   - Read from PTY stdout → send to renderer
   - Write from renderer → PTY stdin
   - Handle resize events

3. **Manage session lifecycle**
   - Close PTY on `close_session()`
   - Clean up resources

**Reference:** Port the logic from `src/bun/session-manager.ts` (the original Bun implementation).

## 📊 Testing Plan

```bash
# 1. Check it builds (even if broken)
npm run build

# 2. Dev mode (hot reload, easier debugging)
npm run dev
# Open http://localhost:1415 in browser
# Try to open a terminal (will fail without real PTY)

# 3. Unit tests (shell, workspace logic)
npm run test

# 4. E2E tests (Playwright)
npm run test:e2e
```

## ⚠️ Known Blockers

1. **Output streaming:** Tauri doesn't have a built-in push mechanism like Electrobun's `sendProxy`. Options:
   - Poll: Renderer calls `get_sessions()` periodically
   - WebSocket: Add tauri-plugin-websocket (adds complexity)
   - Local state: Cache output in renderer, stream via file/IPC

2. **PTY integration:** Need real shell spawning (pty-process crate usage)

3. **xterm.js integration:** Need to import it properly in the Tauri webview

## 📋 Current Repository State

```
feature/tauri-rebuild
├── src-tauri/                  ← NEW: Rust backend
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs             ← Tauri command handlers
│   │   └── session.rs         ← Session manager (needs real PTY)
│   ├── Cargo.toml             ← With pty-process, tokio, uuid
│   └── tauri.conf.json        ← App config
├── src/                        ← OLD: Frontend (needs IPC refactor)
│   ├── bun/                   ← Old Electrobun backend (keep for reference)
│   └── mainview/              ← Frontend (update app.js for Tauri)
├── dist/                       ← Built frontend goes here
├── TAURI_MIGRATION.md          ← Detailed refactoring guide
├── TAURI_NEXT_STEPS.md         ← This file
├── RUST_REBUILD_ANALYSIS.md    ← Framework decision doc
└── package.json               ← Scripts updated to use Tauri
```

## 🎯 Estimated Timeline

- **Frontend IPC refactor:** 2-3 hours
- **Real PTY implementation:** 4-6 hours
- **Testing & debugging:** 2-3 hours
- **Total:** ~8-12 hours of work

## 🚀 Once Complete

You'll have:
- ✅ Tauri-based desktop app (replaces Electrobun)
- ✅ Real Rust backend (PTY spawning, file I/O)
- ✅ Automated testing via Playwright
- ✅ Smaller bundle size (~10MB vs ~40MB)
- ✅ No more double-input bugs

## Questions?

- **Stuck on IPC?** Check `TAURI_MIGRATION.md` for the pattern
- **PTY questions?** See `src/bun/session-manager.ts` for the reference implementation
- **Build issues?** Run `npm run dev` and check console output

Good luck! 🎉
