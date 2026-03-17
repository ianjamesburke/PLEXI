# Session Manager Design — Visible Panes Optimization

## Problem
Currently, we'd stream output from *all* PTY sessions to the renderer, even hidden ones. This wastes CPU, bandwidth, and RAM. Plexi needs to:
1. Only stream output for **visible terminals** (on-screen)
2. Queue output for **background terminals** (off-screen)
3. Flush queued output when they become visible again

## Architecture Decision

### Option A: Pull-based (Frontend requests)
```
Frontend: "Give me output for panel-123"
Backend: Finds session, reads PTY buffer, returns data
```
**Pros:** Simple, minimal memory
**Cons:** Requires polling (bad for latency) or frequent requests

### Option B: Push with visibility awareness (Frontend notifies)
```
Frontend: "I'm looking at panels [123, 456]"
Backend: Streams output only for those panels
```
**Pros:** Real-time, efficient, controlled
**Cons:** Need visibility tracking logic

### Option C: Hybrid (Recommended for Plexi)
```
- Session manager maintains **output ring buffers** per session
- Frontend sends visibility updates via "focus_panel" command
- Backend only streams to **active subscribers** (visible panes)
- Background sessions queue output in ring buffers
- When panel becomes visible: flush ring buffer, then stream
```

## Implementation Plan

### Backend Changes (src-tauri/src/session.rs)

1. **Ring Buffer Storage**
```rust
struct SessionRecord {
    panel_id: String,
    pty: Box<dyn Pty>,
    is_visible: bool,
    output_buffer: RingBuffer<Vec<u8>>, // Circular, ~1MB per session
    subscribers: HashSet<String>,        // Who's listening to this session
}
```

2. **Three modes of output streaming:**
```rust
enum OutputMode {
    Streaming,   // Immediately push to frontend
    Buffering,   // Queue in ring buffer (background pane)
    Paused,      // Not buffering anymore (after full buffer)
}
```

3. **Commands:**
```rust
#[tauri::command]
fn open_session(params: OpenSessionParams) -> Result<SessionStartedMessage>

#[tauri::command]
fn focus_panel(panel_id: String) -> Result<String> // Previously buffered output

#[tauri::command]
fn unfocus_panel(panel_id: String) -> Result<()>

#[tauri::command]
fn write_session(panel_id: String, data: String) -> Result<()>
```

### Frontend Changes (src/mainview/app.js)

1. **On workspace init:**
```javascript
// Tell backend which panels we're looking at
window.__TAURI__.invoke('focus_panel', { panel_id: visiblePanelIds });
```

2. **When user navigates (arrow keys, click):**
```javascript
// Old visible panels
await window.__TAURI__.invoke('unfocus_panel', { panel_id: oldPanel });
// New visible panels
const bufferedOutput = await window.__TAURI__.invoke('focus_panel', { panel_id: newPanel });
// Render buffered output to terminal
xterm.write(bufferedOutput);
// Now stream live output
```

3. **Listen for output events:**
```javascript
// Via WebSocket or custom Tauri event
window.__TAURI__.event.listen('session_output', (e) => {
  const { panel_id, data } = e.payload;
  if (visiblePanels.includes(panel_id)) {
    getXtermForPanel(panel_id).write(data);
  }
});
```

## Ring Buffer Tuning

Each session gets a **ring buffer** (circular byte array):
- **Size:** 1MB per session (configurable)
- **Eviction:** Oldest data falls off when buffer fills
- **Tradeoff:** More buffer = more history, but more memory per session

For typical terminal use:
- 80×24 terminal = ~2KB per screen
- 1MB buffer = ~500 screens of history
- 10 sessions × 1MB = 10MB total (acceptable)

## Reference: How Real Projects Do It

### Zellij (Rust terminal multiplexer)
- Uses **message passing** between panes
- PTY outputs to in-memory buffers per pane
- Only renders visible panes to screen
- Paused panes get occasional flushes (avoid memory bloat)

