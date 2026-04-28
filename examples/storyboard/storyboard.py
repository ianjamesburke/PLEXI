#!/usr/bin/env python3
"""Storyboard — visual POC for a storyboarding tool.

Canvas-style horizontal scene strip. Each scene card has a synthetic
16:9 image placeholder rendered from draw primitives based on description
keywords, plus description and dialogue sections. Fully interactive.
"""
from __future__ import annotations

import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext

# ── Palette ──────────────────────────────────────────────────────────────────

BG            = "#0d0d13"
PANEL_BG      = "#12121a"
BORDER        = "#22222f"
ACCENT        = "#7c6af7"
TEXT          = "#cdd6f4"
TEXT_DIM      = "#585875"
CARD_BG       = "#15151f"
SELECTED_BDR  = "#7c6af7"

# Derived alpha variants
ACCENT_DIM    = "#7c6af740"
BORDER_MED    = "#2a2a3a"

# ── Layout constants ─────────────────────────────────────────────────────────

TOOLBAR_H  = 44.0
CARD_W     = 240.0
CARD_GAP   = 12.0
IMG_W      = 240.0
IMG_H      = 135.0       # 16:9
CARD_PAD   = 10.0
DESC_H     = 72.0        # label + 3 lines
DIAL_H     = 54.0        # italic + 2 lines
EDIT_HINT_H = 20.0
CARD_H     = IMG_H + DESC_H + DIAL_H + EDIT_HINT_H + 4.0  # 4px dividers

CARD_AREA_PAD = 16.0     # left/right margin for the card strip

# ── Default scenes ───────────────────────────────────────────────────────────

DEFAULT_SCENES = [
    {
        "desc": "EXT. City rooftop — wide establishing shot",
        "dialogue": "[silence]",
    },
    {
        "desc": "CU. Subject face, direct to camera",
        "dialogue": '"The thing nobody tells you is..."',
    },
    {
        "desc": "INT. Office, day — medium shot",
        "dialogue": '"...most people stop before it gets good."',
    },
    {
        "desc": "EXT. Street level — tracking shot",
        "dialogue": "",
    },
    {
        "desc": "CU. Hands on keyboard",
        "dialogue": '"Ship it."',
    },
]


# ── Scene image synthesis ─────────────────────────────────────────────────────

def _parse_keywords(desc: str) -> dict:
    """Extract rendering hints from a scene description string."""
    low = desc.lower()
    is_ext = any(k in low for k in ("ext.", "ext ", "exterior", "wide", "street",
                                     "rooftop", "outdoors", "outdoor"))
    is_int = any(k in low for k in ("int.", "int ", "interior", "office", "room",
                                     "indoor", "desk"))
    is_cu  = any(k in low for k in ("cu.", "cu ", "close", "face", "hands",
                                     "closeup", "close-up"))
    is_night = any(k in low for k in ("night", "dark", "dusk", "evening"))
    is_day   = any(k in low for k in ("day", "bright", "sun", "morning", "noon"))

    # Shot type label
    if is_cu:
        shot = "CU"
    elif "wide" in low or "establish" in low:
        shot = "WIDE"
    elif "medium" in low or "med" in low:
        shot = "MED"
    elif "track" in low or "tracking" in low:
        shot = "TRACK"
    else:
        shot = ""

    return {
        "ext": is_ext and not is_cu,
        "int": is_int and not is_cu,
        "cu": is_cu,
        "night": is_night,
        "day": is_day,
        "shot": shot,
    }


