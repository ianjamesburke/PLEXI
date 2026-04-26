#!/usr/bin/env python3
"""Audio Player — POC for #79 (GUI↔Terminal media bridge, v3.4 P2).

Spawned by the file browser when the user opens an audio file. The path
arrives as ``sys.argv[1]``. v3.4 scope is the bridge surface: every
transport action emits an explicit ``RunInLinkedTerminal`` with the
equivalent ``ffplay`` / ``ffmpeg`` invocation, so the user can copy any
GUI action as a plain CLI command.

Decode-in-app via ``audio.rs`` is NOT in scope for this POC — #277 ships
capture only, playback is on the v3.5+ roadmap. Until then, "play" is
the bridge primitive: the linked terminal does the actual playing via
``ffplay``. Apps lacking ``ffplay`` see the command echoed but it errors
in the terminal — surface, not a crash.

Capabilities: ``terminal.bindings`` (#78).

Keys:
  o - Open the file via ffplay (run in linked terminal)
  p - Pause (echo equivalent ffplay pause; ffplay reads stdin keys)
  r - Resume play (re-issue ffplay)
  [ - Seek -5s (ffmpeg -ss helper)
  ] - Seek +5s
  i - ffprobe info dump
  c - Clear log
"""
from __future__ import annotations

import shlex
import sys
import threading
import time

from plexi_sdk import App, RenderContext, CapabilityDeniedError
from plexi_sdk.ui import (
    AppBar,
    Card,
    Column,
    Footer,
    KeyRow,
    Label,
    ScrollLog,
    Section,
    Spacer,
)


SEEK_STEP_MS = 5_000
MAX_LOG_LINES = 200


def _path_from_argv() -> str:
    """The file browser passes the audio path as argv[1]. When launched
    standalone (e.g. command palette), argv is empty — surface that as a
    clear in-pane error instead of crashing."""
    if len(sys.argv) >= 2 and sys.argv[1]:
        return sys.argv[1]
    return ""


class AudioPlayerApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._path: str = _path_from_argv()
        self._terminal_pane_id: int = 0
        self._state: str = "(closed)"
        self._position_ms: int = 0
        self._log_lines: list[str] = []
        self._log_lock = threading.Lock()

        if not self._path:
            ctx.status_summary("Audio Player — no file (launch with a path arg)")
            self.emit.warn("audio-player: no path argument; launch via file browser or pass argv[1]")
            return

        ctx.status_summary(f"Audio Player — {self._path}")
        self.emit.info(f"audio-player: opening {self._path}")

        # Open a linked terminal so every transport action is also a
        # visible CLI command. This is the bridge contract from #79.
        try:
            self._terminal_pane_id = self.emit.request_linked_terminal(
                cwd=None,
                label="audio bridge",
            )
            self._append_log(f"linked terminal pane #{self._terminal_pane_id}")
        except CapabilityDeniedError as e:
            self.emit.error(str(e))
            self._append_log(f"capability denied: {e}")

    # -- Bridge helpers --------------------------------------------------

    def _emit_cli(self, command: str, *, echo: bool = True) -> None:
        """Fire-and-forget the equivalent CLI through the linked
        terminal. No-op if the terminal isn't open."""
        if self._terminal_pane_id == 0:
            self._append_log("(no linked terminal — drop command)")
            return
        self.emit.run_in_linked_terminal(
            self._terminal_pane_id, command, echo=echo,
        )
        self._append_log(f"$ {command}")

    def _quoted_path(self) -> str:
        return shlex.quote(self._path)

    # -- Transport actions ----------------------------------------------

    def _open(self) -> None:
        if not self._path:
            self._append_log("no file — relaunch with a path argument")
            return
        self._state = "play"
        self._position_ms = 0
        self._emit_cli(f"ffplay -autoexit -nodisp {self._quoted_path()}")

    def _pause(self) -> None:
        # ffplay reads `space` from stdin to toggle pause. We can't drive
        # its stdin directly through RunInLinkedTerminal (it injects a
        # newline-terminated command into the shell, not the running
        # process). Echo the equivalent for the user to repro manually.
        self._state = "pause"
        self._emit_cli("# ffplay: press SPACE in the terminal to toggle pause", echo=False)

    def _resume(self) -> None:
        self._state = "play"
        self._emit_cli("# ffplay: press SPACE to resume (or re-run with -ss)", echo=False)

    def _seek(self, delta_ms: int) -> None:
        self._position_ms = max(0, self._position_ms + delta_ms)
        seconds = self._position_ms / 1000.0
        self._emit_cli(
            f"ffplay -autoexit -nodisp -ss {seconds:.3f} {self._quoted_path()}"
        )

    def _probe(self) -> None:
        self._emit_cli(
            f"ffprobe -hide_banner -show_format -show_streams {self._quoted_path()}"
        )

    # -- Logging ---------------------------------------------------------

    def _append_log(self, line: str) -> None:
        ts = time.strftime("%H:%M:%S")
        with self._log_lock:
            self._log_lines.append(f"{ts} {line}")
            if len(self._log_lines) > MAX_LOG_LINES:
                del self._log_lines[: len(self._log_lines) - MAX_LOG_LINES]

    # -- Input -----------------------------------------------------------

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        if k == "o":
            self._open()
        elif k == "p":
            self._pause()
        elif k == "r":
            self._resume()
        elif key == "[" or k == "[":
            self._seek(-SEEK_STEP_MS)
        elif key == "]" or k == "]":
            self._seek(SEEK_STEP_MS)
        elif k == "i":
            self._probe()
        elif k == "c":
            with self._log_lock:
                self._log_lines.clear()
        self.emit.schedule_render(after_ms=20)

    # -- Render ----------------------------------------------------------

    def _status_card(self) -> Card:
        rows: list = [Section("Status")]
        if not self._path:
            rows.append(Label("(no file — launch via file browser or pass argv[1])"))
        else:
            rows.append(Label(f"  path     {self._path}"))
            rows.append(Label(f"  state    {self._state}"))
            rows.append(Label(f"  position {self._position_ms} ms"))
            rows.append(Label(f"  terminal #{self._terminal_pane_id}"))
        return Card(rows)

    def on_render(self, ctx: RenderContext) -> None:
        with self._log_lock:
            log_snapshot = list(self._log_lines)

        children: list = [
            AppBar(title="Audio Player"),
            Label("v3.4 GUI↔Terminal media bridge POC (#79)"),
            Label("Every transport action emits the equivalent CLI in the linked terminal."),
            self._status_card(),
            Section("Keys"),
            Card([
                KeyRow("o", "Open via ffplay"),
                KeyRow("p", "Pause (echo: SPACE in terminal)"),
                KeyRow("r", "Resume"),
                KeyRow("[", f"Seek -{SEEK_STEP_MS} ms (ffplay -ss)"),
                KeyRow("]", f"Seek +{SEEK_STEP_MS} ms"),
                KeyRow("i", "ffprobe info dump"),
                KeyRow("c", "Clear log"),
            ]),
            Section(f"CLI bridge log ({len(log_snapshot)})"),
            Spacer(grow=True),
            ScrollLog(lines=log_snapshot, empty_text="(no actions yet — press o to play)"),
            Footer("ffplay required for actual playback. CoreAudio in-app decode lands v3.5+."),
        ]
        ctx.render(Column(children))


if __name__ == "__main__":
    AudioPlayerApp().run()
