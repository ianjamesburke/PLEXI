"""Tests for AppHarness.assert_no_overlap() and Component.render_into()."""

import textwrap
from pathlib import Path

import pytest

from plexi_sdk.testing import AppHarness


# ---------------------------------------------------------------------------
# Fixture apps
# ---------------------------------------------------------------------------

_OVERLAPPING_APP = textwrap.dedent("""
    from plexi_sdk import App, RenderContext

    class OverlapApp(App):
        def on_render(self, ctx: RenderContext) -> None:
            ctx.clear("#1e1e2e")
            # Two non-background rects that deliberately overlap.
            ctx.rect(10, 10, 100, 50, "#3a3a4a")
            ctx.rect(50, 30, 100, 50, "#585b70")

    OverlapApp().run()
""").lstrip()

_CLEAN_APP = textwrap.dedent("""
    from plexi_sdk import App, RenderContext

    class CleanApp(App):
        def on_render(self, ctx: RenderContext) -> None:
            ctx.clear("#1e1e2e")
            # Header block at top, content block below — no overlap.
            ctx.rect(0, 0, 400, 34, "#313244")   # header band
            ctx.rect(0, 35, 400, 200, "#1e1e2e") # content area (same fill, different y)

    CleanApp().run()
""").lstrip()

_RENDER_INTO_APP = textwrap.dedent("""
    from plexi_sdk import App, RenderContext
    from plexi_sdk.ui import AppBar, Section

    class FlowApp(App):
        def on_render(self, ctx: RenderContext) -> None:
            ctx.clear("#1e1e2e")
            y = 0.0
            y += AppBar("Title").render_into(ctx, 0, y, ctx.w)
            y += Section("Items").render_into(ctx, 0, y, ctx.w)
            ctx.text(0, y, f"content_y={y:.0f}", size=12, color="#cdd6f4")

    FlowApp().run()
""").lstrip()


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


@pytest.fixture
def overlapping_app(tmp_path: Path) -> Path:
    p = tmp_path / "overlap_app.py"
    p.write_text(_OVERLAPPING_APP)
    return p


@pytest.fixture
def clean_app(tmp_path: Path) -> Path:
    p = tmp_path / "clean_app.py"
    p.write_text(_CLEAN_APP)
    return p


@pytest.fixture
def render_into_app(tmp_path: Path) -> Path:
    p = tmp_path / "render_into_app.py"
    p.write_text(_RENDER_INTO_APP)
    return p


def test_assert_no_overlap_raises_on_overlapping_rects(overlapping_app: Path) -> None:
    """assert_no_overlap() raises AssertionError when two rects intersect."""
    with AppHarness(overlapping_app, width=400, height=300) as h:
        h.run(1)
        with pytest.raises(AssertionError, match="Overlapping draw rects detected"):
            h.assert_no_overlap()


def test_assert_no_overlap_passes_on_clean_layout(clean_app: Path) -> None:
    """assert_no_overlap() passes when no non-background rects overlap."""
    with AppHarness(clean_app, width=400, height=300) as h:
        h.run(1)
        h.assert_no_overlap()


def test_render_into_returns_consumed_height(render_into_app: Path) -> None:
    """render_into() flows components — content_y equals sum of component heights."""
    with AppHarness(render_into_app, width=400, height=300) as h:
        cmds = h.run(1)

    text_cmds = [c for c in cmds if c.get("type") == "text" and "content_y=" in c.get("text", "")]
    assert text_cmds, "Expected a text command reporting content_y"
    reported_y = float(text_cmds[0]["text"].split("=")[1])
    # AppBar is 35px (34 band + 1 divider), Section is 28px (8+11+4+1+4) — total ~63px
    assert 55 < reported_y < 75, f"content_y should be ~63px, got {reported_y}"
