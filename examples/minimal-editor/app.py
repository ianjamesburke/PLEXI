#!/usr/bin/env python3
"""Minimal Editor — POC for fs.read / fs.write capability (#1196).

Loads notes.txt from the workspace root on init (creates empty if absent).
Cmd+S saves the current content back via ctx.emit.write_file().
Shows a status line: bytes written on save, char count while editing.
"""

from plexi_sdk import App, RenderContext, FG, MUTED, SURFACE, GREEN, ACCENT, BG, BODY, CAPTION, HINT  # type: ignore[attr-defined]

NOTES_FILE = "notes.txt"


class MinimalEditorApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._lines: list[str] = [""]
        self._cursor_line = 0
        self._status = ""
        self._status_color = MUTED
        self._loading = True
        # Attempt to load existing file.
        try:
            content = self.emit.run_sync(self.emit.read_file(NOTES_FILE))  # type: ignore[attr-defined]
            self._lines = content.split("\n") if content else [""]
            self._cursor_line = max(0, len(self._lines) - 1)
            self._status = f"Loaded {len(content)} chars from {NOTES_FILE}"
            self._status_color = MUTED
        except RuntimeError as e:
            err = str(e)
            if "file not found" in err or "inaccessible" in err:
                # New file — start blank.
                self._lines = [""]
                self._status = f"New file: {NOTES_FILE}"
                self._status_color = MUTED
            else:
                self._status = f"Load error: {err}"
                self._status_color = ACCENT
        finally:
            self._loading = False

    def _char_count(self) -> int:
        return sum(len(ln) for ln in self._lines) + max(0, len(self._lines) - 1)

    def _save(self, ctx: RenderContext) -> None:
        content = "\n".join(self._lines)
        try:
            n = self.emit.run_sync(self.emit.write_file(NOTES_FILE, content))  # type: ignore[attr-defined]
            self._status = f"Saved {n} bytes to {NOTES_FILE}"
            self._status_color = GREEN
        except RuntimeError as e:
            self._status = f"Save error: {e}"
            self._status_color = ACCENT

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        cmd = mods.get("cmd", False)
        if cmd and key == "s":
            self._save(ctx)
            return
        if key == "Enter":
            self._lines.insert(self._cursor_line + 1, "")
            self._cursor_line += 1
        elif key == "Backspace":
            if self._lines[self._cursor_line]:
                self._lines[self._cursor_line] = self._lines[self._cursor_line][:-1]
            elif self._cursor_line > 0:
                self._lines.pop(self._cursor_line)
                self._cursor_line -= 1
        elif key == "up":
            self._cursor_line = max(0, self._cursor_line - 1)
        elif key == "down":
            self._cursor_line = min(len(self._lines) - 1, self._cursor_line + 1)
        elif len(key) == 1 and not cmd:
            self._lines[self._cursor_line] += key
            self._status = f"{self._char_count()} chars"
            self._status_color = MUTED

    def on_render(self, ctx: RenderContext) -> None:
        from plexi_sdk.ui import Column, AppBar, FooterKeys, Footer, Spacer

        ctx.clear(BG)

        _app = self

        class _Body:
            def measure(self, avail_w: float) -> float:
                return 0.0

            def is_grow(self) -> bool:
                return True

            def render(self, ctx2, x: float, y: float, w: float, h: float) -> None:
                if _app._loading:
                    ctx2.text(x, y + 16, "Loading…", size=BODY, color=MUTED)
                    return
                cy = y
                for i, line in enumerate(_app._lines):
                    is_cursor = i == _app._cursor_line
                    cursor = "▌" if is_cursor else ""
                    color = FG if is_cursor else MUTED
                    ctx2.text(x, cy, line + cursor, size=BODY, color=color)
                    cy += BODY + 6
                    if cy > y + h - 4:
                        break

        children = [
            AppBar(title="Minimal Editor", subtitle=NOTES_FILE),
            _Body(),
            Spacer(size=HINT),
        ]
        if self._status:
            children.append(Footer(self._status, color=self._status_color))
        children.append(FooterKeys([(["⌘", "s"], "save")]))
        ctx.render(Column(children))


if __name__ == "__main__":
    MinimalEditorApp().run()
