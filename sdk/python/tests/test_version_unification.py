"""Guards against SDK version drift across files.

There is one source of truth for the SDK version: ``[project].version`` in
``pyproject.toml``. ``plexi_sdk.__version__`` and ``SDK_ID`` must derive from it.
If anyone reintroduces a hardcoded version constant, these tests fail loudly.
"""
from __future__ import annotations

import tomllib
from pathlib import Path
import shutil
import subprocess
import sys

import plexi_sdk


def _pyproject_version() -> str:
    pyproject = Path(plexi_sdk.__file__).resolve().parent.parent / "pyproject.toml"
    with pyproject.open("rb") as f:
        data = tomllib.load(f)
    return str(data["project"]["version"])


def test_dunder_version_matches_pyproject():
    assert plexi_sdk.__version__ == _pyproject_version()


def test_sdk_id_embeds_version():
    assert plexi_sdk.SDK_ID == f"plexi-sdk-py/{plexi_sdk.__version__}"


def _host_sdk_layout(tmp_path: Path) -> Path:
    package_dir = Path(plexi_sdk.__file__).resolve().parent
    sdk_dir = tmp_path / "sdk"
    shutil.copytree(package_dir, sdk_dir / "plexi_sdk")
    shutil.copy2(package_dir.parent / "pyproject.toml", sdk_dir / "pyproject.toml")
    return sdk_dir


def _import_host_sdk(sdk_dir: Path) -> subprocess.CompletedProcess[str]:
    """Import the copied SDK with ``sdk_dir`` as the only non-stdlib path.

    ``-I`` drops the cwd and ``PYTHONPATH``; ``-S`` drops site-packages. Without
    both, the probe silently resolves the real source tree or the editable
    install instead of the layout under test.
    """
    code = (
        f"import sys; sys.path.insert(0, {str(sdk_dir)!r}); "
        "import plexi_sdk; print(plexi_sdk.SDK_ID)"
    )
    return subprocess.run(
        [sys.executable, "-I", "-S", "-c", code],
        capture_output=True,
        cwd=sdk_dir.parent,
        text=True,
    )


def test_version_resolves_from_host_sdk_layout(tmp_path: Path):
    """The host ships the package and metadata side by side under ``sdk/``."""
    result = _import_host_sdk(_host_sdk_layout(tmp_path))

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == f"plexi-sdk-py/{_pyproject_version()}"


def test_missing_host_sdk_metadata_fails_loudly(tmp_path: Path):
    """No pyproject.toml and no installed distribution: refuse to guess a version."""
    sdk_dir = _host_sdk_layout(tmp_path)
    (sdk_dir / "pyproject.toml").unlink()

    result = _import_host_sdk(sdk_dir)

    assert result.returncode != 0
    assert "Plexi SDK metadata is missing" in result.stderr


def test_version_resolves_from_installed_distribution_metadata(tmp_path: Path):
    """Wheel layout: the package ships without pyproject.toml, dist-info carries it."""
    sdk_dir = _host_sdk_layout(tmp_path)
    (sdk_dir / "pyproject.toml").unlink()
    dist_info = sdk_dir / "plexi_sdk-9.9.9.dist-info"
    dist_info.mkdir()
    (dist_info / "METADATA").write_text(
        "Metadata-Version: 2.1\nName: plexi-sdk\nVersion: 9.9.9\n"
    )

    result = _import_host_sdk(sdk_dir)

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "plexi-sdk-py/9.9.9"


def test_no_stale_sdk_version_constant():
    # _constants._SDK_VERSION was the old divergent source — it must stay gone.
    from plexi_sdk import _constants

    assert not hasattr(_constants, "_SDK_VERSION")
