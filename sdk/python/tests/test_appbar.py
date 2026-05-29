"""Tests for AppBar component layout."""

from unittest.mock import MagicMock

from plexi_sdk import theme
from plexi_sdk.ui import AppBar, SPACE_MD


def test_appbar_text_y_centered() -> None:
    """AppBar.render places the title text at exactly (BAND_H - TITLE_SIZE) / 2
    below the component's y origin, inset by SPACE_MD on the left. With no
    explicit accent, the title color resolves to the active theme foreground."""
    bar = AppBar("test")

    # measure() must return BAND_H + DIVIDER_H = 36 + 1 = 37.0
    assert bar.measure(200) == 37.0

    ctx = MagicMock()
    bar.render(ctx, x=0, y=0, w=200, h=37)

    # Expected text_y: 0 + (36 - 16) / 2 = 10.0; text_x: x + SPACE_MD.
    ctx.text.assert_called_once_with(
        SPACE_MD,
        10.0,
        "test",
        size=AppBar.TITLE_SIZE,
        color=theme.fg,
        bold=True,
        max_width=200 - 2 * SPACE_MD,
        elide=True,
    )
