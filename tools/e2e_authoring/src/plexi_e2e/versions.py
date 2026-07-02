"""Version stamping for a session.

A benchmark comparison across time is meaningless unless every session records
the exact CLI, SDK, and channel it ran against (stint 0215 gotcha). This module
resolves those three, from the same sources the scaffold drift metadata
(``plexi.scaffold.toml``) draws on:

  * ``cli``     — ``<binary> --version`` (the installed CLI reports
                  ``CARGO_PKG_VERSION``). ``None`` when the binary is absent,
                  as it is in a plumbing-only dry run.
  * ``sdk``     — ``[project].version`` in ``sdk/python/pyproject.toml`` (the SDK's
                  single source of truth). ``None`` if the repo layout is not found.
  * ``channel`` — the host channel under test, passed through verbatim.

Every resolver is best-effort and honest: a missing source records ``None`` in
the manifest rather than crashing a sweep or inventing a number.
"""

from __future__ import annotations

import logging
import subprocess
import tomllib
from pathlib import Path
from typing import Callable, Sequence

log = logging.getLogger("plexi_e2e.versions")

VersionRunner = Callable[[Sequence[str]], subprocess.CompletedProcess]

# sdk/python/pyproject.toml relative to this file: src/plexi_e2e/versions.py ->
# tools/e2e_authoring/src/plexi_e2e -> repo root is parents[4].
_SDK_PYPROJECT = Path(__file__).resolve().parents[4] / "sdk" / "python" / "pyproject.toml"


def _default_version_runner(argv: Sequence[str]) -> subprocess.CompletedProcess:
    return subprocess.run(list(argv), capture_output=True, text=True, timeout=30)


def cli_version(binary: str, runner: VersionRunner | None = None) -> str | None:
    """The installed CLI's version via ``<binary> --version``.

    ``clap`` prints ``<prog> <semver>``; return the last whitespace token. Returns
    ``None`` when the binary is not installed or the call fails — a dry run has no
    binary, and that absence is recorded honestly rather than raised.
    """
    if not binary:
        raise ValueError("binary is required to resolve cli version")
    run = runner or _default_version_runner
    try:
        completed = run([binary, "--version"])
    except (FileNotFoundError, OSError, subprocess.SubprocessError) as exc:
        log.info("cli version unavailable for %r: %s", binary, exc)
        return None
    if completed.returncode != 0:
        log.info("cli %r --version exited %s", binary, completed.returncode)
        return None
    out = (completed.stdout or "").strip()
    if not out:
        return None
    return out.split()[-1]


def sdk_version(pyproject: Path | None = None) -> str | None:
    """The Python SDK version from ``sdk/python/pyproject.toml``.

    Returns ``None`` if the file is not found (e.g. the tool was copied outside the
    repo) rather than guessing.
    """
    path = pyproject or _SDK_PYPROJECT
    if not path.is_file():
        log.info("sdk pyproject not found at %s", path)
        return None
    with path.open("rb") as fh:
        data = tomllib.load(fh)
    version = data.get("project", {}).get("version")
    return str(version) if version is not None else None


def resolve_versions(
    binary: str,
    channel: str,
    probe_cli: bool = True,
    runner: VersionRunner | None = None,
    sdk_pyproject: Path | None = None,
) -> dict[str, str | None]:
    """The version stamp for a session manifest: CLI, SDK, and channel.

    ``probe_cli`` gates the ``<binary> --version`` call. A dry run executes no
    CLI, so its stamp must record ``cli = None`` rather than a version scraped
    from whatever binary happens to be on PATH — the stamp reflects what actually
    ran, not what could have.
    """
    if not channel:
        raise ValueError("channel is required to stamp versions")
    return {
        "cli": cli_version(binary, runner) if probe_cli else None,
        "sdk": sdk_version(sdk_pyproject),
        "channel": channel,
    }
