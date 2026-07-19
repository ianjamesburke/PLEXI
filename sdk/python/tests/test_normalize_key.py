"""Unit tests for plexi_sdk._keys.normalize_key."""

from plexi_sdk._keys import normalize_key


def test_named_keys_map_to_words():
    assert normalize_key("ArrowLeft") == "left"
    assert normalize_key("Enter") == "return"
    assert normalize_key("Escape") == "escape"


def test_minus_and_equals_still_alias_to_words():
    # Apps that only check "minus"/"equals" (e.g. apps/dev/counter) rely on this.
    assert normalize_key("-") == "minus"
    assert normalize_key("=") == "equals"


def test_slash_passes_through_unchanged():
    # Stint 0462: the host sends "/" (python_key_name() in wasm_python.rs).
    # Aliasing it to "slash" here would re-break apps checking `key == "/"`
    # (e.g. apps/logs/logs.py's search shortcut).
    assert normalize_key("/") == "/"


def test_unmapped_key_passes_through_unchanged():
    assert normalize_key("z") == "z"
