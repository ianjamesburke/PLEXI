"""Live theme singleton.

Populated from the host Init payload so app chrome tracks the host theme
(light/dark + user overrides in config.toml). Until Init arrives the
attributes hold the built-in dark defaults.

Process-wide instance ``theme`` is mutated in place on Init. Never rebind
the name, only set attributes.

``theme.is_dark`` is True when the background luminance is below 0.5.
"""

from __future__ import annotations


# Built-in dark defaults — used before Init and as fallback when the
# host sends an empty/partial theme map.
_DEFAULTS = {
    "bg": "#1e1e2e",
    "bg_darkest": "#11111b",
    "surface": "#313244",
    "highlight": "#45475a",
    "border": "#2a2a3c",
    "fg": "#cdd6f4",
    "muted": "#6c7086",
    "text_section": "#585b70",
    "accent": "#7aa2f7",
    "danger": "#f38ba8",
    "red": "#f38ba8",
    "success": "#a6e3a1",
    "green": "#a6e3a1",
    "warning": "#f9e2af",
    "yellow": "#f9e2af",
}

ROLES = tuple(_DEFAULTS.keys())


def _luminance(hex_color: str) -> float:
    """Perceived luminance of a hex color (0.0 = black, 1.0 = white)."""
    h = hex_color.lstrip("#")
    if len(h) in (3, 4):
        h = "".join(c * 2 for c in h[:3])
    if len(h) < 6:
        return 0.0
    try:
        r, g, b = int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)
    except ValueError:
        return 0.0
    return (0.299 * r + 0.587 * g + 0.114 * b) / 255.0


class Theme:
    """Mutable bag of semantic color roles, each a ``#rrggbb`` string.

    ``is_dark`` is a computed boolean: ``True`` when the background luminance
    is below 0.5 (i.e. the theme is dark-mode). Derived from ``bg`` on every
    ``update_from`` call so it stays in sync with theme hot-reloads.
    """

    # Explicit annotations (not computed) so type checkers resolve ctx.theme.<role>.
    bg: str
    bg_darkest: str
    surface: str
    highlight: str
    border: str
    fg: str
    muted: str
    text_section: str
    accent: str
    danger: str
    red: str
    success: str
    green: str
    warning: str
    yellow: str
    is_dark: bool

    __slots__ = ROLES + ("is_dark",)

    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        for role, value in _DEFAULTS.items():
            setattr(self, role, value)
        self.is_dark = True  # dark default

    def update_from(self, payload: "dict | None") -> None:
        """Overlay host-provided roles. Unknown keys and non-string/empty
        values are ignored so a partial payload never blanks a color."""
        if not payload:
            return
        for role in ROLES:
            value = payload.get(role)
            if isinstance(value, str) and value:
                setattr(self, role, value)
        self.is_dark = _luminance(self.bg) < 0.5


class AppPalette:
    """Light/dark palette for app-defined color tokens.

    Both dicts must have the same keys. ``resolve(theme)`` returns the
    matching set based on ``theme.is_dark``.
    """

    def __init__(self, dark: "dict[str, str]", light: "dict[str, str]") -> None:
        if set(dark.keys()) != set(light.keys()):
            raise ValueError(
                f"AppPalette: dark and light dicts must have the same keys. "
                f"dark-only: {set(dark) - set(light)!r}, "
                f"light-only: {set(light) - set(dark)!r}"
            )
        self._dark = dark
        self._light = light

    def resolve(self, theme: Theme) -> "dict[str, str]":
        """Return the dark or light token set based on ``theme.is_dark``."""
        return self._dark if theme.is_dark else self._light


# Process-wide singleton. Mutated in place on Init — do not rebind.
theme = Theme()
