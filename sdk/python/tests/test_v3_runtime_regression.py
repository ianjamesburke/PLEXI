"""Comprehensive regression tests for the v3 app runtime.

Tests the full protocol surface: every event type dispatched to apps, every
effect type emitted back to the host, state management, frame timing, and
all Core 9 apps booting correctly.

These tests serve as the safety net during the runtime refactor
(ProtocolTransport extraction + V3AppRuntime rewrite).
"""

import json
import os
import subprocess
import sys
import textwrap
import time
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]
SDK_PATH = str(REPO_ROOT / "sdk" / "python")


def _make_env():
    env = dict(os.environ)
    env["PYTHONPATH"] = SDK_PATH
    return env


def _spawn_v3_app(app_file: Path):
    """Spawn a v3 app and return the process."""
    proc = subprocess.Popen(
        [sys.executable, "-m", "plexi_sdk._v3_process", str(app_file)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=_make_env(),
    )
    return proc


def _init_app(proc, state=None, capabilities=None, protocol="pgap/3"):
    """Send init message and wait for ready."""
    msg = {
        "type": "init",
        "app_id": "regression-test",
        "workspace_root": "/tmp",
        "capabilities": capabilities or [],
        "feature_flags": [],
        "width": 640.0,
        "height": 360.0,
    }
    if protocol is not None:
        msg["protocol"] = protocol
    if state is not None:
        msg["state"] = state
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()
    return _collect_until(proc, "ready")


def _init_and_render(proc, state=None, frame_id=1):
    """Init + first render. Returns all events from both phases.

    Effects from init() are emitted AFTER ready (on_init runs post-handshake),
    so they appear in the first render's event stream.
    """
    init_events = _init_app(proc, state=state)
    render_events = _render(proc, frame_id=frame_id)
    return init_events + render_events


def _render(proc, frame_id=1, timer_ids=None):
    """Send render and collect until frame_done."""
    event = {
        "type": "render",
        "frame_id": frame_id,
        "rect": {"x": 0.0, "y": 0.0, "w": 640.0, "h": 360.0},
    }
    if timer_ids is not None:
        event["timer_ids"] = timer_ids
    proc.stdin.write(json.dumps(event) + "\n")
    proc.stdin.flush()
    return _collect_until(proc, "frame_done")


def _send_event(proc, event: dict):
    """Send an arbitrary event to the app."""
    proc.stdin.write(json.dumps(event) + "\n")
    proc.stdin.flush()


def _collect_until(proc, target_type: str, timeout: float = 3.0) -> list[dict]:
    """Read events until we see target_type or timeout."""
    seen = []
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        line = proc.stdout.readline()
        if not line:
            break
        ev = json.loads(line)
        seen.append(ev)
        if ev.get("type") == target_type:
            return seen
    return seen


def _find_events(events: list[dict], event_type: str) -> list[dict]:
    return [e for e in events if e.get("type") == event_type]


def test_protocol_output_flushes_one_batch_per_input_event(monkeypatch):
    from plexi_sdk import _v3_runtime as runtime

    class Output:
        def __init__(self):
            self.writes: list[str] = []
            self.flushes = 0

        def write(self, value: str) -> None:
            self.writes.append(value)

        def flush(self) -> None:
            self.flushes += 1

    output = Output()
    monkeypatch.setattr(runtime.sys, "stdout", output)

    runtime._begin_emit_batch()
    runtime._emit({"type": "component_tree"})
    runtime._emit({"type": "frame_done", "frame_id": 7})
    runtime._finish_emit_batch()

    assert output.flushes == 1
    assert len(output.writes) == 1
    assert output.writes[0].count("\n") == 2


# =============================================================================
# Effect tests: verify each effect type emits the correct protocol message
# =============================================================================


class TestEffects:
    """Each effect returned from init/update should emit the right protocol message."""

    def _app_with_init_effects(self, tmp_path: Path, effects_code: str) -> Path:
        app = tmp_path / "app.py"
        app.write_text(textwrap.dedent(f"""
            from plexi_sdk import state
            from plexi_sdk.effects import *
            from plexi_sdk.events import *
            from plexi_sdk.ui import Text

            def init(size, args):
                return [{effects_code}]

            def update(event):
                return []

            def view():
                return Text("ok")
        """).lstrip())
        return app

    def test_set_scheduler_mode(self, tmp_path):
        app = self._app_with_init_effects(tmp_path, 'SetSchedulerMode("continuous", fps=60)')
        proc = _spawn_v3_app(app)
        try:
            events = _init_and_render(proc)
            modes = _find_events(events, "set_scheduler_mode")
            assert len(modes) == 1
            assert modes[0]["mode"] == "continuous"
            assert modes[0]["fps"] == 60
        finally:
            proc.kill()

    def test_set_status(self, tmp_path):
        app = self._app_with_init_effects(tmp_path, 'SetStatus("hello world")')
        proc = _spawn_v3_app(app)
        try:
            events = _init_and_render(proc)
            statuses = _find_events(events, "status_summary")
            assert any(s.get("text") == "hello world" for s in statuses)
        finally:
            proc.kill()

    def test_set_timer(self, tmp_path):
        app = self._app_with_init_effects(tmp_path, 'SetTimer(id=42, delay_ms=1000, repeat=True)')
        proc = _spawn_v3_app(app)
        try:
            events = _init_and_render(proc)
            timers = _find_events(events, "set_timer")
            assert any(
                t.get("timer_id") == "42"
                and t.get("after_ms") == 1000
                and t.get("repeat") is True
                for t in timers
            )
        finally:
            proc.kill()

    def test_set_state_no_persist(self, tmp_path):
        app = self._app_with_init_effects(tmp_path, 'SetState({"count": 5})')
        proc = _spawn_v3_app(app)
        try:
            events = _init_and_render(proc)
            assert not _find_events(events, "save_app_state")
        finally:
            proc.kill()

    def test_persist_state(self, tmp_path):
        app = self._app_with_init_effects(tmp_path, 'PersistState({"count": 99})')
        proc = _spawn_v3_app(app)
        try:
            events = _init_and_render(proc)
            saves = _find_events(events, "save_app_state")
            assert len(saves) == 1
            assert saves[0]["payload"]["count"] == 99
        finally:
            proc.kill()

    def test_http_fetch(self, tmp_path):
        app = self._app_with_init_effects(
            tmp_path,
            'HttpFetch(url="https://example.com/api", method="GET")'
        )
        proc = _spawn_v3_app(app)
        try:
            events = _init_and_render(proc)
            reqs = _find_events(events, "http_request")
            assert any(r.get("url") == "https://example.com/api" for r in reqs)
        finally:
            proc.kill()

    def test_request_capability(self, tmp_path):
        app = self._app_with_init_effects(tmp_path, 'RequestCapability(name="net.http")')
        proc = _spawn_v3_app(app)
        try:
            events = _init_and_render(proc)
            reqs = _find_events(events, "capability_request")
            assert any(r.get("capability") == "net.http" for r in reqs)
        finally:
            proc.kill()

    def test_close_self(self, tmp_path):
        app = self._app_with_init_effects(tmp_path, 'CloseSelf()')
        proc = _spawn_v3_app(app)
        try:
            events = _init_and_render(proc)
            assert _find_events(events, "close_self")
        finally:
            proc.kill()


# =============================================================================
# Event dispatch tests: verify each event type reaches update() correctly
# =============================================================================


class TestEventDispatch:
    """Each protocol event should be dispatched as the right typed event."""

    def _app_that_echoes_event_type(self, tmp_path: Path) -> Path:
        app = tmp_path / "echo_app.py"
        app.write_text(textwrap.dedent("""
            from plexi_sdk import state
            from plexi_sdk.effects import SetState
            from plexi_sdk.events import *
            from plexi_sdk.ui import Text

            _seen = []

            def init(size, args):
                return []

            def update(event):
                if not isinstance(event, RenderFrame):
                    _seen.append(type(event).__name__)
                return []

            def view():
                return Text(f"events={','.join(_seen)}")
        """).lstrip())
        return app

    def test_key_event(self, tmp_path):
        app = self._app_that_echoes_event_type(tmp_path)
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            _send_event(proc, {"type": "key", "key": "a", "modifiers": {}})
            events = _render(proc)
            trees = _find_events(events, "component_tree")
            assert any("events=KeyEvent" in json.dumps(t) for t in trees)
        finally:
            proc.kill()

    def test_timer_event(self, tmp_path):
        app = self._app_that_echoes_event_type(tmp_path)
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            _send_event(proc, {"type": "timer", "timer_id": "1"})
            events = _render(proc)
            trees = _find_events(events, "component_tree")
            assert any("TimerFired" in json.dumps(t) for t in trees), f"Expected TimerFired in {[json.dumps(t)[:100] for t in trees]}"
        finally:
            proc.kill()

    def test_render_frame_event(self, tmp_path):
        """RenderFrame is dispatched during view() for continuous apps."""
        app = tmp_path / "render_frame_app.py"
        app.write_text(textwrap.dedent("""
            from plexi_sdk import state
            from plexi_sdk.effects import SetSchedulerMode, SetState
            from plexi_sdk.events import RenderFrame
            from plexi_sdk.ui import Text

            _got_render_frame = False

            def init(size, args):
                return [SetSchedulerMode("continuous", fps=60)]

            def update(event):
                global _got_render_frame
                if isinstance(event, RenderFrame):
                    _got_render_frame = True
                return []

            def view():
                return Text(f"got_rf={_got_render_frame}")
        """).lstrip())
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            events = _render(proc)
            trees = _find_events(events, "component_tree")
            assert any("got_rf=True" in json.dumps(t) for t in trees)
        finally:
            proc.kill()

    def test_ui_action_event(self, tmp_path):
        app = self._app_that_echoes_event_type(tmp_path)
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            _send_event(proc, {"type": "ui_action", "handler_id": "btn_1"})
            events = _render(proc)
            trees = _find_events(events, "component_tree")
            assert any("UiAction" in json.dumps(t) for t in trees), f"Expected UiAction in {[json.dumps(t)[:100] for t in trees]}"
        finally:
            proc.kill()

    def test_ui_value_change_event(self, tmp_path):
        app = self._app_that_echoes_event_type(tmp_path)
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            _send_event(proc, {"type": "text_submitted", "id": "input_1", "value": "hello"})
            events = _render(proc)
            trees = _find_events(events, "component_tree")
            assert any("UiValueChange" in json.dumps(t) for t in trees), f"Expected UiValueChange in {[json.dumps(t)[:100] for t in trees]}"
        finally:
            proc.kill()

    def test_focus_changed_event(self, tmp_path):
        app = self._app_that_echoes_event_type(tmp_path)
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            _send_event(proc, {
                "type": "focus_changed",
                "timestamp": "2026-01-01T00:00:00Z",
                "duration_secs": 0,
                "reason": "focus_changed",
            })
            events = _render(proc)
            trees = _find_events(events, "component_tree")
            assert any("FocusChanged" in json.dumps(t) for t in trees), f"Expected FocusChanged in {[json.dumps(t)[:100] for t in trees]}"
        finally:
            proc.kill()

    def test_capability_granted_event(self, tmp_path):
        app = self._app_that_echoes_event_type(tmp_path)
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            _send_event(proc, {"type": "capability_granted", "name": "net.http"})
            events = _render(proc)
            trees = _find_events(events, "component_tree")
            assert any("CapabilityGranted" in json.dumps(t) for t in trees), f"Expected CapabilityGranted in {[json.dumps(t)[:100] for t in trees]}"
        finally:
            proc.kill()


# =============================================================================
# State management tests
# =============================================================================


class TestState:
    """State seeding, access, and mutation through the runtime."""

    def test_host_seeded_state(self, tmp_path):
        """State provided in init message is accessible via state.get()."""
        app = tmp_path / "state_app.py"
        app.write_text(textwrap.dedent("""
            from plexi_sdk import state
            from plexi_sdk.effects import SetState
            from plexi_sdk.ui import Text

            def init(size, args):
                return []

            def update(event):
                return []

            def view():
                return Text(f"val={state.get('seed_key', 'MISSING')}")
        """).lstrip())
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc, state={"seed_key": "hello"})
            events = _render(proc)
            trees = _find_events(events, "component_tree")
            assert any("val=hello" in json.dumps(t) for t in trees)
        finally:
            proc.kill()

    def test_init_receives_host_viewport_size(self, tmp_path):
        app = tmp_path / "size_app.py"
        app.write_text(textwrap.dedent("""
            from plexi_sdk.effects import SetState
            from plexi_sdk import state
            from plexi_sdk.ui import Text

            def init(size, args):
                return [SetState({"size": list(size)})]

            def update(event):
                return []

            def view():
                return Text(f"size={state.get('size')}")
        """).lstrip())
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            events = _render(proc)
            assert "size=[640.0, 360.0]" in json.dumps(events)
        finally:
            proc.kill()

    def test_set_state_visible_in_view(self, tmp_path):
        """SetState in init makes data visible in view() via state.get()."""
        app = tmp_path / "state_app.py"
        app.write_text(textwrap.dedent("""
            from plexi_sdk import state
            from plexi_sdk.effects import SetState
            from plexi_sdk.ui import Text

            def init(size, args):
                return [SetState({"counter": 42})]

            def update(event):
                return []

            def view():
                return Text(f"c={state.get('counter', 0)}")
        """).lstrip())
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            events = _render(proc)
            trees = _find_events(events, "component_tree")
            assert any("c=42" in json.dumps(t) for t in trees)
        finally:
            proc.kill()

    def test_state_mutation_across_renders(self, tmp_path):
        """State mutated in update() persists across render cycles."""
        app = tmp_path / "incr_app.py"
        app.write_text(textwrap.dedent("""
            from plexi_sdk import state
            from plexi_sdk.effects import SetState
            from plexi_sdk.events import KeyEvent
            from plexi_sdk.ui import Text

            def init(size, args):
                return [SetState({"n": 0})]

            def update(event):
                if isinstance(event, KeyEvent):
                    return [SetState({"n": state.get("n", 0) + 1})]
                return []

            def view():
                return Text(f"n={state.get('n', 0)}")
        """).lstrip())
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            _render(proc, frame_id=1)
            _send_event(proc, {"type": "key", "key": "x", "modifiers": {}})
            _send_event(proc, {"type": "key", "key": "x", "modifiers": {}})
            _send_event(proc, {"type": "key", "key": "x", "modifiers": {}})
            events = _render(proc, frame_id=2)
            trees = _find_events(events, "component_tree")
            assert any("n=3" in json.dumps(t) for t in trees)
        finally:
            proc.kill()


