"""Integration tests for AppHarness — headless Python app subprocess runner."""

import textwrap
from pathlib import Path

import pytest

from plexi_sdk.testing import AppHarness

# Minimal test app — counts Enter key presses
_COUNTER_APP = textwrap.dedent("""
    from plexi_sdk import App, RenderContext

    class CounterApp(App):
        def on_init(self, ctx: RenderContext) -> None:
            self._count = 0

        def on_render(self, ctx: RenderContext) -> None:
            ctx.status_summary(f"count={self._count}")
            ctx.rect(0, 0, 100, 50, fill="#ff0000")
            ctx.text(10, 10, f"count={self._count}", size=14, color="#ffffff")

        def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
            if key == "enter":
                self._count += 1

    CounterApp().run()
""").lstrip()


@pytest.fixture
def counter_app(tmp_path: Path) -> Path:
    app_file = tmp_path / "counter_app.py"
    app_file.write_text(_COUNTER_APP)
    return app_file


def test_appharness_boots_and_renders(counter_app: Path) -> None:
    """AppHarness boots the app and returns draw commands on the first render."""
    with AppHarness(counter_app) as h:
        cmds = h.run(1)
    assert len(cmds) > 0, "Expected at least one draw command from first render"


def test_appharness_key_changes_render_output(counter_app: Path) -> None:
    """Injecting a key event changes the draw command output on the next render."""
    with AppHarness(counter_app) as h:
        cmds_before = h.run(1)
        h.key("enter")
        cmds_after = h.run(1)

    # The text command should now show count=1 instead of count=0
    def text_contents(cmds):
        return [c.get("text", "") for c in cmds if c.get("type") == "text"]

    texts_before = text_contents(cmds_before)
    texts_after = text_contents(cmds_after)

    assert "count=0" in texts_before, f"Expected 'count=0' in first render, got: {texts_before}"
    assert "count=1" in texts_after, f"Expected 'count=1' after Enter key, got: {texts_after}"


def test_appharness_context_manager_closes_cleanly(counter_app: Path) -> None:
    """AppHarness.__exit__ shuts down the subprocess without hanging."""
    with AppHarness(counter_app) as h:
        h.run(1)
        proc = h._proc
    # After __exit__, process should be terminated
    assert proc.poll() is not None, "Subprocess should have exited after close()"
