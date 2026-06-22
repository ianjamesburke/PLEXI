import importlib.util
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "wikipedia" / "wikipedia.py"

sys.path.insert(0, str(SDK))


def _load_app_module():
    spec = importlib.util.spec_from_file_location("wikipedia_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_parse_search_reads_opensearch_titles():
    app = _load_app_module()

    assert app._parse_search(["plexi", ["Plexi", "Plexiglass"], [], []]) == [
        "Plexi",
        "Plexiglass",
    ]
