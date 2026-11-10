#!/usr/bin/env python3
"""Verify the host surface census; explicit gaps are visible debt, never proof."""
from __future__ import annotations

import json
import re
import sys
import subprocess
import tempfile
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



BUTTON_BASELINE = "src/ui/button_raw_sites.json"
BUTTON_EXCEPTION = "src/app/text_editor_app.rs"
BUTTON_CALL = re.compile(r"\.\s*(?:button|small_button|selectable_label)\s*\(|\b(?:egui\s*::\s*)?Button\s*::\s*new\s*\(")


def rust_code(source: str) -> str:
    """Mask comments and literals, retaining positions for function attribution.

    Includes raw strings and nested block comments; generated plugin templates
    containing .button() are data, not host widget calls.
    """
    token = re.compile(r'//[^\n]*|/\*|r(\#*)"|"|\'(?:\\.|[^\'\\\n])\'')
    pieces = list(source)
    position = 0
    while match := token.search(source, position):
        start, end = match.span()
        value = match.group()
        if value == "/*":
            depth = 1
            while depth and end < len(source):
                next_token = re.search(r"/\*|\*/", source[end:])
                if next_token is None:
                    end = len(source)
                    break
                depth += 1 if next_token.group() == "/*" else -1
                end += next_token.end()
        elif value.startswith("r"):
            closing = '"' + match.group(1)
            found = source.find(closing, end)
            end = len(source) if found < 0 else found + len(closing)
        elif value == '"':
            while end < len(source):
                if source[end] == "\\":
                    end += 2
                elif source[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
        for index in range(start, min(end, len(source))):
            if pieces[index] != "\n":
                pieces[index] = " "
        position = end
    return "".join(pieces)


def button_sites(root: Path) -> dict[str, dict[str, int]]:
    result = {}
    for path in sorted((root / "src").rglob("*.rs")):
        relative = path.relative_to(root).as_posix()
        if relative == "src/ui/button.rs":
            continue
        source = rust_code(path.read_text())
        sites = {}
        for match in BUTTON_CALL.finditer(source):
            functions = re.findall(r"\bfn\s+(\w+)\s*[<(]", source[:match.start()])
            symbol = functions[-1] if functions else "module"
            call = re.sub(r"\s+", "", match.group())
            site = f"{symbol} | {call}"
            sites[site] = sites.get(site, 0) + 1
        if sites:
            result[relative] = sites
    return result


def check_buttons(root: Path, baseline: dict) -> list[str]:
    errors = []
    if set(baseline) - {BUTTON_EXCEPTION}:
        errors.append("raw button baseline may only contain the stint 1209 text editor exception")
    for source, sites in button_sites(root).items():
        for site, count in sites.items():
            allowed = baseline.get(source, {}).get(site, 0)
            if count > allowed:
                errors.append(f"raw button outside kit: {source}::{site} ({count} > {allowed})")
    # A deleted exception must be removed too; the baseline cannot mask a
    # later reintroduction, and existing entries cannot grow relative to Alpha.
    current = button_sites(root)
    for source, sites in baseline.items():
        for site, count in sites.items():
            if count != current.get(source, {}).get(site, 0):
                errors.append(f"stale raw button baseline: {source}::{site}")
    return errors


def check_button_baseline_shrink(root: Path, baseline: dict) -> list[str]:
    previous = subprocess.run(["git", "show", f"origin/alpha:{BUTTON_BASELINE}"], cwd=root, text=True, capture_output=True)
    if previous.returncode:
        # On introduction, bound the exception by the actual Alpha source,
        # rather than trusting a newly supplied baseline to define its own cap.
        editor = subprocess.run(["git", "show", f"origin/alpha:{BUTTON_EXCEPTION}"], cwd=root, text=True, capture_output=True)
        if editor.returncode:
            return [f"cannot verify raw button baseline against origin/alpha: {editor.stderr.strip()}"]
        with tempfile.TemporaryDirectory(prefix="1211-button-alpha-") as directory:
            scratch = Path(directory)
            path = scratch / BUTTON_EXCEPTION
            path.parent.mkdir(parents=True)
            path.write_text(editor.stdout)
            old = button_sites(scratch)
    else:
        old = json.loads(previous.stdout)["baseline"]
    return [f"raw button baseline grew: {source}::{site}"
            for source, sites in baseline.items() for site, count in sites.items()
            if count > old.get(source, {}).get(site, 0)]


def button_self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="1211-button-guard-") as directory:
        root = Path(directory)
        (root / "src/deep").mkdir(parents=True)
        scratch = root / "src/deep/scratch.rs"
        for raw in ['ui.button("New")', 'ui.small_button("New")', 'ui.selectable_label(false, "New")', 'egui::Button::new("New")', 'Button :: new("New")', 'ui\n .button\n ("New")']:
            scratch.write_text(f"fn scratch() {{ {raw}; }}")
            assert check_buttons(root, {}), f"checker accepted {raw}"
        scratch.write_text('fn scratch() { /* nested /* ui.button("x") */ comment */ let template = r##"ui.button("x")"##; // ui.button("x")\n }')
        assert not check_buttons(root, {})
        scratch.write_text('fn scratch() { crate::ui::button::KitButton::new("New", colors); }')
        assert not check_buttons(root, {})
        assert check_buttons(root, {BUTTON_EXCEPTION: {"removed | .button(": 1}})
    with tempfile.TemporaryDirectory(prefix="1211-button-baseline-") as directory:
        root = Path(directory)
        editor = root / BUTTON_EXCEPTION
        editor.parent.mkdir(parents=True)
        editor.write_text('fn editor() { ui.button("Existing"); }')
        def git(*args):
            return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True)
        git("init", "-q")
        git("add", ".")
        git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "baseline")
        git("update-ref", "refs/remotes/origin/alpha", "HEAD")
        allowed = {BUTTON_EXCEPTION: {"editor | .button(": 1}}
        grown = {BUTTON_EXCEPTION: {"editor | .button(": 2}}
        assert not check_button_baseline_shrink(root, allowed)
        assert check_button_baseline_shrink(root, grown), "initial baseline accepted a new raw button"
        baseline_file = root / BUTTON_BASELINE
        baseline_file.parent.mkdir(parents=True)
        baseline_file.write_text(json.dumps({"baseline": allowed}))
        git("add", ".")
        git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "record exception")
        git("update-ref", "refs/remotes/origin/alpha", "HEAD")
        assert not check_button_baseline_shrink(root, {})
        assert check_button_baseline_shrink(root, grown)
    print("Raw button negative controls: PASS (scratch ui.button and baseline growth rejected)")

def main() -> int:
    try:
        entries = json.loads(REGISTRY.read_text())["surfaces"]
        baseline = json.loads((ROOT / "src/ui/layout_raw_sites.json").read_text())
        button_baseline = json.loads((ROOT / BUTTON_BASELINE).read_text())["baseline"]
        errors = check(ROOT, entries) + check_raw(ROOT, entries, baseline) + check_buttons(ROOT, button_baseline) + check_button_baseline_shrink(ROOT, button_baseline)
        if "--test" in sys.argv:
            self_test(entries)
            raw_self_test(entries, baseline)
            button_self_test()
        if errors:
            print("\n".join(errors), file=sys.stderr)
            return 1
        print(f"Raw button baseline exception: {BUTTON_EXCEPTION} (stint 1209); no other raw sites allowed")
        print("UI surface registry and raw composition guard: PASS (explicit gaps remain listed in layout_surfaces.json)")
        return 0
    except (OSError, ValueError, KeyError, AssertionError) as error:
        print(f"UI layout guard failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
