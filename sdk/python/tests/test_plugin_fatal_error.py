"""A plugin exception reaches the host as one structured `fatal_error` message.

Stint 1328: the runtime used to die with a raw CPython traceback on stderr, so
every surface that reported it led with "plugin exited before responding" and
left the plugin's own exception at the bottom, with guest mount paths. These
tests pin the structured message for the five misuse probes the plugin gauntlet
ran, plus import failures.
"""

import json
import os
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]
SDK_PATH = str(REPO_ROOT / "sdk" / "python")

_GOOD_INIT = "def init(size, args):\n    return []\n"
_GOOD_UPDATE = "def update(event):\n    return []\n"
_GOOD_VIEW = "def view():\n    return Text('ok')\n"
_IMPORTS = (
    "from plexi_sdk import state\n"
    "from plexi_sdk.effects import SetTitle\n"
    "from plexi_sdk.ui import Badge, Column, Text\n"
)


def _run_plugin(tmp_path: Path, source: str, extra_files: dict[str, str] | None = None):
    """Boot the plugin, render once, and return (messages, stderr, returncode)."""
    (tmp_path / "main.py").write_text(source)
    for relative, body in (extra_files or {}).items():
        target = tmp_path / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(body)
    env = dict(os.environ)
    env["PYTHONPATH"] = SDK_PATH
    init = {
        "type": "init",
        "plugin_id": "fatal-error-test",
        "workspace_root": str(tmp_path),
        "capabilities": [],
        "feature_flags": [],
        "protocol": "pgap/4",
        "width": 320.0,
        "height": 240.0,
    }
    render = {
        "type": "render",
        "frame_id": 1,
        "rect": {"x": 0.0, "y": 0.0, "w": 320.0, "h": 240.0},
    }
    proc = subprocess.run(
        [sys.executable, "-m", "plexi_sdk._v3_process", str(tmp_path / "main.py")],
        input=json.dumps(init) + "\n" + json.dumps(render) + "\n",
        capture_output=True,
        text=True,
        env=env,
        timeout=30,
    )
    messages = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
    return messages, proc.stderr, proc.returncode


def _fatal(messages: list[dict]) -> dict:
    fatals = [m for m in messages if m.get("type") == "fatal_error"]
    assert len(fatals) == 1, f"expected exactly one fatal_error, got {messages!r}"
    return fatals[0]


def _line_of(source: str, needle: str) -> int:
    for number, line in enumerate(source.splitlines(), start=1):
        if needle in line:
            return number
    raise AssertionError(f"{needle!r} not in fixture source")


def test_state_set_inside_view_names_the_plugin_line(tmp_path):
    source = _IMPORTS + _GOOD_INIT + _GOOD_UPDATE + textwrap.dedent(
        """\
        def view():
            state.set("last", "x")
            return Text("ok")
        """
    )
    messages, stderr, returncode = _run_plugin(tmp_path, source)
    fatal = _fatal(messages)
    assert fatal["phase"] == "view"
    assert fatal["file"] == "main.py"
    assert fatal["line"] == _line_of(source, 'state.set("last", "x")')
    assert fatal["exception"] == "RuntimeError"
    assert fatal["message"] == (
        "state.set() called inside view() - return SetState from update() instead"
    )
    assert returncode != 0
    assert "Traceback" not in stderr, "the traceback rides the structured message, not stderr"


def test_bad_badge_color_names_the_innermost_plugin_frame(tmp_path):
    source = _IMPORTS + _GOOD_INIT + _GOOD_UPDATE + textwrap.dedent(
        """\
        from plugin import ui as plugin_ui

        def view():
            return plugin_ui.build_view()
        """
    )
    helper = textwrap.dedent(
        """\
        from plexi_sdk.ui import Badge, Column

        def build_view():
            return Column(children=[
                Badge("hi", color="blue"),
            ])
        """
    )
    messages, _stderr, _rc = _run_plugin(
        tmp_path, source, {"plugin/__init__.py": "", "plugin/ui.py": helper}
    )
    fatal = _fatal(messages)
    assert fatal["phase"] == "view"
    assert fatal["file"] == "plugin/ui.py"
    assert fatal["line"] == _line_of(helper, 'Badge("hi", color="blue")')
    assert fatal["exception"] == "ValueError"
    assert fatal["message"].startswith("Badge color must be one of")
    assert "got 'blue'" in fatal["message"]


