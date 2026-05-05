#!/usr/bin/env python3
"""Parallax — visual proof of concept for a non-linear video editor.

Demonstrates the full editor chrome: toolbar with transport controls,
media library panel, preview window with fake video frame, inspector panel,
and a multi-track timeline with clips, ruler, and animated playhead.

No video decoding — all frames are synthetic. Space to play/pause.
"""

import math
import threading
import time

from plexi_sdk import App, RenderContext

# ── Palette ───────────────────────────────────────────────────────────────────
BG          = "#0d0d13"
PANEL_BG    = "#12121a"
PANEL_ALT   = "#16161f"
BORDER      = "#22222f"
BORDER_LT   = "#2e2e3e"
ACCENT      = "#7c6af7"
ACCENT_DIM  = "#352f6a"
TEXT        = "#cdd6f4"
TEXT_DIM    = "#585875"
TEXT_MED    = "#9098b0"
V_CLIP      = "#2a4f7a"
V_CLIP_SEL  = "#3a72b0"
V_CLIP_B    = "#203d60"
A_CLIP      = "#1e5230"
A_CLIP_SEL  = "#2a7844"
PLAYHEAD    = "#f38ba8"
TC_BG       = "#18182a"
SEL_ROW     = "#1e1e30"
WAVEFORM    = "#2a6a3a"

# ── Layout ────────────────────────────────────────────────────────────────────
TOOLBAR_H   = 42
PANEL_L_W   = 188
PANEL_R_W   = 214
TIMELINE_H  = 198
TRACK_HDR_W = 56
TRACK_H     = 36
RULER_H     = 20
PX_PER_SEC  = 52.0   # pixels per second (zoom level)

# ── Timeline data ─────────────────────────────────────────────────────────────
TOTAL_DUR = 22.0

# track_id, start_s, dur_s, label
CLIPS = [
    ("V1",  0.0,  4.8, "Interview A"),
    ("V1",  5.2,  3.1, "Wide Shot"),
    ("V1",  8.8,  5.4, "Close-up"),
    ("V1", 14.8,  4.5, "B-roll"),
    ("V1", 19.8,  2.0, "Outro"),
    ("V2",  2.0,  3.2, "Cutaway"),
    ("V2",  7.5,  3.8, "Overlay"),
    ("V2", 13.0,  2.5, "Title card"),
    ("A1",  0.0, 22.0, "Background music"),
    ("A2",  3.6,  1.4, "SFX"),
    ("A2",  8.9,  1.2, "SFX"),
    ("A2", 14.0,  2.1, "Ambient"),
]

TRACKS = [
    ("V2", "video"),
    ("V1", "video"),
    ("A1", "audio"),
    ("A2", "audio"),
]

MEDIA = [
    ("Interview A.mp4",     "4:32", "video"),
    ("Wide Shot.mp4",       "1:12", "video"),
    ("Close-up.mp4",        "6:05", "video"),
    ("Cutaway.mp4",         "3:45", "video"),
    ("B-roll.mp4",          "4:01", "video"),
    ("Outro.mp4",           "0:48", "video"),
    ("background_music.mp3","3:32", "audio"),
    ("sfx_whoosh.wav",      "0:02", "audio"),
    ("ambient.wav",         "2:10", "audio"),
]


