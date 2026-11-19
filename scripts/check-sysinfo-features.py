#!/usr/bin/env python3
"""Fail if any resolved sysinfo package can run on rayon's global pool.

sysinfo's default `multithread` feature routes even a one-pid process refresh
through rayon's global pool. wasmtime's parallel compilation saturates that
pool, so a terminal spawn (which records its process group via sysinfo) stalled
for seconds (stint 1633). This reads Cargo's resolved graph for every platform
and feature, so a dependency that re-enables the feature through feature
unification fails too. wasmtime's own rayon use is allowed.
"""

import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
COMMAND = ["cargo", "metadata", "--locked", "--all-features", "--format-version", "1"]


def main() -> int:
    try:
        result = subprocess.run(
            COMMAND, cwd=REPO_ROOT, capture_output=True, text=True, check=False
        )
    except OSError as error:
        print(f"check-sysinfo-features: could not run {' '.join(COMMAND)}: {error}", file=sys.stderr)
        return 1
    if result.returncode != 0:
        print(
            f"check-sysinfo-features: {' '.join(COMMAND)} failed (exit {result.returncode}) in {REPO_ROOT}:\n"
            f"{result.stderr.strip()}",
            file=sys.stderr,
        )
        return 1
    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        print(f"check-sysinfo-features: cargo metadata output is not JSON: {error}", file=sys.stderr)
        return 1

    packages = {package["id"]: package for package in metadata["packages"]}
    sysinfo_nodes = [
        node for node in metadata["resolve"]["nodes"] if packages[node["id"]]["name"] == "sysinfo"
    ]
    if not sysinfo_nodes:
        print("check-sysinfo-features: no sysinfo package in the resolved graph", file=sys.stderr)
        return 1

    failures = []
    for node in sysinfo_nodes:
        version = packages[node["id"]]["version"]
        if "multithread" in node["features"]:
            failures.append(f"sysinfo {version} enables feature `multithread`")
        for dep in node["deps"]:
            if packages[dep["pkg"]]["name"] == "rayon":
                failures.append(
                    f"sysinfo {version} depends on rayon {packages[dep['pkg']]['version']}"
                )

    if failures:
        for failure in failures:
            print(f"check-sysinfo-features: {failure}", file=sys.stderr)
        print(
            "check-sysinfo-features: sysinfo must not use rayon's global pool (stint 1633). "
            'Declare it in Cargo.toml with default-features = false, features = ["system"], '
            "and find which dependency re-enables `multithread` with "
            "`cargo tree -e features -i sysinfo`.",
            file=sys.stderr,
        )
        return 1

    for node in sysinfo_nodes:
        version = packages[node["id"]]["version"]
        print(
            f"check-sysinfo-features: sysinfo {version} features={sorted(node['features'])}, no rayon edge"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
