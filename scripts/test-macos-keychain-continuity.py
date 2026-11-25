#!/usr/bin/env python3
"""Prove two signed builds of one PR channel retain one private Keychain seal."""
import argparse
import ctypes
import hashlib
import json
import os
import re
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time

from package import macos_signature
from require_test_hook_binary import require_package_binary, require_receipt_binary


def run(command, env, expected=0, timeout=120):
    result = subprocess.run(list(map(str, command)), env=env, capture_output=True, text=True, timeout=timeout)
    if result.returncode != expected:
        raise AssertionError(f"{command}: exit={result.returncode}\n{result.stdout}\n{result.stderr}")
    return result


def install(package, channel, root, env):
    run([package / "plexi-installer", "--package", package, "--channel", channel,
         "--install-dir", root / "install", "--bin-dir", root / "bin",
         "--applications-dir", root / "applications", "--install-only"], env)
    return require_receipt_binary(root / "install", channel)


def create_private_keychain(path, password):
    security = ctypes.CDLL("/System/Library/Frameworks/Security.framework/Security")
    security.SecKeychainCreate.argtypes = [ctypes.c_char_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_bool, ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
    reference = ctypes.c_void_p()
    status = security.SecKeychainCreate(os.fsencode(path), len(password), password, False, None, ctypes.byref(reference))
    if status != 0:
        raise AssertionError(f"SecKeychainCreate returned {status}")


def wait_ready(binary, env):
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        status = run([binary, "host", "status", "--json"], env, expected=0).stdout
        parsed = json.loads(status)
        if parsed.get("ready"):
            return parsed
        time.sleep(0.25)
    raise AssertionError("host did not become ready")


QUESTION = "Signing continuity acceptance question"


def pane_command(binary, root):
    """A pane command that asks the question as that pane, then keeps listing
    Needs-you as that pane.

    The host places a socket caller by process ancestry: only a process inside
    the pane may report for it, and a pane reads back only what its own
    terminal incarnation filed, never what an earlier pane with its number
    filed."""
    script, reported, listed = root / "pane.sh", root / "seed-question.out", root / "needs-you.json"
    report = [binary, "agent", "report", "--state", "blocked", "--agent", "signing-continuity", "--event", "AskQuestion", "--detail", QUESTION]
    staging = shlex.quote(str(listed) + ".tmp")
    script.write_text("\n".join([
        "#!/bin/sh",
        f"{shlex.join(map(str, report))} >{shlex.quote(str(reported))} 2>&1",
        f"echo \"exit=$?\" >>{shlex.quote(str(reported))}",
        f"while :; do {shlex.quote(str(binary))} needs-you list --json >{staging} 2>&1; /bin/mv {staging} {shlex.quote(str(listed))}; /bin/sleep 0.2; done",
    ]) + "\n")
    script.chmod(0o755)
    listed.unlink(missing_ok=True)
    return script, reported, listed


def asked(listed, previous_id=None):
    """The question in the pane's own latest listing, once it is there."""
    try:
        items = json.loads(listed.read_text(errors="replace"))["items"]
    except (OSError, ValueError, KeyError):
        return None
    if previous_id and any(item.get("id") == previous_id for item in items):
        raise AssertionError("the replacement pane listed the earlier pane's question")
    return next((item for item in items if item.get("id") and item.get("summary") == QUESTION), None)


def log_since(path, offset):
    """The log text written after byte `offset` (a file size), so the offset and
    the slice are in the same unit."""
    return path.read_bytes()[offset:].decode(errors="replace")


def run_host(binary, env, until):
    """Start an ephemeral host whose one pane asks the question, and return
    `until(listed)` once it is truthy."""
    script, reported, listed = pane_command(binary, Path(env["HOME"]).parent)
    run([binary, "host", "start", "--ephemeral", "--pane", f"cwd={env['HOME']},cmd={script}", "--timeout-secs", "90"], env, timeout=120)
    try:
        wait_ready(binary, env)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            found = until(listed)
            if found:
                return found
            time.sleep(0.2)
        listing = listed.read_text(errors="replace") if listed.is_file() else "(the pane never listed)"
        seed = reported.read_text(errors="replace") if reported.is_file() else "(no seed output)"
        raise AssertionError(f"the sealed question was not found; the pane listed: {listing}\nseed report: {seed}")
    finally:
        run([binary, "host", "stop"], env, timeout=120)


def main(args):
    package_a, manifest_a = require_package_binary(args.package_a.resolve())
    package_b, manifest_b = require_package_binary(args.package_b.resolve())
    if sys.platform != "darwin":
        raise SystemExit("This check requires the macOS Keychain.")
    channel = manifest_a["channel"]
    if channel != manifest_b["channel"]:
        raise AssertionError("A and B must belong to the same PR channel")
    if manifest_a.get("macos_signing") != manifest_b.get("macos_signing"):
        raise AssertionError("A and B do not carry one identical public signing identity")
    for package, manifest in ((args.package_a.resolve(), manifest_a), (args.package_b.resolve(), manifest_b)):
        bundles = list(package.glob("*.app"))
        if len(bundles) != 1 or macos_signature(bundles[0]) != manifest.get("macos_signing"):
            raise AssertionError("package signature does not match its signing manifest")
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundles[0])], check=True)
    if hashlib.sha256(package_a.read_bytes()).digest() == hashlib.sha256(package_b.read_bytes()).digest():
        raise AssertionError("A and B must contain different executable bytes")
    if (manifest_a["build_id"], manifest_a["tag"]) == (manifest_b["build_id"], manifest_b["tag"]):
        raise AssertionError("A and B must have distinct compiled identities")
    keychain_state = [subprocess.check_output(["security", operation], text=True) for operation in ("default-keychain", "list-keychains")]
    with tempfile.TemporaryDirectory(prefix="plexi-keychain-continuity-", dir="/tmp") as temporary:
        root = Path(temporary)
        password = b"isolated-keychain-continuity"
        keychain = root / "private.keychain-db"
        create_private_keychain(keychain, password)
        env = {key: value for key, value in os.environ.items() if not key.startswith("PLEXI_")}
        env.pop("ZDOTDIR", None)
        env.update(
            HOME=str(root / "home"),
            PLEXI_DISTRIBUTION_HOME=str(root / "distribution"),
            PLEXI_KEYCHAIN_PATH=str(keychain),
            PLEXI_KEYCHAIN_PASSWORD=password.decode(),
        )
        Path(env["HOME"]).mkdir()
        installed_a = install(args.package_a.resolve(), channel, root, env)
        original = run_host(installed_a, env, asked)
        profile = Path(env["HOME"]) / f".plexi-{channel}"
        first_log = profile / "plexi.log"
        if not first_log.is_file() or "created host permission mac key" not in first_log.read_text(errors="replace"):
            raise AssertionError("package A did not create its host seal in the isolated private Keychain")
        before = first_log.stat().st_size
        installed_b = install(args.package_b.resolve(), channel, root, env)
        if installed_a == installed_b:
            raise AssertionError("installer did not activate a different generation")
        # B's pane is a new incarnation and may not read A's question, and its
        # own ask files a new record. The host's restore log names every open
        # item it loaded from the sealed journal, so A's exact id there proves
        # the record survived the upgrade under its seal.
        restored = re.compile(rf"needs_you: restored open item {re.escape(original['id'])}$", re.M)
        run_host(installed_b, env, lambda listed: asked(listed, original["id"]) and restored.search(log_since(first_log, before)))
        second_log = log_since(first_log, before)
        forbidden = ("key unreadable", "rotated unreadable permission mac key", "unverified grants", "created host permission mac key")
        if any(marker in second_log for marker in forbidden):
            raise AssertionError(f"package B did not retain the host seal: {second_log[-4000:]}")
        if "macOS host keychain" not in second_log:
            raise AssertionError("package B did not use the isolated private Keychain")
        print(f"PASS: distinct {channel} builds retained the sealed Needs-you question without Keychain rotation")
    after = [subprocess.check_output(["security", operation], text=True) for operation in ("default-keychain", "list-keychains")]
    if after != keychain_state:
        raise AssertionError("private-Keychain test changed the user's default or search list")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-a", required=True, type=Path)
    parser.add_argument("--package-b", required=True, type=Path)
    main(parser.parse_args())
