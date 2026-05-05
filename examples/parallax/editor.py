#!/usr/bin/env python3
from __future__ import annotations

import json
import math
import os
import shlex
import subprocess
import sys
import threading
import time

from plexi_sdk import App, RenderContext, CapabilityDeniedError

# ── Palette ──────────────────────────────────────────────────────────────────
BG         = "#0d0d13"
PANEL_BG   = "#12121a"
PANEL_ALT  = "#16161f"
BORDER     = "#22222f"
BORDER_LT  = "#2e2e3e"
ACCENT     = "#7c6af7"
ACCENT_DIM = "#352f6a"
TEXT       = "#cdd6f4"
TEXT_DIM   = "#585875"
TEXT_MED   = "#9098b0"
V_CLIP     = "#2a4f7a"
V_CLIP_SEL = "#3a72b0"
A_CLIP     = "#1e5230"
A_CLIP_SEL = "#2a7844"
PLAYHEAD   = "#f38ba8"
TC_BG      = "#18182a"
IN_COLOR   = "#a6e3a1"
OUT_COLOR  = "#fab387"
ERR_COLOR  = "#f38ba8"

# ── Layout constants ─────────────────────────────────────────────────────────
TOOLBAR_H   = 42
PANEL_L_W   = 180
PANEL_R_W   = 200
TIMELINE_H  = 160
TRACK_HDR_W = 52
TRACK_H     = 36
RULER_H     = 20
PX_PER_SEC  = 52.0

# ── Clip / track types ──────────────────────────────────────────────────────

class Clip:
    __slots__ = ("path", "label", "track", "start_s", "duration_s",
                 "in_s", "out_s", "kind")

    def __init__(self, path: str, label: str, track: str, start_s: float,
                 duration_s: float, kind: str = "video") -> None:
        self.path = path
        self.label = label
        self.track = track
        self.start_s = start_s
        self.duration_s = duration_s
        self.in_s: float = 0.0
        self.out_s: float = duration_s
        self.kind = kind

    @property
    def end_s(self) -> float:
        return self.start_s + self.duration_s

    @property
    def trimmed_duration(self) -> float:
        return self.out_s - self.in_s

    def to_dict(self) -> dict:
        return {
            "path": self.path, "label": self.label, "track": self.track,
            "start_s": self.start_s, "duration_s": self.duration_s,
            "in_s": self.in_s, "out_s": self.out_s, "kind": self.kind,
        }


