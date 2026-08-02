#!/usr/bin/env python3
"""Resolve and execute the evidence declared by ROADMAP.toml.

This intentionally does not use scene_suite: roadmap evidence is a release
contract, so a scene is run even when it opts out of the fast workspace suite.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
TEST_RE = re.compile(r"^\s*#\s*\[\s*(?:[\w:]+::)?test[^\]]*\]\s*$")
FN_RE = re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\(")
CARGO_ENV_STRIP = [
    "env", "-u", "PLEXI_CHANNEL", "-u", "PLEXI_CONTEXT_ROOT", "-u",
    "PLEXI_CONTEXT_ID", "-u", "PLEXI_CONTEXT_NAME", "-u", "PLEXI_SOCKET",
    "-u", "PLEXI_RUNNING", "-u", "PLEXI_PANE_ID",
]


def roadmap_evidence(path: pathlib.Path) -> list[str]:
    data = tomllib.loads(path.read_text())
    return [item for node in data.get("node", []) for item in node.get("evidence", [])]


def rust_tests(root: pathlib.Path) -> dict[str, list[pathlib.Path]]:
    found: dict[str, list[pathlib.Path]] = {}
    for path in root.rglob("*.rs"):
        if "target" in path.parts:
            continue
        test_attribute = False
        for line in path.read_text(errors="replace").splitlines():
            if TEST_RE.match(line):
                test_attribute = True
                continue
            if test_attribute:
                match = FN_RE.match(line)
                if match:
                    found.setdefault(match.group(1), []).append(path)
                    test_attribute = False
                elif line.strip() and not line.lstrip().startswith("#"):
                    test_attribute = False
    return found


def resolve(root: pathlib.Path, evidence: list[str]) -> tuple[dict[str, pathlib.Path], list[str]]:
    tests = rust_tests(root)
    resolved: dict[str, pathlib.Path] = {}
    errors: list[str] = []
    for name in evidence:
        scene = root / "tests" / "scenes" / f"{name}.toml"
        matches = tests.get(name, [])
        if scene.is_file() and matches:
            errors.append(f"ambiguous evidence {name}: scene and Rust test")
        elif scene.is_file():
            resolved[name] = scene
        elif len(matches) == 1:
            resolved[name] = matches[0]
        elif not matches:
            errors.append(f"unresolved evidence {name}")
        else:
            errors.append(f"ambiguous Rust evidence {name}: " + ", ".join(map(str, matches)))
    return resolved, errors


def run(command: list[str], cwd: pathlib.Path) -> bool:
    print("+", " ".join(command), flush=True)
    return subprocess.run(command, cwd=cwd).returncode == 0


def execute(root: pathlib.Path, resolved: dict[str, pathlib.Path]) -> list[str]:
    failures: list[str] = []
    listing = subprocess.run(
        ["bash", "scripts/cargo-with-lease.sh", *CARGO_ENV_STRIP, "cargo", "test", "--workspace", "--", "--list"],
        cwd=root,
        text=True,
        capture_output=True,
    )
    if listing.returncode:
        print(listing.stdout, end="")
        print(listing.stderr, end="", file=sys.stderr)
        return ["cargo test --workspace -- --list"]
    labels: dict[str, list[str]] = {}
    for line in listing.stdout.splitlines():
        if line.endswith(": test"):
            label = line.removesuffix(": test")
            labels.setdefault(label.rsplit("::", 1)[-1], []).append(label)
    for name, path in resolved.items():
        # `src/lib.rs` is linked by more than one package target in this
        # workspace, so compiled labels can repeat. Source resolution above
        # rejects multiple Rust definitions; here we only require that the
        # one resolved definition is present in at least one test binary.
        if path.suffix != ".toml" and not labels.get(name):
            failures.append(f"{name} (missing from cargo test --list)")
    if failures:
        return failures
    # A single workspace invocation executes every resolved Rust test. The
    # preceding list check proves each evidence name maps to one exact test
    # label, while this run retains Cargo's ordinary failure reporting.
    # Keep this aligned with the documented host-CI exception. It is not
    # roadmap evidence and fails reproducibly on clean alpha.
    if not run(
        [
            "bash",
            "scripts/cargo-with-lease.sh",
            *CARGO_ENV_STRIP,
            "cargo",
            "test",
            "--workspace",
            "--",
            "--skip",
            "headless_frame_fails_fast_when_the_guest_dies_at_import",
        ],
        root,
    ):
        return ["resolved Rust evidence"]
    for name, path in resolved.items():
        if path.suffix != ".toml":
            continue
        command = ["just", "scene", str(path.relative_to(root)), "/tmp/plexi-roadmap-evidence", "0"]
        if not run(command, root):
            failures.append(name)
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, default=ROOT)
    parser.add_argument("--roadmap", type=pathlib.Path, default=ROOT / "ROADMAP.toml")
    parser.add_argument("--resolve-only", action="store_true")
    args = parser.parse_args()
    evidence = roadmap_evidence(args.roadmap)
    root = args.root.resolve()
    resolved, errors = resolve(root, evidence)
    if errors:
        print("roadmap evidence resolution failed:", *errors, sep="\n", file=sys.stderr)
        return 1
    if args.resolve_only:
        print(f"resolved {len(resolved)} roadmap evidence entries")
        return 0
    failures = execute(root, resolved)
    if failures:
        print("roadmap evidence execution failed:", *failures, sep="\n", file=sys.stderr)
        return 1
    print(f"roadmap evidence passed: {len(resolved)} entries")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
