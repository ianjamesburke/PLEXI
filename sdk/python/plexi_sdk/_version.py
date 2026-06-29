"""Single source of truth for the SDK version.

The version lives in ``pyproject.toml`` (the ``[project].version`` field).
At runtime it is read via ``importlib.metadata`` when the package is installed.
When running from an uninstalled source checkout (no dist metadata), it is read
directly from the sibling ``pyproject.toml``. Both paths return the same string;
they never diverge because there is exactly one place the number is written.
"""

from __future__ import annotations

import tomllib
from importlib.metadata import PackageNotFoundError, version as _dist_version
from pathlib import Path

_DISTRIBUTION_NAME = "plexi-sdk"


_FALLBACK_VERSION = "0.1.13"


def _read_from_pyproject() -> str | None:
    pyproject = Path(__file__).resolve().parent.parent / "pyproject.toml"
    if not pyproject.is_file():
        return None
    with pyproject.open("rb") as f:
        data = tomllib.load(f)
    return str(data.get("project", {}).get("version", _FALLBACK_VERSION))


def _resolve_version() -> str:
    if source_version := _read_from_pyproject():
        return source_version
    try:
        return _dist_version(_DISTRIBUTION_NAME)
    except PackageNotFoundError:
        return _FALLBACK_VERSION


__version__ = _resolve_version()
