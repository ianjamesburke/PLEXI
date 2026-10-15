#!/usr/bin/env python3
"""Prove two separately packaged Alpha bundles retain one private Keychain seal."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from package import macos_signature


def run(command, env, expected=0, timeout=120):
    result = subprocess.run(list(map(str, command)), env=env, capture_output=True, text=True, timeout=timeout)
    if result.returncode != expected:
        raise AssertionError(f"{command}: exit={result.returncode}\n{result.stdout}\n{result.stderr}")
    return result


def executable(package):
    manifest = json.loads((package / "package.json").read_text())
    return package / manifest["executable"], manifest


def install(package, root, env):
    run([package / "plexi-installer", "--package", package, "--channel", "alpha",
         "--install-dir", root / "install", "--bin-dir", root / "bin",
         "--applications-dir", root / "applications", "--install-only"], env)
    receipt = json.loads((root / "install/alpha/installation.json").read_text())
    return Path(receipt["active"]["path"]) / receipt["active"]["executable"]


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


def start_and_stop(binary, env, seed=False):
    run([binary, "host", "start", "--ephemeral", "--pane", f"cwd={env['HOME']}", "--timeout-secs", "90"], env, timeout=120)
    try:
        status = wait_ready(binary, env)
        if seed:
            panes = json.loads(run([binary, "pane", "list"], env).stdout)
            caller = dict(env, PLEXI_SOCKET=status["socket"], PLEXI_PANE_ID=str(panes[0]["id"]))
            run([binary, "agent", "report", "--state", "blocked", "--agent", "signing-continuity", "--event", "AskQuestion", "--detail", "Signing continuity acceptance question"], caller)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            items = json.loads(run([binary, "needs-you", "list", "--json"], env).stdout)["items"]
            if any(item.get("summary") == "Signing continuity acceptance question" for item in items):
                return items
            time.sleep(0.2)
        raise AssertionError(f"sealed question was not retained: {items}")
    finally:
        run([binary, "host", "stop"], env, timeout=120)


def main(args):
    if sys.platform != "darwin":
        raise SystemExit("This check requires the macOS Keychain.")
    package_a, manifest_a = executable(args.package_a.resolve())
    package_b, manifest_b = executable(args.package_b.resolve())
    if manifest_a["channel"] != "alpha" or manifest_b["channel"] != "alpha":
        raise AssertionError("continuity acceptance is only defined for public Alpha packages")
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
        installed_a = install(args.package_a.resolve(), root, env)
        original = start_and_stop(installed_a, env, seed=True)
        profile = Path(env["HOME"]) / ".plexi-alpha"
        first_log = profile / "plexi.log"
        if not first_log.is_file() or "created host permission mac key" not in first_log.read_text(errors="replace"):
            raise AssertionError("package A did not create its host seal in the isolated private Keychain")
        before = first_log.stat().st_size
        installed_b = install(args.package_b.resolve(), root, env)
        if installed_a == installed_b:
            raise AssertionError("installer did not activate a different generation")
        restored = start_and_stop(installed_b, env)
        if original != restored:
            raise AssertionError(f"sealed Needs-you question changed across upgrade: {original} -> {restored}")
        second_log = first_log.read_text(errors="replace")[before:]
        forbidden = ("key unreadable", "rotated unreadable permission mac key", "unverified grants", "created host permission mac key")
        if any(marker in second_log for marker in forbidden):
            raise AssertionError(f"package B did not retain the host seal: {second_log[-4000:]}")
        if "macOS host keychain" not in second_log:
            raise AssertionError("package B did not use the isolated private Keychain")
        print("PASS: distinct Alpha builds retained the sealed Needs-you question without Keychain rotation")
    after = [subprocess.check_output(["security", operation], text=True) for operation in ("default-keychain", "list-keychains")]
    if after != keychain_state:
        raise AssertionError("private-Keychain test changed the user's default or search list")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-a", required=True, type=Path)
    parser.add_argument("--package-b", required=True, type=Path)
    main(parser.parse_args())
