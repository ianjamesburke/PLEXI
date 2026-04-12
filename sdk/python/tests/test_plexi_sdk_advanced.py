"""Unit tests for plexi_sdk_advanced. Stdlib unittest, no external deps.

Run with:
    python3 sdk/python/tests/test_plexi_sdk_advanced.py
"""

import os
import sys
import time
import unittest

# Make `import plexi_sdk[_advanced]` work when this file is run directly.
HERE = os.path.dirname(os.path.abspath(__file__))
SDK_DIR = os.path.abspath(os.path.join(HERE, ".."))
if SDK_DIR not in sys.path:
    sys.path.insert(0, SDK_DIR)

from plexi_sdk import RenderContext  # noqa: E402
from plexi_sdk_advanced import (  # noqa: E402
    Canvas,
    DragHandler,
    FocusManager,
    FrameTimer,
    HitTester,
    Tween,
    ease_in,
    ease_out_cubic,
    linear,
)


class TestCanvas(unittest.TestCase):
    def test_screen_canvas_roundtrip_identity(self):
        canvas = Canvas()
        for x, y in [(0.0, 0.0), (123.4, -56.7), (1000.0, 999.0)]:
            cx, cy = canvas.screen_to_canvas(x, y)
            sx, sy = canvas.canvas_to_screen(cx, cy)
            self.assertAlmostEqual(sx, x, places=6)
            self.assertAlmostEqual(sy, y, places=6)

    def test_roundtrip_with_offset_and_scale(self):
        canvas = Canvas(offset=(50.0, -20.0), scale=2.5)
        for x, y in [(0.0, 0.0), (10.0, 10.0), (-7.5, 42.0)]:
            cx, cy = canvas.screen_to_canvas(x, y)
            sx, sy = canvas.canvas_to_screen(cx, cy)
            self.assertAlmostEqual(sx, x, places=6)
            self.assertAlmostEqual(sy, y, places=6)

    def test_zoom_to_fit_centers_content(self):
        canvas = Canvas()
        # 100x100 content into 200x200 viewport, no padding for predictability.
        canvas.zoom_to_fit((0.0, 0.0, 100.0, 100.0), (200.0, 200.0), padding=0.0)
        self.assertAlmostEqual(canvas.scale, 2.0, places=6)
        # Content (0,0)..(100,100) at scale 2 → screen (0,0)..(200,200): centered.
        sx, sy = canvas.canvas_to_screen(0.0, 0.0)
        self.assertAlmostEqual(sx, 0.0, places=6)
        self.assertAlmostEqual(sy, 0.0, places=6)
        sx, sy = canvas.canvas_to_screen(100.0, 100.0)
        self.assertAlmostEqual(sx, 200.0, places=6)
        self.assertAlmostEqual(sy, 200.0, places=6)

    def test_transform_patches_and_restores_ctx(self):
        canvas = Canvas(offset=(10.0, 20.0), scale=2.0)
        ctx = RenderContext(800.0, 600.0)
        original_rect = ctx.rect
        original_text = ctx.text
        with canvas.transform(ctx):
            self.assertNotEqual(ctx.rect, original_rect)
            ctx.rect(5.0, 5.0, 10.0, 10.0, fill="#ff0000")
        # Methods restored — bound methods compare equal when same instance+func.
        self.assertEqual(ctx.rect, original_rect)
        self.assertEqual(ctx.text, original_text)
        # The recorded command should reflect canvas-space (5,5) → screen
        # (5*2+10, 5*2+20) = (20, 30) and size 10*2 = 20.
        cmd = ctx._commands[0]
        self.assertEqual(cmd["type"], "rect")
        self.assertAlmostEqual(cmd["x"], 20.0)
        self.assertAlmostEqual(cmd["y"], 30.0)
        self.assertAlmostEqual(cmd["w"], 20.0)
        self.assertAlmostEqual(cmd["h"], 20.0)
        self.assertEqual(cmd["fill"], "#ff0000")

    def test_transform_restores_on_exception(self):
        canvas = Canvas(scale=3.0)
        ctx = RenderContext(800.0, 600.0)
        original_rect = ctx.rect
        with self.assertRaises(RuntimeError):
            with canvas.transform(ctx):
                raise RuntimeError("boom")
        self.assertEqual(ctx.rect, original_rect)


