#!/usr/bin/env python3
"""Invoke Cargo with a fresh, workspace-qualified optional .channel input.

Call inside cargo-with-lease.sh, immediately before Cargo (not around just or an
installer). Raw Cargo and unsupported invocation shapes keep build.rs's
conservative file watch. No source-tree files are created.
"""

import hashlib
import json
import time
import os
from pathlib import Path
import subprocess
import sys

ROOT_ENV = "PLEXI_BUILD_CHANNEL_ROOT"
INPUT_ENV = "PLEXI_BUILD_CHANNEL_INPUT"


def cargo_environment(args, env):
    env = dict(env)
    # Never let an outer invocation supply another workspace's snapshot.
    env.pop(ROOT_ENV, None)
    env.pop(INPUT_ENV, None)
    options = args[:args.index("--")] if "--" in args else args
    toolchain = [options[0]] if options and options[0].startswith("+") else []
    command = options[len(toolchain):len(toolchain) + 1]
    if not command or command[0] not in {"build", "test", "check", "clippy", "run", "rustc"}:
        return env
    # Nightly directory changes require different cwd resolution. Leave these
    # to Cargo with the correct conservative build.rs fallback.
    if any(arg == "-C" or arg.startswith("-C") for arg in options):
        return env
    manifest = []
    for index, arg in enumerate(options):
        if arg == "--manifest-path":
            if index + 1 == len(options):
                return env
            manifest = [arg, options[index + 1]]
        elif arg.startswith("--manifest-path="):
            manifest = [arg]
    locate = subprocess.run(
        ["cargo", *toolchain, "locate-project", "--workspace", "--message-format", "plain", *manifest],
        env=env, capture_output=True, text=True,
    )
    if locate.returncode:
        print("cargo-channel-input: workspace lookup failed; using Cargo file tracking", file=sys.stderr)
        return env
    root = Path(locate.stdout.strip()).resolve().parent
    try:
        content = (root / ".channel").read_bytes()
    except FileNotFoundError:
        fingerprint = "missing"
    else:
        fingerprint = "sha256:" + hashlib.sha256(content).hexdigest()
    env[ROOT_ENV] = str(root)
    env[INPUT_ENV] = fingerprint
    print(f"cargo-channel-input: workspace={root} input={fingerprint}", file=sys.stderr)
    return env


def main():
    try:
        env = cargo_environment(sys.argv[1:], os.environ)
        args = sys.argv[1:]
        telemetry = env.get("PLEXI_BUILD_TELEMETRY")
        if telemetry and args and args[0] == "build" and not any(a.startswith("--message-format") for a in args):
            started = time.monotonic()
            fresh = compiled = 0
            command = ["cargo", *args, "--message-format=json-render-diagnostics"]
            process = subprocess.Popen(command, env=env, stdout=subprocess.PIPE, text=True)
            for line in process.stdout:
                try:
                    message = json.loads(line)
                except ValueError:
                    print(line, end="", flush=True)
                    continue
                if message.get("reason") == "compiler-artifact":
                    fresh += int(message.get("fresh", False))
                    compiled += int(not message.get("fresh", False))
                elif message.get("reason") == "compiler-message":
                    print(message["message"].get("rendered", ""), end="", file=sys.stderr)
            result = process.wait()
            record = {"schema": 1, "event": "cargo-artifacts", "command": command,
                      "fresh_artifacts": fresh, "compiled_artifacts": compiled,
                      "execution_seconds": time.monotonic() - started, "result": result}
            with open(telemetry, "a") as stream:
                stream.write(json.dumps(record) + "\n")
            print("cargo-artifacts: " + json.dumps(record), file=sys.stderr)
            return result
        os.execvpe("cargo", ["cargo", *args], env)
    except OSError as error:
        print(f"cargo-channel-input: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