def _draw_scene_image(ctx: RenderContext, x: float, y: float,
                      w: float, h: float, hints: dict) -> None:
    """Render a synthetic 16:9 scene image using draw primitives."""

    night = hints["night"]
    day   = hints["day"]

    ctx.push_clip(x, y, w, h)

    if hints["ext"]:
        # Exterior scene: sky + horizon + ground
        sky_col   = "#0a0a18" if night else ("#1a2a4a" if not day else "#2a4a7a")
        ground_col = "#0d0d0a" if night else ("#1a1a0f" if not day else "#2a2a1a")
        horizon_col = "#1e1e2a" if night else ("#3a3a2a" if not day else "#4a5a3a")
        # Sky fills top ~60%
        ctx.rect(x, y, w, h * 0.62, fill=sky_col)
        # Horizon band
        ctx.rect(x, y + h * 0.56, w, h * 0.12, fill=horizon_col)
        # Ground fills bottom ~40%
        ctx.rect(x, y + h * 0.62, w, h * 0.38, fill=ground_col)
        # Horizon line
        ctx.line(x, y + h * 0.62, x + w, y + h * 0.62, color="#2a3a2a", width=1.0)
        if night:
            # Stars — small bright dots
            for sx, sy in [(0.1, 0.12), (0.25, 0.07), (0.4, 0.18), (0.6, 0.09),
                           (0.75, 0.22), (0.85, 0.14), (0.55, 0.28), (0.15, 0.30)]:
                ctx.circle(x + sx * w, y + sy * h, 1.0, fill="#cdd6f460")
        else:
            # Sun/sky gradient hint — simple bright circle
            sun_col = "#f9e2af80" if day else "#6c708640"
            ctx.circle(x + w * 0.78, y + h * 0.22, h * 0.12, fill=sun_col)
        # Simple building silhouette for city/rooftop scenes
        if any(k in hints.get("_raw_low", "") for k in ("city", "rooftop", "urban")):
            ctx.rect(x + w * 0.05, y + h * 0.30, w * 0.08, h * 0.32, fill="#0a0a12")
            ctx.rect(x + w * 0.18, y + h * 0.22, w * 0.06, h * 0.40, fill="#0c0c14")
            ctx.rect(x + w * 0.72, y + h * 0.28, w * 0.09, h * 0.34, fill="#0a0a12")
            ctx.rect(x + w * 0.84, y + h * 0.20, w * 0.07, h * 0.42, fill="#0c0c14")

    elif hints["int"]:
        # Interior scene: walls + floor
        wall_col  = "#13131e" if night else ("#18182a" if not day else "#1e1e32")
        floor_col = "#0e0e16" if night else ("#12121c" if not day else "#16162a")
        # Background wall
        ctx.rect(x, y, w, h * 0.70, fill=wall_col)
        # Floor
        ctx.rect(x, y + h * 0.70, w, h * 0.30, fill=floor_col)
        # Wall/floor junction line
        ctx.line(x, y + h * 0.70, x + w, y + h * 0.70, color=BORDER, width=1.0)
        # Perspective floor lines
        cx_ = x + w * 0.5
        for t in [0.3, 0.6, 0.9]:
            ctx.line(cx_, y + h * 0.70,
                     x + t * w, y + h,
                     color="#1e1e2a", width=0.5)
        # Window suggestion (bright rectangle on wall)
        if not night:
            ctx.rect(x + w * 0.60, y + h * 0.12, w * 0.28, h * 0.30,
                     fill="#c0d0e030", radius=2.0)
            # Window frame
            ctx.line(x + w * 0.74, y + h * 0.12,
                     x + w * 0.74, y + h * 0.42, color="#ffffff20", width=1.0)
        # Desk silhouette for office scenes
        if "office" in hints.get("_raw_low", "") or "desk" in hints.get("_raw_low", ""):
            ctx.rect(x + w * 0.10, y + h * 0.64, w * 0.55, h * 0.06,
                     fill="#0c0c18", radius=1.0)
            ctx.rect(x + w * 0.12, y + h * 0.70, w * 0.04, h * 0.14,
                     fill="#0a0a14")
            ctx.rect(x + w * 0.58, y + h * 0.70, w * 0.04, h * 0.14,
                     fill="#0a0a14")

    elif hints["cu"]:
        # Close-up: large oval silhouette centered
        bg_col = "#0c0c16" if night else "#10101e"
        ctx.rect(x, y, w, h, fill=bg_col)
        # Subtle radial vignette suggestion (dark corners)
        ctx.rect(x, y, w, h, fill="#00000030", radius=0.0)
        # Head oval
        head_cx = x + w * 0.5
        head_cy = y + h * 0.42
        head_rx = w * 0.22
        head_ry = h * 0.35
        ctx.circle(head_cx, head_cy, head_rx, fill="#1e1e2a")
        # Neck/shoulder suggestion
        ctx.rect(x + w * 0.38, y + h * 0.70, w * 0.24, h * 0.30, fill="#1a1a26")
        # Shoulder curves (simple arcs as rects)
        ctx.rect(x + w * 0.15, y + h * 0.72, w * 0.28, h * 0.28,
                 fill="#161620", radius=30.0)
        ctx.rect(x + w * 0.57, y + h * 0.72, w * 0.28, h * 0.28,
                 fill="#161620", radius=30.0)
        # Eyes — two small ellipses
        ctx.circle(x + w * 0.44, y + h * 0.40, w * 0.035, fill="#2a2a3a")
        ctx.circle(x + w * 0.56, y + h * 0.40, w * 0.035, fill="#2a2a3a")

    else:
        # Default: abstract dark gradient placeholder
        ctx.rect(x, y, w, h, fill="#0d0d18")
        # Diagonal accent stripe
        ctx.rect(x, y + h * 0.35, w, h * 0.30, fill="#7c6af708", radius=0.0)
        # Center marker
        ctx.circle(x + w * 0.5, y + h * 0.5, min(w, h) * 0.12,
                   fill="#7c6af715")
        ctx.circle(x + w * 0.5, y + h * 0.5, min(w, h) * 0.05,
                   fill="#7c6af730")

    ctx.pop_clip()


