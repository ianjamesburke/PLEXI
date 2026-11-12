#!/usr/bin/env python3
"""Path-specific docs-only gate decision (stint 1079).

A markdown-only diff may skip the local pre-push gate only when every changed
path is markdown that is *provably* not compiled in and not asserted on:

  * not reachable from `include_str!` anywhere under `src/` — both shipped and
    `#[cfg(test)]` sites bind, because a doc-sync assertion makes editing the
    file fail a test; and
  * not under a directory that a `#[cfg(test)]` doc-sync assertion covers.

The embedded set is recomputed from `src/` on every call, so adding an
`include_str!` of a markdown file moves that path into the required-gate set
with no list to edit. A hand-maintained list is a bug.

Usage:
  scripts/docs-gate-scope.py [--root DIR] [--show-embedded] [<changed path>...]

Prints a verdict and the reason for it, exits 0 when the gate may be skipped
and 1 when it must run. This decides the *local pre-push* decision only; the
aggregate record carve-out is a separate contract (stint 1102).
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path, PurePosixPath

INCLUDE_RE = re.compile(r'include_str!\s*\(\s*"([^"]+)"\s*\)')
CFG_TEST_RE = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
DOC_SYNC_MODES = ("shipped", "test")


def _blank_literals(text: str) -> str:
    """Blank Rust string/char literal and comment contents, preserving offsets."""
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            j = n if j == -1 else j
            out[i:j] = [" "] * (j - i)
            i = j
        elif c == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            j = n if j == -1 else j + 2
            for k in range(i, j):
                if text[k] != "\n":
                    out[k] = " "
            i = j
        elif c == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            for k in range(i, j):
                if text[k] != "\n":
                    out[k] = " "
            i = j
        elif c == "'":
            j = i + 1
            if j < n and text[j] == "\\":
                j += 2
            elif j < n:
                j += 1
            if j < n and text[j] == "'":
                j += 1
            for k in range(i, j):
                if text[k] != "\n":
                    out[k] = " "
            i = j
        else:
            i += 1
    return "".join(out)


def _closing_brace(blanked: str, open_index: int) -> int:
    depth = 0
    for i in range(open_index, len(blanked)):
        if blanked[i] == "{":
            depth += 1
        elif blanked[i] == "}":
            depth -= 1
            if depth == 0:
                return i + 1
    return len(blanked)


def _test_regions(blanked: str) -> list[tuple[int, int]]:
    """Character spans of items annotated `#[cfg(test)]`."""
    regions = []
    for match in CFG_TEST_RE.finditer(blanked):
        i = match.end()
        n = len(blanked)
        while i < n and blanked[i].isspace():
            i += 1
        for keyword in ("pub", "unsafe", "async"):
            if blanked.startswith(keyword, i):
                i += len(keyword)
                while i < n and blanked[i].isspace():
                    i += 1
        brace = blanked.find("{", i)
        semi = blanked.find(";", i)
        if brace != -1 and (semi == -1 or brace < semi):
            regions.append((match.start(), _closing_brace(blanked, brace)))
        elif semi != -1:
            regions.append((match.start(), semi + 1))
    return regions


def embedded_markdown(root: Path | str) -> dict[str, str]:
    """Map root-relative markdown paths to 'shipped' or 'test'.

    A path is 'shipped' if any `include_str!` of it lives outside a
    `#[cfg(test)]` item, otherwise 'test'. Shipped wins when both exist.
    """
    root = Path(root)
    embedded: dict[str, str] = {}
    for source in sorted(root.glob("src/**/*.rs")):
        text = source.read_text(encoding="utf-8", errors="replace")
        blanked = _blank_literals(text)
        regions = _test_regions(blanked)
        for match in INCLUDE_RE.finditer(text):
            target = match.group(1)
            if not target.lower().endswith(".md"):
                continue
            resolved = (source.parent / target).resolve()
            try:
                rel = PurePosixPath(resolved.relative_to(root.resolve())).as_posix()
            except ValueError:
                rel = PurePosixPath(resolved.as_posix()).as_posix()
            in_test = any(start <= match.start() < end for start, end in regions)
            mode = "test" if in_test else "shipped"
            if embedded.get(rel) != "shipped":
                embedded[rel] = mode
    return embedded


def asserted_dirs(embedded: dict[str, str]) -> set[str]:
    """Directories covered by a `#[cfg(test)]` doc-sync assertion."""
    dirs = set()
    for path, mode in embedded.items():
        if mode == "test":
            parent = PurePosixPath(path).parent.as_posix()
            dirs.add("." if parent == "." else parent)
    return dirs


def decide(paths, root: Path | str) -> tuple[bool, list[str]]:
    """Return (may_skip, reasons). Empty input is never skippable."""
    embedded = embedded_markdown(root)
    dirs = asserted_dirs(embedded)
    reasons: list[str] = []
    for raw in paths:
        path = PurePosixPath(str(raw).lstrip("./"))
        posix = path.as_posix()
        if not posix.lower().endswith(".md"):
            reasons.append(f"{posix} — not markdown")
        elif posix in embedded:
            mode = embedded[posix]
            why = (
                "embedded in shipped code"
                if mode == "shipped"
                else "embedded in a #[cfg(test)] doc-sync assertion"
            )
            reasons.append(f"{posix} — {why}")
        else:
            parent = path.parent.as_posix()
            hit = next(
                (d for d in sorted(dirs) if parent == d or parent.startswith(d + "/")),
                None,
            )
            if hit:
                reasons.append(
                    f"{posix} — under {hit}/, a directory a #[cfg(test)] "
                    "doc-sync assertion covers"
                )
    if not reasons and len(paths) > 0:
        return True, []
    if not reasons:
        return False, ["no changed paths provided"]
    return False, reasons


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=None)
    parser.add_argument("--show-embedded", action="store_true")
    parser.add_argument(
        "--is-embedded",
        metavar="PATH",
        help="exit 0 when PATH is reachable from include_str!, else 1",
    )
    parser.add_argument("paths", nargs="*")
    args = parser.parse_args(argv)
    root = Path(args.root) if args.root else Path(__file__).resolve().parent.parent
    if args.is_embedded is not None:
        target = PurePosixPath(str(args.is_embedded).lstrip("./")).as_posix()
        return 0 if target in embedded_markdown(root) else 1
    if args.show_embedded:
        for path, mode in sorted(embedded_markdown(root).items()):
            print(f"{mode}\t{path}")
        return 0
    skippable, reasons = decide(args.paths, root)
    if skippable:
        print("docs-only gate: skip")
        print(
            f"reason: all {len(args.paths)} changed path(s) are markdown not "
            "embedded via include_str! and not under a doc-sync-asserted directory"
        )
        print("docs-only, no build")
        return 0
    print("docs-only gate: required")
    for reason in reasons:
        print(f"reason: {reason}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
