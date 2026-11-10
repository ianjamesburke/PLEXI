#!/usr/bin/env python3
"""Verify the host surface census; explicit gaps are visible debt, never proof."""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "src/ui/layout_surfaces.json"


def production(source: str) -> str:
    return re.split(r"(?m)^#\[cfg\(test\)\]\s*\n(?:pub\([^)]*\) )?mod \w+\s*\{", source)[0]


def inventory(root: Path) -> set[str]:
    result = set()
    for directory in ("src/ui", "src/render"):
        for path in (root / directory).glob("*.rs"):
            if path.stem == "mod" or path.stem.endswith("_tests"):
                continue
            result.add(path.relative_to(root).as_posix())
    for directory in ("src/overlays", "src/app"):
        for path in (root / directory).glob("*.rs"):
            source = production(path.read_text())
            for symbol in re.findall(r"\bfn (draw_\w+)\s*\(", source):
                result.add(f"{path.relative_to(root).as_posix()}::{symbol}")
    return result


def check(root: Path, entries: list[dict]) -> list[str]:
    errors = []
    ids = [entry["id"] for entry in entries]
    if len(ids) != len(set(ids)):
        errors.append("duplicate surface registration")
    expected = inventory(root)
    errors += [f"unregistered surface: {item}" for item in sorted(expected - set(ids))]
    errors += [f"stale surface registration: {item}" for item in sorted(set(ids) - expected)]
    for entry in entries:
        fixture = entry.get("fixture")
        gap = entry.get("gap", "").strip()
        if not fixture and not gap:
            errors.append(f"{entry['id']}: needs seeded fixture or explicit unresolved gap")
        if fixture:
            path = root / fixture["file"]
            if not path.is_file():
                errors.append(f"{entry['id']}: missing fixture {path}")
            elif fixture.get("test") and f"fn {fixture['test']}" not in path.read_text():
                errors.append(f"{entry['id']}: missing fixture test")
            if not entry.get("states") or not entry.get("minimum"):
                errors.append(f"{entry['id']}: fixture needs states and minimum policy")
    return errors


def self_test(entries: list[dict]) -> None:
    # Negative controls: an existing surface silently removed or declared
    # covered without evidence must both fail. No source mutation is needed.
    assert any("unregistered surface" in e for e in check(ROOT, entries[1:]))
    broken = [dict(entry) for entry in entries]
    broken[0].pop("fixture", None)
    broken[0].pop("gap", None)
    assert any("needs seeded" in e for e in check(ROOT, broken))


def main() -> int:
    try:
        entries = json.loads(REGISTRY.read_text())["surfaces"]
        errors = check(ROOT, entries)
        if "--test" in sys.argv:
            self_test(entries)
        if errors:
            print("\n".join(errors), file=sys.stderr)
            return 1
        print("UI surface registry: PASS (explicit gaps remain listed in layout_surfaces.json)")
        return 0
    except (OSError, ValueError, KeyError, AssertionError) as error:
        print(f"UI layout guard failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
