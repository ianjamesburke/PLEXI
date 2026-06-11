"""Integration tests for AppHarness — headless Python app subprocess runner."""

import json
import os
import subprocess
import sys
import textwrap
import time
from pathlib import Path

import pytest

from plexi_sdk.testing import AppHarness

# Minimal test app — counts Enter key presses
_COUNTER_APP = textwrap.dedent("""
    from plexi_sdk import App, RenderContext

    class CounterApp(App):
        def on_init(self) -> None:
            self._count = 0

        def on_render(self, ctx: RenderContext) -> None:
            ctx.status_summary(f"count={self._count}")
            ctx.rect(0, 0, 100, 50, fill="#ff0000")
            ctx.text(10, 10, f"count={self._count}", size=14, color="#ffffff")

        def on_key(self, key: str, mods: dict) -> None:
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


def test_appharness_reports_sdk_fatal_errors(tmp_path: Path) -> None:
    """Hook contract mismatches surface as fatal_error instead of silent EOF."""
    bad_app = tmp_path / "bad_app.py"
    bad_app.write_text(textwrap.dedent("""
        from plexi_sdk import App

        class BadApp(App):
            def on_init(self, stale_ctx):
                pass

        BadApp().run()
    """).lstrip())

    repo_root = Path(__file__).resolve().parents[3]
    env = dict(os.environ)
    env["PYTHONPATH"] = str(repo_root / "sdk/python")
    proc = subprocess.Popen(
        [sys.executable, str(bad_app)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )
    assert proc.stdin is not None
    assert proc.stdout is not None
    proc.stdin.write(json.dumps({
        "type": "init",
        "protocol": "pgap/3",
        "app_id": "fatal-test",
        "workspace_root": "/tmp",
        "capabilities": [],
        "feature_flags": [],
    }) + "\n")
    proc.stdin.flush()

    deadline = time.monotonic() + 2.0
    seen = []
    try:
        while time.monotonic() < deadline:
            line = proc.stdout.readline()
            if not line:
                break
            ev = json.loads(line)
            seen.append(ev)
            if ev.get("type") == "fatal_error":
                assert "BadApp.on_init" in ev.get("traceback", "")
                return
    finally:
        proc.kill()
    pytest.fail(f"expected fatal_error, saw {seen}")


@pytest.mark.parametrize("relative_path", [
    "apps/balls/balls.py",
    "apps/snake/snake.py",
    "apps/tetris/tetris.py",
])
def test_core_game_apps_boot_and_render(relative_path: str) -> None:
    """Core game apps must satisfy Init -> Ready -> Render -> FrameDone."""
    repo_root = Path(__file__).resolve().parents[3]
    with AppHarness(repo_root / relative_path, timeout=2.0) as h:
        cmds = h.run(1)
    assert cmds, f"{relative_path} should emit draw/control commands for first frame"
