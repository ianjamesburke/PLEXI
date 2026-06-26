"""Guards against SDK version drift across files.

There is one source of truth for the SDK version: ``[project].version`` in
``pyproject.toml``. ``plexi_sdk.__version__`` and ``SDK_ID`` must derive from it.
If anyone reintroduces a hardcoded version constant, these tests fail loudly.
"""
from __future__ import annotations

import tomllib
from pathlib import Path

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


def test_no_stale_sdk_version_constant():
    # _constants._SDK_VERSION was the old divergent source — it must stay gone.
    from plexi_sdk import _constants

    assert not hasattr(_constants, "_SDK_VERSION")
