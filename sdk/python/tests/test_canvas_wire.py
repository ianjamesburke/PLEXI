"""Canvas wire payload policy (stint 0550).

Canvas commands are JSON-encoded and written to stdout on every frame, so both
the coordinate precision and the JSON separators are per-frame wire cost paid by
every app. Both reductions are schema-preserving: floats stay floats, no key
changes, and the host decoder is untouched.
"""

import json
import subprocess
import sys
import textwrap
from pathlib import Path

from plexi_sdk._v3_runtime import _dump_line
from plexi_sdk.ui import (
    CANVAS_WIRE_DECIMALS,
    Canvas,
    CanvasCircle,
    CanvasLine,
    CanvasRect,
    CanvasText,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
SDK_PATH = str(REPO_ROOT / "sdk" / "python")

RAW = 1134.578349092061
ROUNDED = 1134.58


class TestCoordinatePrecision:
    def test_policy_is_two_decimals(self):
        assert CANVAS_WIRE_DECIMALS == 2

    def test_rect_geometry_is_rounded(self):
        cmd = CanvasRect(RAW, RAW, RAW, RAW, "#ffffff", radius=RAW).to_command()
        assert cmd["x"] == cmd["y"] == cmd["w"] == cmd["h"] == ROUNDED
        assert cmd["radius"] == ROUNDED
        assert cmd["fill"] == "#ffffff"

    def test_circle_geometry_is_rounded(self):
        cmd = CanvasCircle(RAW, RAW, RAW, "#ffffff").to_command()
        assert cmd["cx"] == cmd["cy"] == cmd["r"] == ROUNDED

    def test_line_geometry_is_rounded(self):
        cmd = CanvasLine(RAW, RAW, RAW, RAW, "#ffffff", width=RAW).to_command()
        assert cmd["x1"] == cmd["y1"] == cmd["x2"] == cmd["y2"] == ROUNDED
        assert cmd["width"] == ROUNDED

    def test_text_geometry_is_rounded_and_text_is_untouched(self):
        cmd = CanvasText(RAW, RAW, "hello", size=RAW).to_command()
        assert cmd["x"] == cmd["y"] == cmd["size"] == ROUNDED
        assert cmd["text"] == "hello"

    def test_geometry_stays_float_for_integer_input(self):
        # The host decodes these as floats; an int on the wire would be a
        # schema change even though JSON allows it.
        cmd = CanvasCircle(5, 5, 5, "#ffffff").to_command()
        assert all(isinstance(cmd[k], float) for k in ("cx", "cy", "r"))

    def test_rounding_is_below_one_device_pixel_at_realistic_scales(self):
        # The worst case in the maintained set is a `fit="contain"` canvas with
        # a fixed logical space magnified onto a large pane. Even at 10x — well
        # beyond any real pane/canvas ratio — half a rounding step stays far
        # below one device pixel on a 2x display.
        worst_case_scale = 10.0
        device_pixel_ratio = 2.0
        step = 10 ** -CANVAS_WIRE_DECIMALS
        assert (step / 2) * worst_case_scale * device_pixel_ratio < 0.25


class TestCompactEncoding:
    def test_emit_uses_compact_separators(self):
        line = _dump_line({"type": "frame_done", "frame_id": 1})
        assert line == '{"type":"frame_done","frame_id":1}\n'

    def test_payload_shrinks_but_decodes_identically(self):
        commands = [
            CanvasCircle(i * 7.1234567, i * 3.7654321, 12.5, "#0000003c")
            for i in range(50)
        ]
        node = Canvas(commands, width=640.0, height=360.0).to_node()
        compact = _dump_line(node)
        verbose = json.dumps(
            {
                "type": "canvas",
                "width": 640.0,
                "height": 360.0,
                "grow": True,
                "fit": "fill",
                "commands": [
                    {
                        "type": "circle",
                        "cx": i * 7.1234567,
                        "cy": i * 3.7654321,
                        "r": 12.5,
                        "fill": "#0000003c",
                    }
                    for i in range(50)
                ],
            }
        )
        assert len(compact) < len(verbose) * 0.8
        # Same shape, same keys — only the digits and the whitespace differ.
        decoded = json.loads(compact)
        assert decoded.keys() == json.loads(verbose).keys()
        assert len(decoded["commands"]) == 50
        assert decoded["commands"][7]["cx"] == round(7 * 7.1234567, 2)


APP = textwrap.dedent("""
    from plexi_sdk.ui import Canvas, CanvasCircle

    def init(size, args):
        return []

    def update(event):
        return []

    def view():
        return Canvas(
            [CanvasCircle(1134.578349092061, 172.9363749889064, 16.1097005, "#fff")],
            width=640, height=360,
        )
""").lstrip()


def test_emitted_wire_line_carries_rounded_compact_json(tmp_path):
    """End to end through the real runtime, not just the encoder."""
    import os

    app = tmp_path / "a.py"
    app.write_text(APP)
    env = dict(os.environ)
    env["PYTHONPATH"] = SDK_PATH
    proc = subprocess.Popen(
        [sys.executable, "-m", "plexi_sdk._v3_process", str(app)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )
    try:
        proc.stdin.write(json.dumps({
            "type": "init",
            "app_id": "wire-test",
            "workspace_root": "/tmp",
            "capabilities": [],
            "feature_flags": [],
            "width": 640.0,
            "height": 360.0,
        }) + "\n")
        proc.stdin.flush()
        while json.loads(proc.stdout.readline()).get("type") != "ready":
            pass
        proc.stdin.write(json.dumps({
            "type": "render",
            "frame_id": 1,
            "rect": {"x": 0.0, "y": 0.0, "w": 640.0, "h": 360.0},
        }) + "\n")
        proc.stdin.flush()
        while True:
            line = proc.stdout.readline()
            assert line, "runtime closed before emitting the frame"
            if json.loads(line).get("type") == "component_tree":
                break
        assert '", "' not in line and '": ' not in line
        assert "1134.58" in line
        assert "1134.578" not in line
    finally:
        proc.kill()
