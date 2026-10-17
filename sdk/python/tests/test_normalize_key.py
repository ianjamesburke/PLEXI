"""Unit tests for plexi_sdk._keys.normalize_key."""

import pytest

from plexi_sdk._keys import normalize_key


@pytest.mark.parametrize(
    ("raw", "expected"),
    [
        ("Enter", "enter"),
        ("Return", "enter"),
        ("enter", "enter"),
        ("return", "enter"),
        ("Escape", "escape"),
        ("Esc", "escape"),
        ("Backspace", "backspace"),
        ("Delete", "delete"),
        ("Space", "space"),
        (" ", "space"),
        ("Tab", "tab"),
        ("ArrowUp", "up"),
        ("ArrowDown", "down"),
        ("ArrowLeft", "left"),
        ("ArrowRight", "right"),
        ("Home", "home"),
        ("End", "end"),
        ("PageUp", "pageup"),
        ("PageDown", "pagedown"),
        ("Insert", "insert"),
        ("+", "+"),
        ("plus", "+"),
        ("Plus", "+"),
        ("-", "-"),
        ("minus", "-"),
        ("Minus", "-"),
        ("=", "="),
        ("equals", "="),
        ("Equals", "="),
        ("/", "/"),
        ("slash", "/"),
        ("Slash", "/"),
        ("*", "*"),
        ("asterisk", "*"),
        ("Asterisk", "*"),
        ("F1", "f1"),
        ("F12", "f12"),
        ("A", "a"),
        ("z", "z"),
        ("0", "0"),
        ("9", "9"),
    ],
)
def test_normalize_key_canonical_table(raw: str, expected: str) -> None:
    assert normalize_key(raw) == expected