def _tc(s: float) -> str:
    s = max(0.0, s)
    h  = int(s // 3600)
    m  = int((s % 3600) // 60)
    sc = int(s % 60)
    fr = int((s % 1.0) * 30)
    return f"{h:02d}:{m:02d}:{sc:02d}:{fr:02d}"


def _probe_duration(path: str) -> float | None:
    try:
        r = subprocess.run(
            ["ffprobe", "-v", "quiet", "-print_format", "json",
             "-show_format", path],
            capture_output=True, text=True, timeout=10,
        )
        data = json.loads(r.stdout)
        return float(data["format"]["duration"])
    except Exception:
        return None


# ── Demo clips (used when no project folder is given) ────────────────────────

def _demo_clips() -> list[Clip]:
    return [
        Clip("(demo)", "Interview A", "V1", 0.0,  4.8),
        Clip("(demo)", "Wide Shot",   "V1", 5.2,  3.1),
        Clip("(demo)", "Close-up",    "V1", 8.8,  5.4),
        Clip("(demo)", "B-roll",      "V1", 14.8, 4.5),
        Clip("(demo)", "Outro",       "V1", 19.8, 2.0),
        Clip("(demo)", "Background",  "A1", 0.0, 22.0, kind="audio"),
    ]


def _load_project_clips(folder: str) -> list[Clip]:
    clips: list[Clip] = []
    video_exts = {".mp4", ".mov", ".mkv", ".avi", ".webm"}
    audio_exts = {".mp3", ".wav", ".aac", ".flac", ".ogg"}
    assets_dir = os.path.join(folder, "assets")
    search_dirs = [assets_dir, folder] if os.path.isdir(assets_dir) else [folder]
    offset = 0.0
    for d in search_dirs:
        if not os.path.isdir(d):
            continue
        for name in sorted(os.listdir(d)):
            ext = os.path.splitext(name)[1].lower()
            if ext not in video_exts and ext not in audio_exts:
                continue
            path = os.path.join(d, name)
            dur = _probe_duration(path) or 10.0
            kind = "audio" if ext in audio_exts else "video"
            track = "V1" if kind == "video" else "A1"
            clips.append(Clip(path, name, track, offset, dur, kind))
            if kind == "video":
                offset += dur + 0.5
    return clips


# ═════════════════════════════════════════════════════════════════════════════

class ParallaxEditor(App):
    def __init__(self) -> None:
        super().__init__()
        self._register_tools()

    def _register_tools(self) -> None:
        @self.tool(
            "set_in_point",
            description="Set the in-point (seconds) on the selected clip.",
            schema={
                "type": "object",
                "properties": {
                    "seconds": {"type": "number", "description": "In-point in seconds."},
                },
                "required": ["seconds"],
            },
        )
        def handle_set_in(args: dict) -> dict:
            clip = self._selected_clip()
            if clip is None:
                return {"error": "no clip selected"}
            clip.in_s = max(0.0, min(float(args["seconds"]), clip.out_s))
            self.emit.schedule_render(after_ms=16)
            return {"clip": clip.label, "in_s": clip.in_s}

        @self.tool(
            "set_out_point",
            description="Set the out-point (seconds) on the selected clip.",
            schema={
                "type": "object",
                "properties": {
                    "seconds": {"type": "number", "description": "Out-point in seconds."},
                },
                "required": ["seconds"],
            },
        )
        def handle_set_out(args: dict) -> dict:
            clip = self._selected_clip()
            if clip is None:
                return {"error": "no clip selected"}
            clip.out_s = max(clip.in_s, min(float(args["seconds"]), clip.duration_s))
            self.emit.schedule_render(after_ms=16)
            return {"clip": clip.label, "out_s": clip.out_s}

        @self.tool(
            "move_clip",
            description="Move a clip to a new start time on the timeline.",
            schema={
                "type": "object",
                "properties": {
                    "clip_index": {"type": "integer", "description": "Index of the clip."},
                    "start_s": {"type": "number", "description": "New start time in seconds."},
                },
                "required": ["clip_index", "start_s"],
            },
        )
        def handle_move(args: dict) -> dict:
            idx = int(args["clip_index"])
            if idx < 0 or idx >= len(self._clips):
                return {"error": f"invalid clip index {idx}"}
            self._clips[idx].start_s = max(0.0, float(args["start_s"]))
            self.emit.schedule_render(after_ms=16)
            return {"clip": self._clips[idx].label, "start_s": self._clips[idx].start_s}

        @self.tool(
            "trim_clip",
            description="Trim the selected clip using Parallax CLI and export.",
            schema={
                "type": "object",
                "properties": {
                    "output_path": {"type": "string", "description": "Output file path."},
                },
            },
        )
        def handle_trim(args: dict) -> dict:
            clip = self._selected_clip()
            if clip is None:
                return {"error": "no clip selected"}
            if clip.path == "(demo)":
                return {"error": "demo clips cannot be trimmed"}
            out = args.get("output_path", "")
            if not out:
                base, ext = os.path.splitext(clip.path)
                out = f"{base}_trimmed{ext}"
            self._run_export(clip, out)
            return {"clip": clip.label, "output": out, "in_s": clip.in_s, "out_s": clip.out_s}

        @self.tool(
            "list_clips",
            description="List all clips on the timeline.",
            schema={"type": "object", "properties": {}},
        )
        def handle_list(args: dict) -> dict:  # noqa: ARG001
            return {"clips": [c.to_dict() for c in self._clips]}

        @self.tool(
            "select_clip",
            description="Select a clip by index.",
            schema={
                "type": "object",
                "properties": {
                    "index": {"type": "integer", "description": "Clip index."},
                },
                "required": ["index"],
            },
        )
        def handle_select(args: dict) -> dict:
            idx = int(args["index"])
            if idx < 0 or idx >= len(self._clips):
                return {"error": f"invalid index {idx}"}
            self._sel = idx
            self.emit.schedule_render(after_ms=16)
            return {"selected": self._clips[idx].label}

    # ── Lifecycle ────────────────────────────────────────────────────────────

    async def on_init(self, ctx: RenderContext) -> None:
        self._playing = False
        self._ph: float = 0.0
        self._t0: float = 0.0
        self._sel: int = 0
        self._drag: bool = False
        self._drag_offset: float = 0.0
        self._project_folder: str = ""
        self._terminal_pane: int = 0
        self._status_msg: str = ""
        self._clips: list[Clip] = []

        folder = sys.argv[1] if len(sys.argv) >= 2 and sys.argv[1] else os.environ.get("PARALLAX_PROJECT", "")
        if folder and os.path.isdir(folder):
            self._project_folder = folder
            self._clips = _demo_clips()
            self._status_msg = f"Loading clips from {os.path.basename(folder)}..."
            threading.Thread(
                target=self._load_clips_bg, args=(folder,), daemon=True
            ).start()
        else:
            self._clips = _demo_clips()
            self._status_msg = "Demo mode — set PARALLAX_PROJECT or pass a folder as argv[1]"

        self.emit.info(self._status_msg)
        ctx.status_summary(f"Parallax · {_tc(0)}")

        try:
            self._terminal_pane = await self.emit.request_linked_terminal(
                cwd=self._project_folder or None,
                label="parallax cli",
            )
            self.emit.info(f"linked terminal #{self._terminal_pane}")
        except CapabilityDeniedError as e:
            self.emit.warn(f"terminal.bindings denied: {e}")

    def on_shutdown(self) -> None:
        self._playing = False

    def _load_clips_bg(self, folder: str) -> None:
        clips = _load_project_clips(folder)
        self._clips = clips if clips else _demo_clips()
        self._sel = 0
        self._status_msg = f"Loaded {len(clips)} clips from {os.path.basename(folder)}" if clips else "No media found — showing demo"
        self.emit.info(self._status_msg)
        self.emit.schedule_render(after_ms=16)

    # ── Helpers ──────────────────────────────────────────────────────────────

    def _selected_clip(self) -> Clip | None:
        if 0 <= self._sel < len(self._clips):
            return self._clips[self._sel]
        return None

    def _total_duration(self) -> float:
        if not self._clips:
            return 10.0
        return max(c.end_s for c in self._clips)

    def _tracks(self) -> list[tuple[str, str]]:
        seen: dict[str, str] = {}
        for c in self._clips:
            if c.track not in seen:
                seen[c.track] = c.kind
        return sorted(seen.items(), key=lambda t: (0 if t[1] == "video" else 1, t[0]))

    def _emit_cli(self, cmd: str) -> None:
        if self._terminal_pane == 0:
            return
        self.emit.run_in_linked_terminal(self._terminal_pane, f"# {cmd}", echo=False)

    def _run_export(self, clip: Clip, output: str) -> None:
        cmd = (f"ffmpeg -y -ss {clip.in_s:.3f} -to {clip.out_s:.3f} "
               f"-i {shlex.quote(clip.path)} -c copy {shlex.quote(output)}")
        self._status_msg = f"Exporting: {os.path.basename(output)}..."
        self.emit.info(f"export: {cmd}")
        if self._terminal_pane:
            self.emit.run_in_linked_terminal(self._terminal_pane, cmd, echo=True)
        self.emit.schedule_render(after_ms=16)

    # ── Playback ─────────────────────────────────────────────────────────────

    def _toggle_play(self) -> None:
        if self._playing:
            self._stop()
        else:
            total = self._total_duration()
            if self._ph >= total:
                self._ph = 0.0
            self._start()

    def _start(self) -> None:
        self._t0 = time.monotonic() - self._ph
        self._playing = True
        self.emit.schedule_render(after_ms=16)

    def _stop(self) -> None:
        self._playing = False

    def _seek(self, delta: float) -> None:
        self._stop()
        self._ph = max(0.0, min(self._total_duration(), self._ph + delta))

    # ── Input ────────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if key == " ":
            self._toggle_play()
        elif key in ("left", "j"):
            self._seek(-1.0)
        elif key in ("right", "l"):
            self._seek(1.0)
        elif key == "Home":
            self._stop(); self._ph = 0.0
        elif key == "End":
            self._stop(); self._ph = self._total_duration()
        elif key == "i":
            clip = self._selected_clip()
            if clip:
                clip.in_s = max(0.0, min(self._ph - clip.start_s, clip.out_s))
                self._status_msg = f"In: {clip.in_s:.2f}s"
        elif key == "o":
            clip = self._selected_clip()
            if clip:
                clip.out_s = max(clip.in_s, min(self._ph - clip.start_s, clip.duration_s))
                self._status_msg = f"Out: {clip.out_s:.2f}s"
        elif key == "e":
            clip = self._selected_clip()
            if clip and clip.path != "(demo)":
                base, ext = os.path.splitext(clip.path)
                self._run_export(clip, f"{base}_trimmed{ext}")
        elif key == "Escape":
            self._drag = False
        self.emit.schedule_render(after_ms=16)

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        W, H = ctx.w, ctx.h
        tl_h = min(TIMELINE_H, max(100, H // 3))
        tl_y = H - tl_h
        tracks = self._tracks()

        # ── Toolbar transport buttons ────────────────────────────────────
        if y < TOOLBAR_H:
            bcx = W / 2
            hits = [
                (bcx - 72, lambda: (self._stop(), setattr(self, '_ph', 0.0))),
                (bcx - 44, lambda: self._seek(-1.0)),
                (bcx,      self._toggle_play),
                (bcx + 44, lambda: self._seek(1.0)),
                (bcx + 72, lambda: (self._stop(), setattr(self, '_ph', self._total_duration()))),
            ]
            for bx, fn in hits:
                if abs(x - bx) < 14:
                    fn()
                    break

        # ── Timeline area ────────────────────────────────────────────────
        elif y > tl_y + RULER_H + 4:
            ca_x = PANEL_L_W + TRACK_HDR_W
            raw_t = (x - ca_x) / PX_PER_SEC
            row = int((y - tl_y - RULER_H - 4) / TRACK_H)

            hit_clip = False
            if 0 <= row < len(tracks):
                tid = tracks[row][0]
                for ci, c in enumerate(self._clips):
                    if c.track == tid and c.start_s <= raw_t <= c.end_s:
                        self._sel = ci
                        self._drag = True
                        self._drag_offset = raw_t - c.start_s
                        hit_clip = True
                        break

            if not hit_clip and x >= ca_x:
                self._stop()
                self._ph = max(0.0, min(self._total_duration(), raw_t))
                self._drag = False

        # ── Media panel clip selection ───────────────────────────────────
        elif x < PANEL_L_W and y > TOOLBAR_H + 30:
            idx = int((y - TOOLBAR_H - 30) / 24)
            if 0 <= idx < len(self._clips):
                self._sel = idx

        self.emit.schedule_render(after_ms=16)

    def on_mouse_move(self, ctx: RenderContext, x: float, y: float, buttons: list) -> None:
        if not self._drag:
            return
        # Drop the clip when mouse button is released
        if not buttons:
            self._drag = False
            clip = self._selected_clip()
            if clip:
                self._status_msg = f"Moved {clip.label} → {clip.start_s:.1f}s"
                self._emit_cli(f"# clip '{clip.label}' moved to {clip.start_s:.1f}s")
            self.emit.schedule_render(after_ms=16)
            return
        ca_x = PANEL_L_W + TRACK_HDR_W
        raw_t = (x - ca_x) / PX_PER_SEC
        new_start = max(0.0, raw_t - self._drag_offset)
        clip = self._selected_clip()
        if clip:
            clip.start_s = round(new_start * 10) / 10
            self.emit.schedule_render(after_ms=16)

    # ── Render ───────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        # Advance playhead from wall clock when playing
        if self._playing:
            total = self._total_duration()
            self._ph = time.monotonic() - self._t0
            if self._ph >= total:
                self._ph = total
                self._playing = False
            else:
                self.emit.schedule_render(after_ms=16)

        W, H = ctx.w, ctx.h
        tl_h = min(TIMELINE_H, max(100, H // 3))
        content_h = H - TOOLBAR_H - tl_h
        pr_w = max(200, W - PANEL_L_W - PANEL_R_W)

        ctx.rect(0, 0, W, H, fill=BG)

        self._draw_toolbar(ctx, W)
        self._draw_media_panel(ctx, content_h)
        self._draw_preview(ctx, PANEL_L_W, TOOLBAR_H, pr_w, content_h)
        self._draw_inspector(ctx, PANEL_L_W + pr_w, TOOLBAR_H, PANEL_R_W, content_h)
        self._draw_timeline(ctx, 0, H - tl_h, W, tl_h)

        ctx.status_summary(f"Parallax · {_tc(self._ph)}")

    # ── Toolbar ──────────────────────────────────────────────────────────────

    def _draw_toolbar(self, ctx: RenderContext, W: float) -> None:
        ctx.rect(0, 0, W, TOOLBAR_H, fill=PANEL_BG)
        ctx.line(0, TOOLBAR_H - 1, W, TOOLBAR_H - 1, color=BORDER)

        ctx.text(14, TOOLBAR_H / 2, "Parallax", size=13.0, color=TEXT,
                 bold=True, align="center")

        bcx = W / 2
        bcy = TOOLBAR_H / 2
        icons = [("⏮", bcx - 72), ("⏪", bcx - 44),
                 ("⏸" if self._playing else "▶", bcx),
                 ("⏩", bcx + 44), ("⏭", bcx + 72)]
        for icon, bx in icons:
            ctx.rect(bx - 11, bcy - 10, 22, 20, fill=PANEL_ALT, radius=4.0)
            ctx.text(bx, bcy, icon, size=12.0, color=TEXT, align="center")

        ctx.rect(bcx + 96, bcy - 10, 102, 20, fill=TC_BG, radius=3.0)
        ctx.text(bcx + 147, bcy, _tc(self._ph), size=11.0, color=ACCENT,
                 monospace=True, align="center")

        # Shortcut hints
        ctx.text(W - 10, bcy, "I/O trim  E export  ←→ seek",
                 size=9.0, color=TEXT_DIM, align="right")

    # ── Media panel ──────────────────────────────────────────────────────────

    def _draw_media_panel(self, ctx: RenderContext, H: float) -> None:
        y0 = TOOLBAR_H
        ctx.rect(0, y0, PANEL_L_W, H, fill=PANEL_BG)
        ctx.line(PANEL_L_W - 1, y0, PANEL_L_W - 1, y0 + H, color=BORDER)

        ctx.rect(0, y0, PANEL_L_W, 28, fill=PANEL_ALT)
        ctx.line(0, y0 + 28, PANEL_L_W, y0 + 28, color=BORDER)
        ctx.text(PANEL_L_W / 2, y0 + 14, "CLIPS", size=9.5, color=TEXT_DIM,
                 bold=True, align="center")

        for i, clip in enumerate(self._clips):
            iy = y0 + 32 + i * 24
            if iy > y0 + H - 4:
                break
            if i == self._sel:
                ctx.rect(4, iy - 1, PANEL_L_W - 8, 22, fill="#1e1e30", radius=3.0)
            icon_col = V_CLIP_SEL if clip.kind == "video" else A_CLIP_SEL
            icon = "▶" if clip.kind == "video" else "♪"
            ctx.text(12, iy + 3, icon, size=9.0, color=icon_col)
            ctx.text(24, iy + 3, clip.label, size=10.0, color=TEXT,
                     max_width=PANEL_L_W - 60)
            dur_str = f"{clip.duration_s:.1f}s"
            ctx.text(PANEL_L_W - 10, iy + 3, dur_str, size=9.0,
                     color=TEXT_DIM, align="right")

        # Status bar at bottom of panel
        ctx.text(8, y0 + H - 16, self._status_msg, size=9.0, color=TEXT_MED,
                 max_width=PANEL_L_W - 16)

    # ── Preview ──────────────────────────────────────────────────────────────

    def _draw_preview(self, ctx: RenderContext, x: float, y: float,
                      W: float, H: float) -> None:
        ctx.rect(x, y, W, H, fill="#090910")

        pad = 16
        fw = W - pad * 2
        fh = fw * 9.0 / 16.0
        if fh > H - 60:
            fh = H - 60
            fw = fh * 16.0 / 9.0
        fx = x + (W - fw) / 2
        fy = y + (H - fh - 28) / 2

        ctx.rect(fx, fy, fw, fh, fill="#111118")

        # If the selected clip has a real path, try to show thumbnail via ctx.image
        clip = self._selected_clip()
        if clip and clip.path != "(demo)":
            thumb = self._get_thumbnail(clip)
            if thumb:
                ctx.image(thumb, fx, fy, fw, fh, fit="contain")
            else:
                ctx.text(fx + fw / 2, fy + fh / 2 - 10, clip.label,
                         size=14.0, color=TEXT, align="center")
                ctx.text(fx + fw / 2, fy + fh / 2 + 10, os.path.basename(clip.path),
                         size=10.0, color=TEXT_DIM, align="center")
        else:
            # Synthetic preview for demo mode
            self._draw_synthetic_preview(ctx, fx, fy, fw, fh)

        # Frame corner marks
        clen = 12.0
        corners = [
            (fx, fy, fx + clen, fy, fx, fy + clen),
            (fx + fw, fy, fx + fw - clen, fy, fx + fw, fy + clen),
            (fx, fy + fh, fx + clen, fy + fh, fx, fy + fh - clen),
            (fx + fw, fy + fh, fx + fw - clen, fy + fh, fx + fw, fy + fh - clen),
        ]
        for cx1, cy1, cx2, cy2, cx3, cy3 in corners:
            ctx.line(cx1, cy1, cx2, cy2, color="#3a3a5a", width=1.0)
            ctx.line(cx1, cy1, cx3, cy3, color="#3a3a5a", width=1.0)

        if self._playing:
            ctx.circle(fx + fw - 13, fy + 11, 5.0, fill=PLAYHEAD)
            ctx.text(fx + fw - 26, fy + 11, "REC", size=8.0, color=PLAYHEAD, align="center")

        # Timecode overlay
        ctx.rect(fx + 4, fy + fh - 20, 82, 16, fill="#00000099", radius=2.0)
        ctx.text(fx + 8, fy + fh - 18, _tc(self._ph), size=9.5,
                 color=TEXT, monospace=True)

        # Clip label below frame
        if clip:
            ctx.text(x + W / 2, fy + fh + 6, clip.label, size=11.0,
                     color=TEXT, align="top_center")
            in_out = f"IN {_tc(clip.in_s)}  OUT {_tc(clip.out_s)}"
            ctx.text(x + W / 2, fy + fh + 20, in_out, size=9.5,
                     color=TEXT_DIM, align="top_center", monospace=True)

    def _get_thumbnail(self, clip: Clip) -> str | None:
        if not os.path.isfile(clip.path):
            return None
        thumb_dir = os.path.join(os.path.dirname(clip.path), ".thumbnails")
        os.makedirs(thumb_dir, exist_ok=True)
        thumb_path = os.path.join(thumb_dir, os.path.basename(clip.path) + ".jpg")
        if os.path.isfile(thumb_path):
            return thumb_path
        # Extract thumbnail in background on first access
        threading.Thread(
            target=self._extract_thumbnail,
            args=(clip.path, thumb_path),
            daemon=True,
        ).start()
        return None

    def _extract_thumbnail(self, src: str, dst: str) -> None:
        try:
            subprocess.run(
                ["ffmpeg", "-y", "-ss", "1", "-i", src,
                 "-vframes", "1", "-q:v", "5", dst],
                capture_output=True, timeout=15,
            )
            self.emit.schedule_render(after_ms=100)
        except Exception:
            pass

    def _draw_synthetic_preview(self, ctx: RenderContext, fx: float, fy: float,
                                 fw: float, fh: float) -> None:
        sky_colors = ["#0e1628", "#121e35", "#172440", "#1c2a4a"]
        band_h = fh * 0.55 / len(sky_colors)
        for bi, sc in enumerate(sky_colors):
            ctx.rect(fx, fy + bi * band_h, fw, band_h + 1, fill=sc)
        ctx.rect(fx, fy + fh * 0.48, fw, fh * 0.12, fill="#1e2e48")
        ctx.rect(fx, fy + fh * 0.55, fw, fh * 0.45, fill="#0c0e14")

        buildings = [(0.04, 0.07, 0.52), (0.25, 0.06, 0.50),
                     (0.50, 0.09, 0.48), (0.78, 0.07, 0.52)]
        for (bx_f, bw_f, by_f) in buildings:
            bx = fx + fw * bx_f
            bw = fw * bw_f
            by = fy + fh * by_f
            ctx.rect(bx, by, bw, fh - (by - fy), fill="#080b10")

        ctx.text(fx + fw / 2, fy + fh / 2, "DEMO MODE",
                 size=16.0, color=TEXT_DIM, bold=True, align="center")

    # ── Inspector ────────────────────────────────────────────────────────────

    def _draw_inspector(self, ctx: RenderContext, x: float, y: float,
                         W: float, H: float) -> None:
        ctx.rect(x, y, W, H, fill=PANEL_BG)
        ctx.line(x, y, x, y + H, color=BORDER)

        ctx.rect(x, y, W, 28, fill=PANEL_ALT)
        ctx.line(x, y + 28, x + W, y + 28, color=BORDER)
        ctx.text(x + W / 2, y + 14, "INSPECTOR", size=9.5,
                 color=TEXT_DIM, bold=True, align="center")

        clip = self._selected_clip()
        if clip is None:
            ctx.text(x + 12, y + 44, "No clip selected", size=10.0, color=TEXT_DIM)
            return

        def row(k: str, v: str, iy: float, val_color: str = TEXT) -> None:
            ctx.text(x + 12, iy, k, size=10.0, color=TEXT_DIM)
            ctx.text(x + W - 10, iy, v, size=10.0, color=val_color,
                     align="right", max_width=W - 80)

        iy = y + 40
        row("Clip",     clip.label,               iy); iy += 20
        row("Track",    clip.track,                iy); iy += 20
        row("Source",   os.path.basename(clip.path), iy); iy += 20
        row("Duration", f"{clip.duration_s:.2f}s", iy); iy += 20
        row("Start",    f"{clip.start_s:.2f}s",    iy); iy += 20

        ctx.line(x + 8, iy + 2, x + W - 8, iy + 2, color=BORDER); iy += 14

        ctx.text(x + 12, iy, "TRIM", size=9.5, color=TEXT_DIM, bold=True); iy += 18
        row("In",       _tc(clip.in_s),   iy, IN_COLOR);  iy += 20
        row("Out",      _tc(clip.out_s),  iy, OUT_COLOR); iy += 20
        row("Trimmed",  f"{clip.trimmed_duration:.2f}s", iy); iy += 20

        # Visual in/out bar
        iy += 4
        bar_x = x + 12
        bar_w = W - 24
        ctx.rect(bar_x, iy, bar_w, 8, fill="#111120", radius=3.0)
        if clip.duration_s > 0:
            in_frac = clip.in_s / clip.duration_s
            out_frac = clip.out_s / clip.duration_s
            sel_x = bar_x + bar_w * in_frac
            sel_w = bar_w * (out_frac - in_frac)
            ctx.rect(sel_x, iy, sel_w, 8, fill=ACCENT_DIM, radius=2.0)
            ctx.rect(sel_x, iy, 2, 8, fill=IN_COLOR)
            ctx.rect(sel_x + sel_w - 2, iy, 2, 8, fill=OUT_COLOR)
        iy += 20

        if self._project_folder:
            ctx.line(x + 8, iy + 2, x + W - 8, iy + 2, color=BORDER); iy += 14
            ctx.text(x + 12, iy, "PROJECT", size=9.5, color=TEXT_DIM, bold=True); iy += 18
            ctx.text(x + 12, iy, os.path.basename(self._project_folder),
                     size=10.0, color=TEXT, max_width=W - 24)

    # ── Timeline ─────────────────────────────────────────────────────────────

    def _draw_timeline(self, ctx: RenderContext, x: float, y: float,
                        W: float, H: float) -> None:
        ctx.rect(x, y, W, H, fill=PANEL_BG)
        ctx.line(x, y, x + W, y, color=BORDER_LT)

        tracks = self._tracks()
        total = self._total_duration()

        # Timeline toolbar
        ctx.rect(x, y, W, 24, fill=PANEL_ALT)
        ctx.line(x, y + 24, x + W, y + 24, color=BORDER)
        ctx.text(x + PANEL_L_W + 10, y + 12, "Timeline", size=10.5,
                 color=TEXT, bold=True, align="center")
        ctx.text(x + PANEL_L_W + 80, y + 12, f"{total:.1f}s total",
                 size=9.0, color=TEXT_DIM, align="center")

        # Track headers
        hdr_w = PANEL_L_W + TRACK_HDR_W
        ctx.rect(x, y + 24, hdr_w, H - 24, fill=PANEL_ALT)
        ctx.line(x + hdr_w, y, x + hdr_w, y + H, color=BORDER)

        for ri, (tid, kind) in enumerate(tracks):
            ty = y + 24 + RULER_H + ri * TRACK_H
            hx = x + PANEL_L_W
            ctx.rect(hx, ty, TRACK_HDR_W, TRACK_H, fill=PANEL_ALT)
            ctx.line(hx, ty + TRACK_H, hx + TRACK_HDR_W, ty + TRACK_H, color=BORDER)
            ctx.text(hx + TRACK_HDR_W / 2, ty + TRACK_H / 2,
                     tid, size=11.0, color=TEXT, bold=True, align="center")
            dot_col = V_CLIP_SEL if kind == "video" else A_CLIP_SEL
            ctx.circle(hx + TRACK_HDR_W - 10, ty + TRACK_H / 2, 3.0, fill=dot_col)

        ctx.text(x + PANEL_L_W / 2, y + 12, "TRACKS",
                 size=9.0, color=TEXT_DIM, align="center")

        # Clip area
        ca_x = x + hdr_w
        ca_w = W - hdr_w
        ca_y = y + 24
        ca_h = H - 24

        ctx.push_clip(ca_x, ca_y, ca_w, ca_h)

        # Ruler
        ctx.rect(ca_x, ca_y, ca_w, RULER_H, fill="#111120")
        for ti in range(int(total) + 2):
            rx = ca_x + ti * PX_PER_SEC
            if rx > ca_x + ca_w:
                break
            major = (ti % 5 == 0)
            ctx.line(rx, ca_y + (RULER_H - 8 if major else RULER_H - 4),
                     rx, ca_y + RULER_H, color=BORDER_LT)
            if major:
                ctx.text(rx + 3, ca_y + 3, f"{ti}s", size=8.5, color=TEXT_DIM)

        # Clips
        for ri, (tid, kind) in enumerate(tracks):
            ty = ca_y + RULER_H + ri * TRACK_H
            row_fill = "#111820" if kind == "video" else "#0e1812"
            ctx.rect(ca_x, ty, ca_w, TRACK_H, fill=row_fill)
            ctx.line(ca_x, ty + TRACK_H, ca_x + ca_w, ty + TRACK_H, color=BORDER)

            for ci, c in enumerate(self._clips):
                if c.track != tid:
                    continue
                cx = ca_x + c.start_s * PX_PER_SEC
                cw = c.duration_s * PX_PER_SEC - 2
                if cx + cw < ca_x or cx > ca_x + ca_w:
                    continue

                is_sel = ci == self._sel
                if kind == "video":
                    fill = V_CLIP_SEL if is_sel else V_CLIP
                else:
                    fill = A_CLIP_SEL if is_sel else A_CLIP
                ctx.rect(cx, ty + 3, cw, TRACK_H - 6, fill=fill, radius=3.0)

                # In/out markers on selected clip
                if is_sel and c.duration_s > 0:
                    in_px = cx + (c.in_s / c.duration_s) * cw
                    out_px = cx + (c.out_s / c.duration_s) * cw
                    ctx.rect(in_px, ty + 2, 2, TRACK_H - 4, fill=IN_COLOR)
                    ctx.rect(out_px - 2, ty + 2, 2, TRACK_H - 4, fill=OUT_COLOR)

                # Audio waveform sketch
                if kind == "audio" and cw > 16:
                    for wi in range(0, int(cw) - 4, 4):
                        wh = (math.sin(wi * 0.4 + ci * 2) * 0.4 + 0.5) * (TRACK_H - 16)
                        wy = ty + 3 + (TRACK_H - 6) / 2 - wh / 2
                        ctx.rect(cx + 2 + wi, wy, 2, wh,
                                 fill="#3a9a5a" if is_sel else "#2a6a3a")

                # Clip label
                if cw > 20:
                    ctx.push_clip(cx + 2, ty + 3, cw - 4, TRACK_H - 6)
                    ctx.text(cx + 5, ty + 10, c.label, size=9.0, color=TEXT,
                             max_width=cw - 8)
                    ctx.pop_clip()

                # Selection highlight
                if is_sel:
                    ctx.line(cx, ty + 3, cx + cw, ty + 3, color=ACCENT, width=1.5)
                    ctx.line(cx, ty + TRACK_H - 3, cx + cw, ty + TRACK_H - 3,
                             color=ACCENT, width=1.5)

        # Playhead
        ph_x = ca_x + self._ph * PX_PER_SEC
        if ca_x <= ph_x <= ca_x + ca_w:
            ctx.line(ph_x, ca_y, ph_x, ca_y + ca_h, color=PLAYHEAD, width=1.5)
            ctx.rect(ph_x - 5, ca_y, 10, 10, fill=PLAYHEAD, radius=2.0)

        ctx.pop_clip()


if __name__ == "__main__":
    ParallaxEditor().run()
