"""Unit tests for KeyMap — verifies that handle() resolves correctly for the
three event-delivery paths that live Plexi uses:

  printable (Event::Text)  → on_key("z", {})           — key arrives lowercase
  ctrl chord (Event::Key)  → on_key("Z", {ctrl: true}) — egui Debug fmt gives "Z"
  named key  (Event::Key)  → on_key("return", {})      — _normalize_key applied
"""

from plexi_sdk.widgets import KeyMap


def test_bare_printable_lowercase():
    km = KeyMap()
    km.bind("z", "undo")
    assert km.handle("z", {}) == "undo"


def test_bare_printable_uppercase_normalizes():
    # ctrl+z arrives via Event::Key; egui formats Key::Z as "Z" in PlexiEvent.
    # KeyMap.handle must normalize to lowercase so bind("z") catches it.
    km = KeyMap()
    km.bind("z", "undo", mod="ctrl")
    assert km.handle("Z", {"ctrl": True}) == "undo"


def test_named_key():
    km = KeyMap()
    km.bind("return", "submit")
    assert km.handle("return", {}) == "submit"


def test_modifier_chord_meta_alias():
    # "meta" maps to "cmd" in handle()
    km = KeyMap()
    km.bind("s", "save", mod="cmd")
    assert km.handle("s", {"meta": True}) == "save"


def test_unbound_returns_none():
    km = KeyMap()
    km.bind("q", "quit")
    assert km.handle("z", {}) is None


def test_multiple_bindings_independent():
    km = KeyMap()
    km.bind("q", "quit")
    km.bind("s", "save", mod="cmd")
    km.bind("z", "undo", mod="ctrl")
    assert km.handle("q", {}) == "quit"
    assert km.handle("s", {"meta": True}) == "save"
    # ctrl+z: egui sends "Z" uppercase
    assert km.handle("Z", {"ctrl": True}) == "undo"
    # bare "z" without modifier is not bound
    assert km.handle("z", {}) is None


def test_mod_order_normalized():
    km = KeyMap()
    km.bind("z", "redo", mod="ctrl|shift")
    assert km.handle("z", {"ctrl": True, "shift": True}) == "redo"
    assert km.handle("z", {"shift": True, "ctrl": True}) == "redo"
