from pathlib import Path

import pytest

from plexi_e2e import env


def test_drive_env_strips_stale_and_pins_target():
    base = {
        "PLEXI_PANE_ID": "42",
        "PLEXI_CONTEXT_ID": "7",
        "PLEXI_CONTEXT_ROOT": "/somewhere",
        "PLEXI_RUNNING": "1",
        "PATH": "/usr/bin",
    }
    out = env.drive_env("e2e", home=Path("/home/u"), base_env=base)
    for var in env.STALE_VARS:
        assert var not in out
    assert out["PLEXI_CHANNEL"] == "e2e"
    assert out["PLEXI_SOCKET"] == "/home/u/.plexi-e2e/notify.sock"
    assert out["PATH"] == "/usr/bin"  # unrelated vars preserved


def test_socket_and_log_paths():
    assert env.socket_path("pr-9", Path("/h")) == Path("/h/.plexi-pr-9/notify.sock")
    assert env.log_path("pr-9", Path("/h")) == Path("/h/.plexi-pr-9/plexi.log")
    assert env.profile_dir("pr-9", Path("/h")) == Path("/h/.plexi-pr-9")


def test_empty_channel_throws():
    with pytest.raises(ValueError):
        env.drive_env("")
    with pytest.raises(ValueError):
        env.socket_path("")
