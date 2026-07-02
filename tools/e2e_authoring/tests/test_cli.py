"""Command construction is unit-tested with an injected runner — no host needed."""

import subprocess
from pathlib import Path

import pytest

from plexi_e2e import env
from plexi_e2e.plexi_cli import PlexiCli, PlexiCliError


class FakeRunner:
    def __init__(self, stdout="", returncode=0):
        self.calls = []
        self._stdout = stdout
        self._returncode = returncode

    def __call__(self, argv, env):
        self.calls.append((list(argv), env))
        return subprocess.CompletedProcess(argv, self._returncode, self._stdout, "")


def _cli(runner, home="/h"):
    return PlexiCli("plexi-e2e", "e2e", home=Path(home), runner=runner)


def test_host_start_builds_seed_flags():
    r = FakeRunner()
    _cli(r).host_start([("/tmp/a", "c"), ("/tmp/b", None)], timeout_secs=30)
    argv, e = r.calls[0]
    assert argv == [
        "plexi-e2e", "host", "start", "--timeout-secs", "30",
        "--pane", "cwd=/tmp/a,cmd=c",
        "--pane", "cwd=/tmp/b",
    ]
    # env discipline applied on every invocation
    assert e["PLEXI_CHANNEL"] == "e2e"
    assert e["PLEXI_SOCKET"] == str(env.socket_path("e2e", Path("/h")))


def test_pane_send_and_capture_argv():
    r = FakeRunner(stdout="[]")
    cli = _cli(r)
    cli.pane_send(42, "c\n")
    cli.pane_capture(42, lines=120, from_cursor=5)
    assert r.calls[0][0] == ["plexi-e2e", "pane", "send", "42", "c\n"]
    assert r.calls[1][0] == [
        "plexi-e2e", "pane", "capture", "42", "--lines", "120", "--from-cursor", "5",
    ]


def test_nonzero_exit_raises():
    r = FakeRunner(returncode=1)
    with pytest.raises(PlexiCliError):
        _cli(r).host_start([("/tmp", None)], 30)


def test_host_status_parses_json():
    r = FakeRunner(stdout='{"ready": true, "pid": 99}')
    assert _cli(r).host_status() == {"ready": True, "pid": 99}


def test_events_subscribe_argv():
    cli = _cli(FakeRunner())
    assert cli.events_subscribe_argv("counter", "probe.tick") == [
        "plexi-e2e", "events", "subscribe", "counter", "probe.tick",
    ]
