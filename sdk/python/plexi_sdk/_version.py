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

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11 fallback
    tomllib = None  # type: ignore[assignment]
from importlib.metadata import PackageNotFoundError, version as _distribution_version
from pathlib import Path
import re

_DISTRIBUTION_NAME = "plexi-sdk"
_SECTION_RE = re.compile(r"^\s*\[([^\]]+)\]\s*(?:#.*)?$")
_VERSION_RE = re.compile(r'^\s*version\s*=\s*(?:"([^"]*)"|\'([^\']*)\')\s*(?:#.*)?$')


def _parse_project_version_fallback(text: str) -> dict[str, dict[str, str]]:
    """Minimal ``[project].version`` reader used when stdlib ``tomllib`` is absent."""
    in_project = False
    project: dict[str, str] = {}
    for raw_line in text.splitlines():
        section_match = _SECTION_RE.match(raw_line)
        if section_match is not None:
            in_project = section_match.group(1).strip() == "project"
            continue
        if in_project:
            version_match = _VERSION_RE.match(raw_line)
            if version_match is not None:
                value = version_match.group(1)
                if value is None:
                    value = version_match.group(2) or ""
                project["version"] = value
                break
    return {"project": project} if project else {}


def _from_pyproject() -> str | None:
    """Read the version from ``pyproject.toml`` beside the package, if present."""
    pyproject = Path(__file__).resolve().parent.parent / "pyproject.toml"
    if not pyproject.is_file():
        return None
    if tomllib is not None:
        with pyproject.open("rb") as f:
            data = tomllib.load(f)
    else:
        data = _parse_project_version_fallback(pyproject.read_text(encoding="utf-8"))
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
