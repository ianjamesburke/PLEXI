import json

import pytest

from plexi_e2e.capture import SessionCapture


def test_capture_creates_structure_and_writes(tmp_path):
    cap = SessionCapture(tmp_path / "sess1")
    assert (cap.dir / "transcript.md").is_file()
    assert (cap.dir / "observations.jsonl").is_file()
    assert (cap.dir / "friction.md").is_file()

    cap.append_observation("host_status", "cli", {"ready": True})
    cap.append_observation("pane_capture", "pane_capture", {"lines": ["hi"]})
    obs = cap.read_observations()
    assert len(obs) == 2
    assert obs[0]["kind"] == "host_status"
    assert obs[0]["source"] == "cli"
    assert "ts" in obs[0]

    cap.append_transcript("child says hello")
    assert "child says hello" in (cap.dir / "transcript.md").read_text()


def test_outcome_roundtrip(tmp_path):
    cap = SessionCapture(tmp_path / "sess2")
    cap.write_outcome("worked", None, 3, ["c"], [], notes="done")
    out = json.loads((cap.dir / "outcome.json").read_text())
    assert out["status"] == "worked"
    assert out["turns"] == 3
    assert out["commands_observed"] == ["c"]


def test_invalid_outcome_status_throws(tmp_path):
    cap = SessionCapture(tmp_path / "sess3")
    with pytest.raises(ValueError):
        cap.write_outcome("mostly-ok", None, 1, [], [])


def test_capture_refuses_existing_dir(tmp_path):
    (tmp_path / "dup").mkdir()
    with pytest.raises(FileExistsError):
        SessionCapture(tmp_path / "dup")
