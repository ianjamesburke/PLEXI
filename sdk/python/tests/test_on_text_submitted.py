"""Tests for on_text_submitted hook (issue #1067).

Verifies that:
  - on_text_submitted fires when a text_submitted event arrives
  - it does NOT fire during on_render (no side effect coupling)
  - backward compat: _take_text_submission still works for apps that poll during render
"""

import textwrap
from pathlib import Path

import pytest

from plexi_sdk.testing import AppHarness

# App that overrides on_text_submitted and tracks calls.
# Exposes call count via draw commands so tests can assert from outside.
_HOOK_APP = textwrap.dedent("""
    from plexi_sdk import App, RenderContext

    class HookApp(App):
        def on_init(self) -> None:
            self._hook_calls: list[tuple[str, str]] = []
            self._render_count = 0

        def on_render(self, ctx: RenderContext) -> None:
            self._render_count += 1
            ctx.text_input("field1", x=10, y=10, w=200, placeholder="enter text")
            count = len(self._hook_calls)
            ctx.text(10, 50, f"hook_calls={count}", size=14, color="#ffffff")
            ctx.text(10, 70, f"renders={self._render_count}", size=14, color="#ffffff")

        async def on_text_submitted(self, id: str, text: str) -> None:
            self._hook_calls.append((id, text))

    HookApp().run()
""").lstrip()

# App that does NOT override on_text_submitted — backward compat path via on_render polling.
_COMPAT_APP = textwrap.dedent("""
    from plexi_sdk import App, RenderContext

    class CompatApp(App):
        def on_init(self) -> None:
            self._last_submitted: str | None = None

        def on_render(self, ctx: RenderContext) -> None:
            submitted = ctx.text_input("field1", x=10, y=10, w=200, placeholder="enter text")
            if submitted is not None:
                self._last_submitted = submitted
            value = self._last_submitted or "none"
            ctx.text(10, 50, f"last={value}", size=14, color="#ffffff")

    CompatApp().run()
""").lstrip()


@pytest.fixture
def hook_app(tmp_path: Path) -> Path:
    p = tmp_path / "hook_app.py"
    p.write_text(_HOOK_APP)
    return p


@pytest.fixture
def compat_app(tmp_path: Path) -> Path:
    p = tmp_path / "compat_app.py"
    p.write_text(_COMPAT_APP)
    return p


def _text_values(cmds: list[dict]) -> list[str]:
    return [c.get("text", "") for c in cmds if c.get("type") == "text"]


def test_on_text_submitted_fires_on_event(hook_app: Path) -> None:
    """on_text_submitted is called once when a text_submitted event arrives."""
    with AppHarness(hook_app) as h:
        h.run(1)  # initial render — hook not called yet
        h.text_submit("field1", "hello")
        h.run(1)  # event loop dispatches the hook task here
        cmds = h.run(1)  # draw commands now reflect the hook call

    texts = _text_values(cmds)
    assert any("hook_calls=1" in t for t in texts), (
        f"Expected hook_calls=1 in draw commands after text_submit, got: {texts}"
    )


def test_on_text_submitted_not_triggered_by_render(hook_app: Path) -> None:
    """Rendering without a text_submitted event does not call on_text_submitted."""
    with AppHarness(hook_app) as h:
        cmds = h.run(3)  # three render frames, no text_submitted sent

    texts = _text_values(cmds)
    assert any("hook_calls=0" in t for t in texts), (
        f"Expected hook_calls=0 after renders with no submission, got: {texts}"
    )
    assert any("renders=3" in t for t in texts), (
        f"Expected renders=3 in draw commands, got: {texts}"
    )


def test_on_text_submitted_receives_id_and_text(hook_app: Path) -> None:
    """on_text_submitted hook fires with the correct id and text."""
    with AppHarness(hook_app) as h:
        h.run(1)
        h.text_submit("field1", "world")
        h.run(1)
        cmds = h.run(1)

    texts = _text_values(cmds)
    assert any("hook_calls=1" in t for t in texts), (
        f"Expected hook_calls=1 in draw commands, got: {texts}"
    )


def test_backward_compat_render_polling(compat_app: Path) -> None:
    """Apps that haven't adopted on_text_submitted still receive submissions via ctx.text_input()."""
    with AppHarness(compat_app) as h:
        h.run(1)
        h.text_submit("field1", "compat-value")
        cmds = h.run(1)

    texts = _text_values(cmds)
    assert any("last=compat-value" in t for t in texts), (
        f"Expected 'last=compat-value' in draw commands for compat polling path, got: {texts}"
    )
