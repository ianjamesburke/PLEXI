from __future__ import annotations

from ._theme import _DEFAULTS as _TD

# ── Font sizes (float, points) ────────────────────────────────────────────────
TITLE      = 22.0
HEADING    = 18.0
BODY       = 15.0
CAPTION    = 13.0
HINT       = 12.0
MONO_BODY  = 14.0
MONO_SMALL = 12.0

# ── Layout (float, pixels) ────────────────────────────────────────────────────
PAD       = 16.0
PAD_TIGHT =  8.0
HEADER_H  = 48.0
STATUS_H  = 44.0


# ── Color constants (dark-mode defaults; derive from _theme._DEFAULTS) ────────
BG        = _TD["bg"]
FG        = _TD["fg"]
ACCENT    = _TD["accent"]
SURFACE   = _TD["surface"]
HIGHLIGHT = _TD["highlight"]
MUTED     = _TD["muted"]
GREEN     = _TD["green"]
RED       = _TD["red"]
YELLOW    = _TD["yellow"]


def rgba(r: int, g: int, b: int, a: int = 255) -> str:
    """Return an 8-digit hex color string #rrggbbaa."""
    return f"#{r:02x}{g:02x}{b:02x}{a:02x}"


def dim(hex_color: str, alpha: int) -> str:
    """Return hex_color with the given alpha (0-255). Strips existing alpha."""
    h = hex_color.lstrip("#")[:6]
    return f"#{h}{alpha:02x}"
