"""Key normalization: egui Debug / word names to SDK canonical key strings."""
from __future__ import annotations

_KEY_ALIASES: dict[str, str] = {
    "arrowleft": "left",
    "left": "left",
    "arrowright": "right",
    "right": "right",
    "arrowup": "up",
    "up": "up",
    "arrowdown": "down",
    "down": "down",
    "enter": "enter",
    "return": "enter",
    "escape": "escape",
    "esc": "escape",
    "backspace": "backspace",
    "delete": "delete",
    "insert": "insert",
    "home": "home",
    "end": "end",
    "pageup": "pageup",
    "page_up": "pageup",
    "pagedown": "pagedown",
    "page_down": "pagedown",
    "tab": "tab",
    "space": "space",
    " ": "space",
    "plus": "+",
    "+": "+",
    "minus": "-",
    "-": "-",
    "equals": "=",
    "equal": "=",
    "=": "=",
    "slash": "/",
    "/": "/",
    "asterisk": "*",
    "multiply": "*",
    "*": "*",
    "backslash": "\\",
    "\\": "\\",
    "semicolon": ";",
    ";": ";",
    "colon": ":",
    ":": ":",
    "quote": "'",
    "'": "'",
    "backtick": "`",
    "`": "`",
    "comma": ",",
    ",": ",",
    "period": ".",
    ".": ".",
    "openbracket": "[",
    "open_bracket": "[",
    "[": "[",
    "closebracket": "]",
    "close_bracket": "]",
    "]": "]",
    "opencurlybracket": "{",
    "open_curly_bracket": "{",
    "{": "{",
    "closecurlybracket": "}",
    "close_curly_bracket": "}",
    "}": "}",
    "pipe": "|",
    "|": "|",
    "questionmark": "?",
    "?": "?",
    "exclamationmark": "!",
    "!": "!",
}


def normalize_key(key: str) -> str:
    if not key:
        return ""
    if key == " ":
        return "space"
    lowered = key.lower()
    if lowered in _KEY_ALIASES:
        return _KEY_ALIASES[lowered]
    if lowered.startswith("num") and len(lowered) == 4 and lowered[3].isdigit():
        return lowered[3]
    return lowered

