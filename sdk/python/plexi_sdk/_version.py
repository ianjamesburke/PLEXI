"""Single source of truth for the SDK version.

The version lives in ``pyproject.toml`` (the ``[project].version`` field).
The host copies that metadata beside the package at runtime. Missing or malformed
metadata is an installation error: returning a default would make the SDK claim
a version that was never declared.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

def _read_from_pyproject() -> str:
    pyproject = Path(__file__).resolve().parent.parent / "pyproject.toml"
    if not pyproject.is_file():
        raise RuntimeError(
            f"Plexi SDK metadata is missing: expected {pyproject}. "
            "Reinstall Plexi to restore the bundled SDK."
        )
    with pyproject.open("rb") as f:
        data = tomllib.load(f)
    try:
        version = data["project"]["version"]
    except KeyError as e:
        raise RuntimeError(
            f"Plexi SDK metadata is invalid: {pyproject} has no project.version."
        ) from e
    if not isinstance(version, str) or not version:
        raise RuntimeError(
            f"Plexi SDK metadata is invalid: {pyproject} has an empty project.version."
        )
    return version


__version__ = _read_from_pyproject()
