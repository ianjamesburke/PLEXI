from plexi_e2e.capture import SessionCapture
from plexi_e2e.index import build_index, write_index


def _write_session(root, name, *, fixture_id, difficulty, dry_run):
    cap = SessionCapture(root / name)
    cap.write_manifest({
        "session_id": name,
        "channel": "e2e",
        "dry_run": dry_run,
        "fixture": {"id": fixture_id, "difficulty": difficulty},
        "wall_clock_secs": 0.5,
        "parent_turns": 1,
        "versions": {"cli": "0.1.16", "sdk": "0.1.16", "channel": "e2e"},
    })
    return cap


def test_index_lists_all_sessions_newest_first(tmp_path):
    root = tmp_path / "sessions"
    root.mkdir()
    _write_session(root, "20260101T000000Z_counter_aaa", fixture_id="counter", difficulty="easy", dry_run=True)
    _write_session(root, "20260202T000000Z_form_bbb", fixture_id="form", difficulty="medium", dry_run=True)

    text = build_index(root)
    assert "| Session |" in text
    assert "counter" in text and "form" in text
    # newest (form, later timestamp) appears before counter
    assert text.index("form") < text.index("counter")
    assert "0.1.16" in text


def test_index_handles_empty_root(tmp_path):
    root = tmp_path / "sessions"
    root.mkdir()
    text = build_index(root)
    assert "No sessions captured yet" in text


def test_write_index_creates_file(tmp_path):
    root = tmp_path / "sessions"
    root.mkdir()
    _write_session(root, "20260101T000000Z_counter_aaa", fixture_id="counter", difficulty="easy", dry_run=True)
    dest = write_index(root)
    assert dest.is_file()
    assert dest.name == "INDEX.md"
