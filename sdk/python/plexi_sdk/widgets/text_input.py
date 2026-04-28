"""TextInput widget — host-owned text entry primitive.

This is the v3.1 PGAP `TextInput` / `TextSubmitted` pair (issue #283). The
host owns the underlying buffer entirely — the app emits a `TextInput`
DrawCommand each frame (placeholder, position, width) and gets exactly
one `PlexiEvent::TextSubmitted` back when the user presses Enter.

When `multiline=True`, the host renders a multi-line editor; Enter submits
and Shift+Enter inserts a newline. When `multiline=False` (the default),
the host renders a single-line editor and Enter submits.

The host auto-focuses the widget on the first frame it appears, so the user
can type immediately without clicking.

Distinct from `TextArea` in the same package. `TextArea` is a multi-line
app-managed editor (apps own the buffer, render it as Rect+Text
primitives). `TextInput` is submit-only, with the host owning state —
apps cannot inspect the typed value between keystrokes.

Real-time validation (per-keystroke value access) is intentionally out
of scope for v3.1 — see issue #283 option A.
"""
from __future__ import annotations

import json
import sys
import threading

_LOCK = threading.Lock()


def _emit(obj: dict) -> None:
    """Thread-safe JSON line write to stdout. Mirrors the SDK helper."""
    with _LOCK:
        sys.stdout.write(json.dumps(obj) + "\n")
        sys.stdout.flush()


def emit_text_input(
    id: str,
    x: float,
    y: float,
    w: float,
    placeholder: str,
    multiline: bool = False,
) -> None:
    """Emit a single-frame `DrawCommand::TextInput`.

    Apps should call this on every frame they want the input visible —
    omitting it on a frame removes the field. The host renders an
    `egui::TextEdit` at `(x, y)` with width `w` and the given placeholder;
    the buffer state lives on the host, keyed on `id`.

    The host auto-focuses the widget on its first visible frame so the user
    can type immediately without clicking.

    When `multiline=True`, Enter submits and Shift+Enter inserts a newline.
    When `multiline=False` (default), Enter submits immediately.

    On submit, the host sends `PlexiEvent::TextSubmitted { id, value }` and
    clears its buffer. The next frame's emit starts empty.
    """
    _emit({
        "type": "text_input",
        "id": id,
        "x": x,
        "y": y,
        "w": w,
        "placeholder": placeholder,
        "multiline": multiline,
    })