class TestWireFormat:
    def _text_app(self, tmp_path: Path) -> Path:
        app = tmp_path / "wire_app.py"
        app.write_text(textwrap.dedent("""
            from plexi_sdk.ui import Text

            def init(size, args):
                return []

            def update(event):
                return []

            def view():
                return Text("wire-format")
        """).lstrip())
        return app

    def test_pgap_protocol_emits_render_command_component_tree(self, tmp_path):
        proc = _spawn_v3_app(self._text_app(tmp_path))
        try:
            _init_app(proc)
            events = _render(proc)
            tree = _find_events(events, "component_tree")[0]
            assert "root" in tree and "tree" not in tree
            assert tree["frame_id"] == 1
        finally:
            proc.kill()

    def test_wasm_protocol_emits_indexed_component_tree(self, tmp_path):
        proc = _spawn_v3_app(self._text_app(tmp_path))
        try:
            _init_app(proc, protocol=None)
            events = _render(proc)
            tree = _find_events(events, "component_tree")[0]
            assert "tree" in tree and "root" not in tree
            assert tree["frame_id"] == 1
        finally:
            proc.kill()


# =============================================================================
# Frame timing tests (the bug category that prompted this refactor)
# =============================================================================


class TestFrameTiming:
    """Verify frame elapsed time is computed correctly."""

    def test_elapsed_is_nonzero_between_renders(self, tmp_path):
        """Elapsed time between two consecutive renders must be > 0."""
        app = tmp_path / "timing_app.py"
        app.write_text(textwrap.dedent("""
            from plexi_sdk import state
            from plexi_sdk.effects import SetSchedulerMode, SetState
            from plexi_sdk.events import RenderFrame
            from plexi_sdk.ui import Text

            _elapsed_values = []

            def init(size, args):
                return [SetSchedulerMode("continuous", fps=60)]

            def update(event):
                if isinstance(event, RenderFrame):
                    _elapsed_values.append(event.elapsed)
                return []

            def view():
                return Text(f"e={_elapsed_values}")
        """).lstrip())
        proc = _spawn_v3_app(app)
        try:
            _init_app(proc)
            _render(proc, frame_id=1)
            time.sleep(0.05)
            events = _render(proc, frame_id=2)
            trees = _find_events(events, "component_tree")
            tree_text = json.dumps(trees)
            # Second render must have elapsed > 0.01 (we slept 50ms)
            # Parse the elapsed list from the component tree text
            import re
            match = re.search(r"e=\[(.*?)\]", tree_text)
            assert match, f"Could not find elapsed values in {tree_text[:200]}"
            values = [float(v.strip()) for v in match.group(1).split(",") if v.strip()]
            assert len(values) >= 2, f"Expected at least 2 elapsed values, got {values}"
            assert values[1] > 0.01, (
                f"Second frame elapsed should be >10ms (slept 50ms), got {values[1]:.6f}s. "
                "This is the exact bug we fixed: base class clobbering _last_render_time."
            )
        finally:
            proc.kill()

    def test_balls_physics_progresses(self, tmp_path):
        """The balls app physics must actually advance between frames."""
        from plexi_sdk.testing import AppHarness
        balls_path = REPO_ROOT / "apps" / "dev" / "balls" / "balls.py"
        with AppHarness(balls_path, timeout=3.0) as h:
            cmds1 = h.run(1)
            time.sleep(0.05)
            cmds2 = h.run(1)

        def extract_ticks(cmds):
            for cmd in cmds:
                if cmd.get("type") == "component_tree":
                    text = json.dumps(cmd)
                    import re
                    m = re.search(r"ticks (\d+)", text)
                    if m:
                        return int(m.group(1))
            return None

        t1 = extract_ticks(cmds1)
        t2 = extract_ticks(cmds2)
        assert t1 is not None and t2 is not None, f"Could not find tick count: {t1}, {t2}"
        assert t2 > t1, (
            f"Balls ticks did not advance between frames: {t1} -> {t2}. "
            "Physics is frozen (elapsed ≈ 0 bug)."
        )


