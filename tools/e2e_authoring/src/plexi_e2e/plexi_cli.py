"""Thin, logged wrappers over the Plexi CLI drive primitives.

Composes existing commands rather than reaching into the host:
``host start/stop/status``, ``pane list/new/send/key/capture/state/name``, and
``events subscribe``. Every call is logged with its argv (so the observation log
is a faithful record of what the parent did), runs under the scrubbed drive env
(see :mod:`plexi_e2e.env`), and surfaces failures loudly — no silent fallback.

The subprocess runner is injectable so command construction is unit-testable
without a running host.
"""

from __future__ import annotations

import json
import logging
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Sequence

from .env import drive_env

Runner = Callable[[Sequence[str], dict[str, str]], subprocess.CompletedProcess]


class PlexiCliError(RuntimeError):
    """A Plexi CLI command exited non-zero or produced unparseable output."""


@dataclass
class CliResult:
    argv: list[str]
    returncode: int
    stdout: str
    stderr: str

    def json(self) -> object:
        try:
            return json.loads(self.stdout)
        except json.JSONDecodeError as exc:
            raise PlexiCliError(
                f"command {self.argv!r} did not return JSON: {exc}\nstdout: {self.stdout!r}"
            ) from exc


def _default_runner(argv: Sequence[str], env: dict[str, str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        list(argv), env=env, capture_output=True, text=True, timeout=120
    )


class PlexiCli:
    """Drive commands for one channel's host-under-test."""

    def __init__(
        self,
        binary: str,
        channel: str,
        home: Path | None = None,
        logger: logging.Logger | None = None,
        runner: Runner | None = None,
    ) -> None:
        if not binary:
            raise ValueError("binary is required")
        if not channel:
            raise ValueError("channel is required")
        self.binary = binary
        self.channel = channel
        self.home = home
        self.log = logger or logging.getLogger("plexi_e2e.cli")
        self._run = runner or _default_runner

    # -- env / invocation -------------------------------------------------

    def _env(self) -> dict[str, str]:
        return drive_env(self.channel, self.home)

    def invoke(self, args: Sequence[str], check: bool = True) -> CliResult:
        argv = [self.binary, *args]
        env = self._env()
        self.log.info("cli invoke: %s", " ".join(argv))
        completed = self._run(argv, env)
        result = CliResult(
            argv=argv,
            returncode=completed.returncode,
            stdout=completed.stdout or "",
            stderr=completed.stderr or "",
        )
        if check and result.returncode != 0:
            raise PlexiCliError(
                f"command {argv!r} exited {result.returncode}\nstderr: {result.stderr}"
            )
        return result

    # -- host lifecycle ---------------------------------------------------

    def host_start(self, seed_panes: Sequence[tuple[str, str | None]], timeout_secs: int) -> CliResult:
        args = ["host", "start", "--timeout-secs", str(timeout_secs)]
        for cwd, cmd in seed_panes:
            spec = f"cwd={cwd}"
            if cmd:
                spec += f",cmd={cmd}"
            args += ["--pane", spec]
        return self.invoke(args)

    def host_status(self) -> dict:
        result = self.invoke(["host", "status", "--json"], check=False)
        if not result.stdout.strip():
            return {}
        parsed = result.json()
        if not isinstance(parsed, dict):
            raise PlexiCliError(f"host status returned non-object JSON: {parsed!r}")
        return parsed

    def host_stop(self) -> CliResult:
        return self.invoke(["host", "stop"], check=False)

    # -- pane drive / observe --------------------------------------------

    def pane_list(self) -> list:
        parsed = self.invoke(["pane", "list"]).json()
        if not isinstance(parsed, list):
            raise PlexiCliError(f"pane list returned non-array JSON: {parsed!r}")
        return parsed

    def pane_new(self, cmd: str | None, name: str | None, cwd: str | None) -> CliResult:
        args = ["pane", "new"]
        if cmd:
            args.append(cmd)
        if name:
            args += ["-n", name]
        if cwd:
            args += ["--cwd", cwd]
        return self.invoke(args)

    def pane_name(self, pane_id: int, name: str) -> CliResult:
        return self.invoke(["pane", "name", str(pane_id), name])

    def pane_send(self, pane_id: int, text: str) -> CliResult:
        return self.invoke(["pane", "send", str(pane_id), text])

    def pane_key(self, pane_id: int, key: str) -> CliResult:
        return self.invoke(["pane", "key", str(pane_id), key])

    def pane_capture(self, pane_id: int, lines: int = 80, from_cursor: int | None = None) -> object:
        args = ["pane", "capture", str(pane_id), "--lines", str(lines)]
        if from_cursor is not None:
            args += ["--from-cursor", str(from_cursor)]
        return self.invoke(args).json()

    def pane_state(self, pane_id: int) -> object:
        return self.invoke(["pane", "state", str(pane_id)]).json()

    # -- events -----------------------------------------------------------

    def events_subscribe_argv(self, app_id: str, stream: str) -> list[str]:
        """The argv for a streaming subscribe. Streaming is caller-driven
        (Popen), so return the argv + env rather than block here."""
        return [self.binary, "events", "subscribe", app_id, stream]
