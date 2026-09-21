"""Docs-only guard: adversarial fixture stays capability-empty."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_manifest_declares_no_capabilities():
    text = (ROOT / "manifest.toml").read_text()
    assert 'capabilities = []' in text
    assert 'id = "isolation-redteam"' in text


def test_readme_states_honest_threat_model():
    readme = (ROOT / "README.md").read_text()
    assert "Docker-level" in readme or "Docker" in readme
    assert "capability" in readme.lower()
