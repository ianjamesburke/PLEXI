# Handoff: Build the Plexi App Protocol Test Harness

**Date:** 2026-04-11
**Priority:** Layer 0 — unblocked, no dependencies
**Output:** `~/.plexi-alpha/plexi_test.py` (ships with the SDK alongside `plexi_sdk.py`)

---

## What You're Building

A Python test harness that lets developers (and agents) test Plexi apps without running Plexi. It spawns an app as a subprocess, sends JSON events on stdin, reads JSON draw commands on stdout, and asserts on the results.

This is the single most important developer tool for the Plexi app ecosystem. Every app — Parallax, stills generator, audio mixer, whatever — gets tested through this harness. Agents use it to iterate on apps in background worktrees.

---

## How Plexi Apps Work

Plexi apps are Python scripts that communicate via newline-delimited JSON over stdin/stdout.

**Plexi sends events (stdin):**
```json
{"type": "init", "width": 800, "height": 600, "launch_dir": "/tmp/test"}
{"type": "render", "width": 800, "height": 600}
{"type": "key", "key": "a", "modifiers": {"command": false, "shift": false, "ctrl": false}}
{"type": "click", "x": 100, "y": 200, "button": "left"}
{"type": "command", "text": "/clear"}
{"type": "shutdown"}
```

**App sends draw commands (stdout):**
```json
{"type": "rect", "x": 0, "y": 0, "w": 800, "h": 600, "fill": "#1e1e2e", "radius": 0.0}
{"type": "text", "x": 20, "y": 20, "text": "Hello Plexi!", "size": 16, "color": "#cdd6f4", "monospace": false, "bold": false}
{"type": "line", "x1": 0, "y1": 52, "x2": 800, "y2": 52, "color": "#313244", "width": 1.0}
{"type": "list", "items": [{"label": "Item 1", "secondary": "desc", "is_dir": false}], "selected": 0, "item_height": 40.0}
{"type": "frame_done"}
```

**App sends API requests (stdout):**
```json
{"type": "cost_report", "app_id": "test", "service": "anthropic", "cost_usd": 0.01, "timestamp": "2026-04-11T00:00:00Z"}
```

**State protocol (new — implement in harness even if apps don't use it yet):**
```json
// Harness sends:
{"type": "get_state"}
// App responds:
{"type": "state", "user_state": {...}, "derived": {...}, "session": {...}, "persistent": {...}}

// Harness sends:
{"type": "set_state", "user_state": {...}, "derived": {...}, "session": {...}, "persistent": {...}}
// App restores and re-renders on next render event
```

---

## What the Harness Should Do

### Core Class: `AppTestHarness`

```python
class AppTestHarness:
    def __init__(self, entry_point: str, launch_dir: str = "/tmp/plexi-test"):
        """Spawn the app as a subprocess."""
        
    def send_init(self, width=800, height=600, launch_dir=None):
        """Send init event. Called once at start."""
        
    def send_render(self, width=None, height=None):
        """Send render event. Returns list of draw commands up to frame_done."""
        
    def send_key(self, key: str, command=False, shift=False, ctrl=False):
        """Send a key event."""
        
    def send_click(self, x: float, y: float, button="left"):
        """Send a click event."""
        
    def send_command(self, text: str):
        """Send a command event (e.g., '/clear')."""
    
    def get_state(self) -> dict:
        """Send get_state, return the state dict."""
        
    def set_state(self, state: dict):
        """Send set_state to restore app state."""
    
    def read_until_frame_done(self, timeout=5.0) -> list[dict]:
        """Read stdout lines until frame_done. Returns list of draw commands."""
        
    def read_events(self, timeout=1.0) -> list[dict]:
        """Read any pending stdout events (cost_report, api requests, etc.)."""
    
    def shutdown(self):
        """Send shutdown event, wait for process to exit."""
    
    def assert_text_visible(self, text: str, frames=None):
        """Assert that a text draw command contains the given string."""
        
    def assert_rect_at(self, x: float, y: float, frames=None):
        """Assert that a rect exists at approximately (x, y)."""
    
    def find_texts(self, frames=None) -> list[str]:
        """Extract all text content from draw commands."""
```

### Convenience Functions

```python
def test_app_lifecycle(entry_point: str):
    """Smoke test: init → render → verify frame_done → shutdown. No crashes."""
    
def test_app_state_symmetry(entry_point: str):
    """Get state, set state, get state again. Verify they match."""
    
def test_app_key_handling(entry_point: str, keys: list[str]):
    """Send a sequence of keys, verify app doesn't crash, state changes."""
```

---

## Example Test

```python
from plexi_test import AppTestHarness

def test_wikipedia_search():
    h = AppTestHarness("wikipedia.py")
    h.send_init()
    
    # Verify initial render shows search UI
    frames = h.send_render()
    texts = h.find_texts(frames)
    assert any("Search" in t or "Wikipedia" in t for t in texts)
    
    # Type a search query
    for char in "python":
        h.send_key(char)
    h.send_key("Enter")
    
    # Wait for results (app fetches async, need multiple renders)
    import time
    time.sleep(2)
    frames = h.send_render()
    texts = h.find_texts(frames)
    
    # Should have search results
    assert len(texts) > 3, f"Expected results, got: {texts}"
    
    h.shutdown()
```

---

## Key Files to Reference

- **SDK (the thing apps import):** `~/.plexi-alpha/apps/wikipedia/plexi_sdk.py`
- **Example app:** `~/.plexi-alpha/apps/wikipedia/wikipedia.py`
- **App protocol spec:** `~/Documents/GitHub/PLEXI/docs/specs/subsystems/app-infrastructure.md`
- **Parallax app spec (state protocol):** `~/Documents/GitHub/parallax/docs/parallax-plexi-app-spec.md` §9-11

---

## Implementation Notes

- Use `subprocess.Popen` with `stdin=PIPE, stdout=PIPE, stderr=PIPE`
- Set `PYTHONUNBUFFERED=1` in the subprocess environment (apps use line buffering, but belt-and-suspenders)
- Read stdout in a background thread to avoid deadlocks (app might emit multiple lines before reading stdin)
- Timeout on all reads — if the app hangs, the test should fail with a clear message, not block forever
- `frame_done` is the sentinel — everything between a render event and frame_done is one frame's draw commands
- The harness should capture stderr and include it in assertion error messages (app tracebacks are useful for debugging)

---

## Where to Put It

```
~/.plexi-alpha/
  plexi_sdk.py       ← existing SDK
  plexi_test.py      ← THIS FILE (the harness)
```

It ships alongside the SDK. Any app developer can `from plexi_test import AppTestHarness` and write tests.

---

## Success Criteria

1. Can spawn any existing Plexi app (Wikipedia, Plexi Browser) and run a smoke test without Plexi running
2. Can send key sequences and verify state changes via get_state
3. Can detect crashes (non-zero exit code, stderr output) and report clearly
4. Can run in CI (no display, no GUI, pure subprocess)
5. Tests complete in under 10 seconds for simple apps

---

## After This

Once the harness exists, the next steps are:
1. Write smoke tests for all existing apps
2. Use the harness in Parallax app development (Layer 3)
3. Integrate into CI (run on every PR)
4. Agents use the harness to iterate on apps in background worktrees
