"""The live path must leave a complete, honest session dir even when a drive
command fails — outcome.json = failed, not a missing file."""

import json
from pathlib import Path

import pytest

from plexi_e2e.config import Fixture, SessionConfig
from plexi_e2e.plexi_cli import PlexiCliError
from plexi_e2e.runner import E2ESession

FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"


class FakeCli:
    """Boots ready, then blows up on the first drive command."""

    def __init__(self):
        self.stopped = False
        self._status_calls = 0

    def host_start(self, seed, timeout):
        return None

    def host_status(self):
        # preflight sees no host; readiness poll and teardown see it up.
        self._status_calls += 1
        return {} if self._status_calls == 1 else {"ready": True, "pid": 1}

    def host_stop(self):
        self.stopped = True

    def pane_list(self):
        raise PlexiCliError("pane list exploded")


def _live_config(tmp_path):
    fx = Fixture.load(FIXTURES / "counter.toml")
    return SessionConfig(
        channel="e2e", fixture=fx, sessions_root=tmp_path / "sessions",
        binary="plexi-e2e", dry_run=False, home=tmp_path / "home",
        boot_timeout_secs=1,
    )


def test_drive_failure_records_failed_outcome(tmp_path, monkeypatch):
    monkeypatch.setattr("plexi_e2e.runner.shutil.which", lambda _b: "/usr/bin/plexi-e2e")
    session = E2ESession(_live_config(tmp_path))
    fake = FakeCli()
    monkeypatch.setattr(session, "cli", fake)

    with pytest.raises(PlexiCliError):
        session.run()

    # session dir still complete: manifest + failed outcome recorded
    sessions = list((tmp_path / "sessions").iterdir())
    assert len(sessions) == 1
    d = sessions[0]
    outcome = json.loads((d / "outcome.json").read_text())
    assert outcome["status"] == "failed"
    assert outcome["stalled_at"] == "drive"
    assert any("PlexiCliError" in e for e in outcome["errors"])
    assert (d / "manifest.json").is_file()
    assert fake.stopped is True  # teardown ran
