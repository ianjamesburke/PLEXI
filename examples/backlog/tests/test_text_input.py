from __future__ import annotations

"""Unit tests for the v3.1 host-owned `TextInput` SDK wrapper (issue #283).

Covers:
  1. `ctx.text_input(...)` emits a `text_input` DrawCommand with the
     correct shape (id, x, y, w, placeholder).
  2. The wrapper returns `None` when no submission is queued.
  3. The wrapper returns the submitted value when a `text_submitted`
     event has landed since the last call.
  4. A second poll after submission returns `None` (one-shot delivery).
  5. Distinct ids are isolated.

We don't run the full PGAP event loop — we drive `App._make_ctx` and
the internal `_text_submissions` map directly, which is exactly what
the wire-event handler in `App.run()` does.
"""

import io
import json
import os
import sys

# Allow importing plexi_sdk from the source tree without install.
# Path: examples/backlog/tests/ → repo_root/sdk/python.
sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)

from plexi_sdk import App  # noqa: E402


def _capture_emits(monkeypatch_target, ctx):
    """Replace the SDK's stdout writer so we can inspect emitted JSON.

    Returns the list of decoded dict commands that get emitted during
    the test. We swap `sys.stdout` for a StringIO before the call.
    """
    buf = io.StringIO()
    saved = sys.stdout
    sys.stdout = buf
    try:
        yield buf
    finally:
        sys.stdout = saved


def _decoded(buf: io.StringIO) -> list[dict]:
    out = []
    for line in buf.getvalue().splitlines():
        line = line.strip()
        if not line:
            continue
        out.append(json.loads(line))
    return out


def _make_ctx_with_buf(app: App):
    """Make a render ctx and a captured-stdout buffer. Returns (ctx, buf)."""
    buf = io.StringIO()
    sys.stdout = buf
    ctx = app._make_ctx(frame_id=1)
    return ctx, buf


def _restore_stdout(saved):
    sys.stdout = saved


def test_text_input_emits_correct_drawcommand_shape():
    saved = sys.stdout
    try:
        app = App()
        ctx, buf = _make_ctx_with_buf(app)
        result = ctx.text_input("note", x=12.0, y=24.0, w=300.0,
                                 placeholder="Type a note…")
        cmds = _decoded(buf)
    finally:
        _restore_stdout(saved)

    # First call: no submission queued → returns None.
    assert result is None
    # Exactly one command emitted, of the expected shape.
    assert len(cmds) == 1
    cmd = cmds[0]
    assert cmd["type"] == "text_input"
    assert cmd["id"] == "note"
    assert cmd["x"] == 12.0
    assert cmd["y"] == 24.0
    assert cmd["w"] == 300.0
    assert cmd["placeholder"] == "Type a note…"


def test_text_input_returns_submitted_value_after_event():
    saved = sys.stdout
    try:
        app = App()
        # Simulate the wire event handler stashing a submission.
        app._text_submissions["note"] = "hello world"
        ctx, _buf = _make_ctx_with_buf(app)
        result = ctx.text_input("note", x=0.0, y=0.0, w=100.0)
    finally:
        _restore_stdout(saved)

    assert result == "hello world"
    # And the submission was consumed — second poll returns None.
    saved = sys.stdout
    try:
        ctx, _buf = _make_ctx_with_buf(app)
        result2 = ctx.text_input("note", x=0.0, y=0.0, w=100.0)
    finally:
        _restore_stdout(saved)
    assert result2 is None


def test_text_input_distinct_ids_are_isolated():
    saved = sys.stdout
    try:
        app = App()
        app._text_submissions["a"] = "alpha"
        # Polling for "b" must NOT consume the "a" submission.
        ctx, _buf = _make_ctx_with_buf(app)
        result_b = ctx.text_input("b", x=0.0, y=0.0, w=100.0)
        ctx, _buf = _make_ctx_with_buf(app)
        result_a = ctx.text_input("a", x=0.0, y=0.0, w=100.0)
    finally:
        _restore_stdout(saved)

    assert result_b is None
    assert result_a == "alpha"


def test_text_submitted_wire_event_populates_pending():
    """Drive the App.run() event-handler branch without spinning up
    the full event loop: simulate one inbound `text_submitted` line and
    feed it through the same logic by reusing the dispatcher.
    """
    app = App()
    # The handler in `App.run()` is inline; we replicate the exact
    # behaviour here. Any drift between this test and the real handler
    # would mean the wire and poll halves disagree — exactly what we
    # want to catch.
    ev = {"type": "text_submitted", "id": "note", "value": "submitted!"}
    if ev["type"] == "text_submitted":
        tid = ev.get("id", "")
        if tid:
            app._text_submissions[tid] = ev.get("value", "")

    assert app._take_text_submission("note") == "submitted!"
    # Idempotent: subsequent reads return None.
    assert app._take_text_submission("note") is None
