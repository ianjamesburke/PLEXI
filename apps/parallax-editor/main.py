#!/usr/bin/env python3
"""Parallax Editor — GUI wrapper for `parallax produce` and `parallax plan`.

Streams subprocess stdout live into the log area. Supports Brief and Plan
modes, aspect ratio selection, and single-scene production via `--scene N`.
"""

import os
import queue
import subprocess
import threading

from plexi_sdk import (
    App, RenderContext,
    BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, RED, GREEN, YELLOW,
    BODY, CAPTION, HINT, MONO_SMALL,
    PAD, PAD_TIGHT,
    PRIORITY_NORMAL,
    dim,
)

# ── Constants ──────────────────────────────────────────────────────────────────

POLL_TIMER   = "poll_subprocess"
POLL_MS      = 80          # ~12 fps log refresh while subprocess is running
MAX_LOG_LINES = 200

ASPECTS = ["9:16", "16:9", "1:1", "4:3", "3:4"]

# Layout geometry
HEADER_H = 36.0
ROW_H    = 32.0
BTN_H    = 30.0
BTN_W    = 110.0
SMALL_BTN_W = 90.0
LABEL_W  = 70.0
STATUS_H = 24.0

# Spinner frames
_SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]


class ParallaxEditorApp(App):

    # ── Init ───────────────────────────────────────────────────────────────────

    def on_init(self, ctx: RenderContext) -> None:
        self._folder: str = ""
        self._mode: str = "brief"        # "brief" | "plan"
        self._file: str = ""
        self._aspect: str = "9:16"
        self._scene: str = ""            # empty = all scenes; digits = single scene

        # Subprocess state
        self._running: bool = False
        self._proc: subprocess.Popen | None = None
        self._log_lines: list[str] = []
        self._log_queue: queue.Queue[str] = queue.Queue()
        self._status: str = "idle"       # "idle" | "running" | "done" | error msg
        self._last_exit_code: int | None = None

        # Track which text_input field is active (for keyboard routing)
        self._active_field: str = "folder"

        # Spin counter for animation
        self._spin_tick: int = 0

        self.emit.info("parallax-editor ready")

    # ── Subprocess management ──────────────────────────────────────────────────

    def _build_produce_cmd(self) -> list[str]:
        cmd = ["parallax", "produce", "--folder", self._folder]
        if self._mode == "brief":
            cmd += ["--brief", self._file]
        else:
            cmd += ["--plan", self._file]
        if self._aspect:
            cmd += ["--aspect", self._aspect]
        if self._scene.strip():
            cmd += ["--scene", self._scene.strip()]
        return cmd

    def _build_plan_cmd(self) -> list[str]:
        cmd = ["parallax", "plan", "--folder", self._folder]
        if self._file:
            cmd += ["--brief", self._file]
        return cmd

    def _launch(self, cmd: list[str]) -> None:
        """Start subprocess and a reader thread. Called from the render thread."""
        if self._running:
            return
        self._log_lines.clear()
        self._log_queue = queue.Queue()
        self._running = True
        self._status = "running"
        self._last_exit_code = None
        self.emit.info(f"parallax-editor launching: {' '.join(cmd)}")

        try:
            self._proc = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
            )
        except FileNotFoundError:
            self._running = False
            self._status = "error: parallax not found — is it on your PATH?"
            return
        except Exception as e:
            self._running = False
            self._status = f"error: {e}"
            return

        def _reader(proc: subprocess.Popen, q: queue.Queue) -> None:
            try:
                assert proc.stdout is not None
                for line in proc.stdout:
                    q.put(line.rstrip())
            except Exception as exc:
                q.put(f"[reader error: {exc}]")
            finally:
                proc.wait()
                q.put(None)  # sentinel — process finished

        t = threading.Thread(target=_reader, args=(self._proc, self._log_queue), daemon=True)
        t.start()

    def _drain_log_queue(self, ctx: RenderContext) -> bool:
        """Drain pending lines from the subprocess reader queue.

        Returns True if the subprocess finished this poll cycle.
        """
        done = False
        try:
            while True:
                item = self._log_queue.get_nowait()
                if item is None:
                    done = True
                    break
                self._log_lines.append(item)
                if len(self._log_lines) > MAX_LOG_LINES:
                    self._log_lines = self._log_lines[-MAX_LOG_LINES:]
        except queue.Empty:
            pass

        if done:
            exit_code = self._proc.returncode if self._proc else -1
            self._last_exit_code = exit_code
            self._running = False
            self._proc = None
            # Don't overwrite "cancelled" — the user explicitly stopped the job
            # and exit_code will be non-zero (SIGTERM), which would misleadingly
            # show "error (exit -15)" instead.
            if self._status != "cancelled":
                if exit_code == 0:
                    self._status = "done"
                else:
                    self._status = f"error (exit {exit_code})"

        return done

    # ── Input helpers ──────────────────────────────────────────────────────────

    def _can_produce(self) -> bool:
        return bool(self._folder.strip() and self._file.strip() and not self._running)

    # ── on_timer ───────────────────────────────────────────────────────────────

    def on_timer(self, ctx: RenderContext, timer_id: str) -> None:
        if timer_id != POLL_TIMER:
            return
        self._drain_log_queue(ctx)
        self._spin_tick += 1
        if self._running:
            # Keep polling while subprocess is alive
            ctx.set_timer(POLL_TIMER, POLL_MS)

    # ── on_key ─────────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        # Tab cycles through editable fields
        if key == "Tab":
            fields = ["folder", "file", "scene"]
            idx = fields.index(self._active_field) if self._active_field in fields else 0
            self._active_field = fields[(idx + 1) % len(fields)]
            return

        # Mode toggle: b = brief, p = plan
        if key == "b" and not self._running:
            self._mode = "brief"
            return
        if key == "p" and not self._running:
            self._mode = "plan"
            return

        # Aspect shortcuts: 1-5
        if key in ("1", "2", "3", "4", "5") and not self._running:
            idx = int(key) - 1
            if idx < len(ASPECTS):
                self._aspect = ASPECTS[idx]
            return

        # Enter to produce (when can_produce)
        if key == "Return" and mods.get("meta") and self._can_produce():
            cmd = self._build_produce_cmd()
            self._launch(cmd)
            ctx.set_timer(POLL_TIMER, POLL_MS)
            return

        # Shift+P to plan only (brief mode only, when not running)
        if key == "P" and mods.get("shift") and self._mode == "brief" and not self._running:
            if self._folder.strip():
                cmd = self._build_plan_cmd()
                self._launch(cmd)
                ctx.set_timer(POLL_TIMER, POLL_MS)
            return

        # Escape to cancel running job
        if key == "Escape" and self._running and self._proc:
            try:
                self._proc.terminate()
            except Exception:
                pass
            self._status = "cancelled"
            return

    # ── on_render ──────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        w = ctx.w
        pad = PAD
        field_x = pad + LABEL_W + PAD_TIGHT
        field_w = w - field_x - pad

        y = pad

        # ── App bar ──────────────────────────────────────────────────────────
        ctx.rect(0.0, 0.0, w, HEADER_H, SURFACE)
        ctx.rect(0.0, HEADER_H - 1.0, w, 1.0, HIGHLIGHT)
        ctx.text(pad, (HEADER_H - BODY) / 2.0, "Parallax Editor",
                 size=BODY, color=FG, bold=True)
        y = HEADER_H + PAD_TIGHT

        # ── Project folder ──────────────────────────────────────────────────
        self._render_label(ctx, pad, y, "Folder")
        submitted = ctx.text_input(
            "folder", x=field_x, y=y, w=field_w,
            placeholder="/path/to/project",
        )
        if submitted is not None:
            self._folder = submitted
        y += ROW_H + PAD_TIGHT

        # ── Mode toggle ──────────────────────────────────────────────────────
        self._render_label(ctx, pad, y, "Mode")
        btn_y = y
        for i, mode in enumerate(("brief", "plan")):
            bx = field_x + i * (SMALL_BTN_W + PAD_TIGHT)
            active = self._mode == mode
            self._render_button(ctx, bx, btn_y, SMALL_BTN_W, BTN_H, mode.capitalize(),
                                active=active, disabled=self._running)
        y += ROW_H + PAD_TIGHT

        # ── File path ────────────────────────────────────────────────────────
        label_text = "brief.yaml" if self._mode == "brief" else "plan.yaml"
        self._render_label(ctx, pad, y, label_text)
        submitted = ctx.text_input(
            "file", x=field_x, y=y, w=field_w,
            placeholder="brief.yaml or plan.yaml",
        )
        if submitted is not None:
            self._file = submitted
        y += ROW_H + PAD_TIGHT

        # ── Aspect ratio ─────────────────────────────────────────────────────
        self._render_label(ctx, pad, y, "Aspect")
        aspect_btn_w = (field_w - PAD_TIGHT * (len(ASPECTS) - 1)) / len(ASPECTS)
        for i, asp in enumerate(ASPECTS):
            bx = field_x + i * (aspect_btn_w + PAD_TIGHT)
            active = self._aspect == asp
            self._render_button(ctx, bx, y, aspect_btn_w, BTN_H, asp,
                                active=active, disabled=self._running)
        y += ROW_H + PAD_TIGHT

        # ── Scene (optional) ─────────────────────────────────────────────────
        self._render_label(ctx, pad, y, "Scene")
        scene_hint = ctx.text_input(
            "scene", x=field_x, y=y, w=80.0,
            placeholder="all",
        )
        if scene_hint is not None:
            self._scene = scene_hint
        # scene label hint
        ctx.text(field_x + 88.0, y + (BTN_H - HINT) / 2.0,
                 "leave blank for all scenes",
                 size=HINT, color=MUTED)
        y += ROW_H + PAD_TIGHT

        # ── Divider ──────────────────────────────────────────────────────────
        ctx.rect(pad, y, w - 2 * pad, 1.0, HIGHLIGHT)
        y += PAD_TIGHT + 2.0

        # ── Action buttons ───────────────────────────────────────────────────
        produce_disabled = not self._can_produce()
        self._render_button(ctx, pad, y, BTN_W, BTN_H, "Produce ⌘↩",
                            active=False, disabled=produce_disabled, primary=True)

        if self._mode == "brief":
            plan_disabled = not (self._folder.strip() and not self._running)
            self._render_button(ctx, pad + BTN_W + PAD_TIGHT, y, BTN_W, BTN_H,
                                "Plan Only ⇧P",
                                active=False, disabled=plan_disabled)

        if self._running:
            spinner = _SPINNER[self._spin_tick % len(_SPINNER)]
            ctx.text(pad + BTN_W * 2 + PAD_TIGHT * 3, y + (BTN_H - BODY) / 2.0,
                     f"{spinner}  running…  (Esc to cancel)",
                     size=CAPTION, color=ACCENT)

        y += BTN_H + PAD_TIGHT

        # ── Status bar ───────────────────────────────────────────────────────
        status_color = self._status_color()
        ctx.rect(pad, y, w - 2 * pad, STATUS_H, dim(SURFACE, 180), radius=4.0)
        ctx.text(pad + PAD_TIGHT, y + (STATUS_H - CAPTION) / 2.0,
                 self._status_text(),
                 size=CAPTION, color=status_color,
                 max_width=w - 2 * pad - PAD_TIGHT * 2, elide=True)
        y += STATUS_H + PAD_TIGHT

        # ── Log area ─────────────────────────────────────────────────────────
        log_h = max(0.0, ctx.h - y - pad)
        if log_h > 0:
            ctx.rect(pad, y, w - 2 * pad, log_h, dim(SURFACE, 120), radius=4.0)
            ctx.rect(pad, y, w - 2 * pad, 1.0, HIGHLIGHT)
            self._render_log(ctx, pad + PAD_TIGHT, y + PAD_TIGHT,
                             w - 2 * pad - PAD_TIGHT * 2, log_h - PAD_TIGHT * 2)

        # Keyboard hint strip at very bottom
        ctx.shortcuts(
            x=pad, y=ctx.h - 16.0, max_width=w - 2 * pad,
            pairs=[
                ("⌘↩", "produce"),
                ("⇧P", "plan only"),
                ("b/p", "mode"),
                ("1-5", "aspect"),
                ("Tab", "next field"),
            ],
            font_size=HINT,
        )

    # ── Render helpers ─────────────────────────────────────────────────────────

    def _render_label(self, ctx: RenderContext, x: float, y: float,
                      text: str) -> None:
        ctx.text(x, y + (ROW_H - CAPTION) / 2.0, text,
                 size=CAPTION, color=MUTED,
                 max_width=LABEL_W, elide=True)

    def _render_button(self, ctx: RenderContext,
                       x: float, y: float, w: float, h: float,
                       label: str, *,
                       active: bool = False,
                       disabled: bool = False,
                       primary: bool = False) -> None:
        if disabled:
            fill = dim(SURFACE, 100)
            border = dim(HIGHLIGHT, 80)
            text_color = dim(MUTED, 120)
        elif active:
            fill = dim(ACCENT, 60)
            border = ACCENT
            text_color = FG
        elif primary:
            fill = dim(ACCENT, 40)
            border = ACCENT
            text_color = ACCENT
        else:
            fill = SURFACE
            border = HIGHLIGHT
            text_color = FG

        ctx.rect(x, y, w, h, fill, radius=4.0)
        # Border — four thin rects
        ctx.rect(x, y, w, 1.0, border)
        ctx.rect(x, y + h - 1.0, w, 1.0, border)
        ctx.rect(x, y, 1.0, h, border)
        ctx.rect(x + w - 1.0, y, 1.0, h, border)

        ctx.text(x + w / 2.0, y + (h - CAPTION) / 2.0, label,
                 size=CAPTION, color=text_color,
                 align="top_center", max_width=w - 4.0, elide=True)

    def _render_log(self, ctx: RenderContext,
                    x: float, y: float, w: float, h: float) -> None:
        if not self._log_lines:
            ctx.text(x, y, "no output yet",
                     size=HINT, color=MUTED)
            return
        line_h = MONO_SMALL + 3.0
        visible = max(1, int(h / line_h))
        # Newest at bottom: take the last `visible` lines, render top to bottom
        lines = self._log_lines[-visible:]
        for i, line in enumerate(lines):
            ctx.text(x, y + i * line_h, line,
                     size=MONO_SMALL, color=FG,
                     monospace=True, max_width=w, elide=True)

    def _status_color(self) -> str:
        if self._status == "idle":
            return MUTED
        if self._status == "running":
            return ACCENT
        if self._status == "done":
            return GREEN
        if self._status == "cancelled":
            return YELLOW
        if self._status.startswith("error"):
            return RED
        return MUTED

    def _status_text(self) -> str:
        if self._status == "idle":
            return "Ready — configure a project folder and file, then press Produce."
        if self._status == "running":
            spinner = _SPINNER[self._spin_tick % len(_SPINNER)]
            return f"{spinner}  running parallax…"
        return self._status


ParallaxEditorApp().run()
