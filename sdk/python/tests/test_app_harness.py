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


def test_sdk_v3_process_set_state_is_runtime_only(tmp_path: Path) -> None:
    """SDK v3 SetState updates runtime state without implicit persistence."""
    app_file = tmp_path / "v3_state_app.py"
    app_file.write_text(textwrap.dedent("""
        from plexi_sdk import state
        from plexi_sdk.effects import SetState
        from plexi_sdk.ui import Text

        def init(size, args):
            return [SetState({"count": state.get("count", 0) + 1})]

        def update(event):
            return []

        def view():
            return Text(str(state.get("count", 0)))
    """).lstrip())

    repo_root = Path(__file__).resolve().parents[3]
    env = dict(os.environ)
    env["PYTHONPATH"] = str(repo_root / "sdk/python")
    proc = subprocess.Popen(
        [sys.executable, "-m", "plexi_sdk._v3_process", str(app_file)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )
    assert proc.stdin is not None
    assert proc.stdout is not None
    try:
        proc.stdin.write(json.dumps({
            "type": "init",
            "protocol": "pgap/3",
            "app_id": "v3-state-test",
            "workspace_root": "/tmp",
            "capabilities": [],
            "feature_flags": [],
            "state": {"count": 41},
        }) + "\n")
        proc.stdin.flush()

        seen = []
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline:
            line = proc.stdout.readline()
            if not line:
                break
            ev = json.loads(line)
            seen.append(ev)
            if ev.get("type") == "ready":
                break
        proc.stdin.write(json.dumps({
            "type": "render",
            "frame_id": 1,
            "rect": {"x": 0.0, "y": 0.0, "w": 200.0, "h": 100.0},
        }) + "\n")
        proc.stdin.flush()
        while time.monotonic() < deadline:
            line = proc.stdout.readline()
            if not line:
                break
            ev = json.loads(line)
            seen.append(ev)
            if ev.get("type") == "frame_done":
                assert not any(item.get("type") == "save_app_state" for item in seen)
                assert any(
                    item.get("type") == "component_tree"
                    and item["root"].get("text") == "42"
                    for item in seen
                )
                return
    finally:
        proc.kill()
    pytest.fail(f"expected runtime SetState render without save_app_state, saw {seen}")


def test_sdk_v3_process_persist_state_saves_host_state(tmp_path: Path) -> None:
    """PersistState is the explicit durable app-state effect."""
    app_file = tmp_path / "v3_persist_app.py"
    app_file.write_text(textwrap.dedent("""
        from plexi_sdk import state
        from plexi_sdk.effects import PersistState
        from plexi_sdk.ui import Text

        def init(size, args):
            return [PersistState({"count": state.get("count", 0) + 1})]

        def update(event):
            return []

        def view():
            return Text(str(state.get("count", 0)))
    """).lstrip())

    repo_root = Path(__file__).resolve().parents[3]
    env = dict(os.environ)
    env["PYTHONPATH"] = str(repo_root / "sdk/python")
    proc = subprocess.Popen(
        [sys.executable, "-m", "plexi_sdk._v3_process", str(app_file)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )
    assert proc.stdin is not None
    assert proc.stdout is not None
    try:
        proc.stdin.write(json.dumps({
            "type": "init",
            "protocol": "pgap/3",
            "app_id": "v3-persist-test",
            "workspace_root": "/tmp",
            "capabilities": [],
            "feature_flags": [],
            "state": {"count": 41},
        }) + "\n")
        proc.stdin.flush()

        seen = []
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline:
            line = proc.stdout.readline()
            if not line:
                break
            ev = json.loads(line)
            seen.append(ev)
            if ev.get("type") == "save_app_state":
                assert ev["payload"]["count"] == 42
                return
    finally:
        proc.kill()
    pytest.fail(f"expected save_app_state with seeded count, saw {seen}")


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
        h._send({"type": "timer", "timer_id": "1"})
        timer_cmds = h.run(1)
    assert cmds, f"{relative_path} should emit draw/control commands for first frame"
    assert not any(cmd.get("type") == "save_app_state" for cmd in timer_cmds)
    trees = [cmd for cmd in cmds if cmd.get("type") == "component_tree"]
    assert trees, f"{relative_path} should render through the SDK v3 component tree"
    assert _contains_node_type(trees[0]["root"], "canvas")


def _contains_node_type(node: dict, node_type: str) -> bool:
    if node.get("type") == node_type:
        return True
    for child in node.get("children", []):
        if _contains_node_type(child, node_type):
            return True
    child = node.get("child")
    return isinstance(child, dict) and _contains_node_type(child, node_type)
