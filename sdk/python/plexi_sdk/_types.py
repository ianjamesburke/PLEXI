from __future__ import annotations

from dataclasses import dataclass
from typing import List, Optional


class CapabilityDeniedError(RuntimeError):
    """Raised when the host rejects a brokered call because the app's manifest
    didn't declare the required capability. Distinct from generic RuntimeError
    so apps can catch the gate-denial path explicitly."""


# ── Video substrate (#345) types ──────────────────────────────────────────────

@dataclass
class VideoHandle:
    """Result of `Emitter.open_video`. `handle_id` is opaque — pass it back to
    `set_video_state` and `close_video`. The associated `Pipe` delivers
    decoded RGBA8 frames (one frame per `pipe.read_frame()`) of length
    `width * height * 4`."""
    handle_id: int
    width: int
    height: int
    fps: float
    duration_ms: int
    pipe: "object"  # Pipe — avoid circular import; typed as object here


# ── Typed draw command dataclasses (#627) ─────────────────────────────────────

# Canonical text-align vocabulary (#2198). Single source of truth — the host
# deserializes these as a closed serde enum, so any other value fails the frame.
VALID_TEXT_ALIGN = frozenset({
    "left_top", "center_top", "right_top",
    "left_center", "center_center", "right_center",
    "left_bottom", "center_bottom", "right_bottom",
})


@dataclass
class RectCommand:
    """Typed constructor for ctx.rect(). Validates geometry at construction."""
    x: float
    y: float
    w: float
    h: float
    fill: str
    radius: float = 0.0

    def __post_init__(self) -> None:
        if self.w < 0 or self.h < 0:
            raise ValueError(f"RectCommand: w and h must be non-negative, got w={self.w}, h={self.h}")
        if not self.fill.startswith("#"):
            raise ValueError(f"RectCommand: fill must be a hex color string (e.g. '#1e1e2e'), got {self.fill!r}")


@dataclass
class TextCommand:
    """Typed constructor for ctx.text(). Validates align at construction."""
    x: float
    y: float
    text: str
    size: float
    color: str
    monospace: bool = False
    bold: bool = False
    align: str = "left_top"
    max_width: Optional[float] = None
    elide: bool = True
    selectable: bool = False

    def __post_init__(self) -> None:
        if self.align not in VALID_TEXT_ALIGN:
            raise ValueError(
                f"TextCommand: align must be one of {sorted(VALID_TEXT_ALIGN)}, got {self.align!r}"
            )
        if self.size <= 0:
            raise ValueError(f"TextCommand: size must be positive, got {self.size}")


@dataclass
class BadgeCommand:
    """Typed constructor for ctx.badge()."""
    x: float
    y_center: float
    label: str
    fill: str = ""
    fg: str = ""
    font_size: float = 11.0
    radius: float = 8.0

    def __post_init__(self) -> None:
        if self.font_size <= 0:
            raise ValueError(f"BadgeCommand: font_size must be positive, got {self.font_size}")
        if self.radius < 0:
            raise ValueError(f"BadgeCommand: radius must be non-negative, got {self.radius}")


@dataclass
class TextInputSpec:
    """Typed constructor for ctx.text_input()."""
    id: str
    x: float
    y: float
    w: float
    placeholder: str = ""
    multiline: bool = False
    h: float = 24.0

    def __post_init__(self) -> None:
        if not self.id:
            raise ValueError("TextInputSpec: id must be a non-empty string")
        if self.w <= 0:
            raise ValueError(f"TextInputSpec: w must be positive, got {self.w}")


@dataclass
class ShortcutPair:
    """Typed constructor for one entry in ctx.shortcuts() pairs."""
    keys: List[str]
    description: str = ""

    def __post_init__(self) -> None:
        if not self.keys:
            raise ValueError("ShortcutPair: keys must be a non-empty list")


@dataclass
class NotifyOption:
    """Typed constructor for one option in ctx.notify_choice()."""
    label: str
    value: Optional[str] = None
    shortcut: Optional[str] = None

    def __post_init__(self) -> None:
        if not self.label:
            raise ValueError("NotifyOption: label must be a non-empty string")

    def to_dict(self) -> dict:
        d: dict = {"label": self.label}
        if self.value is not None:
            d["value"] = self.value
        if self.shortcut is not None:
            d["shortcut"] = self.shortcut
        return d
