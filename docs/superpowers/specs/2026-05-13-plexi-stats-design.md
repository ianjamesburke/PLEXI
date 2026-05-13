# Plexi Stats — Design Spec

A Plexi app that reads `events.jsonl` and displays a gaming-stats-style dashboard: full-bleed directory treemap, summary stats overlay, and a 24-hour timeline strip.

## Data Source

- **File**: `~/.plexi-{channel}/events.jsonl` (global, append-only JSONL)
- **Event type**: `focus_changed` — fires when a pane loses focus
- **Fields used**: `cwd`, `duration_secs`, `context_name`, `pane_id`, `timestamp`
- **Window**: last 24 hours from current time
- **Refresh**: manual only, triggered by `r` keybind

### Profile Directory Resolution

The app needs `~/.plexi-{channel}/events.jsonl`. Resolution order:
1. Parse `PLEXI_SOCKET` env var — the socket path contains the profile dir (e.g. `/Users/ian/.plexi-alpha/plexi.sock` → `~/.plexi-alpha/`)
2. Fallback: scan `~/.plexi*/events.jsonl` and use the one with the most recent modification time

## Data Model

### Directory Tree

1. Parse all `focus_changed` events from the last 24h.
2. Group by `cwd`, summing `duration_secs` and counting visits per unique path.
3. Build a tree rooted at `~`. Each node holds:
   - `path` (absolute)
   - `label` (last path component, or `~` for root)
   - `self_duration` (time spent directly in this directory, not children)
   - `total_duration` (self + all descendants)
   - `visits` (count of focus events with this exact `cwd`)
   - `children` (sorted descending by `total_duration`)
4. Paths are shortened relative to `~` for display.

### Timeline Entries

Ordered list of `(start_time, end_time, cwd, color_index)`. Derived from events:
- `end_time` = event `timestamp`
- `start_time` = `end_time - duration_secs`
- `color_index` = assigned from the top-level directory's position in the sorted list

### Summary Stats

- **Total active time**: sum of all `duration_secs` in the 24h window
- **Visits**: count of `focus_changed` events
- **Projects**: count of distinct `context_name` values

## Layout (top to bottom)

### 1. Breadcrumb Bar (~20px)

```
~ › 24h overview                          r to refresh · click to drill
```

Left: current treemap root path. Right: hint text. Updates on drill-down.

### 2. Treemap (fills remaining space minus stats bar and timeline)

Squarified treemap algorithm. Input: children of the current root node, sorted descending by `total_duration`. Output: rectangles filling the available bounding box.

**Rendering per cell:**
- Fill with the directory's assigned color
- Bottom-left: parent path (dimmed, small), directory name (bold), duration (large)
- Top-right: visit count (dimmed, small)
- Cells too small for text render as colored blocks only (threshold: width < 60px or height < 40px)

**Color palette**: 8 fixed hues cycled across top-level directories. When drilled into a subdirectory, children use lighter/darker shades of the parent's hue.

**Drill-down**: clicking a cell makes it the new root. Its children fill the treemap. Breadcrumb updates. Escape/Backspace pops up one level.

### 3. Stats Overlay Bar (~30px)

Centered row of three metrics:

```
6h 00m active     47 visits     5 projects
```

Each value in a distinct accent color. Subtle background, bordered top and bottom.

### 4. Timeline Strip (~40px)

Horizontal bar spanning the last 24 hours, left (24h ago) to right (now).

- Each focus session is a colored block (same color as its treemap cell)
- Gaps between sessions are dark/empty
- Hour markers below: `12a  4a  8a  12p  4p  8p  now`
- Clicking a timeline block highlights the corresponding directory in the treemap (brief flash or border)

## Interaction

| Input | Action |
|-------|--------|
| Click treemap cell | Drill into that directory (children become treemap) |
| Escape / Backspace | Go up one level in treemap |
| Click timeline block | Highlight corresponding directory in treemap |
| `r` | Re-read `events.jsonl` and rebuild all data |

## Squarify Algorithm

Inline implementation (~60 lines). Standard squarified treemap:
1. Sort items descending by value
2. Lay out in rows, choosing horizontal or vertical split to minimize aspect ratio
3. Recurse until all items are placed

No external dependencies.

## App Structure

```
examples/stats/
├── manifest.toml
└── stats.py
```

### manifest.toml

```toml
schema_version = 1

[app]
id = "stats"
type = "app"
name = "Plexi Stats"
version = "0.1.0"
description = "Gaming-style dashboard showing directory time, visits, and activity timeline from Plexi focus events."
entry = "stats.py"

[app.capabilities]
capabilities = []

[launch]
layout_hint = { side = "right", split = 0.4 }
```

### stats.py — Module Outline

1. **Constants**: color palette, layout metrics, font sizes
2. **`parse_events(path, hours=24)`**: read JSONL, filter to `focus_changed` in window, return list of event dicts
3. **`build_tree(events)`**: group by `cwd`, build tree, compute rollups, return root node
4. **`build_timeline(events)`**: return ordered list of `(start, end, cwd, color)` tuples
5. **`squarify(items, rect)`**: compute treemap layout, return list of `(item, x, y, w, h)`
6. **`StatsApp(App)`**:
   - `on_init`: resolve events.jsonl path, parse, build tree + timeline + stats
   - `on_render`: draw breadcrumb → treemap → stats bar → timeline
   - `on_key`: handle `r` (refresh), Escape/Backspace (drill up)
   - `on_click`: hit-test treemap cells (drill down) and timeline blocks (highlight)

## Non-Goals

- Real-time / auto-refresh (manual only)
- Peak concurrent panes metric (focus events are sequential, can't reliably derive)
- Per-file granularity (only directory-level)
- Historical data beyond 24h (could be added later with a time-range selector)
- Persistence of drill-down state across restarts
