import pathlib

TEMPLATE = pathlib.Path(__file__).parent.parent / "plexi_sdk/templates/app_init.py"


def test_template_uses_sdk_v3_lifecycle():
    src = TEMPLATE.read_text()
    assert "def init(size, args)" in src
    assert "def update(event)" in src
    assert "def view()" in src
    assert "class " not in src
    assert "App(" not in src


def test_template_uses_effect_return_state():
    src = TEMPLATE.read_text()
    assert "state.get" in src
    assert "SetState" in src
    assert "state.set" not in src
    assert "self.state" not in src


def test_template_uses_v3_ui_nodes():
    src = TEMPLATE.read_text()
    assert "from plexi_sdk.ui import AppBar, Column, FooterKeys, Text" in src
    assert "RenderContext" not in src
    assert "Label" not in src
