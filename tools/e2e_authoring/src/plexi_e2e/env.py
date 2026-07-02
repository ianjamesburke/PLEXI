"""Env discipline for drive commands.

``pane`` / ``events`` subcommands resolve their target host from ``PLEXI_SOCKET``
and inherit the launching pane's identity from ``PLEXI_PANE_ID`` /
``PLEXI_CONTEXT_ID`` / ``PLEXI_CONTEXT_ROOT`` / ``PLEXI_RUNNING``. When the runner
itself runs inside a Plexi pane those variables point at the *wrong* host. Every
drive command must therefore run under a scrubbed env that points explicitly at
the host-under-test's own socket and channel, with the inherited pane identity
stripped. This is the trap documented in the drive-host skill; centralize it so
no call site can forget it.
"""

from __future__ import annotations

import os
from pathlib import Path

# Inherited from the launching pane; stale for the host-under-test. Always strip.
STALE_VARS = (
    "PLEXI_PANE_ID",
    "PLEXI_CONTEXT_ID",
    "PLEXI_CONTEXT_ROOT",
    "PLEXI_RUNNING",
)


def profile_dir(channel: str, home: Path | None = None) -> Path:
    """The channel-scoped profile directory, ``~/.plexi-<channel>``."""
    if not channel:
        raise ValueError("channel is required to resolve a profile dir")
    base = home if home is not None else Path.home()
    return base / f".plexi-{channel}"


def socket_path(channel: str, home: Path | None = None) -> Path:
    """The host socket for ``channel``: ``~/.plexi-<channel>/notify.sock``."""
    return profile_dir(channel, home) / "notify.sock"


def log_path(channel: str, home: Path | None = None) -> Path:
    """The channel log: ``~/.plexi-<channel>/plexi.log``."""
    return profile_dir(channel, home) / "plexi.log"


def drive_env(
    channel: str,
    home: Path | None = None,
    base_env: dict[str, str] | None = None,
) -> dict[str, str]:
    """Build the scrubbed env for a drive command targeting ``channel``.

    Strips the inherited pane identity and pins ``PLEXI_SOCKET`` /
    ``PLEXI_CHANNEL`` to the host-under-test.
    """
    if not channel:
        raise ValueError("channel is required to build a drive env")
    env = dict(os.environ if base_env is None else base_env)
    for var in STALE_VARS:
        env.pop(var, None)
    env["PLEXI_SOCKET"] = str(socket_path(channel, home))
    env["PLEXI_CHANNEL"] = channel
    return env
