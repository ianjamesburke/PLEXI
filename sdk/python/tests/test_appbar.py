"""Tests for AppBar component layout."""

from unittest.mock import MagicMock, call

from plexi_sdk.ui import AppBar


def test_appbar_text_y_centered() -> None:
    """AppBar.render places the title text at exactly (BAND_H - TITLE_SIZE) / 2
    below the component's y origin — no empirical nudge."""
    bar = AppBar("test")

    # measure() must return BAND_H + DIVIDER_H = 36 + 1 = 37.0
    assert bar.measure(200) == 37.0

    ctx = MagicMock()
    bar.render(ctx, x=0, y=0, w=200, h=37)

    # Expected text_y: 0 + (36 - 16) / 2 = 10.0
    ctx.text.assert_called_once_with(
        0,
        10.0,
        "test",
        size=AppBar.TITLE_SIZE,
        color=bar.accent,
        bold=True,
        max_width=200,
        elide=True,
    )
