"""
plexi_sdk_advanced.py — Advanced UI SDK for Plexi (Python).

Higher-level abstractions over plexi_sdk.py for canvas/game/interactive apps:
canvas transforms, hit testing, drag handling, focus routing, frame timing,
and tween animation. Zero new dependencies — stdlib + plexi_sdk only.

Apps copy this file next to plexi_sdk.py:

    from plexi_sdk import App
    from plexi_sdk_advanced import Canvas, HitTester, FrameTimer, Tween, ease_out_cubic

Spec: docs/specs/proposals/core-advanced-ui-sdk.md

NOTE: Several modules here depend on Rust-side protocol extensions that have
not landed yet (mouse_down/up/move events, scroll, delta_time on render). The
Python side is shipped first so apps that need it can begin work; the dynamic
parts gracefully degrade until the protocol catches up. See follow-up issue
listed in DEV_LOG for the Rust-side work.
"""

from __future__ import annotations

import math
import time
from dataclasses import dataclass, field
from typing import Any, Callable, Optional, Tuple

from plexi_sdk import RenderContext  # noqa: F401  (re-exported for convenience)


# ----------------------------------------------------------------------------
# Canvas — pan/zoom transform context
# ----------------------------------------------------------------------------


class Canvas:
    """Pan/zoom coordinate space for draw calls.

    Use as a context manager via ``canvas.transform(ctx)`` to apply offset+scale
    to all draws inside the block.
    """

    def __init__(
        self,
        offset: Tuple[float, float] = (0.0, 0.0),
        scale: float = 1.0,
        bounds: Optional[Tuple[float, float, float, float]] = None,
    ):
        self.offset: Tuple[float, float] = offset
        self.scale: float = scale
        # Optional content bounds (x, y, w, h) — used by zoom_to_fit / clamping.
        self.bounds: Optional[Tuple[float, float, float, float]] = bounds

    # -- coordinate conversion ------------------------------------------------

    def screen_to_canvas(self, x: float, y: float) -> Tuple[float, float]:
        """Convert screen pixel to canvas coordinate."""
        ox, oy = self.offset
        return ((x - ox) / self.scale, (y - oy) / self.scale)

    def canvas_to_screen(self, x: float, y: float) -> Tuple[float, float]:
        """Convert canvas coordinate to screen pixel."""
        ox, oy = self.offset
        return (x * self.scale + ox, y * self.scale + oy)

    # -- viewport fitting -----------------------------------------------------

    def zoom_to_fit(
        self,
        content_bounds: Tuple[float, float, float, float],
        viewport: Tuple[float, float],
        padding: float = 20.0,
    ) -> None:
        """Set offset/scale so ``content_bounds`` fills the viewport with padding."""
        cx, cy, cw, ch = content_bounds
        vw, vh = viewport
        if cw <= 0 or ch <= 0 or vw <= 0 or vh <= 0:
            return
        avail_w = max(1.0, vw - 2 * padding)
        avail_h = max(1.0, vh - 2 * padding)
        self.scale = min(avail_w / cw, avail_h / ch)
        # Center the content rect inside the viewport at the new scale.
        scaled_w = cw * self.scale
        scaled_h = ch * self.scale
        self.offset = (
            (vw - scaled_w) / 2.0 - cx * self.scale,
            (vh - scaled_h) / 2.0 - cy * self.scale,
        )

    # -- transform context manager -------------------------------------------

    def transform(self, ctx: RenderContext) -> "_CanvasTransform":
        """Returns a context manager that patches ``ctx`` draw methods to
        pre-apply offset+scale. Restores originals on exit.
        """
        return _CanvasTransform(self, ctx)

    # -- input (stub for future protocol extensions) -------------------------

    def handle_input(self, event: Any) -> bool:  # pragma: no cover
        """Stub. Pan/zoom from mouse_move/scroll requires Rust-side protocol
        events that don't exist yet. Returns False (event not handled).
        """
        return False


class _CanvasTransform:
    """Context manager that monkey-patches ``ctx`` draw methods to apply the
    canvas's offset+scale. The simple SDK's RenderContext doesn't expose a
    transform stack, so we patch the bound methods for the duration of the
    block. Original methods restored on exit (even on exception).
    """

    # Methods to wrap. Each entry is (method_name, point_args, size_args).
    # point_args are scaled-and-offset; size_args are scale-only.
    _METHODS: Tuple[Tuple[str, Tuple[str, ...], Tuple[str, ...]], ...] = (
        ("rect", ("x", "y"), ("w", "h")),
        ("text", ("x", "y"), ("size",)),
        ("line", ("x1", "y1", "x2", "y2"), ("width",)),
        ("image", ("x", "y"), ("w", "h")),
    )

    def __init__(self, canvas: Canvas, ctx: RenderContext):
        self._canvas = canvas
        self._ctx = ctx
        self._originals: dict = {}

    def __enter__(self) -> "_CanvasTransform":
        canvas = self._canvas
        ctx = self._ctx

        for name, point_args, size_args in self._METHODS:
            class_method = getattr(type(ctx), name, None)
            if class_method is None:
                continue
            # Capture whether the instance had its own attribute pre-patch so
            # we restore exactly (delete vs. reset).
            had_instance_attr = name in ctx.__dict__
            original_instance_value = ctx.__dict__.get(name)
            self._originals[name] = (had_instance_attr, original_instance_value)
            bound_original = getattr(ctx, name)
            setattr(ctx, name, _wrap_draw(bound_original, canvas, point_args, size_args))
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        for name, (had_instance_attr, original_value) in self._originals.items():
            if had_instance_attr:
                setattr(self._ctx, name, original_value)
            else:
                # Delete the patched instance attribute so descriptor lookup
                # falls back through to the class method (clean restore).
                try:
                    delattr(self._ctx, name)
                except AttributeError:
                    pass
        self._originals.clear()


