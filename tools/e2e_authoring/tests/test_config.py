from pathlib import Path

import pytest

from plexi_e2e.config import Fixture, FixtureError, SessionConfig, default_binary_for

FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"


def test_counter_fixture_loads():
    fx = Fixture.load(FIXTURES / "counter.toml")
    assert fx.id == "counter"
    assert fx.difficulty == "easy"
    assert "spacebar" in fx.prompt
    assert fx.child_launch == "c"
    assert fx.seed_panes and fx.seed_panes[0].cwd.endswith("counter")
    assert "persistence_save_remember" in fx.answers


def test_missing_required_field_throws(tmp_path):
    bad = tmp_path / "bad.toml"
    bad.write_text('[fixture]\nid = "x"\n', encoding="utf-8")  # no difficulty/prompt/child
    with pytest.raises(FixtureError):
        Fixture.load(bad)


def test_empty_prompt_throws(tmp_path):
    bad = tmp_path / "bad.toml"
    bad.write_text(
        '[fixture]\nid="x"\ndifficulty="easy"\ndescription="d"\nprompt="  "\n'
        '[child]\nlaunch="c"\ncwd="/tmp"\n',
        encoding="utf-8",
    )
    with pytest.raises(FixtureError):
        Fixture.load(bad)


def test_session_config_requires_channel_and_binary():
    fx = Fixture.load(FIXTURES / "counter.toml")
    with pytest.raises(ValueError):
        SessionConfig(channel="", fixture=fx, sessions_root=Path("/tmp"), binary="plexi-e2e")
    with pytest.raises(ValueError):
        SessionConfig(channel="e2e", fixture=fx, sessions_root=Path("/tmp"), binary="")


def test_default_binary_for():
    assert default_binary_for("e2e") == "plexi-e2e"
    assert default_binary_for("pr-2360") == "plexi-pr-2360"
