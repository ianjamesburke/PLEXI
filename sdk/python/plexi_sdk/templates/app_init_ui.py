"""View builders for __DISPLAY_NAME__.

Pure functions from app data to a component tree — no effects, no state
reads beyond what ``main.py`` passes in. Components describe UI; effects
(returned from ``main.py``) describe host work.

Useful primitives include ActionBar(), Card(), Section(), Badge(),
Divider(), TextEdit(), SelectList(), and Scrollable(). Editable controls
deliver UiValueChange. Use FooterKeys(), SPACE_MD, and padding=SPACE_MD.
Logging levels: log.debug, log.info, log.warn, log.error.
"""

from plexi_sdk.ui import SPACE_MD, AppBar, Column, FooterKeys, Text


def build_view(count: int) -> Column:
    return Column([
        AppBar("__DISPLAY_NAME__"),
        Text(str(count), bold=True),
        FooterKeys([
            ("+", "increment"),
            ("-", "decrement"),
            ("r", "reset"),
        ]),
    ], grow=True, padding=SPACE_MD)