# ── Modal overlay helpers ─────────────────────────────────────────────────────

def _draw_modal(ctx: RenderContext, title: str, input_id: str,
                placeholder: str, current: str) -> "str | None":
    """Render a centered edit modal with a text_input. Returns submitted value or None."""
    mw = min(500.0, ctx.w - 80.0)
    mh = 110.0
    mx = (ctx.w - mw) / 2.0
    my = (ctx.h - mh) / 2.0

    # Scrim
    ctx.rect(0, 0, ctx.w, ctx.h, fill="#00000080")

    # Modal panel
    ctx.rect(mx, my, mw, mh, fill="#1a1a28", radius=10.0)
    # Border
    ctx.rect(mx, my, mw, 1.0, fill=ACCENT)
    ctx.rect(mx, my + mh - 1.0, mw, 1.0, fill=BORDER)
    ctx.rect(mx, my, 1.0, mh, fill=BORDER)
    ctx.rect(mx + mw - 1.0, my, 1.0, mh, fill=BORDER)

    # Title
    ctx.text(mx + 16.0, my + 14.0, title, size=13.0, color=TEXT, bold=True)
    # Hint
    ctx.text(mx + mw - 90.0, my + 14.0, "Enter ↩  Esc cancel",
             size=10.0, color=TEXT_DIM)

    # Text input widget
    submitted = ctx.text_input(
        input_id,
        x=mx + 12.0,
        y=my + 40.0,
        w=mw - 24.0,
        placeholder=placeholder,
    )
    return submitted


# ── App ───────────────────────────────────────────────────────────────────────

class StoryboardApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        self._scenes: list[dict] = [
            {"desc": s["desc"], "dialogue": s["dialogue"]}
            for s in DEFAULT_SCENES
        ]
        self._sel: int = 0
        self._scroll_offset: float = 0.0  # horizontal px offset into card strip

        # Edit mode: None | "desc" | "dialogue"
        self._edit_mode: "str | None" = None

        # Pending delete confirmation
        self._confirm_delete: bool = False

        # Card strip viewport width (set during render)
        self._strip_w: float = 0.0

    # ── Scene helpers ─────────────────────────────────────────────────────────

    def _add_scene_after(self, idx: int) -> None:
        new = {"desc": "New scene", "dialogue": ""}
        self._scenes.insert(idx + 1, new)
        self._sel = idx + 1

    def _delete_scene(self, idx: int) -> None:
        if len(self._scenes) <= 1:
            return
        self._scenes.pop(idx)
        self._sel = min(idx, len(self._scenes) - 1)

    def _scroll_to_selected(self) -> None:
        """Adjust scroll offset so the selected card is fully visible."""
        card_left = self._sel * (CARD_W + CARD_GAP)
        card_right = card_left + CARD_W
        viewport_w = self._strip_w
        if viewport_w <= 0:
            return
        if card_left < self._scroll_offset:
            self._scroll_offset = card_left - CARD_AREA_PAD
        elif card_right > self._scroll_offset + viewport_w:
            self._scroll_offset = card_right - viewport_w + CARD_AREA_PAD
        self._scroll_offset = max(0.0, self._scroll_offset)

    # ── Input ──────────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        # Edit mode: only Escape escapes; text_input handles everything else
        if self._edit_mode is not None:
            if key == "Escape":
                self._edit_mode = None
            return

        # Confirm-delete mode
        if self._confirm_delete:
            if key in ("y", "Return", "Enter"):
                self._delete_scene(self._sel)
                self._confirm_delete = False
            elif key in ("n", "Escape"):
                self._confirm_delete = False
            return

        if key == "ArrowLeft":
            self._sel = max(0, self._sel - 1)
            self._scroll_to_selected()
        elif key == "ArrowRight":
            self._sel = min(len(self._scenes) - 1, self._sel + 1)
            self._scroll_to_selected()
        elif key == "n":
            self._add_scene_after(self._sel)
            self._scroll_to_selected()
        elif key == "d":
            if len(self._scenes) > 1:
                self._confirm_delete = True
        elif key == "e":
            self._edit_mode = "desc"
        elif key == "t":
            self._edit_mode = "dialogue"
        elif key == "Escape":
            self._confirm_delete = False
        elif key.isdigit() and key != "0":
            idx = int(key) - 1
            if 0 <= idx < len(self._scenes):
                self._sel = idx
                self._scroll_to_selected()

    # ── Rendering ─────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h

        # Background
        ctx.rect(0, 0, w, h, fill=BG)

        # ── Toolbar ──────────────────────────────────────────────────────────
        ctx.rect(0, 0, w, TOOLBAR_H, fill=PANEL_BG)
        ctx.line(0, TOOLBAR_H, w, TOOLBAR_H, color=BORDER, width=1.0)

        # Title
        ctx.text(16.0, 14.0, "Storyboard", size=15.0, color=TEXT, bold=True)

        # Scene count badge
        scene_label = f"{self._sel + 1} / {len(self._scenes)}"
        ctx.text(130.0, 15.0, scene_label, size=12.0, color=TEXT_DIM)

        # [+ Scene] button
        btn_x = 200.0
        btn_y = 9.0
        ctx.rect(btn_x, btn_y, 70.0, 26.0, fill="#1e1e30", radius=6.0)
        ctx.rect(btn_x, btn_y, 70.0, 1.0, fill=BORDER)
        ctx.rect(btn_x, btn_y + 25.0, 70.0, 1.0, fill=BORDER)
        ctx.rect(btn_x, btn_y, 1.0, 26.0, fill=BORDER)
        ctx.rect(btn_x + 69.0, btn_y, 1.0, 26.0, fill=BORDER)
        ctx.text(btn_x + 14.0, btn_y + 7.0, "+ Scene", size=11.0, color=ACCENT)

        # Keyboard hints
        hints = "← → navigate  •  n new  •  e edit desc  •  t edit dialogue  •  d delete  •  1–9 jump"
        ctx.text(w - 16.0, 15.0, hints, size=10.0, color=TEXT_DIM,
                 align="right", max_width=w - 290.0, elide=True)

        # ── Card strip ───────────────────────────────────────────────────────
        strip_y = TOOLBAR_H + 20.0
        strip_h = h - strip_y
        self._strip_w = w

        # Clip the card area
        ctx.push_clip(0, strip_y, w, strip_h)

        for i, scene in enumerate(self._scenes):
            card_x = CARD_AREA_PAD + i * (CARD_W + CARD_GAP) - self._scroll_offset
            card_y = strip_y + 10.0

            # Skip cards entirely off-screen
            if card_x + CARD_W < 0 or card_x > w:
                continue

            self._draw_card(ctx, card_x, card_y, i, scene)

        ctx.pop_clip()

        # ── Modals ────────────────────────────────────────────────────────────
        if self._edit_mode == "desc":
            scene = self._scenes[self._sel]
            submitted = _draw_modal(
                ctx,
                title=f"Scene {self._sel + 1} — Description",
                input_id="edit_desc",
                placeholder="Describe the scene…",
                current=scene["desc"],
            )
            if submitted is not None:
                scene["desc"] = submitted
                self._edit_mode = None

        elif self._edit_mode == "dialogue":
            scene = self._scenes[self._sel]
            submitted = _draw_modal(
                ctx,
                title=f"Scene {self._sel + 1} — Dialogue / Transcript",
                input_id="edit_dialogue",
                placeholder="Enter dialogue or transcript…",
                current=scene["dialogue"],
            )
            if submitted is not None:
                scene["dialogue"] = submitted
                self._edit_mode = None

        elif self._confirm_delete:
            # Confirm-delete banner
            bw = 340.0
            bh = 60.0
            bx = (w - bw) / 2.0
            by = (h - bh) / 2.0
            ctx.rect(0, 0, w, h, fill="#00000060")
            ctx.rect(bx, by, bw, bh, fill="#1e0d0d", radius=8.0)
            ctx.rect(bx, by, bw, 1.0, fill="#f38ba8")
            ctx.text(bx + 20.0, by + 12.0,
                     f"Delete scene {self._sel + 1}?", size=13.0, color="#f38ba8", bold=True)
            ctx.text(bx + 20.0, by + 36.0,
                     "y / Enter = confirm    n / Esc = cancel",
                     size=11.0, color=TEXT_DIM)

    def _draw_card(self, ctx: RenderContext, x: float, y: float,
                   idx: int, scene: dict) -> None:
        """Render a single scene card."""
        selected = idx == self._sel

        # Card background
        ctx.rect(x, y, CARD_W, CARD_H, fill=CARD_BG, radius=8.0)

        # Border (accent if selected, else dim)
        bdr = SELECTED_BDR if selected else BORDER
        bdr_w = 1.5 if selected else 1.0
        ctx.rect(x, y, CARD_W, 1.0, fill=bdr)
        ctx.rect(x, y + CARD_H - 1.0, CARD_W, 1.0, fill=bdr)
        ctx.rect(x, y, 1.0, CARD_H, fill=bdr)
        ctx.rect(x + CARD_W - 1.0, y, 1.0, CARD_H, fill=bdr)

        if selected:
            # Subtle accent glow on top edge
            ctx.rect(x + 2.0, y, CARD_W - 4.0, 2.0, fill=ACCENT_DIM, radius=0.0)

        # ── 1. Scene image (16:9) ─────────────────────────────────────────────
        img_y = y
        hints = _parse_keywords(scene["desc"])
        # Attach raw lowercased desc for secondary checks inside renderer
        hints["_raw_low"] = scene["desc"].lower()
        _draw_scene_image(ctx, x, img_y, IMG_W, IMG_H, hints)

        # Clip the image badge overlay to the image rect
        ctx.push_clip(x, img_y, IMG_W, IMG_H)

        # Scene number badge (top-left)
        ctx.rect(x + 6.0, img_y + 6.0, 26.0, 18.0, fill=ACCENT, radius=4.0)
        ctx.text(x + 9.0, img_y + 8.0, str(idx + 1), size=10.0, color="#ffffff",
                 bold=True)

        # Shot type label (top-right)
        shot = hints.get("shot", "")
        if shot:
            ctx.text(x + IMG_W - 6.0, img_y + 9.0, shot, size=9.0,
                     color=TEXT_DIM, align="right")

        ctx.pop_clip()

        # ── 2. Description section ─────────────────────────────────────────────
        desc_y = img_y + IMG_H
        # Divider
        ctx.rect(x, desc_y, CARD_W, 1.0, fill=BORDER_MED)
        desc_y += 1.0

        ctx.push_clip(x + CARD_PAD, desc_y, CARD_W - CARD_PAD * 2, DESC_H)

        ctx.text(x + CARD_PAD, desc_y + 6.0, "DESCRIPTION", size=9.0,
                 color=TEXT_DIM, bold=True)

        # Wrap description text into up to 3 lines
        desc_lines = _wrap_text(scene["desc"], max_chars=30, max_lines=3)
        for li, line in enumerate(desc_lines):
            ctx.text(x + CARD_PAD, desc_y + 20.0 + li * 16.0, line,
                     size=11.5, color=TEXT, max_width=CARD_W - CARD_PAD * 2,
                     elide=True)

        ctx.pop_clip()

        # ── 3. Dialogue / transcript section ──────────────────────────────────
        dial_y = desc_y + DESC_H
        # Divider
        ctx.rect(x, dial_y, CARD_W, 1.0, fill=BORDER_MED)
        dial_y += 1.0

        ctx.push_clip(x + CARD_PAD, dial_y, CARD_W - CARD_PAD * 2, DIAL_H)

        ctx.text(x + CARD_PAD, dial_y + 6.0, "DIALOGUE", size=9.0,
                 color=TEXT_DIM, bold=True)

        dial_text = scene["dialogue"] or "—"
        dial_lines = _wrap_text(dial_text, max_chars=30, max_lines=2)
        for li, line in enumerate(dial_lines):
            # Italic-style: slightly dimmer, no bold
            ctx.text(x + CARD_PAD, dial_y + 20.0 + li * 16.0, line,
                     size=11.0, color="#9090aa",
                     max_width=CARD_W - CARD_PAD * 2, elide=True)

        ctx.pop_clip()

        # ── 4. Edit hint at bottom ─────────────────────────────────────────────
        hint_y = dial_y + DIAL_H
        ctx.rect(x, hint_y, CARD_W, 1.0, fill=BORDER_MED)
        hint_y += 1.0

        ctx.push_clip(x, hint_y, CARD_W, EDIT_HINT_H)
        if selected:
            ctx.text(x + CARD_W / 2.0, hint_y + 5.0,
                     "e edit desc  •  t dialogue",
                     size=9.0, color=TEXT_DIM, align="top_center",
                     max_width=CARD_W - 8.0, elide=True)
        ctx.pop_clip()


# ── Text wrap utility ─────────────────────────────────────────────────────────

def _wrap_text(text: str, max_chars: int, max_lines: int) -> list[str]:
    """Simple word-wrap into a list of strings."""
    if not text:
        return []
    words = text.split()
    lines: list[str] = []
    current = ""
    for word in words:
        candidate = word if not current else f"{current} {word}"
        if len(candidate) <= max_chars:
            current = candidate
        else:
            if current:
                lines.append(current)
                current = word
            else:
                lines.append(word[:max_chars - 1] + "…")
                current = ""
            if len(lines) >= max_lines:
                break
    if current and len(lines) < max_lines:
        lines.append(current)
    # Truncate last line with ellipsis if content was cut
    if len(lines) == max_lines and current and len(current) > max_chars:
        last = lines[-1]
        if not last.endswith("…"):
            lines[-1] = last[:max_chars - 1] + "…"
    return lines


if __name__ == "__main__":
    StoryboardApp().run()
