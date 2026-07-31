"""Blessed codecs for markdown-format app state (``[state] format = "markdown"``).

A markdown state file is a plain document the host round-trips verbatim under
a single ``document`` key — humans and agents edit it directly, so the reader
must be tolerant of what they write while the writer emits one canonical
shape (otherwise every app save churns the user's formatting).

Checklist convention:

- Reader (:func:`parse_checklist`) accepts ``- [ ]`` / ``- [x]`` / ``* [x]``,
  case-insensitive ``x``, loose whitespace; every non-matching line is
  skipped.
- Writer (:func:`render_checklist`) emits exactly ``- [ ] text`` /
  ``- [x] text`` lines with a trailing newline.
"""
from __future__ import annotations

import re
from dataclasses import dataclass


@dataclass
class ChecklistItem:
    text: str
    done: bool


# Tolerant line shape: bullet (- or *), brackets holding an optional x,
# whitespace anywhere humans put it.
_ITEM_RE = re.compile(r"^\s*[-*]\s*\[\s*([xX]?)\s*\]\s*(.*)$")


def parse_checklist(text: str) -> list[ChecklistItem]:
    """Read every checklist item out of a markdown document.

    Non-matching lines (headings, prose, blanks) are skipped, not errors —
    a checklist may live inside a larger document.
    """
    items: list[ChecklistItem] = []
    for line in text.splitlines():
        match = _ITEM_RE.match(line)
        if match is None:
            continue
        items.append(
            ChecklistItem(text=match.group(2).rstrip(), done=match.group(1) != "")
        )
    return items


def render_checklist(items: list[ChecklistItem]) -> str:
    """Write items in the canonical shape: ``- [ ] `` / ``- [x] `` per line,
    trailing newline. An empty list renders as an empty document."""
    if not items:
        return ""
    return "".join(
        f"- [{'x' if item.done else ' '}] {item.text}\n" for item in items
    )
