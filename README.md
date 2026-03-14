# Plexi

![Plexi screenshot](media/screenshot.png)

An experiment in spatial terminal window management.

(Basically tmux but for omarchy kids. lol)

Loosely inspired by [this rant](https://www.youtube.com/watch?v=EUE8N6mqtGg) — although I've been dreaming of something similar for years.

Built with **Electrobun** (Bun + WebView) and **xterm.js** (for now).

---

## Quick Start

1. **New pane right** — `Cmd+N` / **New pane below** — `Cmd+Shift+N`
2. **Navigate panes** — `Cmd+Arrow` or `Cmd+H/J/K/L`
3. **Switch contexts** — `Cmd+1`, `Cmd+2`, etc.

Your layout and working directories are saved automatically to local storage — pick up where you left off.

---

## The Present

*   **Infinite 2D canvas** — terminals arranged on a spatial grid, navigable with arrow keys or vim-style `h/j/k/l`
*   **Contexts** — named workspaces to separate projects; switch between them with `Cmd+1–9`, rename or delete on the fly
*   **Sidebar & minimap** — visual overview of your layout; click nodes to jump to a terminal
*   **Overlay minimap** — toggleable full-canvas map (`Cmd+M`)
*   **Terminal management** — open new terminals to the right (`Cmd+N`) or below (`Cmd+Shift+N`), close with `Cmd+W`
*   **Workspace persistence** — layout, context, and working directories saved locally
*   **Copy/paste** — selection-aware clipboard support
*   **Font zoom** — `Cmd++/−` to adjust terminal font size
*   **Keyboard reference** — `Cmd+/` to show all shortcuts
*   **Ghost slot hints** — empty adjacent slots show shortcut hints when the canvas is sparse
*   **Status toolbar** — shows current context, working directory, and active process name

## The Future

*   **Other Node Types**: Embedding full web browsers and Excalidraw whiteboards directly on the canvas the terminals.
*   **True Session Persistence and Multiplexing**: A headless daemon so underlying PTYs and SSH connections stay alive in the background when you close the UI. (SSH auto-connect, connection pooling)
*   **Support Multiple Workspaces**: Add the abbility to switch between whole families of contexts (might be overkill)
*   **libghostty Integration**: Swap out `xterm.js` for `libghostty` to get GPU-accelerated, native-grade terminal rendering.
*   **Ergonomics**: Vi-style copy mode and scrollback buffers. (as well as many other Vim style interactions on the canvas)
*   **Pane Management**: Considering tmux-style split pane management within a single canvas node.

---

## Known Issues

*   **Graphics rendering isn't great:** `xterm.js` is okay for now, but this will be fixed with the planned migration to `libghostty`.
*   **opencode visually bugs sometimes.**
*   No SSH atm (Will add when i add session persistance)

---

## Development

```bash
# Install dependencies
bun install

# Start dev server
bun run dev

# Run Playwright e2e verification (ALWAYS RUN BEFORE COMMITS)
bun run test:e2e
```

---
