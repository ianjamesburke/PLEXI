import json

import pytest

from plexi_e2e.capture import SessionCapture
from plexi_e2e.scorecard import ScorecardError, build_scorecard, write_scorecard


def _session(tmp_path, *, dry_run: bool, versions: dict | None = None) -> SessionCapture:
    cap = SessionCapture(tmp_path / "sess")
    manifest = {
        "session_id": cap.session_id,
        "channel": "e2e",
        "dry_run": dry_run,
        "fixture": {"id": "counter", "difficulty": "easy"},
        "wall_clock_secs": 1.5,
        "parent_turns": 2,
    }
    if versions is not None:
        manifest["versions"] = versions
    cap.write_manifest(manifest)
    return cap


def test_dry_run_scorecard_is_plan_only(tmp_path):
    cap = _session(tmp_path, dry_run=True, versions={"cli": None, "sdk": "0.1.16", "channel": "e2e"})
    card = build_scorecard(cap.dir)
    assert card.mode == "dry-run"
    assert card.outcome == "plan-only"
    assert card.wall_clock_secs == 1.5
    assert card.parent_turns == 2
    assert card.lines_of_code is None
    assert card.versions["sdk"] == "0.1.16"
    assert card.timings.first_interactive_secs is None


def test_live_scorecard_reads_outcome_and_metrics(tmp_path):
    cap = _session(tmp_path, dry_run=False, versions={"cli": "0.1.16", "sdk": "0.1.16", "channel": "e2e"})
    cap.append_observation("host_status", "cli", {"ready": False})
    cap.append_observation("host_status", "cli", {"ready": True})
    cap.append_observation("pane_capture", "pane_capture", {"round": 1, "lines": ["hello"]})
    cap.append_observation("pane_capture", "pane_capture", {"round": 2, "lines": []})
    cap.append_observation("code_metrics", "cli", {"loc": 42, "files": 3, "root": "/x"})
    cap.write_outcome("worked", None, 3, ["c"], [], notes="done")

    card = build_scorecard(cap.dir)
    assert card.mode == "live"
    assert card.outcome == "worked"
    assert card.child_turns == 1  # only the round with non-empty lines
    assert card.lines_of_code == 42
    assert card.commands_used == ["c"]
    assert card.timings.host_ready_secs is not None
    assert card.timings.first_child_output_secs is not None


def test_missing_manifest_raises(tmp_path):
    (tmp_path / "empty").mkdir()
    with pytest.raises(ScorecardError):
        build_scorecard(tmp_path / "empty")


def test_live_session_missing_outcome_raises(tmp_path):
    # A live capture with no outcome.json is corrupt/aborted, not a valid failure.
    cap = _session(tmp_path, dry_run=False, versions={"cli": "0.1.16", "sdk": "0.1.16", "channel": "e2e"})
    with pytest.raises(ScorecardError):
        build_scorecard(cap.dir)


def test_versions_fallback_when_absent(tmp_path):
    cap = _session(tmp_path, dry_run=True, versions=None)
    card = build_scorecard(cap.dir)
    assert card.versions == {"cli": None, "sdk": None, "channel": "e2e"}


def test_write_scorecard_roundtrips(tmp_path):
    cap = _session(tmp_path, dry_run=True, versions={"cli": None, "sdk": "0.1.16", "channel": "e2e"})
    dest = write_scorecard(cap.dir)
    data = json.loads(dest.read_text())
    assert data["outcome"] == "plan-only"
    assert data["schema_version"] == 1
    assert data["timings"]["host_ready_secs"] is None
