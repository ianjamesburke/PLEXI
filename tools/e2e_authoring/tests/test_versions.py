import subprocess

from plexi_e2e.versions import cli_version, resolve_versions, sdk_version


def _fake_run(stdout: str, returncode: int = 0):
    def run(argv):
        return subprocess.CompletedProcess(argv, returncode, stdout=stdout, stderr="")
    return run


def test_cli_version_parses_last_token():
    assert cli_version("plexi", _fake_run("plexi 0.1.16")) == "0.1.16"


def test_cli_version_none_when_missing_binary():
    def run(argv):
        raise FileNotFoundError(argv[0])
    assert cli_version("plexi-nope", run) is None


def test_cli_version_none_on_nonzero():
    assert cli_version("plexi", _fake_run("", returncode=2)) is None


def test_cli_version_empty_binary_raises():
    import pytest

    with pytest.raises(ValueError):
        cli_version("")


def test_sdk_version_reads_repo_pyproject():
    # Resolves the real sdk/python/pyproject.toml from the repo layout.
    version = sdk_version()
    assert version is not None
    assert version[0].isdigit()


def test_sdk_version_none_when_pyproject_absent(tmp_path):
    assert sdk_version(tmp_path / "nope.toml") is None


def test_resolve_versions_shape():
    versions = resolve_versions("plexi-nope", "e2e", runner=_fake_run("plexi 9.9.9"))
    assert versions == {"cli": "9.9.9", "sdk": sdk_version(), "channel": "e2e"}


def test_resolve_versions_requires_channel():
    import pytest

    with pytest.raises(ValueError):
        resolve_versions("plexi", "")


def test_resolve_versions_skips_cli_probe_when_disabled():
    # A dry run stamps cli=None even when a binary is present on PATH; the runner
    # must not scrape a version from a CLI it never executed.
    def run(argv):
        raise AssertionError("cli must not be probed when probe_cli=False")

    versions = resolve_versions("plexi", "e2e", probe_cli=False, runner=run)
    assert versions["cli"] is None
