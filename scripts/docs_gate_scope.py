#!/usr/bin/env python3
"""Path-specific docs-only gate decision (stints 1079 and 1102).

A diff may skip the build, suite and Clippy only when every changed path is
markdown that no Rust source in the tree can observe. A path is *binding*
(not skippable) when any tracked `.rs` file, in any workspace crate or
`build.rs`:

  * names it with `include_str!` or `include_bytes!` (shipped or test);
  * embeds a directory containing it with `include_dir!` (so `apps/**` and
    `sdk/python/plexi_sdk/**` are compiled into the binary);
  * names it, or a directory containing it, as a string literal inside an
    item that roots a path at `env!("CARGO_MANIFEST_DIR")`, which is how
    tests read and walk repository files at runtime;
  * names it by its exact repository path as a string literal anywhere; or
  * it sits under a directory that a `#[cfg(test)]` `include_str!`
    doc-sync assertion covers.

The binding set is recomputed from the tree on every call, so a new
`include_str!`, `include_dir!` or manifest-rooted read moves a path into the
required set with no list to edit. A hand-maintained list is a bug. Anything
that cannot be resolved fails closed.

`build.rs` hashes every tracked file into the build identity; that is not
behaviour, and installed validation still builds the exact head, so it does
not make markdown binding.

Usage:
  scripts/docs_gate_scope.py [--root DIR] [--show-embedded] [<changed path>...]
  scripts/docs_gate_scope.py [--root DIR] --is-embedded PATH
  scripts/docs_gate_scope.py [--root DIR] --proof REV <changed path>...

Prints a verdict and the reason for it, exits 0 when the gate may be skipped
and 1 when it must run. `--proof REV` prints the JSON proof an aggregate batch
record carries as `local.docs_proof`; `babysitter-policy.py` recomputes it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath

INCLUDE_FILE_RE = re.compile(r'\binclude_(?:str|bytes)!\s*\(\s*"([^"]+)"\s*\)')
INCLUDE_DIR_RE = re.compile(r'\binclude_dir!\s*\(\s*"([^"]+)"\s*\)')
MANIFEST_DIR_RE = re.compile(r'\benv!\s*\(\s*"CARGO_MANIFEST_DIR"\s*\)')
CFG_TEST_RE = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
FN_RE = re.compile(r"\bfn\s+[A-Za-z_]\w*")


class ScopeError(RuntimeError):
    pass


def _scan(text: str) -> tuple[str, list[tuple[int, str]]]:
    """Blank Rust string/char literal and comment contents, preserving offsets.

    Also returns each string literal as (offset, contents); literals inside
    comments are not code and are not returned.
    """
    out = list(text)
    literals: list[tuple[int, str]] = []
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
            literals.append((i, text[i + 1:j - 1]))
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
    return "".join(out), literals


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


def _item_span(blanked: str, start: int) -> tuple[int, int]:
    """Span of the item starting at `start`: through its body or its `;`."""
    brace = blanked.find("{", start)
    semi = blanked.find(";", start)
    if brace != -1 and (semi == -1 or brace < semi):
        return start, _closing_brace(blanked, brace)
    return start, (semi + 1 if semi != -1 else len(blanked))


def _test_regions(blanked: str) -> list[tuple[int, int]]:
    """Character spans of items annotated `#[cfg(test)]`."""
    return [_item_span(blanked, match.start()) for match in CFG_TEST_RE.finditer(blanked)]


def _manifest_regions(blanked: str, text: str) -> list[tuple[int, int]]:
    """Spans of the innermost `fn` (or statement) using `env!("CARGO_MANIFEST_DIR")`."""
    fns = [_item_span(blanked, match.start()) for match in FN_RE.finditer(blanked)]
    regions = []
    for match in MANIFEST_DIR_RE.finditer(text):
        enclosing = [span for span in fns if span[0] <= match.start() < span[1]]
        if enclosing:
            regions.append(max(enclosing, key=lambda span: span[0]))
        else:
            start = max(blanked.rfind(";", 0, match.start()), blanked.rfind("}", 0, match.start())) + 1
            regions.append(_item_span(blanked, start))
    return regions


def _git(root: Path, *args: str, stdin: bytes | None = None) -> bytes:
    try:
        return subprocess.run(("git", "-C", str(root), *args), input=stdin,
                              check=True, capture_output=True).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", b"") or b""
        raise ScopeError(f"git {' '.join(args[:2])} failed in {root}: "
                         f"{detail.decode(errors='replace').strip() or error}") from error


def tree(root: Path | str, rev: str | None = None) -> tuple[set[str], dict[str, str]]:
    """(tracked paths, {.rs path: text}) for the working tree or commit `rev`."""
    root = Path(root)
    if rev is None:
        listing = _git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard")
    else:
        listing = _git(root, "ls-tree", "-r", "-z", "--name-only", rev)
    tracked = {path for path in listing.decode().split("\0") if path}
    rust = sorted(path for path in tracked if path.endswith(".rs"))
    sources: dict[str, str] = {}
    if rev is None:
        for path in rust:
            try:
                sources[path] = (root / path).read_text(encoding="utf-8", errors="replace")
            except FileNotFoundError:
                continue  # deleted in the working tree but still in the index
            except OSError as error:
                raise ScopeError(f"cannot read {root / path}: {error}") from error
    elif rust:
        batch = _git(root, "cat-file", "--batch",
                     stdin="".join(f"{rev}:{path}\n" for path in rust).encode())
        offset = 0
        for path in rust:
            header_end = batch.index(b"\n", offset)
            header = batch[offset:header_end].split()
            if len(header) != 3 or header[1] != b"blob":
                raise ScopeError(f"cannot read {rev}:{path} in {root}")
            size = int(header[2])
            sources[path] = batch[header_end + 1:header_end + 1 + size].decode("utf-8", errors="replace")
            offset = header_end + 1 + size + 1
    return tracked, sources


def _crate_root(path: str, tracked: set[str]) -> PurePosixPath:
    parent = PurePosixPath(path).parent
    while True:
        if (parent / "Cargo.toml").as_posix() in tracked or parent.as_posix() == ".":
            return parent
        parent = parent.parent


def _normalize(path: PurePosixPath) -> str | None:
    """Lexically resolve `..`; None when the path escapes the repository."""
    parts: list[str] = []
    for part in path.parts:
        if part in ("", "."):
            continue
        if part == "..":
            if not parts:
                return None
            parts.pop()
        else:
            parts.append(part)
    return "/".join(parts) if parts else "."


def binding_paths(root: Path | str, rev: str | None = None) -> dict[str, dict[str, str]]:
    """Map each binding repo path (file or directory) to {"mode", "how"}.

    `mode` is "shipped" when any reference sits outside a `#[cfg(test)]` item,
    otherwise "test". Markdown files and directories only: a docs-only diff
    contains nothing else.
    """
    tracked, sources = tree(root, rev)
    dirs = {parent.as_posix() for path in tracked for parent in PurePosixPath(path).parents}
    dirs.discard(".")
    bindings: dict[str, dict[str, str]] = {}

    def bind(path: str | None, mode: str, how: str, origin: str) -> None:
        if path is None or path == ".":
            raise ScopeError(f"{origin}: {how} reference resolves outside the repository or to its root")
        if not (path.lower().endswith(".md") or path in dirs):
            return
        current = bindings.get(path)
        if current is None or (current["mode"] == "test" and mode == "shipped"):
            bindings[path] = {"mode": mode, "how": how}

    for source, text in sources.items():
        blanked, literals = _scan(text)
        tests = _test_regions(blanked)
        crate = _crate_root(source, tracked)

        def mode_at(index: int) -> str:
            return "test" if any(start <= index < end for start, end in tests) else "shipped"

        for match in INCLUDE_FILE_RE.finditer(text):
            target = _normalize(PurePosixPath(source).parent / match.group(1))
            if match.group(1).lower().endswith(".md"):
                bind(target, mode_at(match.start()), "include_str", source)
        for match in INCLUDE_DIR_RE.finditer(text):
            raw = match.group(1)
            if raw.startswith("$CARGO_MANIFEST_DIR"):
                bind(_normalize(crate / raw[len("$CARGO_MANIFEST_DIR"):].lstrip("/")),
                     mode_at(match.start()), "include_dir", source)
            elif "$" in raw or raw.startswith("/"):
                raise ScopeError(f"{source}: cannot resolve include_dir!(\"{raw}\")")
            else:
                for base in {crate, PurePosixPath(".")}:
                    bind(_normalize(base / raw), mode_at(match.start()), "include_dir", source)
        manifest = _manifest_regions(blanked, text)
        for offset, literal in literals:
            literal = literal.strip()
            if not literal or "\\" in literal or "{" in literal:
                continue
            rel = literal.lstrip("/")
            in_manifest_item = any(start <= offset < end for start, end in manifest)
            for base in {crate, PurePosixPath(".")}:
                candidate = _normalize(base / rel)
                if candidate in (None, "."):
                    continue
                if in_manifest_item and (candidate in tracked or candidate in dirs):
                    bind(candidate, mode_at(offset), "manifest-read", source)
                elif "/" in rel and candidate in tracked and candidate.lower().endswith(".md"):
                    bind(candidate, mode_at(offset), "path-literal", source)
    return bindings


def asserted_dirs(bindings: dict[str, dict[str, str]]) -> set[str]:
    """Directories covered by a `#[cfg(test)]` `include_str!` doc-sync assertion."""
    return {PurePosixPath(path).parent.as_posix() for path, info in bindings.items()
            if info["how"] == "include_str" and info["mode"] == "test"}


def _binding_for(path: str, bindings: dict[str, dict[str, str]]) -> tuple[str, dict[str, str]] | None:
    hits = [(key, info) for key, info in bindings.items() if path == key or path.startswith(key + "/")]
    return max(hits, key=lambda hit: len(hit[0]), default=None)


def decide_with(paths, bindings: dict[str, dict[str, str]]) -> tuple[bool, list[str]]:
    """Return (may_skip, reasons) against a computed binding set."""
    if not paths:
        return False, ["no changed paths provided"]
    dirs = asserted_dirs(bindings)
    reasons: list[str] = []
    for raw in paths:
        posix = PurePosixPath(str(raw).removeprefix("./")).as_posix()
        hit = _binding_for(posix, bindings)
        if not posix.lower().endswith(".md"):
            reasons.append(f"{posix}: not markdown")
        elif hit:
            key, info = hit
            where = posix if key == posix else f"{key}/"
            reasons.append(f"{posix}: {where} is bound by {info['how']} in {info['mode']} code")
        else:
            parent = PurePosixPath(posix).parent.as_posix()
            covered = next((d for d in sorted(dirs) if parent == d or parent.startswith(d + "/")), None)
            if covered:
                reasons.append(f"{posix}: under {covered}/, a directory a #[cfg(test)] "
                               "doc-sync assertion covers")
    return not reasons, reasons


def decide(paths, root: Path | str, rev: str | None = None) -> tuple[bool, list[str]]:
    try:
        bindings = binding_paths(root, rev)
    except ScopeError as error:
        return False, [f"binding set unavailable: {error}"]
    return decide_with(paths, bindings)


def proof(paths, root: Path | str, rev: str) -> dict:
    """The record-carried proof: verdict plus a digest of the binding set at commit `rev`."""
    rev = _git(Path(root), "rev-parse", "--verify", "--end-of-options", f"{rev}^{{commit}}").decode().strip()
    bindings = binding_paths(root, rev)
    skip, reasons = decide_with(paths, bindings)
    lines = "".join(f"{info['mode']}\t{info['how']}\t{path}\n" for path, info in sorted(bindings.items()))
    return {"rev": rev, "paths": sorted(paths), "skip": skip, "reasons": reasons,
            "binding_digest": hashlib.sha256(lines.encode()).hexdigest()}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", default=None)
    parser.add_argument("--show-embedded", action="store_true")
    parser.add_argument("--is-embedded", metavar="PATH",
                        help="exit 0 when PATH is binding (not skippable as docs), else 1")
    parser.add_argument("--proof", metavar="REV",
                        help="print the JSON proof for the changed paths at commit REV")
    parser.add_argument("paths", nargs="*")
    args = parser.parse_args(argv)
    root = Path(args.root) if args.root else Path(__file__).resolve().parent.parent
    try:
        if args.proof is not None:
            print(json.dumps(proof(args.paths, root, args.proof), indent=2))
            return 0
        if args.is_embedded is not None:
            return 1 if decide([args.is_embedded], root)[0] else 0
        if args.show_embedded:
            for path, info in sorted(binding_paths(root).items()):
                print(f"{info['mode']}\t{info['how']}\t{path}")
            return 0
    except ScopeError as error:
        print(f"docs_gate_scope: {error}", file=sys.stderr)
        return 1
    skippable, reasons = decide(args.paths, root)
    if skippable:
        print("docs-only gate: skip")
        print(f"reason: all {len(args.paths)} changed path(s) are markdown no Rust source can observe")
        print("docs-only, no build")
        return 0
    print("docs-only gate: required")
    for reason in reasons:
        print(f"reason: {reason}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
