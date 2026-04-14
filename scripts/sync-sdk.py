#!/usr/bin/env python3
"""sync-sdk.py — copy the canonical plexi_sdk.py to every examples/*/plexi_sdk.py.

The canonical Python SDK lives at sdk/python/plexi_sdk.py. Each example app
vendors its own copy next to its entry file so the app runs hermetically at
install time. This script keeps those vendored copies in lockstep with the
canonical.

Usage:
    scripts/sync-sdk.py           # copy canonical -> every example (in place)
    scripts/sync-sdk.py --check   # exit 1 if any example is out of sync,
                                  # no writes. Suitable for CI / pre-commit.

Exit codes:
    0  all examples already match (or were successfully synced)
    1  --check found drift, or a filesystem error occurred
"""

from __future__ import annotations

import argparse
import filecmp
import shutil
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
CANONICAL = REPO_ROOT / "sdk" / "python" / "plexi_sdk.py"
EXAMPLES_GLOB = "examples/*/plexi_sdk.py"


def find_targets() -> list[Path]:
    return sorted(REPO_ROOT.glob(EXAMPLES_GLOB))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit nonzero if any example is out of sync; do not write",
    )
    args = parser.parse_args()

    if not CANONICAL.is_file():
        print(f"error: canonical SDK not found at {CANONICAL}", file=sys.stderr)
        return 1

    targets = find_targets()
    if not targets:
        print(f"warning: no targets matched {EXAMPLES_GLOB}")
        return 0

    out_of_sync: list[Path] = []
    synced: list[Path] = []

    for target in targets:
        if target.is_file() and filecmp.cmp(CANONICAL, target, shallow=False):
            continue
        out_of_sync.append(target)

    if args.check:
        if out_of_sync:
            print(
                f"{len(out_of_sync)} example(s) out of sync with {CANONICAL.relative_to(REPO_ROOT)}:",
                file=sys.stderr,
            )
            for t in out_of_sync:
                print(f"  {t.relative_to(REPO_ROOT)}", file=sys.stderr)
            print(
                "\nrun `scripts/sync-sdk.py` to fix.",
                file=sys.stderr,
            )
            return 1
        print(f"ok: all {len(targets)} example(s) in sync")
        return 0

    for target in out_of_sync:
        try:
            shutil.copyfile(CANONICAL, target)
            synced.append(target)
        except OSError as e:
            print(f"error: failed to copy to {target}: {e}", file=sys.stderr)
            return 1

    if synced:
        print(f"synced {len(synced)} example(s):")
        for t in synced:
            print(f"  {t.relative_to(REPO_ROOT)}")
    else:
        print(f"ok: all {len(targets)} example(s) already in sync")
    return 0


if __name__ == "__main__":
    sys.exit(main())
