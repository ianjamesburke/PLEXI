"""Plexi Python SDK.

Apps expose module-level ``init(size, args)``, ``update(event)``, and ``view()``
functions. The V3AppRuntime drives the event loop: host sends JSON events on
stdin, app responds with effects and component trees on stdout.

Python apps are sandboxed. They run through the CPython-in-WASM adapter
(`src/host/wasm_python.rs`) inside their own `wasmtime::Store` with isolated
linear memory — the same component boundary a Rust WASM app gets, not a native
subprocess. An app reaches only the host interfaces Plexi links for its world,
and protected effects (file, network, AI, clipboard, pane, audio, GPU) each
require an explicit capability grant the user approves on first request. There
is no ambient process access to escape to. See `docs/wasm-runtime.md`
§ Security Model for the full boundary.
"""

from ._version import __version__

SDK_ID = f"plexi-sdk-py/{__version__}"

from ._v3_state import StateSnapshot, log, state
from . import effects as effects
from . import events as events
from . import tools as tools

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
    ProgressBar as ProgressBar,
    TextEdit as TextEdit,
)
from ._theme import theme, Theme, AppPalette
