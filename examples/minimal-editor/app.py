#!/usr/bin/env python3
"""Minimal Editor — POC for fs.read / fs.write capability (#1196).

Loads notes.txt from the workspace root on init (creates empty if absent).
Cmd+S saves the current content back via ctx.emit.write_file().
Shows a status line: bytes written on save, char count while editing.
"""

from plexi_sdk import App, RenderContext, FG, MUTED, GREEN, ACCENT, BG, BODY  # type: ignore[attr-defined]

NOTES_FILE = "notes.txt"


class MinimalEditorApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self._lines: list[str] = [""]
        self._cursor_line = 0
        self._status = ""
        self._status_color = MUTED
        self._loading = True
        # Attempt to load existing file.
        try:
            content = await self.emit.read_file(NOTES_FILE)  # type: ignore[attr-defined]
            self._lines = content.split("\n") if content else [""]
            self._cursor_line = max(0, len(self._lines) - 1)
            self._status = f"Loaded {len(content)} chars from {NOTES_FILE}"
            self._status_color = MUTED
        except RuntimeError as e:
            err = str(e)
            if "file not found" in err or "inaccessible" in err:
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

    async def _save(self, ctx: RenderContext) -> None:
        content = "\n".join(self._lines)
        try:
            n = await self.emit.write_file(NOTES_FILE, content)  # type: ignore[attr-defined]
            self._status = f"Saved {n} bytes to {NOTES_FILE}"
            self._status_color = GREEN
        except RuntimeError as e:
            self._status = f"Save error: {e}"
            self._status_color = ACCENT

    async def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        cmd = mods.get("cmd", False)
        if cmd and key == "s":
            await self._save(ctx)
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
        ctx.clear(BG)
        pad = 16.0
        y = pad

        ctx.text(pad, y, "Minimal Editor", size=16.0, color=FG, bold=True)
        y += 28.0
        ctx.text(pad, y, NOTES_FILE, size=BODY, color=MUTED)
        y += 24.0
        ctx.rect(pad, y, ctx.w - pad * 2, 1.0, fill=MUTED, radius=0.0)
        y += 12.0

        if self._loading:
            ctx.text(pad, y, "Loading…", size=BODY, color=MUTED)
        else:
            for i, line in enumerate(self._lines):
                is_cursor = i == self._cursor_line
                text = line + ("▌" if is_cursor else "")
                ctx.text(pad, y, text if text else ("▌" if is_cursor else " "), size=BODY, color=FG if is_cursor else MUTED)
                y += BODY + 4.0
                if y > ctx.h - 60:
                    remaining = len(self._lines) - i - 1
                    if remaining > 0:
                        ctx.text(pad, y, f"… {remaining} more line(s)", size=BODY, color=MUTED)
                    break

        if self._status:
            ctx.text(pad, ctx.h - 36.0, self._status, size=BODY, color=self._status_color)
        ctx.text(pad, ctx.h - 18.0, "⌘S  save", size=BODY, color=MUTED)


if __name__ == "__main__":
    MinimalEditorApp().run()
