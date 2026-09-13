"""Import paths for every app's test suite.

`plexi app test` runs `uv run pytest tests/` with the app directory as cwd, so
each suite needs two things on `sys.path`: the worktree's `sdk/python` (the
`plexi_sdk` under test, not an installed copy) and the app's own directory (its
entry module). This conftest is the single place that arranges both — test
modules import `plexi_sdk` and their app module directly.

The app directory is derived per collected test file (`apps/<app>/tests/…` ->
`apps/<app>`) rather than added for every app at once, so one app's entry module
can never shadow another's.
"""

from __future__ import annotations

import sys
from pathlib import Path

_APPS_ROOT = Path(__file__).resolve().parent
_SDK_ROOT = _APPS_ROOT.parent / "sdk" / "python"


def _prepend(path: Path) -> None:
    entry = str(path)
    if entry not in sys.path:
        sys.path.insert(0, entry)


_prepend(_SDK_ROOT)


def pytest_collectstart(collector) -> None:
    """Put the owning app's directory on the path before its tests import it."""
    path = getattr(collector, "path", None)
    if path is None or path.suffix != ".py":
        return
    app_dir = path.parent.parent
    if app_dir.is_relative_to(_APPS_ROOT) and app_dir != _APPS_ROOT:
        _prepend(app_dir)