def test_view_returning_a_str_names_the_type_and_the_fix(tmp_path):
    source = _IMPORTS + _GOOD_INIT + _GOOD_UPDATE + textwrap.dedent(
        """\
        def view():
            return "hello"
        """
    )
    messages, _stderr, _rc = _run_plugin(tmp_path, source)
    fatal = _fatal(messages)
    assert fatal["phase"] == "view"
    assert fatal["file"] == "main.py"
    assert fatal["line"] == _line_of(source, "def view():")
    assert fatal["exception"] == "TypeError"
    assert "view() returned str" in fatal["message"]
    assert "plexi_sdk.ui component" in fatal["message"]
    assert "Unknown UINode type" not in fatal["message"]


def test_init_with_no_parameters_names_the_expected_signature(tmp_path):
    source = _IMPORTS + "def init():\n    return []\n" + _GOOD_UPDATE + _GOOD_VIEW
    messages, _stderr, _rc = _run_plugin(tmp_path, source)
    fatal = _fatal(messages)
    assert fatal["phase"] == "init"
    assert fatal["file"] == "main.py"
    assert fatal["line"] == _line_of(source, "def init():")
    assert fatal["exception"] == "TypeError"
    assert "def init(size, args)" in fatal["message"]
    assert "def init()" in fatal["message"]


@pytest.mark.parametrize(
    ("name", "bad_def", "expected"),
    [
        ("update", "def update():\n    return []\n", "def update(event)"),
        ("view", "def view(size):\n    return Text('ok')\n", "def view()"),
    ],
)
def test_wrong_update_or_view_arity_names_the_expected_signature(
    tmp_path, name, bad_def, expected
):
    parts = {"update": _GOOD_UPDATE, "view": _GOOD_VIEW}
    parts[name] = bad_def
    source = _IMPORTS + _GOOD_INIT + parts["update"] + parts["view"]
    messages, _stderr, _rc = _run_plugin(tmp_path, source)
    fatal = _fatal(messages)
    assert fatal["phase"] == name
    assert fatal["file"] == "main.py"
    assert fatal["exception"] == "TypeError"
    assert expected in fatal["message"]


def test_a_type_error_raised_inside_the_plugin_is_not_rewritten_as_arity(tmp_path):
    source = _IMPORTS + _GOOD_INIT + _GOOD_UPDATE + textwrap.dedent(
        """\
        def view():
            return len(5)
        """
    )
    messages, _stderr, _rc = _run_plugin(tmp_path, source)
    fatal = _fatal(messages)
    assert fatal["line"] == _line_of(source, "return len(5)")
    assert "has no len()" in fatal["message"]


def test_effect_returned_from_view_names_the_view(tmp_path):
    source = _IMPORTS + _GOOD_INIT + _GOOD_UPDATE + textwrap.dedent(
        """\
        def view():
            return SetTitle("nope")
        """
    )
    messages, _stderr, _rc = _run_plugin(tmp_path, source)
    fatal = _fatal(messages)
    assert fatal["phase"] == "view"
    assert fatal["file"] == "main.py"
    assert fatal["line"] == _line_of(source, "def view():")
    assert fatal["exception"] == "TypeError"
    assert fatal["message"].startswith("view() returned the effect SetTitle")


def test_import_failure_is_structured(tmp_path):
    source = "import nonexistent_module_for_test\n" + _GOOD_INIT + _GOOD_UPDATE + _GOOD_VIEW
    messages, _stderr, _rc = _run_plugin(tmp_path, source)
    fatal = _fatal(messages)
    assert fatal["phase"] == "import"
    assert fatal["file"] == "main.py"
    assert fatal["line"] == 1
    assert fatal["exception"] == "ModuleNotFoundError"


def test_syntax_error_names_the_offending_line(tmp_path):
    source = _GOOD_INIT + "def update(event)\n    return []\n" + _GOOD_VIEW
    messages, _stderr, _rc = _run_plugin(tmp_path, source)
    fatal = _fatal(messages)
    assert fatal["phase"] == "import"
    assert fatal["file"] == "main.py"
    assert fatal["line"] == _line_of(source, "def update(event)")
    assert fatal["exception"] == "SyntaxError"


def test_traceback_uses_plugin_relative_paths(tmp_path):
    source = _IMPORTS + _GOOD_INIT + _GOOD_UPDATE + textwrap.dedent(
        """\
        def view():
            raise KeyError("missing")
        """
    )
    messages, _stderr, _rc = _run_plugin(tmp_path, source)
    fatal = _fatal(messages)
    traceback_text = fatal["traceback"]
    assert 'File "main.py", line' in traceback_text
    assert str(tmp_path) not in traceback_text
    assert SDK_PATH not in traceback_text
    assert 'File "plexi_sdk/_v3_runtime.py"' in traceback_text
    assert traceback_text.rstrip().endswith("KeyError: 'missing'")
