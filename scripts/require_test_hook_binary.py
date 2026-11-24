#!/usr/bin/env python3
"""Scan Plexi executable bytes before release packaging or isolated keychain tests."""

import argparse
import json
import mmap
import os
from pathlib import Path
import re
import subprocess
import sys


HOOK_NAMES = (
    b"PLEXI_KEYCHAIN_PATH",
    b"PLEXI_KEYCHAIN_PASSWORD",
    b"PLEXI_OPENROUTER_BASE_URL",
    b"PLEXI_PICKER_SCRIPT",
)
BUILD_HINT = "build a PLEXI_BUILD_TEST_CHANNEL=pr-<N> release build and install it as plexi-pr-<N>"


def refuse(binary, reason, status=1):
    print(f"Refused {binary}: {reason}; {BUILD_HINT}.", file=sys.stderr)
    raise SystemExit(status)


def present_names(binary):
    if not binary.is_file():
        raise OSError("expected a readable regular file")
    with binary.open("rb") as source:
        if not os.fstat(source.fileno()).st_size:
            return set()
        with mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as data:
            return {name for name in HOOK_NAMES if data.find(name) >= 0}


def require_test_hook_binary(binary, *, installed=False):
    binary = Path(binary)
    try:
        present = present_names(binary)
    except OSError as error:
        refuse(binary, str(error), 2)
    missing = [name.decode() for name in HOOK_NAMES if name not in present]
    if missing:
        refuse(binary, f"missing {', '.join(missing)}")
    if installed:
        # current_exe() follows symlinks. Even a PR build ignores the hooks
        # when its real basename differs from its compiled test channel.
        path = binary.resolve()
        match = re.fullmatch(r"plexi-(pr-[0-9]+)(?:\.exe|\.EXE)?", path.name)
        if not match or path.name != binary.name:
            refuse(binary, f"expected a numbered PR executable with the same basename as {path}")
        # --build-info exits before profile or host initialization. Do not
        # pass any caller overrides to this identity-only invocation.
        env = {k: v for k, v in os.environ.items() if not k.startswith("PLEXI_")}
        try:
            info = json.loads(subprocess.check_output([path, "--build-info"], env=env, timeout=15))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            refuse(binary, f"could not read compiled channel ({error})")
        if not isinstance(info, dict) or info.get("test_channel") != match[1]:
            refuse(binary, f"compiled test channel does not match {path.name}")
        return path
    return binary


def require_package_binary(package):
    try:
        manifest = json.loads((package / "package.json").read_text())
        binary = package / manifest["executable"]
    except (OSError, ValueError, KeyError, TypeError) as error:
        refuse(package, f"could not locate packaged executable ({error})", 2)
    path = require_test_hook_binary(binary, installed=True)
    if path.name != f"plexi-{manifest.get('channel')}":
        refuse(binary, "package channel does not match executable")
    return path, manifest


def require_receipt_binary(install_dir, channel):
    receipt = install_dir / channel / "installation.json"
    try:
        active = json.loads(receipt.read_text())["active"]
        binary = Path(active["path"]) / active["executable"]
    except (OSError, ValueError, KeyError, TypeError) as error:
        refuse(receipt, f"could not locate installed executable ({error})", 2)
    path = require_test_hook_binary(binary, installed=True)
    if path.name != f"plexi-{channel}":
        refuse(binary, "installed executable changed channel")
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--installed", action="store_true", help="also verify the runtime channel and print the executable path")
    mode.add_argument("--release", action="store_true", help="require all test hook names to be absent")
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    if args.release:
        try:
            if not args.binary.stat().st_size:
                raise OSError("empty binary")
            present = present_names(args.binary)
        except OSError as error:
            print(f"could not scan {args.binary}: {error}", file=sys.stderr)
            return 2
        for name in sorted(present):
            print(f"FAIL: {args.binary} contains {name.decode()}", file=sys.stderr)
        if present:
            return 1
        print(f"PASS: no test hook names in {args.binary}")
    else:
        path = require_test_hook_binary(args.binary, installed=args.installed)
        if args.installed:
            print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
