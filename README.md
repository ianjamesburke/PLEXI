# Plexi

![Plexi screenshot](media/screenshot.png)

An experiment in spatial terminal window management.

(Basically I'm trying to build tmux but for omarchy kids. lol)

Loosely inspired by [this rant](https://www.youtube.com/watch?v=EUE8N6mqtGg) — although I've been dreaming of something similar for years.

Built with **Electrobun** (Bun + WebView) and **xterm.js** (for now).

---

## Currently Working

Right now, I'm just focused on getting a functional frontend window manager working:

...

## The Future

Once the basic canvas feels good, the plan is to build the backend daemon features that actually make this a `tmux` alternative.

*   **True Session Persistence**: A headless daemon so underlying PTYs and SSH connections stay alive in the background when you close the UI.
*   **libghostty Integration**: I plan to eventually swap out `xterm.js` for `libghostty` to get GPU-accelerated, native-grade terminal rendering.
*   **Other Node Types**: Embedding full web browsers and Excalidraw whiteboards directly on the canvas alongside your terminals.
*   **Advanced Multiplexing**: SSH auto-connect, connection pooling, and visual routing lines showing relationships between nodes.
*   **Ergonomics**: Vi-style copy mode and scrollback buffers.

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
