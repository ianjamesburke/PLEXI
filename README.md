<p align="center">
  <img src="assets/app-icon.png" width="100" alt="Plexi icon" />
</p>

<h1 align="center">Plexi</h1>

<p align="center">An experiment in spatial terminal window management.</p>

<p align="center">
  <img src="media/screenshot-1.png" width="48%" alt="Screenshot 1" />
  &nbsp;
  <img src="media/screenshot-2.png" width="48%" alt="Screenshot 2" />
</p>

<p align="center"><em>basically tmux for omarchy babes</em></p>

**Tested on Mac only** — Linux may work but hasn't been tested.

Loosely inspired by [this rant](https://www.youtube.com/watch?v=EUE8N6mqtGg) — although I've been dreaming of something similar for years.

---

> **⚠️ Not production ready.** Electrobun may be too early-stage to support a project of this complexity — there's a confusing bug in the local build that I haven't been able to crack, and I may need to switch to Electron. See [Known Issues](#known-issues) below. If you have experience with Electrobun internals and want to help, please let me know.

---

## Quick Start

1. **New pane right** — `Cmd+N` / **New pane below** — `Cmd+Shift+N`
2. **Navigate panes** — `Cmd+Arrow` or `Cmd+H/J/K/L`
3. **Switch contexts** — `Cmd+Opt+1`, `Cmd+Opt+2`, etc.

Your layout and working directories are saved automatically to local storage — pick up where you left off.

---

## The Present

*   **Infinite 2D canvas** — terminals arranged on a spatial grid, navigable with arrow keys or vim-style `h/j/k/l`
*   **Contexts** — named workspaces to separate projects; cycle between them with `Cmd+[` and `Cmd+]` , rename or delete on the fly
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

*   **Coding Agent Support**: Session labels, awaiting-response indicators, and notifications — so you can run multiple agents across panes without babysitting them.
*   **Other Node Types**: Embedding full web browsers and Excalidraw whiteboards directly on the canvas the terminals.
*   **True Session Persistence and Multiplexing**: A headless daemon so underlying PTYs and SSH connections stay alive in the background when you close the UI. (SSH auto-connect, connection pooling)
*   **Support Multiple Workspaces**: Add the abbility to switch between whole families of contexts (might be overkill)
*   **libghostty Integration**: Swap out `xterm.js` for `libghostty` to get GPU-accelerated, native-grade terminal rendering.
*   **Ergonomics**: Vi-style copy mode and scrollback buffers. (as well as many other Vim style interactions on the canvas)
*   **Pane Management**: Considering tmux-style split pane management within a single canvas node.

---

## Known Issues


*   **opencode visually bugs sometimes.**
*   No SSH atm (Will add when i add session persistance)
*   **Graphics rendering isn't great:** `xterm.js` is okay for now, but this will be fixed with the planned migration to `libghostty`.
*   **Built app double-input glitch:** I'm seeing double inputs on the built version of the app. It works fine when running dev mode, but the standalone built app just does not want to behave. If anybody can figure it out, that would be spectacular.
    Current state: `bunx electrobun run` against the built app works perfectly. Double-clicking the `.app` is what breaks.
    Things we've tried that did not fix it:
    - hiding xterm helper/composition layers more aggressively
    - changing terminal input handling paths (`onData` vs `onKey`)
    - disabling terminal transparency / forcing an opaque background
    - deduping PTY output with sequence numbers
    - switching the built app to CEF instead of the native renderer
    Current suspects:
    - the standalone app launch path is different from `electrobun run`
    - shell startup / environment differences when launched from Finder vs terminal
    - the build surface is more complicated than it needs to be, especially around copying view assets into `views/`
    I still suspect the build surface may be wrong or at least noisier than necessary, but at this point it does not look like a simple CSS bug.

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
