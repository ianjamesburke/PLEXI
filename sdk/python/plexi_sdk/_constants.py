from __future__ import annotations

# ── SDK version (single source of truth) ──────────────────────────────────────
_SDK_VERSION = "0.5.0"

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

# ── Notification priority tiers ───────────────────────────────────────────────
# Higher = more urgent. Queue sorts priority DESC, arrival ASC. See the
# NOTIFICATIONS block in the module docstring for guidance on which tier to
# pick. Range 0..200 is the "app band" — stay inside it. A future release
# may reserve priorities above 200 for user overrides so apps can't yell
# themselves to the top; staying in-band today keeps you forward-compatible.
PRIORITY_LOW      = 0
PRIORITY_NORMAL   = 50
PRIORITY_HIGH     = 100
PRIORITY_CRITICAL = 200


def rgba(r: int, g: int, b: int, a: int = 255) -> str:
    """Return an 8-digit hex color string #rrggbbaa."""
    return f"#{r:02x}{g:02x}{b:02x}{a:02x}"


def dim(hex_color: str, alpha: int) -> str:
    """Return hex_color with the given alpha (0-255). Strips existing alpha."""
    h = hex_color.lstrip("#")[:6]
    return f"#{h}{alpha:02x}"
