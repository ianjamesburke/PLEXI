"""TextInput widget — host-owned single-line text entry primitive.

This is the v3.1 PGAP `TextInput` / `TextSubmitted` pair (issue #283). The
host owns the underlying buffer entirely — the app emits a `TextInput`
DrawCommand each frame (placeholder, position, width) and gets exactly
one `PlexiEvent::TextSubmitted` back when the user presses Enter.

Distinct from `TextArea` in the same package. `TextArea` is a multi-line
app-managed editor (apps own the buffer, render it as Rect+Text
primitives). `TextInput` is single-line and submit-only, with the host
owning state — apps cannot inspect the typed value between keystrokes.

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


def emit_text_input(id: str, x: float, y: float, w: float, placeholder: str) -> None:
    """Emit a single-frame `DrawCommand::TextInput`.

    Apps should call this on every frame they want the input visible —
    omitting it on a frame removes the field. The host renders an
    `egui::TextEdit::singleline` at `(x, y)` with width `w` and the
    given placeholder; the buffer state lives on the host, keyed on
    `id`.

    On Enter, the host sends `PlexiEvent::TextSubmitted { id, value }`
    and clears its buffer. The next frame's emit starts empty.
    """
    _emit({
        "type": "text_input",
        "id": id,
        "x": x,
        "y": y,
        "w": w,
        "placeholder": placeholder,
    })
