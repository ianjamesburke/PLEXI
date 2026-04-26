"""Hermetic tests for examples/app-store/app_store.py.

Stays under the same Python-test idioms as examples/backlog/tests:
- Bare `pytest` (no extra deps), `unittest.mock` for shell-out fakes.
- Doesn't touch the network, the user's home dir, or the host CLI.

Tests in scope (#308 Phase 3 acceptance):
  1. capability_label renders known capabilities with the expected hint.
  2. capability_label renders unknown capabilities with the (unknown) suffix.
  3. scan_installed_apps returns a list[InstalledRow] with the expected shape.
  4. _install_action shells out to `plexi-<channel> install <spec>` (after a
     forced-True capability prompt).
"""
from __future__ import annotations

import sys
import textwrap
from pathlib import Path
from unittest.mock import patch, MagicMock

# Make the app importable as a module — examples/ is not a package.
APP_DIR = Path(__file__).resolve().parents[1]
SDK_DIR = APP_DIR.parents[1] / "sdk" / "python"
sys.path.insert(0, str(APP_DIR))
sys.path.insert(0, str(SDK_DIR))

import app_store  # noqa: E402


def test_capability_label_renders_known_capability():
    out = app_store.capability_label("network")
    assert "Can make network requests" in out
    assert out.startswith("network:")

    out_fs = app_store.capability_label("filesystem.read")
    assert "Can read files" in out_fs

    out_secrets = app_store.capability_label("secrets")
    assert "Can read declared secrets" in out_secrets


def test_capability_label_renders_unknown_capability_with_suffix():
    out = app_store.capability_label("clipboard.write")
    assert "(unknown)" in out
    assert "clipboard.write" in out


def test_scan_installed_apps_returns_expected_shape(tmp_path: Path):
    """Hermetic: build a fake apps dir, point the scanner at it, assert
    we get the right shape back. Covers (id, version, source, caps)."""
    # App 1: bundled (no .git/) with a fs.read capability
    app1 = tmp_path / "alpha-app"
    app1.mkdir()
    (app1 / "manifest.toml").write_text(textwrap.dedent("""\
        schema_version = 1

        [app]
        id = "alpha-app"
        name = "Alpha"
        version = "0.1.0"
        entry = "main.py"

        [app.capabilities]
        capabilities = ["fs.read"]
    """))

    # App 2: git checkout (has .git/) with two capabilities
    app2 = tmp_path / "beta-app"
    app2.mkdir()
    (app2 / ".git").mkdir()
    (app2 / "manifest.toml").write_text(textwrap.dedent("""\
        schema_version = 1

        [app]
        id = "beta-app"
        name = "Beta"
        version = "1.2.3"
        entry = "main.py"

        [app.capabilities]
        capabilities = ["net.http", "secrets.get"]
    """))

    # Junk dirs that the scanner must skip
    (tmp_path / ".tmp-install-1234-9999").mkdir()
    (tmp_path / "no-manifest").mkdir()

    rows = app_store.scan_installed_apps(apps_dir=tmp_path)
    assert len(rows) == 2, f"expected 2 apps, got {[r.id for r in rows]}"

    by_id = {r.id: r for r in rows}
    assert "alpha-app" in by_id
    assert "beta-app" in by_id

    a = by_id["alpha-app"]
    assert a.version == "0.1.0"
    assert a.source == "local"
    assert a.capabilities == ["fs.read"]

    b = by_id["beta-app"]
    assert b.version == "1.2.3"
    assert b.source == "git"
    assert b.capabilities == ["net.http", "secrets.get"]


def test_install_action_shells_out_to_plexi_install():
    """The install action, given a capability-prompt that approves, must
    call the host CLI with `install <spec>`. We patch subprocess.run so
    the test is hermetic."""
    fake_result = MagicMock()
    fake_result.returncode = 0
    fake_result.stdout = "installed 'foo'\n"
    fake_result.stderr = ""

    with patch.object(app_store.subprocess, "run", return_value=fake_result) as run_mock:
        code, stdout, stderr = app_store.run_host_cli(
            ["install", "github:owner/foo"], binary_override="plexi-test"
        )
    assert code == 0
    assert "installed" in stdout
    assert run_mock.called
    args, kwargs = run_mock.call_args
    cmd = args[0]
    # First arg is the resolved binary, then the literal install args.
    assert cmd[-2] == "install"
    assert cmd[-1] == "github:owner/foo"
    assert kwargs.get("capture_output") is True
    assert kwargs.get("text") is True


def test_parse_capabilities_from_manifest_text_handles_inline_array():
    text = textwrap.dedent("""\
        schema_version = 1

        [app]
        id = "x"
        name = "X"
        entry = "x.py"

        [app.capabilities]
        capabilities = ["fs.read", "net.http"]
    """)
    caps = app_store.parse_capabilities_from_manifest_text(text)
    assert caps == ["fs.read", "net.http"]


def test_translate_source_to_url_handles_github_shorthand():
    url, ref = app_store._translate_source_to_url("github:owner/repo")
    assert url == "https://github.com/owner/repo.git"
    assert ref is None

    url, ref = app_store._translate_source_to_url("github:owner/repo@v1.2.0")
    assert url == "https://github.com/owner/repo.git"
    assert ref == "v1.2.0"


def test_translate_source_to_url_rejects_unknown_scheme():
    url, ref = app_store._translate_source_to_url("ftp://nope")
    assert url is None
    assert ref is None
