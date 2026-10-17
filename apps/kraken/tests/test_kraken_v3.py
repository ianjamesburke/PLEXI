import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / "apps" / "kraken" / "main.py"


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


def test_select_list_mouse_selection_and_refresh_subtitle():
    import plexi_sdk as sdk
    from plexi_sdk import _v3_state
    from plexi_sdk.effects import SetState
    from plexi_sdk.events import UiAction

    app = _load_app_module()
    _v3_state._state = sdk.StateSnapshot(dict(app.DEFAULT_STATE, loading=True), {})
    _v3_state._in_view = False

    node = app.view().to_node()
    header = node["children"][0]
    assert header["type"] == "app_bar"
    assert header["subtitle"] == "market ticker · refreshing…"
    select_list = node["children"][1]
    assert select_list["on_select"] == "pair"

    effects = app.update(UiAction("pair:2"))
    updated = next(effect for effect in effects if isinstance(effect, SetState)).data
    assert updated["selected"] == 2

