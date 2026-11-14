#!/usr/bin/env python3
"""Path-specific docs-only gate decision (stints 1079 and 1102).

A diff may skip the build, suite and Clippy only when every changed path is
markdown that no Rust source in the tree can observe. A path is *binding*
(not skippable) when any tracked `.rs` file, in any workspace crate or
`build.rs`:

  * names it with `include_str!` or `include_bytes!` (shipped or test);
  * embeds a directory containing it with `include_dir!` (so `plugins/**` and
    `sdk/python/plexi_sdk/**` are compiled into the binary);
  * names it, or a directory containing it, as a string literal inside the
    `fn` (or statement) that names `"CARGO_MANIFEST_DIR"` (`env!`,
    `std::env::var`, `var_os`), which is how tests and `build.rs` read and
    walk repository files at runtime;
  * names it by its exact repository path as a string literal anywhere.

A file-level reference binds that file only, never its siblings. A walked or
listed directory binds every file under it, whatever the walk filters on.

Paths resolve through tracked symlinks, and a symlink inside a bound
directory binds its target. The binding set is recomputed from the tree on
every call, so a new reference moves a path into the required set with no
list to edit. A hand-maintained list is a bug.

These fail closed (the gate is required): an `include_*!` argument that is
neither a literal nor `concat!(env!("CARGO_MANIFEST_DIR"), "...")`, an
`include_dir!` with another variable, a reference that escapes the
repository, a templated `.md` literal with no static directory in a
manifest-rooted item, a scanner that loses literal sync, and any git error.

Known limits (not detected): a helper that returns the manifest root while
its caller names the path (`fn root()` then `read_dir(root().join("x"))`),
and cwd-relative reads (`read_to_string("NORTH_STAR.md")`).

`build.rs` hashes every tracked file into the build identity; that is not
behaviour, and installed validation still builds the exact head, so it does
not make markdown binding.

Usage:
  scripts/docs_gate_scope.py [--root DIR] [--show-embedded] [<changed path>...]
  scripts/docs_gate_scope.py [--root DIR] --proof HEAD --base BASE <changed path>...

Prints a verdict and the reason for it, exits 0 only when the gate may be
skipped; any other exit, including a crash, means it must run. `--proof`
prints the JSON proof an aggregate batch record carries as
`local.docs_proof`; `babysitter-policy.py` recomputes it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath

INCLUDE_RE = re.compile(r"\binclude_(str|bytes|dir)!\s*\(")
CONCAT_RE = re.compile(r"concat!\s*\(")
RAW_RE = re.compile(r'[bc]?r(#*)"')
CFG_TEST_RE = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
FN_RE = re.compile(r"\bfn\s+[A-Za-z_]\w*")
MANIFEST = "CARGO_MANIFEST_DIR"


class ScopeError(RuntimeError):
    pass


def _scan(text: str) -> tuple[str, dict[int, str], list[tuple[int, int]]]:
    """Blank Rust literal and comment contents, preserving offsets.

    Returns (blanked, {literal start offset: contents}, comment spans). Raw
    strings (`r"…"`, `r#"…"#`, `br#"…"#`) are literals; a literal's offset is
    its first character, prefix included.
    """
    out = list(text)
    literals: dict[int, str] = {}
    comments: list[tuple[int, int]] = []
    i, n = 0, len(text)

    def blank(start: int, end: int) -> None:
        for k in range(start, end):
            if text[k] != "\n":
                out[k] = " "

    while i < n:
        c = text[i]
        raw = RAW_RE.match(text, i) if c in "bcr" and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")) else None
        if c == "/" and text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j == -1 else j
            comments.append((i, j))
            blank(i, j)
            i = j
        elif c == "/" and text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            comments.append((i, j))
            blank(i, j)
            i = j
        elif raw:
            close = '"' + raw.group(1)
            end = text.find(close, raw.end())
            if end == -1:
                raise ScopeError("unterminated raw string literal")
            literals[i] = text[raw.end():end]
            blank(i, end + len(close))
            i = end + len(close)
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
            literals[i] = text[i + 1:j - 1]
            blank(i, j)
            i = j
        elif c == "'":
            j = i + 1
            if j < n and text[j] == "\\":
                j += 2
            elif j < n:
                j += 1
            if j < n and text[j] == "'":
                j += 1
            blank(i, j)
            i = j
        else:
            i += 1
    return "".join(out), literals, comments


def _closing(blanked: str, open_index: int, pair: str = "{}") -> int:
    depth = 0
    for i in range(open_index, len(blanked)):
        if blanked[i] == pair[0]:
            depth += 1
        elif blanked[i] == pair[1]:
            depth -= 1
            if depth == 0:
                return i + 1
    return len(blanked)


def _item_span(blanked: str, start: int) -> tuple[int, int]:
    """Span of the item starting at `start`: through its body or its `;`."""
    brace = blanked.find("{", start)
    semi = blanked.find(";", start)
    if brace != -1 and (semi == -1 or brace < semi):
        return start, _closing(blanked, brace)
    return start, (semi + 1 if semi != -1 else len(blanked))


def _git(root: Path, *args: str, stdin: bytes | None = None) -> bytes:
    try:
        return subprocess.run(("git", "-C", str(root), *args), input=stdin,
                              check=True, capture_output=True).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", b"") or b""
        raise ScopeError(f"git {' '.join(args[:2])} failed in {root}: "
                         f"{detail.decode(errors='replace').strip() or error}") from error


def _cat(root: Path, rev: str, paths: list[str]) -> dict[str, bytes]:
    if not paths:
        return {}
    batch = _git(root, "cat-file", "--batch", stdin="".join(f"{rev}:{p}\n" for p in paths).encode())
    blobs, offset = {}, 0
    for path in paths:
        header_end = batch.find(b"\n", offset)
        header = batch[offset:header_end].split() if header_end != -1 else []
        if len(header) != 3 or header[1] != b"blob":
            raise ScopeError(f"cannot read {rev}:{path} in {root}")
        size = int(header[2])
        blobs[path] = batch[header_end + 1:header_end + 1 + size]
        offset = header_end + 1 + size + 1
    return blobs


def tree(root: Path | str, rev: str | None = None):
    """(tracked paths, {.rs path: text}, {symlink path: target}) for the work tree or `rev`."""
    root = Path(root)
    if rev is None:
        listing = _git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard")
        tracked = {path for path in listing.decode().split("\0") if path}
        links, sources = {}, {}
        for path in sorted(tracked):
            full = root / path
            try:
                if full.is_symlink():
                    links[path] = os.readlink(full)
                elif path.endswith(".rs"):
                    sources[path] = full.read_text(encoding="utf-8", errors="replace")
            except FileNotFoundError:
                continue  # deleted in the work tree but still in the index
            except OSError as error:
                raise ScopeError(f"cannot read {full}: {error}") from error
        return tracked, sources, links
    tracked, link_paths = set(), []
    for entry in _git(root, "ls-tree", "-r", "-z", rev).decode().split("\0"):
        if not entry:
            continue
        meta, path = entry.split("\t", 1)
        tracked.add(path)
        if meta.split()[0] == "120000":
            link_paths.append(path)
    rust = sorted(path for path in tracked if path.endswith(".rs") and path not in link_paths)
    sources = {path: blob.decode("utf-8", errors="replace") for path, blob in _cat(root, rev, rust).items()}
    links = {path: blob.decode() for path, blob in _cat(root, rev, sorted(link_paths)).items()}
    return tracked, sources, links


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


def _resolve(path: str | None, links: dict[str, str]) -> str | None:
    """Follow tracked symlinks in any prefix of `path`."""
    for _ in range(40):
        if path is None or path == ".":
            return path
        parts = path.split("/")
        for index in range(1, len(parts) + 1):
            prefix = "/".join(parts[:index])
            if prefix in links:
                target = links[prefix]
                if target.startswith("/"):
                    return None
                path = _normalize(PurePosixPath(prefix).parent / target / "/".join(parts[index:]))
                break
        else:
            return path
    raise ScopeError(f"symlink loop resolving {path}")


def binding_paths(root: Path | str, rev: str | None = None) -> dict[str, dict]:
    """Map each binding repo path (file or directory) to {"mode", "how"}.

    `mode` is "shipped" when any reference sits outside a `#[cfg(test)]` item,
    otherwise "test". Markdown files and directories only: a docs-only diff
    contains nothing else.
    """
    tracked, sources, links = tree(root, rev)
    dirs = {parent.as_posix() for path in tracked for parent in PurePosixPath(path).parents}
    dirs.discard(".")
    bindings: dict[str, dict] = {}

    def bind(path: str | None, mode: str, how: str, origin: str) -> None:
        resolved = _resolve(path, links)
        if resolved is None or resolved == ".":
            raise ScopeError(f"{origin}: {how} reference {path!r} resolves outside the repository or to its root")
        # An include_dir! target binds even when empty: deleting its last file breaks the build.
        if how != "include_dir" and not (resolved.lower().endswith(".md") or resolved in dirs):
            return
        current = bindings.get(resolved)
        if current is None or (current["mode"] == "test" and mode == "shipped"):
            bindings[resolved] = {"mode": mode, "how": how}

    for source, text in sources.items():
        blanked, literals, comments = _scan(text)
        crate = _crate_root(source, tracked)
        tests = [_item_span(blanked, m.start()) for m in CFG_TEST_RE.finditer(blanked)]
        fns = [_item_span(blanked, m.start()) for m in FN_RE.finditer(blanked)]

        def mode_at(index: int) -> str:
            return "test" if any(start <= index < end for start, end in tests) else "shipped"

        def literal_after(index: int) -> tuple[int, str] | None:
            while index < len(text) and text[index].isspace():
                index += 1
            return (index, literals[index]) if index in literals else None

        for match in INCLUDE_RE.finditer(blanked):
            kind, mode = match.group(1), mode_at(match.start())
            found = literal_after(match.end())
            if found is not None:
                raw = found[1]
                if kind == "dir":
                    if raw.startswith("$" + MANIFEST):
                        bind(_normalize(crate / raw[len(MANIFEST) + 1:].lstrip("/")), mode, "include_dir", source)
                    elif "$" in raw or raw.startswith("/"):
                        raise ScopeError(f"{source}: cannot resolve include_dir!(\"{raw}\")")
                    else:
                        for base in {crate, PurePosixPath(".")}:
                            bind(_normalize(base / raw), mode, "include_dir", source)
                elif raw.lower().endswith(".md"):
                    bind(_normalize(PurePosixPath(source).parent / raw), mode, "include_str", source)
                continue
            rest = match.end()
            while rest < len(blanked) and blanked[rest].isspace():
                rest += 1
            concat = CONCAT_RE.match(blanked, rest)
            if concat:
                end = _closing(blanked, concat.end() - 1, "()")
                parts = [value for offset, value in sorted(literals.items()) if concat.end() <= offset < end]
                if len(parts) == 2 and parts[0] == MANIFEST:
                    target = _normalize(crate / parts[1].lstrip("/"))
                    if kind == "dir" or parts[1].lower().endswith(".md"):
                        bind(target, mode, "include_dir" if kind == "dir" else "include_str", source)
                    continue
            raise ScopeError(f"{source}: cannot resolve include_{kind}! argument")

        manifest = []
        for offset, value in literals.items():
            if value == MANIFEST:
                enclosing = [span for span in fns if span[0] <= offset < span[1]]
                manifest.append(max(enclosing, key=lambda span: span[0]) if enclosing else _item_span(
                    blanked, max(blanked.rfind(";", 0, offset), blanked.rfind("}", 0, offset)) + 1))
        # Every quoted CARGO_MANIFEST_DIR in code must be a literal the scanner saw;
        # otherwise it lost string/code sync and the manifest rule would be blind.
        for match in re.finditer(f'"{MANIFEST}"', text):
            if blanked[match.start() + 1] != " ":
                raise ScopeError(f"{source}: scanner lost literal sync near offset {match.start()}")

        for offset, value in literals.items():
            literal = value.strip()
            if not literal:
                continue
            in_manifest = any(start <= offset < end for start, end in manifest)
            if "{" in literal or "\\" in literal:
                if in_manifest and ".md" in literal.lower():
                    prefix = literal[:min(i for i in (literal.find("{"), literal.find("\\")) if i != -1)]
                    static = prefix.lstrip("/").rpartition("/")[0]
                    if not static:
                        raise ScopeError(f"{source}: cannot resolve templated markdown path {literal!r}")
                    for base in {crate, PurePosixPath(".")}:
                        candidate = _normalize(base / static)
                        if candidate not in (None, ".") and candidate in dirs:
                            bind(candidate, mode_at(offset), "manifest-read", source)
                continue
            rel = literal.lstrip("/")
            for base in {crate, PurePosixPath(".")}:
                candidate = _normalize(base / rel)
                if candidate in (None, "."):
                    continue
                if in_manifest and (candidate in tracked or candidate in dirs):
                    bind(candidate, mode_at(offset), "manifest-read", source)
                elif "/" in rel and candidate in tracked and candidate.lower().endswith(".md"):
                    bind(candidate, mode_at(offset), "path-literal", source)

    # A symlink inside a bound directory embeds or exposes its target too.
    while True:
        before = len(bindings)
        for link in sorted(links):
            hit = _binding_for(link, bindings)
            if hit:
                bind(link, hit[1]["mode"], hit[1]["how"], f"symlink {link}")
        if len(bindings) == before:
            return bindings


def _binding_for(path: str, bindings: dict[str, dict]):
    """The most specific binding covering `path`."""
    hits = [(key, info) for key, info in bindings.items() if path == key or path.startswith(key + "/")]
    return max(hits, key=lambda hit: len(hit[0]), default=None)


def decide_with(paths, bindings: dict[str, dict]) -> tuple[bool, list[str]]:
    """Return (may_skip, reasons) against a computed binding set."""
    if not paths:
        return False, ["no changed paths provided"]
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
    return not reasons, reasons


def decide(paths, root: Path | str, rev: str | None = None) -> tuple[bool, list[str]]:
    try:
        bindings = binding_paths(root, rev)
    except ScopeError as error:
        return False, [f"binding set unavailable: {error}"]
    return decide_with(paths, bindings)


def _commit(root: Path, rev: str) -> str:
    return _git(root, "rev-parse", "--verify", "--end-of-options", f"{rev}^{{commit}}").decode().strip()


def _digest(bindings: dict[str, dict]) -> str:
    lines = "".join(f"{info['mode']}\t{info['how']}\t{path}\n" for path, info in sorted(bindings.items()))
    return hashlib.sha256(lines.encode()).hexdigest()


def proof(paths, root: Path | str, head: str, base: str) -> dict:
    """The record-carried proof. The squash lands on `base`, so the paths must be
    skippable against the binding set of both trees."""
    root = Path(root)
    head, base = _commit(root, head), _commit(root, base)
    at_head, at_base = binding_paths(root, head), binding_paths(root, base)
    head_skip, head_reasons = decide_with(paths, at_head)
    base_skip, base_reasons = decide_with(paths, at_base)
    return {"head": head, "base": base, "paths": sorted(paths), "skip": head_skip and base_skip,
            "reasons": head_reasons + [f"at base: {reason}" for reason in base_reasons],
            "digests": {"head": _digest(at_head), "base": _digest(at_base)}}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", default=None)
    parser.add_argument("--show-embedded", action="store_true")
    parser.add_argument("--proof", metavar="HEAD", help="print the JSON proof for the changed paths")
    parser.add_argument("--base", metavar="BASE", help="the commit the batch lands on (with --proof)")
    parser.add_argument("paths", nargs="*")
    args = parser.parse_args(argv)
    root = Path(args.root) if args.root else Path(__file__).resolve().parent.parent
    try:
        if args.proof is not None:
            if args.base is None:
                parser.error("--proof requires --base")
            print(json.dumps(proof(args.paths, root, args.proof, args.base), indent=2))
            return 0
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
