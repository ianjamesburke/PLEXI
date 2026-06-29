"""Plexi Python SDK v3 for native ProcessApp apps.

App modules expose exactly three lifecycle functions:

``init(size, args) -> list``
    Called once at launch. Return startup effects such as ``SetTitle`` or
    ``SetState``.

``update(event) -> list``
    Called for keyboard, mouse, timer, render, and host-result events. Return
    effects; do not mutate Plexi state directly.

``view() -> Component``
    Called after state changes to produce the current component tree. Keep it
    pure: read ``state`` here, but return state-changing effects from
    ``update``.

Useful entry points:
``plexi_sdk.effects`` for effect dataclasses,
``plexi_sdk.events`` for event dataclasses, and
``plexi_sdk.ui`` for declarative components.

SDK v3 Python apps run as reviewed native processes through ``ProcessApp``.
Capabilities gate host APIs; they are not a process sandbox. CPython-in-WASM is
deferred and is not this runtime.
"""

from ._version import __version__ as __version__

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
