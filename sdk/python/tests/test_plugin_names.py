"""The plugin vocabulary is a clean public API break."""

import plexi_sdk
from plexi_sdk import events, testing, ui


def test_plugin_names_are_public_without_app_aliases():
    assert hasattr(testing, "PluginHarness")
    assert hasattr(ui, "SurfaceHeader")
    assert hasattr(plexi_sdk, "PluginPalette")
    assert hasattr(events, "PluginEvent")
    for module, old in [
        (testing, "AppHarness"), (ui, "AppBar"),
        (plexi_sdk, "AppPalette"), (events, "AppEvent"),
    ]:
        assert not hasattr(module, old), old
