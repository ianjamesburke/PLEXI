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


RAW_CALL = re.compile(r"\.(?:horizontal(?:_wrapped)?|with_layout|allocate_exact_size|allocate_ui_at_rect|layout_no_wrap|text|galley(?:_with_override_text_color)?|interact|put)\s*\(")


def raw_sites(root: Path) -> dict[str, dict[str, int]]:
    """Conservative syntax guard, not a Rust semantic proof. Scope is the
    enclosing named function; identical calls retain multiplicity."""
    result = {}
    for directory in ("src/ui", "src/render", "src/overlays", "src/app"):
        for path in sorted((root / directory).glob("*.rs")):
            if path.stem.endswith("_tests"):
                continue
            symbol = "module"
            sites = {}
            for line in production(path.read_text()).splitlines():
                if match := re.search(r"\bfn (\w+)", line):
                    symbol = match[1]
                if directory == "src/app" and not symbol.startswith("draw_"):
                    continue
                if line.lstrip().startswith("//") or not RAW_CALL.search(line):
                    continue
                site = f"{symbol} | {' '.join(line.split())}"
                sites[site] = sites.get(site, 0) + len(RAW_CALL.findall(line))
            if sites:
                result[path.relative_to(root).as_posix()] = sites
    return result


def check_raw(root: Path, entries: list[dict], baseline: dict, sources=None) -> list[str]:
    sources = raw_sites(root) if sources is None else sources
    errors = []
    approved = {}
    registered = {entry["id"]: entry for entry in entries}
    for exception in baseline["approved"]:
        fixture = registered.get(exception["surface"], {}).get("fixture")
        if not exception.get("rationale", "").strip() or not fixture:
            errors.append(f"{exception['source']}: raw exception requires rationale and registered fixture")
            continue
        key = (exception["source"], exception["site"])
        if key in approved:
            errors.append(f"{key}: duplicate raw exception")
        approved[key] = exception["count"]
    for source, sites in sources.items():
        legacy = baseline["legacy"].get(source, {})
        for site, count in sites.items():
            allowed = legacy.get(site, 0) + approved.get((source, site), 0)
            if count > allowed:
                errors.append(f"unregistered raw composition: {source}::{site} ({count} > {allowed})")
    return errors


def raw_self_test(entries: list[dict], baseline: dict) -> None:
    fixture = next(entry for entry in entries if entry.get("fixture"))
    source = fixture["id"].split("::")[0]
    site = 'approved_canvas | painter.text(position, align, label, font, color);'
    synthetic = {source: {site: 1}}
    assert check_raw(ROOT, entries, {"legacy": {}, "approved": []}, synthetic)
    approved = {"source": source, "site": site, "count": 1, "surface": fixture["id"], "rationale": "Custom canvas owns its coordinates; the registered fixture exercises its clipping boundary."}
    assert not check_raw(ROOT, entries, {"legacy": {}, "approved": [approved]}, synthetic)
    approved["rationale"] = ""
    assert check_raw(ROOT, entries, {"legacy": {}, "approved": [approved]}, synthetic)
    # Copying an approved site into a second location must be reviewed again.
    approved["rationale"] = "Custom canvas fixture"
    assert check_raw(ROOT, entries, {"legacy": {}, "approved": [approved]}, {source: {site: 2}})


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
        baseline = json.loads((ROOT / "src/ui/layout_raw_sites.json").read_text())
        errors = check(ROOT, entries) + check_raw(ROOT, entries, baseline)
        if "--test" in sys.argv:
            self_test(entries)
            raw_self_test(entries, baseline)
        if errors:
            print("\n".join(errors), file=sys.stderr)
            return 1
        print("UI surface registry and raw composition guard: PASS (explicit gaps remain listed in layout_surfaces.json)")
        return 0
    except (OSError, ValueError, KeyError, AssertionError) as error:
        print(f"UI layout guard failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
