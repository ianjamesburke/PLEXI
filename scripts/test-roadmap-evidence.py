#!/usr/bin/env python3
"""Regression fixtures for the ROADMAP evidence gate's failure boundaries."""

from __future__ import annotations

import os
import pathlib
import subprocess
import tempfile
import textwrap


SCRIPT = pathlib.Path(__file__).with_name("roadmap-evidence.py")


def write(path: pathlib.Path, body: str, executable: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    if executable:
        path.chmod(0o755)


def gate(root: pathlib.Path, roadmap: pathlib.Path, bin_dir: pathlib.Path) -> subprocess.CompletedProcess[str]:
    env = os.environ | {"PATH": f"{bin_dir}:{os.environ['PATH']}"}
    return subprocess.run(
        ["python3", str(SCRIPT), "--root", str(root), "--roadmap", str(roadmap)],
        text=True,
        capture_output=True,
        env=env,
    )


def fixture(name: str, evidence: str) -> tuple[tempfile.TemporaryDirectory[str], pathlib.Path, pathlib.Path, pathlib.Path]:
    temp = tempfile.TemporaryDirectory(prefix=f"plexi-roadmap-{name}-")
    root = pathlib.Path(temp.name)
    roadmap = root / "ROADMAP.toml"
    write(roadmap, f'[[node]]\nid = "fixture"\nevidence = ["{evidence}"]\n')
    bin_dir = root / "bin"
    write(bin_dir / "cargo", "#!/bin/sh\nexit 0\n", True)
    return temp, root, roadmap, bin_dir


def main() -> int:
    # A typo must not silently turn into a skipped proof.
    temp, root, roadmap, bin_dir = fixture("unresolved", "missing-proof")
    try:
        result = gate(root, roadmap, bin_dir)
        assert result.returncode != 0 and "unresolved evidence missing-proof" in result.stderr, result
    finally:
        temp.cleanup()

    # suite = false is still executed; the fake just deliberately fails only if invoked.
    temp, root, roadmap, bin_dir = fixture("skipped", "must-run")
    try:
        write(root / "tests/scenes/must-run.toml", "suite = false\nsteps = []\n")
        write(bin_dir / "just", "#!/bin/sh\nexit 23\n", True)
        result = gate(root, roadmap, bin_dir)
        assert result.returncode != 0 and "must-run" in result.stderr, result
    finally:
        temp.cleanup()

    # A resolved Rust test must fail the release gate when its command fails.
    temp, root, roadmap, bin_dir = fixture("failing", "failing_proof")
    try:
        write(root / "src/lib.rs", "#[test]\nfn failing_proof() {}\n")
        write(
            bin_dir / "cargo",
            textwrap.dedent(
                """\
                #!/bin/sh
                if [ "$3" = "--" ] && [ "$4" = "--list" ]; then
                  echo 'suite::failing_proof: test'
                  exit 0
                fi
                exit 31
                """
            ),
            True,
        )
        result = gate(root, roadmap, bin_dir)
        assert result.returncode != 0 and "resolved Rust evidence" in result.stderr, result
    finally:
        temp.cleanup()
    print("roadmap evidence regression fixtures passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
