"""Live theme singleton.

Populated from the host ``Init`` payload so app-drawn chrome tracks the host
theme (light/dark + user ``[theme]`` overrides in ``config.toml``). Until ``Init``
arrives the attributes hold the built-in dark defaults, so apps
and ``ui.py`` components still render correctly pre-handshake.

The module exposes a single process-wide instance, ``theme``, which is mutated
**in place** on ``Init``. Every module that did ``from ._theme import theme`` sees
the update — so never rebind the name, only set attributes.

Apps read colors via ``ctx.theme.<role>`` (e.g. ``ctx.theme.accent``). Both the
SDK-semantic names (``bg``/``surface``/``muted``/...) and ANSI aliases
(``red``/``green``/``yellow``) are available; pick whichever reads best.
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


class Theme:
    """Mutable bag of semantic color roles, each a ``#rrggbb`` string."""

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

    __slots__ = ROLES

    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        for role, value in _DEFAULTS.items():
            setattr(self, role, value)

    def update_from(self, payload: "dict | None") -> None:
        """Overlay host-provided roles. Unknown keys and non-string/empty
        values are ignored so a partial payload never blanks a color."""
        if not payload:
            return
        for role in ROLES:
            value = payload.get(role)
            if isinstance(value, str) and value:
                setattr(self, role, value)


# Process-wide singleton. Mutated in place on Init — do not rebind.
theme = Theme()
