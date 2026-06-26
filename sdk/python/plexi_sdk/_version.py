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


def _read_from_pyproject() -> str:
    pyproject = Path(__file__).resolve().parent.parent / "pyproject.toml"
    if not pyproject.is_file():
        raise RuntimeError(
            f"cannot resolve SDK version: {_DISTRIBUTION_NAME} is not installed and "
            f"{pyproject} does not exist"
        )
    with pyproject.open("rb") as f:
        data = tomllib.load(f)
    try:
        return str(data["project"]["version"])
    except KeyError as e:
        raise RuntimeError(
            f"cannot resolve SDK version: {pyproject} is missing [project].version"
        ) from e


def _resolve_version() -> str:
    try:
        return _dist_version(_DISTRIBUTION_NAME)
    except PackageNotFoundError:
        return _read_from_pyproject()


__version__ = _resolve_version()