def _wrap_draw(
    original: Callable,
    canvas: Canvas,
    point_args: Tuple[str, ...],
    size_args: Tuple[str, ...],
) -> Callable:
    """Wrap a draw method so coordinates are translated through the canvas."""

    def wrapped(*args, **kwargs):
        # Re-read offset/scale on every call so live updates apply.
        cox, coy = canvas.offset
        s = canvas.scale

        # Map positional args by inspecting parameter names of the original.
        # Easiest robust path: convert positional to kwargs by reading the
        # method's parameter order. Use __code__.co_varnames (skip 'self').
        code = getattr(original, "__code__", None)
        if code is not None and args:
            varnames = code.co_varnames[1 : code.co_argcount]
            for i, val in enumerate(args):
                if i < len(varnames):
                    kwargs[varnames[i]] = val
            args = ()

        # Pair x/y point args (translate + scale).
        # point_args may be a flat list of names: handle as pairs (x,y),(x1,y1)...
        for i in range(0, len(point_args), 2):
            xn = point_args[i]
            yn = point_args[i + 1] if i + 1 < len(point_args) else None
            if xn in kwargs and yn is not None and yn in kwargs:
                kwargs[xn] = kwargs[xn] * s + cox
                kwargs[yn] = kwargs[yn] * s + coy

        # Scale-only args (sizes, widths).
        for sn in size_args:
            if sn in kwargs:
                kwargs[sn] = kwargs[sn] * s

        return original(**kwargs)

    return wrapped


# ----------------------------------------------------------------------------
# HitTester — register rectangles, test a point
# ----------------------------------------------------------------------------


@dataclass
class HitRegion:
    """A registered hit-test rectangle."""

    id: Any
    x: float
    y: float
    w: float
    h: float

    def contains(self, x: float, y: float) -> bool:
        return self.x <= x < self.x + self.w and self.y <= y < self.y + self.h


class HitTester:
    """Register rectangles with IDs each frame, then test a point.

    O(n) is fine for MVP — designed for ~100 nodes max. Last-registered wins
    (matches painter's-model draw order: top-of-stack is hit first).
    """

    def __init__(self):
        self._regions: list[HitRegion] = []

    def clear(self) -> None:
        """Reset registered regions. Call at the start of each render frame."""
        self._regions.clear()

    def register(self, id: Any, x: float, y: float, w: float, h: float) -> HitRegion:
        """Add a region. Returns the created HitRegion."""
        region = HitRegion(id=id, x=x, y=y, w=w, h=h)
        self._regions.append(region)
        return region

    def test(self, x: float, y: float) -> Optional[HitRegion]:
        """Return topmost (last-registered) region containing the point."""
        for region in reversed(self._regions):
            if region.contains(x, y):
                return region
        return None


# ----------------------------------------------------------------------------
# DragHandler — minimal drag state machine
# ----------------------------------------------------------------------------


class DragHandler:
    """Drag-gesture state machine.

    NOTE: Full drag requires ``mouse_move`` and ``mouse_up`` events that are
    not yet in the Plexi protocol. Apps using this get partial functionality
    until those events land. Track via the follow-up issue referenced in
    DEV_LOG entry for this PR.
    """

    def __init__(self, threshold: float = 4.0):
        self.threshold: float = threshold
        self.active: bool = False
        self.payload: Any = None
        self._start: Tuple[float, float] = (0.0, 0.0)
        self._last: Tuple[float, float] = (0.0, 0.0)
        self._armed: bool = False

    def start(self, x: float, y: float, payload: Any = None) -> None:
        """Begin tracking a potential drag at (x, y)."""
        self._start = (x, y)
        self._last = (x, y)
        self.payload = payload
        self._armed = True
        self.active = False

    def update(self, x: float, y: float) -> Tuple[float, float]:
        """Return delta since last update. Returns (0, 0) before threshold."""
        if not self._armed:
            return (0.0, 0.0)
        if not self.active:
            sx, sy = self._start
            if math.hypot(x - sx, y - sy) >= self.threshold:
                self.active = True
                self._last = (x, y)
                return (0.0, 0.0)
            return (0.0, 0.0)
        lx, ly = self._last
        dx, dy = x - lx, y - ly
        self._last = (x, y)
        return (dx, dy)

    def end(self) -> Any:
        """Stop tracking. Returns the payload supplied at start."""
        payload = self.payload
        self.active = False
        self._armed = False
        self.payload = None
        return payload


