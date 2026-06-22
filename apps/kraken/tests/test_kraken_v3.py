import importlib.util
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "kraken" / "main.py"

sys.path.insert(0, str(SDK))


def _load_app_module():
    spec = importlib.util.spec_from_file_location("kraken_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_parse_prices_maps_kraken_pair_to_requested_pair():
    app = _load_app_module()
    result = {
        "XXBTZUSD": {
            "c": ["65000.1", "0.1"],
            "b": ["64999.0"],
            "a": ["65001.0"],
            "h": ["66000.0"],
            "l": ["64000.0"],
        }
    }

    prices = app._parse_prices(["XBTUSD"], result)

    assert prices["XBTUSD"]["last"] == "65000.1"
    assert prices["XBTUSD"]["bid"] == "64999.0"
