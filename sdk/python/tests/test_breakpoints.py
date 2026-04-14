"""Tests for the breakpoint dispatcher and auto min-size fallback.

Stdlib only, runnable via pytest OR as a standalone script.
"""

from __future__ import annotations

import io
import json
import os
import sys
import unittest

# Make `import plexi_sdk` work when invoked directly.
HERE = os.path.dirname(os.path.abspath(__file__))
SDK_DIR = os.path.abspath(os.path.join(HERE, ".."))
if SDK_DIR not in sys.path:
    sys.path.insert(0, SDK_DIR)

from plexi_sdk import App, RenderContext  # noqa: E402


class _ReconfigurableStringIO(io.StringIO):
    """StringIO that tolerates the sys.stdout.reconfigure(...) call the SDK
    makes at the top of App.run() — real sys.stdout has this method but
    io.StringIO doesn't, so we stub it out for tests."""

    def reconfigure(self, *args, **kwargs):
        pass


class BreakpointDispatchTests(unittest.TestCase):
    def test_breakpoint_dispatch_picks_largest_match(self):
        app = App(app_id="test")
        fired: list[str] = []

        @app.breakpoint(min_width=800, min_height=500)
        def render_full(ctx: RenderContext) -> None:
            fired.append("full")

        @app.breakpoint(min_width=400)
        def render_compact(ctx: RenderContext) -> None:
            fired.append("compact")

        @app.breakpoint()
        def render_fallback(ctx: RenderContext) -> None:
            fired.append("fallback")

        cases = [
            (1200.0, 800.0, "full"),
            (600.0, 600.0, "compact"),
            (200.0, 200.0, "fallback"),
        ]
        for w, h, expected in cases:
            fired.clear()
            fn = app._pick_breakpoint(w, h)
            self.assertIsNotNone(fn, f"no match at {w}x{h}")
            fn(RenderContext(w, h))
            self.assertEqual(fired, [expected], f"at {w}x{h}")

    def test_breakpoint_fallback_used_when_no_match(self):
        app = App(app_id="test")
        fired: list[str] = []

        @app.breakpoint(min_width=1000, min_height=800)
        def big_only(ctx: RenderContext) -> None:
            fired.append("big")

        @app.breakpoint()
        def fallback(ctx: RenderContext) -> None:
            fired.append("fallback")

        fn = app._pick_breakpoint(500.0, 400.0)
        self.assertIsNotNone(fn)
        fn(RenderContext(500.0, 400.0))
        self.assertEqual(fired, ["fallback"])

    def test_breakpoint_and_on_render_mutually_exclusive(self):
        app = App(app_id="test")

        @app.on_render
        def render(ctx: RenderContext) -> None:
            pass

        @app.breakpoint()
        def alt(ctx: RenderContext) -> None:
            pass

        # Drive run() with an empty stdin so validation fires then we exit cleanly.
        old_stdin = sys.stdin
        old_stdout = sys.stdout
        sys.stdin = io.StringIO("")
        sys.stdout = _ReconfigurableStringIO()
        try:
            with self.assertRaises(RuntimeError) as cm:
                app.run()
            self.assertIn("mutually exclusive", str(cm.exception))
        finally:
            sys.stdin = old_stdin
            sys.stdout = old_stdout

    def test_min_size_auto_fallback_renders_arrow(self):
        app = App(app_id="test")
        app.set_min_size(400, 200)

        rendered: list[bool] = []

        @app.on_render
        def render(ctx: RenderContext) -> None:
            rendered.append(True)

        # Capture stdout to read emitted draw commands.
        old_stdout = sys.stdout
        old_stdin = sys.stdin
        captured = _ReconfigurableStringIO()
        sys.stdout = captured
        # Send one render at a too-small size, then shutdown.
        events = (
            '{"type":"render","width":320,"height":180,"delta_time":0}\n'
            '{"type":"shutdown"}\n'
        )
        sys.stdin = io.StringIO(events)
        try:
            app.run()
        finally:
            sys.stdout = old_stdout
            sys.stdin = old_stdin

        # User render fn must NOT have been called at too-small.
        self.assertEqual(rendered, [])

        lines = [l for l in captured.getvalue().splitlines() if l.strip()]
        cmds = [json.loads(l) for l in lines]
        types = [c.get("type") for c in cmds]
        self.assertIn("rect", types, "fallback background rect missing")
        self.assertIn("text", types, "fallback label text missing")
        self.assertIn("frame_done", types)

        texts = [c.get("text", "") for c in cmds if c.get("type") == "text"]
        joined = " | ".join(texts)
        self.assertIn("min size: 400 x 200", joined)
        self.assertIn("current: 320 x 180", joined)
        # At least one arrow glyph for one of the axes.
        self.assertTrue(
            any(a in joined for a in ("\u2192", "\u2193", "\u2198")),
            f"no directional arrow in fallback text: {joined!r}",
        )

    def test_min_size_passes_through_when_large_enough(self):
        app = App(app_id="test")
        app.set_min_size(400, 200)

        rendered: list[tuple[float, float]] = []

        @app.on_render
        def render(ctx: RenderContext) -> None:
            rendered.append((ctx.width, ctx.height))

        old_stdout = sys.stdout
        old_stdin = sys.stdin
        sys.stdout = _ReconfigurableStringIO()
        events = (
            '{"type":"render","width":800,"height":600,"delta_time":0}\n'
            '{"type":"shutdown"}\n'
        )
        sys.stdin = io.StringIO(events)
        try:
            app.run()
        finally:
            sys.stdout = old_stdout
            sys.stdin = old_stdin

        self.assertEqual(rendered, [(800.0, 600.0)])


if __name__ == "__main__":
    unittest.main()
