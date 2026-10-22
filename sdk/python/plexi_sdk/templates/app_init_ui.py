"""View builders for __DISPLAY_NAME__.

Pure functions from app data to a component tree — no effects, no state
reads beyond what ``main.py`` passes in. Components describe UI; effects
(returned from ``main.py``) describe host work.

Useful primitives include ActionBar(), Card(), Section(), Badge(),
Divider(), TextEdit(), SelectList(), and Scrollable(). Editable controls
deliver UiValueChange. Use FooterKeys(), SPACE_MD, and padding=SPACE_MD.
Logging levels: log.debug, log.info, log.warn, log.error.
"""

from plexi_sdk.ui import SPACE_MD, AppBar, Button, Card, Column, Keyed, Text


def _counter(label: str, count: int, action_prefix: str) -> Card:
    return Card([
        Text(label, bold=True),
        Text(str(count)),
        Keyed(Button("+", on_click=f"{action_prefix}:increment"), key=f"{action_prefix}-increment"),
        Keyed(Button("-", on_click=f"{action_prefix}:decrement"), key=f"{action_prefix}-decrement"),
        Keyed(Button("Reset", on_click=f"{action_prefix}:reset"), key=f"{action_prefix}-reset"),
    ])


def build_view(left_count: int, right_count: int) -> Column:
    return Column([
        AppBar("__DISPLAY_NAME__"),
        Keyed(_counter("Left counter", left_count, "left"), key="left-counter"),
        Keyed(_counter("Right counter", right_count, "right"), key="right-counter"),
    ], grow=True, padding=SPACE_MD)
