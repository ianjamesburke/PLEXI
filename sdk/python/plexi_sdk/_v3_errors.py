"""Structured report of the exception that ends a plugin's run.

The runtime sends one `fatal_error` message before it exits, so every host
surface can lead with the plugin's own exception (file, line, type, message)
without scraping a CPython traceback off stderr. Paths are reported relative
to the plugin folder: the guest mount point is a host detail the author never
chose and cannot open.
"""
from __future__ import annotations

import inspect
import traceback
from pathlib import Path
from types import ModuleType
from typing import Any

import plexi_sdk

#: The signature each entry function must accept, as the author writes it.
ENTRY_SIGNATURES = {
    "init": "def init(size, args)",
    "update": "def update(event)",
    "view": "def view()",
}

_SDK_ROOT = Path(plexi_sdk.__file__).resolve().parent.parent


def call_entry(module: ModuleType, name: str, *args: Any) -> Any:
    """Call a plugin entry function; a wrong arity names the expected signature.

    CPython raises the arity `TypeError` in the caller, before any plugin code
    runs, so its traceback stops at this frame. A `TypeError` raised inside the
    plugin has deeper frames and passes through untouched.
    """
    function = getattr(module, name)
    try:
        return function(*args)
    except TypeError as exc:
        raised_before_entering = (
            exc.__traceback__ is not None and exc.__traceback__.tb_next is None
        )
        if not raised_before_entering:
            raise
        raise TypeError(
            f"{name}() is called as `{ENTRY_SIGNATURES[name]}`, but the plugin "
            f"defines `def {name}{_signature_text(function)}`. Change the "
            f"definition to `{ENTRY_SIGNATURES[name]}:`."
        ) from None


def _signature_text(function: Any) -> str:
    try:
        return str(inspect.signature(function))
    except (TypeError, ValueError):
        return "(...)"


def fatal_error_fields(
    exc: BaseException,
    *,
    phase: str,
    plugin_root: Path,
    module: ModuleType | None,
) -> dict:
    """Describe `exc` as the fields of the runtime's `fatal_error` message.

    `file` and `line` point at the innermost frame inside the plugin folder.
    When the exception never entered plugin code (a wrong return value, a wrong
    arity) they point at the entry function the runtime was calling.
    """
    plugin_root = plugin_root.resolve()
    location = (
        _syntax_error_location(exc, plugin_root)
        or _innermost_plugin_frame(exc, plugin_root)
        or _entry_definition(module, phase, plugin_root)
    )
    summary = traceback.TracebackException.from_exception(exc)
    _relativize(summary, plugin_root)
    return {
        "phase": phase,
        "file": location[0] if location else None,
        "line": location[1] if location else None,
        "exception": type(exc).__name__,
        "message": _message_text(exc),
        "traceback": "".join(summary.format()),
    }


def _message_text(exc: BaseException) -> str:
    """`str(exc)`, or a placeholder when the exception's own `__str__` raises."""
    try:
        return str(exc)
    except Exception:
        return f"<unprintable {type(exc).__name__} object>"


def _plugin_relative(filename: str, plugin_root: Path) -> str | None:
    try:
        return Path(filename).resolve().relative_to(plugin_root).as_posix()
    except (OSError, ValueError):
        return None


def _display_path(filename: str, plugin_root: Path) -> str:
    relative = _plugin_relative(filename, plugin_root)
    if relative is not None:
        return relative
    try:
        return Path(filename).resolve().relative_to(_SDK_ROOT).as_posix()
    except (OSError, ValueError):
        return filename


def _syntax_error_location(exc: BaseException, plugin_root: Path) -> tuple[str, int | None] | None:
    if not isinstance(exc, SyntaxError) or not exc.filename:
        return None
    relative = _plugin_relative(exc.filename, plugin_root)
    return (relative, exc.lineno) if relative is not None else None


def _innermost_plugin_frame(exc: BaseException, plugin_root: Path) -> tuple[str, int | None] | None:
    found = None
    for frame, lineno in traceback.walk_tb(exc.__traceback__):
        relative = _plugin_relative(frame.f_code.co_filename, plugin_root)
        if relative is not None:
            found = (relative, lineno)
    return found


def _entry_definition(
    module: ModuleType | None, phase: str, plugin_root: Path
) -> tuple[str, int | None] | None:
    code = getattr(getattr(module, phase, None), "__code__", None)
    if code is None:
        return None
    relative = _plugin_relative(code.co_filename, plugin_root)
    return (relative, code.co_firstlineno) if relative is not None else None


def _relativize(summary: traceback.TracebackException, plugin_root: Path) -> None:
    """Rewrite every frame's filename in `summary` and its chained exceptions."""
    seen: set[int] = set()
    pending: list[traceback.TracebackException | None] = [summary]
    while pending:
        current = pending.pop()
        if current is None or id(current) in seen:
            continue
        seen.add(id(current))
        for frame in current.stack:
            # Read the source line before renaming: the lookup is by filename.
            _ = frame.line
            frame.filename = _display_path(frame.filename, plugin_root)
        if isinstance(getattr(current, "filename", None), str):
            current.filename = _display_path(current.filename, plugin_root)
        pending.append(current.__cause__)
        pending.append(current.__context__)
        pending.extend(getattr(current, "exceptions", None) or [])
