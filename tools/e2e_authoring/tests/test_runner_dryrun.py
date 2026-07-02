import json
from pathlib import Path

from plexi_e2e.config import Fixture, SessionConfig
from plexi_e2e.runner import E2ESession

FIXTURES = Path(__file__).resolve().parents[1] / "fixtures"


def _config(tmp_path, **kw):
    fx = Fixture.load(FIXTURES / "counter.toml")
    return SessionConfig(
        channel="e2e", fixture=fx, sessions_root=tmp_path / "sessions",
        binary="plexi-e2e", dry_run=True, home=tmp_path / "home", **kw,
    )


def test_dry_run_writes_complete_session_dir(tmp_path):
    result = E2ESession(_config(tmp_path)).run()
    d = result.session_dir
    assert result.dry_run is True
    for name in ("manifest.json", "prompt.toml", "transcript.md",
                 "observations.jsonl", "friction.md", "plan.json"):
        assert (d / name).is_file(), f"missing {name}"

    manifest = json.loads((d / "manifest.json").read_text())
    assert manifest["dry_run"] is True
    assert manifest["channel"] == "e2e"
    assert manifest["fixture"]["id"] == "counter"
    assert manifest["schema_version"] == 1

    plan = json.loads((d / "plan.json").read_text())
    stages = {s["stage"] for s in plan}
    assert stages == {"preflight", "boot", "drive", "observe", "teardown"}
    # boot step carries the seeded workspace pane
    boot = next(s for s in plan if s["stage"] == "boot" and "start" in s["argv"])
    assert "--pane" in boot["argv"]

    # the initial prompt was recorded as an intervention observation
    obs = [json.loads(l) for l in (d / "observations.jsonl").read_text().splitlines() if l.strip()]
    assert any(o["kind"] == "intervention" for o in obs)


def test_repeatability_two_independent_dirs(tmp_path):
    r1 = E2ESession(_config(tmp_path)).run()
    r2 = E2ESession(_config(tmp_path)).run()
    assert r1.session_dir != r2.session_dir
    assert r1.session_id != r2.session_id
    assert r1.session_dir.is_dir() and r2.session_dir.is_dir()
    # no shared mutable state — both have their own manifest
    assert (r1.session_dir / "manifest.json").is_file()
    assert (r2.session_dir / "manifest.json").is_file()


def test_fixture_copied_verbatim(tmp_path):
    result = E2ESession(_config(tmp_path)).run()
    copied = (result.session_dir / "prompt.toml").read_text()
    original = (FIXTURES / "counter.toml").read_text()
    assert copied == original


def test_seed_cwd_tilde_expanded_in_plan(tmp_path):
    # host start does no shell expansion; the runner must expand `~` itself.
    result = E2ESession(_config(tmp_path)).run()
    plan = json.loads((result.session_dir / "plan.json").read_text())
    boot = next(s for s in plan if s["stage"] == "boot" and "start" in s["argv"])
    pane_arg = boot["argv"][boot["argv"].index("--pane") + 1]
    assert "~" not in pane_arg
    assert pane_arg.startswith("cwd=/")
