# PyFlow — Visual Python Function Editor

**Status:** Spec  
**Last updated:** 2026-04-11  
**Depends on:** Advanced UI SDK, text editor primitive  
**App type:** Out-of-process (Python)

---

## Summary

A visual node canvas where each node is a Python function. Wire nodes together by connecting return types to input parameters. Double-click a node to edit the function body in an inline text editor. The canvas reads and writes real `.py` files — PyFlow is a view, not a separate format.

---

## Core Concept

- Every node = one Python `def` at module scope.
- The canvas = one `.py` file.
- Wiring nodes = connecting a function's return value to another function's input parameter.
- The underlying files are plain Python. You can always open them in Zed. PyFlow reads and writes them.

---

## Canvas View

### Nodes

Each node is a card rendered on the canvas:

```
┌─────────────────────────┐
│      transform_data     │  ← function name (editable on double-click)
├─────────────────────────┤
│ ● raw_data: dict        │  ← input ports (left edge)
│ ● threshold: float      │
├─────────────────────────┤
│ → list[dict]            │  ← output port (bottom or right edge)
└─────────────────────────┘
```

- **Input ports** (left side): one per function parameter. Label = `name: type`. Connected ports show a filled circle; unconnected show an empty circle.
- **Output port** (right side): one per function (the return type). Label = `→ return_type`.
- **Function name**: centered at top of node. Editable — renaming updates the `def` line in the `.py` file.
- **Color coding**: type-mismatch edges render red. Connected edges render in the accent color.

### Edges

Edges are cubic bezier curves (`bezier` draw command) connecting an output port to an input port. They follow standard node-editor routing: horizontal out from the source, curve to horizontal in at the target.

### Canvas Interaction

| Action | Behavior |
|--------|----------|
| Click node | Select it (highlight border) |
| Double-click node | Open the node editor modal |
| Click + drag node | Move it on the canvas |
| Click + drag from port | Start wiring an edge |
| Release on compatible port | Complete the edge |
| Release on empty space | Cancel the edge |
| Middle-click drag / two-finger scroll | Pan the canvas |
| Cmd+scroll / pinch | Zoom |
| Backspace / Delete (with node selected) | Delete the node (and its function from the file) |
| Right-click node | Context menu: Rename, Duplicate, Delete, View Source |
| `n` or toolbar button | Add new node (creates empty function) |

### Toolbar

Fixed bar at top of pane (not affected by canvas pan/zoom):

```
[+ Add Node]  [▶ Run All]  [🧪 Test Mode]  [📁 Open .py]  [File: utils.py ▼]
```

- **Add Node** — creates a new empty function, places node at center of viewport
- **Run All** — executes the full graph from entry points (see Test Runner)
- **Test Mode** — toggles inline output display on edges
- **Open .py** — opens the underlying file in the system editor
- **File selector** — dropdown of `.py` files in the working directory, each gets its own canvas

---

## Node Editor Modal

Opens on double-click. Rendered as a modal overlay within the pane (semi-transparent backdrop, centered panel).

### Layout

```
┌──────────────────────────────────────────────┐
│  ✕                                    [Test] │
├──────────────────────────────────────────────┤
│  INPUTS                                      │
│  ┌──────────┐ : ┌──────┐  ┌─────────┐       │
│  │ raw_data │   │ dict │  │ = None  │  [×]  │
│  └──────────┘   └──────┘  └─────────┘       │
│  ┌──────────┐ : ┌───────┐                    │
│  │ threshold│   │ float │              [×]  │
│  └──────────┘   └───────┘                    │
│  [+ Add Input]                               │
├──────────────────────────────────────────────┤
│  ┌──────────────────────────────────────┐    │
│  │ 1 │ # Transform the raw data        │    │
│  │ 2 │ filtered = [                     │    │
│  │ 3 │     d for d in raw_data          │    │
│  │ 4 │     if d.get("score", 0) > thre… │    │
│  │ 5 │ ]                                │    │
│  │ 6 │ return filtered                  │    │
│  └──────────────────────────────────────┘    │
├──────────────────────────────────────────────┤
│  RETURN TYPE: ┌───────────┐                  │
│               │ list[dict]│                  │
│               └───────────┘                  │
├──────────────────────────────────────────────┤
│  TEST                                        │
│  raw_data = [{"score": 5}, {"score": 1}]     │
│  threshold = 3.0                             │
│  ──────────────────────────────────           │
│  Result: [{"score": 5}]  ✓ list[dict]  12ms │
└──────────────────────────────────────────────┘
```

### Signature Bar (top section)

Not free-text — structured input fields:

- Each parameter is a row: `[name field] : [type field] [= default field] [delete button]`
- **Name field**: text input. Autocomplete pulls from module-level names (variable registry).
- **Type field**: text input with dropdown suggestions (`str`, `int`, `float`, `bool`, `dict`, `list`, `list[str]`, etc.). Free-text allowed for custom types.
- **Default field**: optional. Shown only if the parameter has a default.
- **Add Input button**: appends a new parameter row and a new port on the canvas.

Changes here immediately update the `def` line in the code editor below and the ports on the canvas node.

### Code Editor (middle section)

A `text_editor` primitive in code mode:

```json
{
  "type": "text_editor",
  "id": "node_editor_body",
  "config": {
    "mode": "code",
    "language": "python",
    "line_numbers": true,
    "tab_size": 4
  }
}
```

