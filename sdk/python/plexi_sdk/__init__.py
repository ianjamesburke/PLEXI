"""Plexi Python SDK.

Apps expose module-level ``init(size, args)``, ``update(event)``, and ``view()``
functions. The V3AppRuntime drives the event loop: host sends JSON events on
stdin, app responds with effects and component trees on stdout.

SDK v3 Python apps run through native ProcessApp. Capabilities gate PGAP host
APIs; they are not a process sandbox.

A future CPython-in-WASM compatibility runtime may provide a sandboxed Python
app route. That work is deferred and is not the current SDK v3 execution path.
"""

__version__ = "4.0.0"
SDK_ID = f"plexi-sdk-py/{__version__}"

from ._v3_state import StateSnapshot, log, state
from . import effects as effects
from . import events as events

from ._constants import (
    TITLE, HEADING, BODY, CAPTION, HINT, MONO_BODY, MONO_SMALL,
    PAD, PAD_TIGHT, HEADER_H, STATUS_H,
    PRIORITY_LOW, PRIORITY_NORMAL, PRIORITY_HIGH, PRIORITY_CRITICAL,
    BG, FG, ACCENT, SURFACE, HIGHLIGHT, MUTED, GREEN, RED, YELLOW,
    rgba, dim,
)
from ._types import (
    CapabilityDeniedError, VideoHandle,
    RectCommand, TextCommand, BadgeCommand, ShortcutPair, NotifyOption,
)
from ._protocol import AiResponse, MidiPortInfo, MidiDeviceList, AudioDeviceInfo, AudioDeviceList, PROTOCOL_VERSION
from .ui import (
    Tabs as Tabs,
    Grid as Grid,
    Toggle as Toggle,
    Clickable as Clickable,
    ProgressBar as ProgressBar,
    TextEdit as TextEdit,
)
from ._theme import theme, Theme, AppPalette

_workspace_root: str = ""
pane_width: float = 0.0
pane_height: float = 0.0
canvas_width: float = 0.0
canvas_height: float = 0.0
keys_held: set[str] = set()
_state: StateSnapshot | None = None
_in_view: bool = False