class TestHitTester(unittest.TestCase):
    def test_topmost_wins_on_overlap(self):
        ht = HitTester()
        ht.register("bottom", 0, 0, 100, 100)
        ht.register("top", 50, 50, 100, 100)
        # Inside both — top (last-registered) wins.
        hit = ht.test(60, 60)
        self.assertIsNotNone(hit)
        self.assertEqual(hit.id, "top")
        # Inside only bottom.
        hit = ht.test(10, 10)
        self.assertIsNotNone(hit)
        self.assertEqual(hit.id, "bottom")
        # Outside both.
        self.assertIsNone(ht.test(500, 500))

    def test_clear_resets_state(self):
        ht = HitTester()
        ht.register("a", 0, 0, 10, 10)
        ht.clear()
        self.assertIsNone(ht.test(5, 5))


class TestDragHandler(unittest.TestCase):
    def test_threshold_not_yet_passed(self):
        drag = DragHandler(threshold=4.0)
        drag.start(100.0, 100.0, payload="node-1")
        # Move 2px — under threshold.
        dx, dy = drag.update(102.0, 100.0)
        self.assertEqual((dx, dy), (0.0, 0.0))
        self.assertFalse(drag.active)

    def test_threshold_activates_drag(self):
        drag = DragHandler(threshold=4.0)
        drag.start(100.0, 100.0, payload="node-1")
        # Move 5px — over threshold; first activation returns (0,0).
        dx, dy = drag.update(105.0, 100.0)
        self.assertEqual((dx, dy), (0.0, 0.0))
        self.assertTrue(drag.active)
        # Subsequent move yields the actual delta.
        dx, dy = drag.update(110.0, 103.0)
        self.assertAlmostEqual(dx, 5.0)
        self.assertAlmostEqual(dy, 3.0)
        # Cumulative deltas track from the previous update, not from start.
        dx, dy = drag.update(108.0, 105.0)
        self.assertAlmostEqual(dx, -2.0)
        self.assertAlmostEqual(dy, 2.0)

    def test_payload_round_trips(self):
        drag = DragHandler(threshold=1.0)
        drag.start(0.0, 0.0, payload={"id": 7})
        drag.update(2.0, 0.0)
        payload = drag.end()
        self.assertEqual(payload, {"id": 7})
        self.assertFalse(drag.active)
        self.assertIsNone(drag.payload)


class TestFrameTimer(unittest.TestCase):
    def test_ready_fires_after_interval(self):
        ft = FrameTimer(interval=0.05)
        # Immediately after construction, not ready.
        self.assertFalse(ft.ready())
        # Wait past interval.
        time.sleep(0.07)
        self.assertTrue(ft.ready())
        # Resets — not ready again immediately.
        self.assertFalse(ft.ready())

    def test_set_interval(self):
        ft = FrameTimer(interval=10.0)
        ft.set_interval(0.01)
        self.assertEqual(ft.interval, 0.01)
        time.sleep(0.02)
        self.assertTrue(ft.ready())


class TestTween(unittest.TestCase):
    def test_value_at_endpoints(self):
        tw = Tween(start=0.0, end=100.0, duration=1.0, easing=linear)
        # t=0
        self.assertAlmostEqual(tw.value(tw._t0), 0.0)
        # t=duration
        self.assertAlmostEqual(tw.value(tw._t0 + 1.0), 100.0)
        # Past duration clamps to end.
        self.assertAlmostEqual(tw.value(tw._t0 + 99.0), 100.0)

    def test_monotonic_between(self):
        tw = Tween(start=0.0, end=100.0, duration=1.0, easing=linear)
        prev = -1.0
        for i in range(11):
            v = tw.value(tw._t0 + i / 10.0)
            self.assertGreaterEqual(v, prev)
            prev = v

    def test_easing_function_used(self):
        # ease_in is t*t — at t=0.5, value should be 0.25 of the way.
        tw = Tween(start=0.0, end=100.0, duration=1.0, easing=ease_in)
        v = tw.value(tw._t0 + 0.5)
        self.assertAlmostEqual(v, 25.0, places=4)
        # ease_out_cubic at t=0.5 = 1 - 0.5^3 = 0.875
        tw2 = Tween(start=0.0, end=100.0, duration=1.0, easing=ease_out_cubic)
        v2 = tw2.value(tw2._t0 + 0.5)
        self.assertAlmostEqual(v2, 87.5, places=4)


class TestFocusManager(unittest.TestCase):
    def test_set_and_current(self):
        focus = FocusManager()
        self.assertIsNone(focus.current)
        focus.set("editor")
        self.assertEqual(focus.current, "editor")
        focus.set(None)
        self.assertIsNone(focus.current)

    def test_dispatch_to_registered_handler(self):
        focus = FocusManager()
        received = []
        focus.register("editor", lambda key: received.append(key))
        # Nothing focused — dispatch is a no-op and returns False.
        self.assertFalse(focus.dispatch("a"))
        focus.set("editor")
        self.assertTrue(focus.dispatch("x"))
        self.assertEqual(received, ["x"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
