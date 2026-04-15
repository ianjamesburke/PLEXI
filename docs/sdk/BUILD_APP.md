# Build a Plexi App — Agent Development Skill

This is the development loop for building Plexi apps. It's test-driven: write the tests first, then iterate the app until they pass. An agent (or human) follows this flow to produce a working app from a description.

---

## Prerequisites

Two files must be available:
- `plexi_sdk.py` — the app SDK (import `App`, decorate handlers, call `app.run()`)
- `plexi_test.py` — the test harness (import `AppTestHarness`, spawn apps, assert on output)

Both live at `~/.plexi-alpha/`. Read them to understand the protocol before starting.

---

## The Loop

### Step 1: Write the manifest

Create `manifest.toml` with app metadata and required capabilities:

```toml
[app]
id = "my-app"
name = "My App"
entry = "app.py"
version = "0.1.0"
description = "One sentence."

[app.capabilities]
filesystem = "read_only"

[app.secrets]
required = []
optional = []
```

### Step 2: Write optimistic tests

Before writing any app code, write tests that describe what a perfect app looks like:

```python
from plexi_test import AppTestHarness

def test_app_starts():
    h = AppTestHarness("app.py")
    h.send_init()
    frames = h.send_render()
    assert len(frames) > 0, "App should render something"
    h.assert_text_visible("My App", frames)
    h.shutdown()

def test_app_handles_input():
    h = AppTestHarness("app.py")
    h.send_init()
    h.send_key("a")
    frames = h.send_render()
    # Assert whatever the app should show after pressing 'a'
    h.shutdown()

def test_app_state_roundtrip():
    h = AppTestHarness("app.py")
    h.send_init()
    h.send_key("a")
    state = h.get_state()
    h.set_state(state)
    state2 = h.get_state()
    assert state == state2, "State should survive roundtrip"
    h.shutdown()
```

All tests will fail. That's correct.

### Step 3: Write the app

Build `app.py` using `plexi_sdk.py`. Implement enough to pass the first test (rendering something). Run the tests.

### Step 4: Iterate (the two-way loop)

```
Run tests
  │
  ├─ All pass → Done. Go to Step 5.
  │
  ├─ Some fail →
  │     │
  │     ├─ Attempt 1: Fix the APP code. Run tests.
  │     │
  │     ├─ Still failing?
  │     │     │
  │     │     ├─ Attempt 2: Examine the TESTS. Are they asserting
  │     │     │  the right thing? Fix if wrong. Run tests.
  │     │     │
  │     │     ├─ Still failing?
  │     │     │     │
  │     │     │     ├─ Attempt 3: Fix the APP again with fresh eyes.
  │     │     │     │  Read the error message carefully. Run tests.
  │     │     │     │
  │     │     │     ├─ Still failing?
  │     │     │     │     │
  │     │     │     │     └─ STOP. Report: which test, what error,
  │     │     │     │        what you tried. Human decides next step.
  │     │     │     │
  │     │     │     └─ Pass → continue to next failing test
  │     │     │
  │     │     └─ Pass → continue to next failing test
  │     │
  │     └─ Pass → continue to next failing test
  │
  └─ Loop until all pass or 3 failed attempts on a single test
```

**Rules:**
- Never skip a failing test. Fix or escalate.
- Alternate between app and test fixes. Don't only stare at one side.
- After 3 failed attempts on one test, stop and report. Don't loop forever.
- Each attempt should try something DIFFERENT, not the same fix again.

### Step 5: Write agents.md

After the app works, write `agents.md` describing how an agent can interact with it programmatically:

```markdown
# My App — Agent Interface

## Available Commands
- `/do-thing` — does the thing
- `/set-value <name> <value>` — sets a value

## Key Events
- `Enter` — confirm current selection
- `j`/`k` — navigate up/down

## State Shape
- `user_state.selected_item` — currently selected item index
- `user_state.items` — list of items
- `persistent.saved_data` — data written to disk
```

### Step 6: Install and smoke test

```bash
cp -r ./my-app ~/.plexi-alpha/apps/my-app/
```

Open Plexi, verify the app appears and works visually. The tests verified the protocol — this step verifies the rendering looks right.

---

## File Structure When Done

```
my-app/
  manifest.toml
  app.py
  plexi_sdk.py      ← copied from SDK
  plexi_test.py      ← copied from SDK
  agents.md
  tests/
    test_app.py
```

---

## For Agents Building Apps

If you are an AI agent following this skill:

1. **Read `plexi_sdk.py` first.** Understand the protocol, the decorators, the draw commands.
2. **Read `plexi_test.py` second.** Understand the harness API.
3. **Write tests BEFORE code.** The tests are the spec. They define what "done" means.
4. **Follow the iteration loop exactly.** Don't skip the alternating app/test fix pattern.
5. **Report after 3 failed attempts.** Don't spin forever. A human should see what's stuck.
6. **Write `agents.md` last.** You just built the app — you know exactly what it can do. Document it for the next agent.
