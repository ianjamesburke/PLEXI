"""Single source of truth for the SDK version.

The version is declared once, in ``pyproject.toml`` (``[project].version``), and
reaches the running package by one of two routes:

- **Host / source layout** — the host copies ``pyproject.toml`` beside the
  package directory, so it is read from there.
- **Installed distribution** — a wheel carries that same declared value in its
  ``dist-info`` metadata, so it is read via ``importlib.metadata``.

Both routes resolve the same declaration. There is deliberately no literal
default: a missing version is an installation error, and returning a made-up one
would make the SDK claim a version that was never declared.
"""

from __future__ import annotations

import tomllib
from importlib.metadata import PackageNotFoundError, version as _distribution_version
from pathlib import Path

_DISTRIBUTION_NAME = "plexi-sdk"


def _from_pyproject() -> str | None:
    """Read the version from ``pyproject.toml`` beside the package, if present."""
    pyproject = Path(__file__).resolve().parent.parent / "pyproject.toml"
    if not pyproject.is_file():
        return None
    with pyproject.open("rb") as f:
        data = tomllib.load(f)
    try:
        declared = data["project"]["version"]
    except KeyError as e:
        raise RuntimeError(
            f"Plexi SDK metadata is invalid: {pyproject} has no project.version."
        ) from e
    if not isinstance(declared, str) or not declared:
        raise RuntimeError(
            f"Plexi SDK metadata is invalid: {pyproject} has an empty project.version."
        )
    return declared


def _from_distribution() -> str | None:
    """Read the version from installed distribution metadata, if installed."""
    try:
        return _distribution_version(_DISTRIBUTION_NAME)
    except PackageNotFoundError:
        return None


def _resolve_version() -> str:
    resolved = _from_pyproject() or _from_distribution()
    if resolved is None:
        package_dir = Path(__file__).resolve().parent
        raise RuntimeError(
            "Plexi SDK metadata is missing: no pyproject.toml beside "
            f"{package_dir} and no installed '{_DISTRIBUTION_NAME}' distribution. "
            "Reinstall Plexi to restore the bundled SDK."
        )
    return resolved


__version__ = _resolve_version()
