"""Widget primitives for Plexi apps.

Import from this namespace:

    from plexi_sdk.widgets import ScrollState, TextBuffer, TextArea, TextAreaTheme
    from plexi_sdk.widgets.text_input import emit_text_input

`text_input` is the v3.1 single-line submit-only primitive (host owns
the buffer). `TextArea` is a separate multi-line app-managed widget.
"""
from plexi_sdk.widgets.scroll import ScrollState
from plexi_sdk.widgets.text_buffer import TextBuffer, Cursor, Selection
from plexi_sdk.widgets.text_area import TextArea, TextAreaTheme
from plexi_sdk.widgets.text_input import emit_text_input
from plexi_sdk.widgets.button import Button, ButtonStyle
from plexi_sdk.widgets.keymap import KeyMap
from plexi_sdk.widgets.list_view import ListView

__all__ = [
    "ScrollState",
    "TextBuffer", "Cursor", "Selection",
    "TextArea", "TextAreaTheme",
    "emit_text_input",
    "Button", "ButtonStyle",
    "KeyMap",
    "ListView",
]