# =============================================================================
# Timer repeat behavior
# =============================================================================


class TestTimers:
    """Timer effects and repeat semantics."""

    def test_repeating_timer_is_owned_by_host_after_initial_registration(self, tmp_path):
        """The guest must not add round-trip drift by re-arming repeating timers."""
        app = tmp_path / "timer_app.py"
        app.write_text(textwrap.dedent("""
            from plexi_sdk import state
            from plexi_sdk.effects import SetState, SetTimer
            from plexi_sdk.events import TimerFired
            from plexi_sdk.ui import Text

            def init(size, args):
                return [SetTimer(id=1, delay_ms=500, repeat=True), SetState({"fires": 0})]

            def update(event):
                if isinstance(event, TimerFired):
                    return [SetState({"fires": state.get("fires", 0) + 1})]
                return []

            def view():
                return Text(f"fires={state.get('fires', 0)}")
        """).lstrip())
        proc = _spawn_v3_app(app)
        try:
            all_events = _init_and_render(proc, frame_id=1)
            timers = _find_events(all_events, "set_timer")
            assert any(t.get("timer_id") == "1" for t in timers)

            # Fire the timer
            _send_event(proc, {"type": "timer", "timer_id": "1"})
            events = _render(proc, frame_id=2)
            # The host owns the fixed cadence after the initial registration.
            rearms = _find_events(events, "set_timer")
            assert not rearms
            assert len(_find_events(events, "schedule_render")) == 1
        finally:
            proc.kill()

        wasm_proc = _spawn_v3_app(app)
        try:
            _init_app(wasm_proc, protocol=None)
            _render(wasm_proc, frame_id=1)
            events = _render(wasm_proc, frame_id=2, timer_ids=["1"])
            assert not _find_events(events, "schedule_render")
            assert "fires=1" in json.dumps(events)
        finally:
            wasm_proc.kill()


