"""Widget primitives for Plexi apps.

Import from this namespace:

    from plexi_sdk.widgets import ScrollState, TextBuffer, TextArea, TextAreaTheme
"""
from plexi_sdk.widgets.scroll import ScrollState
from plexi_sdk.widgets.text_buffer import TextBuffer, Cursor, Selection
from plexi_sdk.widgets.text_area import TextArea, TextAreaTheme

__all__ = [
    "ScrollState",
    "TextBuffer", "Cursor", "Selection",
    "TextArea", "TextAreaTheme",
]