# ----------------------------------------------------------------------------
# FocusManager — route keyboard input to named widgets
# ----------------------------------------------------------------------------


class FocusManager:
    """Tracks which named widget currently owns keyboard focus."""

    def __init__(self):
        self.current: Optional[str] = None
        self._handlers: dict[str, Callable] = {}

    def set(self, name: Optional[str]) -> None:
        """Set the focused widget name (or None to clear)."""
        self.current = name

    def register(self, name: str, handler: Callable) -> None:
        """Register a handler for a named widget. Optional — apps can also
        check ``focus.current`` directly in their on_key handler."""
        self._handlers[name] = handler

    def dispatch(self, *args, **kwargs) -> bool:
        """Dispatch to the current focus handler if registered. Returns True
        if a handler was called."""
        if self.current and self.current in self._handlers:
            self._handlers[self.current](*args, **kwargs)
            return True
        return False


# ----------------------------------------------------------------------------
# FrameTimer — fixed-interval tick loop using wall-clock time
# ----------------------------------------------------------------------------


class FrameTimer:
    """Ticks True every ``interval`` seconds. Uses ``time.monotonic()`` so it
    works without protocol-level delta_time. ``dt_override`` is accepted for
    forward compatibility once Plexi sends delta_time on render events.
    """

    def __init__(self, interval: float):
        self.interval: float = interval
        self._last_tick: float = time.monotonic()

    def ready(self, dt_override: Optional[float] = None) -> bool:
        """Return True once per ``interval`` window. Resets the window on True."""
        now = time.monotonic()
        if (now - self._last_tick) >= self.interval:
            self._last_tick = now
            return True
        return False

    def elapsed(self) -> float:
        """Seconds since the last tick that returned True."""
        return time.monotonic() - self._last_tick

    def set_interval(self, new_interval: float) -> None:
        """Change the tick interval (e.g. snake speed-up)."""
        self.interval = new_interval


# ----------------------------------------------------------------------------
# Easing functions
# ----------------------------------------------------------------------------


def linear(t: float) -> float:
    return t


def ease_in(t: float) -> float:
    return t * t


def ease_out(t: float) -> float:
    return 1.0 - (1.0 - t) * (1.0 - t)


def ease_in_out(t: float) -> float:
    if t < 0.5:
        return 2.0 * t * t
    return 1.0 - (-2.0 * t + 2.0) ** 2 / 2.0


def ease_out_cubic(t: float) -> float:
    return 1.0 - (1.0 - t) ** 3


def ease_out_bounce(t: float) -> float:
    n1 = 7.5625
    d1 = 2.75
    if t < 1.0 / d1:
        return n1 * t * t
    if t < 2.0 / d1:
        t -= 1.5 / d1
        return n1 * t * t + 0.75
    if t < 2.5 / d1:
        t -= 2.25 / d1
        return n1 * t * t + 0.9375
    t -= 2.625 / d1
    return n1 * t * t + 0.984375


# ----------------------------------------------------------------------------
# Tween — interpolate a value over wall-clock time
# ----------------------------------------------------------------------------


class Tween:
    """Interpolate a value from ``start`` to ``end`` over ``duration`` seconds."""

    def __init__(
        self,
        start: float,
        end: float,
        duration: float,
        easing: Callable[[float], float] = linear,
    ):
        self.start: float = start
        self.end: float = end
        self.duration: float = max(1e-9, duration)
        self.easing: Callable[[float], float] = easing
        self._t0: float = time.monotonic()

    def reset(self) -> None:
        """Restart the tween from t=0 at the current wall-clock time."""
        self._t0 = time.monotonic()

    def value(self, now: Optional[float] = None) -> float:
        """Return the interpolated value at the current (or supplied) time."""
        t_now = now if now is not None else time.monotonic()
        elapsed = t_now - self._t0
        if elapsed <= 0.0:
            return self.start
        if elapsed >= self.duration:
            return self.end
        t = elapsed / self.duration
        return self.start + (self.end - self.start) * self.easing(t)

    @property
    def done(self) -> bool:
        return (time.monotonic() - self._t0) >= self.duration


# ----------------------------------------------------------------------------
# LayerStack — TODO: deferred until simple SDK exposes a draw-order hook.
# ----------------------------------------------------------------------------
#
# The simple SDK already accumulates draw commands in append order and flushes
# them at frame end (RenderContext._commands + _flush). A LayerStack would need
# to either (a) intercept _commands during a `with layer.draw(...)` block and
# tag each command with a layer index, then sort on flush, or (b) maintain its
# own per-layer command lists. Both work, but require either patching
# _commands or running the user's draw callbacks twice. Spec calls this
# deferrable; skipping for MVP.