# =============================================================================
# Core 9 app integration tests
# =============================================================================


@pytest.mark.parametrize("relative_path", [
    "apps/balls/balls.py",
    "apps/snake/snake.py",
    "apps/tetris/tetris.py",
    "apps/chess/chess.py",
    "apps/calc/calc.py",
    "apps/csv_viewer/csv_viewer.py",
    "apps/todo/todo.py",
    "apps/stats/stats.py",
    "apps/logs/logs.py",
    "apps/kraken/main.py",
    "apps/github-issues/main.py",
    "apps/permissions/main.py",
    "apps/wikipedia/wikipedia.py",
])
def test_core_app_boots_and_renders(relative_path: str) -> None:
    """Every Core app must satisfy Init -> Ready -> Render -> FrameDone."""
    from plexi_sdk.testing import AppHarness
    app_path = REPO_ROOT / relative_path
    if not app_path.exists():
        pytest.skip(f"{relative_path} not found")
    with AppHarness(app_path, timeout=3.0) as h:
        cmds = h.run(1)
    assert cmds, f"{relative_path} should emit draw/control commands"
    trees = [c for c in cmds if c.get("type") == "component_tree"]
    assert trees, f"{relative_path} should render a component tree"


@pytest.mark.parametrize("relative_path,expected_effect", [
    ("apps/balls/balls.py", "set_scheduler_mode"),
    ("apps/snake/snake.py", "set_timer"),
    ("apps/tetris/tetris.py", "set_scheduler_mode"),
    ("apps/kraken/main.py", "set_timer"),
    ("apps/logs/logs.py", "set_timer"),
    ("apps/stats/stats.py", "set_timer"),
])
def test_core_app_emits_expected_effect(relative_path: str, expected_effect: str) -> None:
    """Apps that need timers/schedulers must emit them during boot."""
    from plexi_sdk.testing import AppHarness
    app_path = REPO_ROOT / relative_path
    if not app_path.exists():
        pytest.skip(f"{relative_path} not found")
    with AppHarness(app_path, timeout=3.0) as h:
        cmds = h.run(1)
    assert any(c.get("type") == expected_effect for c in cmds), (
        f"{relative_path} should emit {expected_effect}"
    )


@pytest.mark.parametrize("relative_path,key", [
    ("apps/calc/calc.py", "1"),
    ("apps/tetris/tetris.py", "left"),
])
def test_core_app_responds_to_key(relative_path: str, key: str) -> None:
    """Interactive apps must change their rendered output on key press."""
    from plexi_sdk.testing import AppHarness
    app_path = REPO_ROOT / relative_path
    if not app_path.exists():
        pytest.skip(f"{relative_path} not found")
    with AppHarness(app_path, timeout=3.0) as h:
        cmds_before = h.run(1)
        h.key(key)
        cmds_after = h.run(1)

    tree_before = json.dumps([c for c in cmds_before if c.get("type") == "component_tree"])
    tree_after = json.dumps([c for c in cmds_after if c.get("type") == "component_tree"])
    assert tree_before != tree_after, (
        f"{relative_path} component tree should change after key '{key}'"
    )
