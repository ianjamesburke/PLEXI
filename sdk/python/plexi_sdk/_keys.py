"""Key normalization: egui Debug names to SDK canonical names."""
from __future__ import annotations

_KEY_ALIASES: dict[str, str] = {
    "ArrowLeft": "left",
    "ArrowRight": "right",
    "ArrowUp": "up",
    "ArrowDown": "down",
    "Enter": "return",
    "Escape": "escape",
    "Backspace": "backspace",
    "Tab": "tab",
    "Space": "space",
    " ": "space",
    "-": "minus",
    "=": "equals",
    "+": "plus",
    "[": "open_bracket",
    "]": "close_bracket",
    "\\": "backslash",
    ";": "semicolon",
    "'": "quote",
    "`": "backtick",
    ",": "comma",
    ".": "period",
    "/": "slash",
}


def normalize_key(key: str) -> str:
    return _KEY_ALIASES.get(key, key)
