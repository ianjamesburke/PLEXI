# plexi demo — Interactive Onboarding Tutorial

**Status:** Approved  
**Date:** 2026-05-26  
**Scope:** MVP — 2 steps (split pane, navigate panes). Proves the full loop: welcome → keybinding prompt → event detection → progress feedback → completion.

---

## Goal

`plexi demo` is a terminal-based interactive tutorial that teaches new users the two most fundamental Plexi keybindings: splitting a pane (⌘D) and navigating between panes (⌘L / ⌘H). It runs inside an existing Plexi pane, watches the event log for state changes, and gives real-time feedback as the user completes each step.

---

## Architecture

Three components:

1. **`HostEvent::PaneSplit`** — new event variant, emitted by the host on every pane split.
2. **`plexi demo` CLI command** — top-level subcommand, no further subcommands.
3. **`demo_cli()` function** — the interactive loop that renders UI, polls events, and tracks progress.

---

## Component 1 — `HostEvent::PaneSplit`

**File:** `src/event_log.rs`

Add to the `HostEvent` enum:

```rust
PaneSplit {
    pane_id: u64,       // ID of the newly created pane
    direction: String,  // "horizontal" | "vertical" | "right" | "down"
    timestamp: String,
}
```

**Emit sites:** wherever the four split actions dispatch in the host (`SplitHorizontal`, `SplitVertical`, `SplitRight`, `SplitDown`). One grep of `SplitHorizontal` across `src/` will find the dispatch point. Emit after the new pane is created and assigned a pane_id.

**Wire format** (JSONL line):
```json
{"source":"plexi/plexi","kind":"pane_split","pane_id":23,"direction":"horizontal","timestamp":"..."}
```

---

## Component 2 — CLI plumbing

**File:** `src/cli_args.rs`

Add `Demo` as a top-level variant in the `Cmd` enum:

```rust
/// Run the interactive onboarding tutorial.
Demo,
```

**File:** `src/main.rs` (or wherever `Cmd` is matched)

Add dispatch:
```rust
Cmd::Demo => std::process::exit(crate::cli::demo_cli()),
```

---

## Component 3 — `demo_cli()`

**File:** `src/cli.rs`

### Flow

```
1. Print welcome banner
2. Record current byte offset of events.jsonl (tail position)
3. STEP 1 — Split a pane
   a. Print step 1 prompt with [ ⌘D ] key chip
   b. Poll loop (100ms): read new JSONL lines from offset
   c. Match: kind == "pane_split"
   d. Print "✓  1 / 2" progress line
4. STEP 2 — Navigate panes
   a. Print HJKL compass graphic
   b. Print "Press ⌘L to move right"
   c. Poll: FocusChanged where pane_id != starting_pane_id
   d. Print "Now press ⌘H to come back"
   e. Poll: FocusChanged where pane_id == starting_pane_id
   f. Print "✓  2 / 2  —  You know Plexi."
5. Exit 0
```

### Starting pane ID

Read from the `PLEXI_PANE_ID` environment variable (set inside Plexi panes). If unset (user ran the command outside Plexi), print a friendly error: `"Run this inside a Plexi pane: plexi-alpha demo"` and exit 1.

### Event log path

Use the channel-correct config dir:
```rust
let events_path = crate::config::config_dir().join("events.jsonl");
```

This automatically resolves to `~/.plexi-alpha/events.jsonl` on alpha, `~/.plexi/events.jsonl` on stable, etc.

### Tail-from-offset

At startup, `seek_to_end` to record the current file size as `start_offset`. The poll loop opens the file, seeks to `start_offset`, reads any new bytes, splits on newlines, and parses each line as `serde_json::Value`. This ensures the demo only reacts to events emitted *after* the command started.

### Polling interval

`std::thread::sleep(Duration::from_millis(100))` between reads. Imperceptible latency for a keybinding action.

### ANSI output

No external crate. Use raw ANSI escape codes:
- Dim/reset: `\x1b[2m` / `\x1b[0m`
- Bold: `\x1b[1m`
- Green: `\x1b[32m`
- Cyan: `\x1b[36m`

### Welcome banner

```
  ┌─────────────────────────────────┐
  │         Welcome to Plexi        │
  │   An interactive quick-start    │
  └─────────────────────────────────┘

  You'll learn 2 essential moves.
  Follow the prompts — Plexi will
  detect each action automatically.
```

### Step 1 prompt

```
  ─────────────────────────────────
  Step 1 of 2 — Split a pane

  Press  [ ⌘D ]  to split this pane.

  (Waiting...)
```

### Progress after step 1

```
  ✓  1 / 2
```

### Step 2 prompt + HJKL graphic

```
  ─────────────────────────────────
  Step 2 of 2 — Navigate panes

         [ K ]
    [ H ]     [ L ]
         [ J ]

  Press  [ ⌘L ]  to move right.
  (Waiting...)
```

After detecting rightward focus change:

```
  Good. Now press  [ ⌘H ]  to come back.
  (Waiting...)
```

### Completion

```
  ✓  2 / 2

  You know Plexi.
  Try  plexi --help  to see everything else.
```

---

## Error handling

| Situation | Behavior |
|---|---|
| `PLEXI_PANE_ID` not set | Print usage hint, exit 1 |
| `events.jsonl` does not exist | Print "Plexi doesn't appear to be running", exit 1 |
| File read error mid-loop | Log warning, continue polling (transient) |
| User hits Ctrl+C | Default signal handling — exits cleanly |

---

## What this is NOT

- Not a full SDK app — no manifest, no renderer, no SDK dependency.
- Not persistent — exits after 2/2 complete.
- Not testing all keybindings — scope is deliberately 2 steps for MVP.

---

## Future steps (not in this PR)

- Step 3: Open an app (`plexi open`)
- Step 4: Use the command palette (⌘K)
- Configurable keybinding display pulled from `keys.rs` at runtime
- `--reset` flag to replay from the beginning
