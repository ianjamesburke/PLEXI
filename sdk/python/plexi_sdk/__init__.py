"""Python SDK v2 for Plexi PGAP apps.

Normal apps implement ``view()`` and return a component tree:

    from plexi_sdk import App
    from plexi_sdk.ui import AppBar, Column, FooterKeys, Label, Spacer

    class CounterApp(App):
        def on_init(self) -> None:
            self.count = self.state.get("count", 0)

        def view(self):
            return Column([
                AppBar("Counter"),
                Spacer(grow=True),
                Label(str(self.count), bold=True),
                Spacer(grow=True),
                FooterKeys([("+", "increment"), ("-", "decrement")]),
            ])

        def on_key(self, key: str, mods: dict) -> None:
            if key in ("plus", "equals"):
                self.count += 1
            elif key == "minus":
                self.count -= 1
            self.state.save({"count": self.count})
            self.emit.schedule_render()

    CounterApp().run()

Use ``on_render(ctx)`` only for games, animations, realtime visualizations, or
other pixel-control apps. Never override both ``view()`` and ``on_render(ctx)``.

Host-brokered actions live on ``self.emit``:

    await self.emit.http_get(url)       # requires net.http
    await self.emit.secret_get("KEY")   # requires secrets.get
    await self.emit.ai_query("low", system, messages)  # requires ai.query

Capabilities gate PGAP host APIs. Python apps are native subprocesses, not a
process sandbox.
"""

__version__ = "0.5.0"
SDK_ID = f"plexi-sdk-py/{__version__}"

from ._constants import (
    TITLE, HEADING, BODY, CAPTION, HINT, MONO_BODY, MONO_SMALL,
    PAD, PAD_TIGHT, HEADER_H, STATUS_H,
    PRIORITY_LOW, PRIORITY_NORMAL, PRIORITY_HIGH, PRIORITY_CRITICAL,
    rgba, dim,
)
from ._types import (
    CapabilityDeniedError, VideoHandle,
    RectCommand, TextCommand, BadgeCommand, TextInputSpec, ShortcutPair, NotifyOption,
)
from ._protocol import AiResponse, MidiPortInfo, MidiDeviceList, AudioDeviceInfo, AudioDeviceList, PROTOCOL_VERSION
from ._emitter import Emitter, _emit, _make_async_queue, _LOCK
from ._pipe import Pipe
from ._render_context import RenderContext, COMPACT_DEFAULT, REGULAR_DEFAULT
from ._app import App, Arg
from ._state import State as State
from .ui import (
    Tabs as Tabs,
    Grid as Grid,
    Toggle as Toggle,
    Clickable as Clickable,
    ProgressBar as ProgressBar,
    TextEdit as TextEdit,
)
from ._theme import theme, Theme, AppPalette