The `def` line is NOT shown in the editor — it's generated from the signature bar above. The editor contains only the function body (everything after `def name(args) -> type:`). This prevents the signature and the structured inputs from getting out of sync.

### Return Bar (bottom section)

- **Return type**: text field with same type suggestions as parameter types.
- Changes here update the `-> type` annotation in the generated `def` line and the output port label on the canvas.

### Test Section (bottom)

- One input field per parameter, pre-filled with last-used test values.
- **Test button**: executes the function in an isolated subprocess with the provided inputs.
- **Result display**: shows return value, actual type (with checkmark if it matches declared return type or red X if mismatch), and execution time.
- Test values persist across sessions (stored in the sidecar file).

---

## Variable Registry

A sidebar panel (toggleable) listing all module-scope names:

- Constants (`MAX_RETRIES = 3`)
- Imported names (`from pathlib import Path`)
- Class instances
- Other module-level assignments

The registry is read from the AST of the current `.py` file. Functions reference these names freely. The signature bar's autocomplete pulls from this list.

Clicking a registry entry highlights all nodes that reference it (edges glow or nodes get a subtle badge).

---

## Dependency Manager

A panel (accessible from toolbar or Cmd+D) showing all imports:

```
MODULE IMPORTS
──────────────
json           (stdlib)     used by: parse_data, format_output
pathlib.Path   (stdlib)     used by: load_file
requests       (pip)        used by: fetch_data  ⚠ not installed
──────────────
[+ Add Import]
```

- Lists every `import` and `from x import y` in the file.
- Shows which functions use each import.
- Flags uninstalled packages (checks against the current Python environment).
- **Hot import toggle**: if enabled for an import, it's placed inside the function body (`import json` at the top of the function) instead of module-level. Useful for heavy/optional deps.

---

## File Sync

### Load (.py → canvas)

1. AST-parse the `.py` file.
2. Extract all top-level `def` functions.
3. For each function: extract name, parameters (with types and defaults), return type, body.
4. Create a node for each function.
5. Infer edges: if function A's return type matches function B's parameter type AND B calls A, draw an edge. (Heuristic — the user can also wire manually.)
6. Node positions are stored in a sidecar file: `<filename>.pyflow.json` — contains `{nodes: {func_name: {x, y}}, test_data: {func_name: {param: value}}}`.

### Save (canvas → .py)

1. For each node, generate the `def` line from the structured signature.
2. Append the function body from the text editor.
3. Write module-level imports at the top.
4. Write module-level constants/assignments after imports.
5. Write functions in topological order (callees before callers) or preserve original file order.
6. Preserve comments and docstrings within function bodies.
7. Write the file. It's always valid Python.

### External Edit Detection

File watcher (via Plexi's file watcher capability or polling) detects changes to the `.py` file. On change:
1. Re-parse AST.
2. Diff against current canvas state.
3. Update changed nodes non-destructively (preserve positions of unchanged nodes).
4. If a function was added externally, place its node at a default position.
5. If a function was deleted externally, remove its node.

---

## Test Runner

### Per-Node Test

Runs a single function with test data from the Test Section:

1. Write a temporary script that imports the module, calls the function with test args, prints the result as JSON.
2. Execute via `RunCommand` capability.
3. Parse stdout for result, stderr for errors.
4. Display in the Test Section of the modal.

### Graph Test (post-MVP)

Runs the full execution graph from a selected start node:

1. Topologically sort from the start node.
2. Execute each function in order, passing outputs to inputs.
3. Display each intermediate result inline on the connecting edge.
4. Stop and highlight the failing node on exception.

---

## Manifest

```toml
[app]
id = "pyflow"
name = "PyFlow"
version = "0.1.0"
description = "Visual Python function editor"

[capabilities]
filesystem = "read_write"
mouse_tracking = true

[advanced]
sdk = "advanced"
```

---

## Permissions

- `filesystem.read_write` — reads `.py` files, writes `.py` files and `.pyflow.json` sidecar.
- `RunCommand` — executes Python scripts for testing functions.
- No network access needed.
- No secrets needed.

---

## MVP Scope

1. **Canvas with nodes** — render functions as cards, pan/zoom, select.
2. **AST parse on load** — read a `.py` file, create nodes with correct signatures and ports.
3. **Node editor modal** — signature bar + text editor primitive for function body + return type.
4. **Save to .py** — generate valid Python from canvas state.
5. **Per-node test** — execute a function with test inputs, show result.
6. **Edge wiring** — drag from output port to input port, type-check on connection.

**Defer:** Variable registry sidebar, dependency manager panel, graph test runner, external edit detection, hot imports, auto-inferred edges on load.

---

## Sidecar File Format

`utils.pyflow.json`:

```json
{
  "version": 1,
  "nodes": {
    "transform_data": { "x": 200, "y": 100 },
    "format_output": { "x": 500, "y": 100 },
    "load_file": { "x": 50, "y": 300 }
  },
  "edges": [
    { "from": "transform_data", "to": "format_output", "to_param": "data" }
  ],
  "test_data": {
    "transform_data": {
      "raw_data": "[{\"score\": 5}, {\"score\": 1}]",
      "threshold": "3.0"
    }
  }
}
```

Checked into version control alongside the `.py` file. The `.py` is always the source of truth for code; the sidecar only stores layout and test data.