def _tc(s: float) -> str:
    s = max(0.0, s)
    h  = int(s // 3600)
    m  = int((s % 3600) // 60)
    sc = int(s % 60)
    fr = int((s % 1.0) * 30)
    return f"{h:02d}:{m:02d}:{sc:02d}:{fr:02d}"


class VideoEditor(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._playing    = False
        self._ph         = 0.0        # playhead seconds
        self._t0         = 0.0        # monotonic time at last play start
        self._sel_clip   = 0          # selected clip index
        self._sel_media  = 0          # selected media item index
        self._tool       = "select"   # select | cut | slip
        ctx.status_summary("Parallax · 00:00:00:00")

    # ── Input ─────────────────────────────────────────────────────────────────
    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if key == " ":
            self._toggle_play()
        elif key in ("left", "j"):
            self._stop(); self._ph = max(0.0, self._ph - 1.0)
        elif key in ("right", "l"):
            self._stop(); self._ph = min(TOTAL_DUR, self._ph + 1.0)
        elif key == "Home":
            self._stop(); self._ph = 0.0
        elif key == "End":
            self._stop(); self._ph = TOTAL_DUR
        elif key in ("1", "2", "3"):
            tools = {"1": "select", "2": "cut", "3": "slip"}
            self._tool = tools[key]
        self.emit.schedule_render(after_ms=16)

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        W, H = ctx.w, ctx.h
        tl_h = min(TIMELINE_H, max(120, H // 3))
        tl_y = H - tl_h
        tl_x = PANEL_L_W + TRACK_HDR_W

        # Toolbar transport
        if y < TOOLBAR_H:
            bcx = W / 2
            hit = [
                (bcx - 72, self._go_start),
                (bcx - 44, lambda: self._step(-1)),
                (bcx,      self._toggle_play),
                (bcx + 44, lambda: self._step(1)),
                (bcx + 72, self._go_end),
            ]
            for bx, fn in hit:
                if abs(x - bx) < 14:
                    fn()
                    break
            # Tool buttons
            for i, tool in enumerate(("select", "cut", "slip")):
                tx = W - 130 + i * 34
                if abs(x - tx) < 14:
                    self._tool = tool
                    break

        # Media panel
        elif x < PANEL_L_W and y > TOOLBAR_H + 58:
            idx = int((y - TOOLBAR_H - 58) / 26)
            if 0 <= idx < len(MEDIA):
                self._sel_media = idx

        # Timeline
        elif y > tl_y + RULER_H + 4:
            raw_t = (x - tl_x) / PX_PER_SEC
            row   = int((y - tl_y - RULER_H - 4) / TRACK_H)
            # Try clip hit first
            hit_clip = False
            if 0 <= row < len(TRACKS):
                tid = TRACKS[row][0]
                for ci, (cid, cs, cd, _) in enumerate(CLIPS):
                    if cid == tid and cs <= raw_t <= cs + cd:
                        self._sel_clip = ci
                        hit_clip = True
                        break
            # Otherwise seek
            if not hit_clip and x >= tl_x:
                self._stop()
                self._ph = max(0.0, min(TOTAL_DUR, raw_t))

        self.emit.schedule_render(after_ms=16)

    # ── Play control ──────────────────────────────────────────────────────────
    def _toggle_play(self) -> None:
        if self._playing:
            self._stop()
        else:
            if self._ph >= TOTAL_DUR:
                self._ph = 0.0
            self._start()

    def _start(self) -> None:
        self._t0 = time.monotonic() - self._ph
        self._playing = True

        def _tick():
            while self._playing:
                time.sleep(0.033)
                if self._playing:
                    self._ph = time.monotonic() - self._t0
                    if self._ph >= TOTAL_DUR:
                        self._ph = TOTAL_DUR
                        self._playing = False
                    self.emit.schedule_render(after_ms=16)

        threading.Thread(target=_tick, daemon=True).start()

    def _stop(self) -> None:
        self._playing = False

    def _step(self, d: float) -> None:
        self._stop()
        self._ph = max(0.0, min(TOTAL_DUR, self._ph + d))

    def _go_start(self) -> None:
        self._stop(); self._ph = 0.0

    def _go_end(self) -> None:
        self._stop(); self._ph = TOTAL_DUR

    # ── Render ────────────────────────────────────────────────────────────────
    def on_render(self, ctx: RenderContext) -> None:
        W, H = ctx.w, ctx.h
        tl_h     = min(TIMELINE_H, max(120, H // 3))
        content_h = H - TOOLBAR_H - tl_h
        pr_w     = max(240, W - PANEL_L_W - PANEL_R_W)

        ctx.rect(0, 0, W, H, fill=BG)

        self._draw_toolbar(ctx, W)
        self._draw_media(ctx, PANEL_L_W, content_h)
        self._draw_preview(ctx, PANEL_L_W, TOOLBAR_H, pr_w, content_h)
        rx = PANEL_L_W + pr_w
        self._draw_inspector(ctx, rx, TOOLBAR_H, PANEL_R_W, content_h)
        self._draw_timeline(ctx, 0, H - tl_h, W, tl_h)

        ctx.status_summary(f"Parallax · {_tc(self._ph)}")

    # ── Toolbar ───────────────────────────────────────────────────────────────
    def _draw_toolbar(self, ctx: RenderContext, W: float) -> None:
        ctx.rect(0, 0, W, TOOLBAR_H, fill=PANEL_BG)
        ctx.line(0, TOOLBAR_H - 1, W, TOOLBAR_H - 1, color=BORDER)

        # App name
        ctx.text(14, TOOLBAR_H / 2, "Parallax", size=13.0, color=TEXT,
                 bold=True, align="center")

        # Transport — centered
        bcx = W / 2
        bcy = TOOLBAR_H / 2
        icons = [("⏮", bcx - 72), ("⏪", bcx - 44),
                 ("⏸" if self._playing else "▶", bcx),
                 ("⏩", bcx + 44), ("⏭", bcx + 72)]
        for icon, bx in icons:
            ctx.rect(bx - 11, bcy - 10, 22, 20, fill=PANEL_ALT, radius=4.0)
            ctx.text(bx, bcy, icon, size=12.0, color=TEXT, align="center")

        # Timecode
        ctx.rect(bcx + 96, bcy - 10, 102, 20, fill=TC_BG, radius=3.0)
        ctx.text(bcx + 147, bcy, _tc(self._ph), size=11.0, color=ACCENT,
                 monospace=True, align="center")

        # Tool selector
        tool_labels = [("S", "select"), ("C", "cut"), ("⇄", "slip")]
        for i, (icon, tool) in enumerate(tool_labels):
            bx = W - 128 + i * 34
            ctx.rect(bx - 11, bcy - 10, 22, 20,
                     fill=ACCENT_DIM if self._tool == tool else PANEL_ALT,
                     radius=4.0)
            ctx.text(bx, bcy, icon, size=11.0, color=TEXT, align="center")

    # ── Media library ─────────────────────────────────────────────────────────
    def _draw_media(self, ctx: RenderContext, W: float, H: float) -> None:
        y0 = TOOLBAR_H
        ctx.rect(0, y0, W, H, fill=PANEL_BG)
        ctx.line(W - 1, y0, W - 1, y0 + H, color=BORDER)

        # Section header
        ctx.rect(0, y0, W, 30, fill=PANEL_ALT)
        ctx.line(0, y0 + 30, W, y0 + 30, color=BORDER)
        ctx.text(12, y0 + 15, "PROJECT", size=9.5, color=TEXT_DIM,
                 bold=True, align="center")

        # Search bar placeholder
        ctx.rect(8, y0 + 36, W - 16, 20, fill=BG, radius=4.0)
        ctx.text(16, y0 + 46, "Search media…", size=10.0, color=TEXT_DIM, align="center")

        # Items
        for i, (name, dur, kind) in enumerate(MEDIA):
            iy = y0 + 62 + i * 26
            if iy > y0 + H - 4:
                break
            if i == self._sel_media:
                ctx.rect(4, iy - 1, W - 8, 24, fill=SEL_ROW, radius=3.0)
            icon_col = "#3a72b0" if kind == "video" else "#2a7844"
            ctx.text(12, iy + 5, "▶" if kind == "video" else "♪",
                     size=9.0, color=icon_col, align="top_left")
            ctx.text(24, iy + 5, name, size=10.0, color=TEXT,
                     max_width=W - 60, align="top_left")
            ctx.text(W - 8, iy + 5, dur, size=9.0, color=TEXT_DIM, align="right")

    # ── Preview window ────────────────────────────────────────────────────────
    def _draw_preview(self, ctx: RenderContext, x: float, y: float,
                      W: float, H: float) -> None:
        ctx.rect(x, y, W, H, fill="#090910")

        # Compute 16:9 frame rect
        pad   = 16
        fw    = W - pad * 2
        fh    = fw * 9.0 / 16.0
        if fh > H - 60:
            fh = H - 60
            fw = fh * 16.0 / 9.0
        fx = x + (W - fw) / 2
        fy = y + (H - fh - 28) / 2

        # ── Synthetic video frame ──────────────────────────────────────────
        # Sky gradient (stacked bands)
        sky_colors = ["#0e1628", "#121e35", "#172440", "#1c2a4a"]
        band_h = fh * 0.55 / len(sky_colors)
        for bi, sc in enumerate(sky_colors):
            ctx.rect(fx, fy + bi * band_h, fw, band_h + 1, fill=sc)
        # Horizon glow
        ctx.rect(fx, fy + fh * 0.48, fw, fh * 0.12, fill="#1e2e48")

        # Ground
        ctx.rect(fx, fy + fh * 0.55, fw, fh * 0.45, fill="#0c0e14")

        # Building silhouettes
        buildings = [
            (0.04, 0.07, 0.52),
            (0.13, 0.10, 0.42),
            (0.25, 0.06, 0.50),
            (0.34, 0.11, 0.44),
            (0.50, 0.09, 0.48),
            (0.62, 0.13, 0.40),
            (0.78, 0.07, 0.52),
            (0.88, 0.05, 0.53),
        ]
        for (bx_f, bw_f, by_f) in buildings:
            bx = fx + fw * bx_f
            bw = fw * bw_f
            by = fy + fh * by_f
            ctx.rect(bx, by, bw, fh - (by - fy), fill="#080b10")
            # Lit windows
            for wxi in range(int(bw // 8)):
                for wyi in range(6):
                    wx = bx + wxi * 8 + 2
                    wy = by + 6 + wyi * 10
                    if wy + 7 > fy + fh:
                        break
                    lit = (int(bx_f * 100 + wxi * 7 + wyi * 13) % 5) != 0
                    if lit:
                        ctx.rect(wx, wy, 4, 6, fill="#c8a44a")

        # Subject silhouette (interview person)
        px = fx + fw * 0.38
        py = fy + fh * 0.22
        ctx.rect(px,           py + fh * 0.22, fw * 0.11, fh * 0.5, fill="#12161e")
        ctx.circle(px + fw * 0.055, py + fh * 0.11, fw * 0.038, fill="#12161e")

        # Focus rack dots (cinematography decoration)
        for di in range(3):
            dot_x = fx + fw * (0.72 + di * 0.06)
            ctx.circle(dot_x, fy + fh * 0.82, 3.0, fill="#3a4a5a")

        # Frame corner marks
        clen = 12.0
        corner_lines = [
            [(fx, fy, fx + clen, fy), (fx, fy, fx, fy + clen)],
            [(fx + fw, fy, fx + fw - clen, fy), (fx + fw, fy, fx + fw, fy + clen)],
            [(fx, fy + fh, fx + clen, fy + fh), (fx, fy + fh, fx, fy + fh - clen)],
            [(fx + fw, fy + fh, fx + fw - clen, fy + fh),
             (fx + fw, fy + fh, fx + fw, fy + fh - clen)],
        ]
        for corner_set in corner_lines:
            for (x1, y1, x2, y2) in corner_set:
                ctx.line(x1, y1, x2, y2, color="#3a3a5a", width=1.0)

        # REC dot when playing
        if self._playing:
            ctx.circle(fx + fw - 13, fy + 11, 5.0, fill=PLAYHEAD)
            ctx.text(fx + fw - 26, fy + 11, "REC", size=8.0,
                     color=PLAYHEAD, align="center")

        # Timecode overlay
        ctx.rect(fx + 4, fy + fh - 20, 82, 16, fill="#00000099", radius=2.0)
        ctx.text(fx + 8, fy + fh - 18, _tc(self._ph), size=9.5,
                 color=TEXT, monospace=True)

        # Lower third
        lt_y = fy + fh * 0.72
        ctx.rect(fx + 10, lt_y, fw * 0.55, 38, fill="#00000077")
        ctx.line(fx + 10, lt_y, fx + 10 + fw * 0.55, lt_y, color=ACCENT, width=2.0)
        sel = CLIPS[self._sel_clip]
        ctx.text(fx + 16, lt_y + 6,  sel[3], size=11.0, color=TEXT, bold=True)
        ctx.text(fx + 16, lt_y + 22, f"Duration {sel[2]:.1f}s",
                 size=9.0, color=TEXT_MED)

        # Clip label below frame
        ctx.text(x + W / 2, fy + fh + 6, sel[3], size=11.0,
                 color=TEXT, align="top_center")
        ctx.text(x + W / 2, fy + fh + 20,
                 f"{_tc(sel[1])} → {_tc(sel[1] + sel[2])}",
                 size=9.5, color=TEXT_DIM, align="top_center", monospace=True)

    # ── Inspector ─────────────────────────────────────────────────────────────
    def _draw_inspector(self, ctx: RenderContext, x: float, y: float,
                         W: float, H: float) -> None:
        ctx.rect(x, y, W, H, fill=PANEL_BG)
        ctx.line(x, y, x, y + H, color=BORDER)

        ctx.rect(x, y, W, 30, fill=PANEL_ALT)
        ctx.line(x, y + 30, x + W, y + 30, color=BORDER)
        ctx.text(x + W / 2, y + 15, "INSPECTOR", size=9.5,
                 color=TEXT_DIM, bold=True, align="center")

        clip = CLIPS[self._sel_clip]
        tid, cs, cd, label = clip

        def _row(k: str, v: str, iy: float) -> None:
            ctx.text(x + 12, iy, k, size=10.0, color=TEXT_DIM)
            ctx.text(x + W - 10, iy, v, size=10.0, color=TEXT,
                     align="right", max_width=W - 72)

        iy = y + 44
        _row("Clip",     label,        iy); iy += 22
        _row("Track",    tid,          iy); iy += 22
        _row("In",       _tc(cs),      iy); iy += 22
        _row("Out",      _tc(cs + cd), iy); iy += 22
        _row("Duration", f"{cd:.2f}s", iy); iy += 22
        _row("Speed",    "100%",       iy); iy += 22

        ctx.line(x + 8, iy + 2, x + W - 8, iy + 2, color=BORDER); iy += 14

        ctx.text(x + 12, iy, "EFFECTS", size=9.5, color=TEXT_DIM, bold=True); iy += 18
        effects = [("Color Grade", True), ("Lumetri", False), ("Noise Reduce", False)]
        for name, active in effects:
            ctx.circle(x + 17, iy + 6, 3.5, fill=ACCENT if active else "#333348")
            ctx.text(x + 28, iy + 2, name, size=10.0,
                     color=TEXT if active else TEXT_DIM)
            iy += 20

        ctx.line(x + 8, iy + 2, x + W - 8, iy + 2, color=BORDER); iy += 14
        ctx.text(x + 12, iy, "AUDIO", size=9.5, color=TEXT_DIM, bold=True); iy += 18

        # VU meters
        meter_h = 72
        for ch, label in enumerate(("L", "R")):
            mx = x + 24 + ch * 72
            level = 0.0
            if self._playing:
                t = time.monotonic()
                base = 0.65 - ch * 0.08
                level = base + math.sin(t * 4.0 + ch) * 0.12
                level = max(0.0, min(1.0, level))
            ctx.rect(mx, iy, 14, meter_h, fill=BG, radius=2.0)
            fh2 = meter_h * level
            col = (A_CLIP_SEL if level < 0.75
                   else "#d4a84b" if level < 0.9 else PLAYHEAD)
            ctx.rect(mx, iy + meter_h - fh2, 14, fh2, fill=col, radius=1.0)
            ctx.text(mx + 7, iy + meter_h + 5, label, size=9.0,
                     color=TEXT_DIM, align="top_center")

    # ── Timeline ──────────────────────────────────────────────────────────────
    def _draw_timeline(self, ctx: RenderContext, x: float, y: float,
                        W: float, H: float) -> None:
        ctx.rect(x, y, W, H, fill=PANEL_BG)
        ctx.line(x, y, x + W, y, color=BORDER_LT)

        # Timeline toolbar strip
        ctx.rect(x, y, W, 26, fill=PANEL_ALT)
        ctx.line(x, y + 26, x + W, y + 26, color=BORDER)
        ctx.text(x + PANEL_L_W + 10, y + 13, "Timeline", size=10.5,
                 color=TEXT, bold=True, align="center")
        ctx.text(x + PANEL_L_W + 80, y + 13, f"{PX_PER_SEC:.0f}px/s",
                 size=9.0, color=TEXT_DIM, align="center")

        # Left block: track header column
        hdr_w = PANEL_L_W + TRACK_HDR_W
        ctx.rect(x, y + 26, hdr_w, H - 26, fill=PANEL_ALT)
        ctx.line(x + hdr_w, y, x + hdr_w, y + H, color=BORDER)

        # Clip content area
        ca_x = x + hdr_w
        ca_w = W - hdr_w
        ca_y = y + 26
        ca_h = H - 26

        ctx.push_clip(ca_x, ca_y, ca_w, ca_h)

        # Ruler
        ctx.rect(ca_x, ca_y, ca_w, RULER_H, fill="#111120")
        for ti in range(int(TOTAL_DUR) + 1):
            rx = ca_x + ti * PX_PER_SEC
            if rx < ca_x or rx > ca_x + ca_w:
                continue
            major = (ti % 5 == 0)
            ctx.line(rx, ca_y + (RULER_H - 8 if major else RULER_H - 4),
                     rx, ca_y + RULER_H, color=BORDER_LT, width=1.0)
            if major:
                ctx.text(rx + 3, ca_y + 3, f"{ti}s", size=8.5, color=TEXT_DIM)

        # Track rows
        for ri, (tid, kind) in enumerate(TRACKS):
            ty2 = ca_y + RULER_H + ri * TRACK_H
            row_fill = "#111820" if kind == "video" else "#0e1812"
            ctx.rect(ca_x, ty2, ca_w, TRACK_H, fill=row_fill)
            ctx.line(ca_x, ty2 + TRACK_H, ca_x + ca_w, ty2 + TRACK_H,
                     color=BORDER, width=1.0)

            # Clips
            for ci, (cid, cs, cd, clabel) in enumerate(CLIPS):
                if cid != tid:
                    continue
                cx2 = ca_x + cs * PX_PER_SEC
                cw  = cd * PX_PER_SEC - 2
                if cx2 + cw < ca_x or cx2 > ca_x + ca_w:
                    continue
                is_sel = ci == self._sel_clip
                if kind == "video":
                    fill = V_CLIP_SEL if is_sel else (V_CLIP if ci % 2 == 0 else V_CLIP_B)
                else:
                    fill = A_CLIP_SEL if is_sel else A_CLIP
                ctx.rect(cx2, ty2 + 3, cw, TRACK_H - 6, fill=fill, radius=3.0)
                # Waveform sketch for audio
                if kind == "audio" and cw > 16:
                    for wi in range(0, int(cw) - 4, 4):
                        wh = (math.sin(wi * 0.4 + ci * 2) * 0.4 + 0.5) * (TRACK_H - 16)
                        wy = ty2 + 3 + (TRACK_H - 6) / 2 - wh / 2
                        ctx.rect(cx2 + 2 + wi, wy, 2, wh,
                                 fill=WAVEFORM if not is_sel else "#3a9a5a",
                                 radius=0.0)
                # Clip label
                if cw > 18:
                    ctx.push_clip(cx2 + 2, ty2 + 3, cw - 4, TRACK_H - 6)
                    ctx.text(cx2 + 5, ty2 + 10, clabel, size=9.0, color=TEXT,
                             max_width=cw - 8)
                    ctx.pop_clip()
                # Selection ring
                if is_sel:
                    ctx.line(cx2,      ty2 + 3,          cx2 + cw, ty2 + 3,
                             color=ACCENT, width=1.5)
                    ctx.line(cx2,      ty2 + TRACK_H - 3, cx2 + cw, ty2 + TRACK_H - 3,
                             color=ACCENT, width=1.5)
                    ctx.line(cx2,      ty2 + 3,           cx2,      ty2 + TRACK_H - 3,
                             color=ACCENT, width=1.0)
                    ctx.line(cx2 + cw, ty2 + 3,           cx2 + cw, ty2 + TRACK_H - 3,
                             color=ACCENT, width=1.0)

        # Playhead
        ph_x = ca_x + self._ph * PX_PER_SEC
        if ca_x <= ph_x <= ca_x + ca_w:
            ctx.line(ph_x, ca_y, ph_x, ca_y + ca_h, color=PLAYHEAD, width=1.5)
            ctx.rect(ph_x - 5, ca_y, 10, 10, fill=PLAYHEAD, radius=2.0)

        ctx.pop_clip()

        # Track headers
        for ri, (tid, kind) in enumerate(TRACKS):
            ty2 = ca_y + RULER_H + ri * TRACK_H
            hx  = x + PANEL_L_W
            dot_col = "#3a72b0" if kind == "video" else "#2a7844"
            ctx.rect(hx, ty2, TRACK_HDR_W, TRACK_H, fill=PANEL_ALT)
            ctx.line(hx, ty2 + TRACK_H, hx + TRACK_HDR_W, ty2 + TRACK_H,
                     color=BORDER)
            ctx.text(hx + TRACK_HDR_W / 2, ty2 + TRACK_H / 2,
                     tid, size=11.0, color=TEXT, bold=True, align="center")
            ctx.circle(hx + TRACK_HDR_W - 10, ty2 + TRACK_H / 2, 3.0, fill=dot_col)

        # Mini label in left header
        ctx.text(x + PANEL_L_W / 2, y + 13, "TRACKS",
                 size=9.0, color=TEXT_DIM, align="center")


if __name__ == "__main__":
    VideoEditor().run()