### Tmux (C terminal multiplexer)
- Maintains **scroll-back buffer** per pane (~50k lines)
- Only redraws visible panes
- Detached clients don't receive updates

### Alacritty (Rust terminal emulator)
- Single PTY, streams to grid buffer
- Grid only renders visible area (truncates off-screen)
- PTY reading is non-blocking, event-driven

## Why This Matters for Plexi

1. **Memory:** Without buffering, 100 hidden terminals would all accumulate output in RAM
2. **CPU:** Parsing xterm.js sequences for hidden terminals wastes CPU
3. **Bandwidth:** Tauri IPC has overhead; minimize messages
4. **UX:** User navigates back to a pane, sees full history (not blank)

## Implementation Steps

### Phase 1: Minimal (Get it working)
- Session manager with ring buffers
- Focus/unfocus commands
- Basic output streaming to visible panels only

### Phase 2: Optimized (Polish)
- Configurable ring buffer size
- Smart flushing (don't lose data)
- Event-based output (instead of polling)
- Metrics: track buffer fill levels

### Phase 3: Advanced (Nice-to-have)
- Compress old data in ring buffer
- Persist scrollback to disk
- Share buffers between similar sessions

## Code Template

```rust
// In src-tauri/src/session.rs

use std::collections::VecDeque;

const RING_BUFFER_SIZE: usize = 1024 * 1024; // 1MB

struct SessionRecord {
    panel_id: String,
    pty: Box<dyn std::process::Child>,
    is_visible: bool,
    output_ring: VecDeque<u8>,  // Circular buffer
}

impl SessionManager {
    fn focus_panel(&mut self, panel_id: &str) -> Result<Vec<u8>, String> {
        let session = self.sessions.get_mut(panel_id)?;
        session.is_visible = true;
        // Return buffered output
        Ok(session.output_ring.iter().copied().collect())
    }

    fn unfocus_panel(&mut self, panel_id: &str) -> Result<(), String> {
        let session = self.sessions.get_mut(panel_id)?;
        session.is_visible = false;
        Ok(())
    }

    fn append_output(&mut self, panel_id: &str, data: &[u8]) -> Result<(), String> {
        let session = self.sessions.get_mut(panel_id)?;
        
        // Add to ring buffer
        for byte in data {
            if session.output_ring.len() >= RING_BUFFER_SIZE {
                session.output_ring.pop_front(); // Evict oldest
            }
            session.output_ring.push_back(*byte);
        }

        // If visible, stream to frontend
        if session.is_visible {
            // TODO: Send via event
        }
        Ok(())
    }
}
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_eviction() {
        let mut session = SessionRecord::new("test".to_string());
        let data = vec![0; RING_BUFFER_SIZE + 100];
        session.append_output(&data);
        assert_eq!(session.output_ring.len(), RING_BUFFER_SIZE);
        assert_eq!(session.output_ring.front(), Some(&100)); // Oldest 100 bytes evicted
    }

    #[test]
    fn test_focus_returns_buffered() {
        let mut manager = SessionManager::new();
        manager.open_session("test".to_string());
        manager.append_output("test", b"hello");
        manager.unfocus_panel("test"); // Hide it
        manager.append_output("test", b" world"); // Still buffer it
        let buffered = manager.focus_panel("test").unwrap();
        assert_eq!(buffered, b"hello world");
    }
}
```

## Next Steps

1. Implement `SessionRecord` with `VecDeque<u8>` ring buffer
2. Add `focus_panel` and `unfocus_panel` commands
3. Wire up frontend to call focus when panes change
4. Test: open multiple panes, navigate around, verify no data loss
5. Measure memory usage with 10+ hidden sessions

---

**TL;DR:** Use ring buffers per session. Only stream visible terminals. Frontend tells backend what's visible. Background sessions queue output until they're shown.
