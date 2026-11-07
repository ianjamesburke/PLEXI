#!/usr/bin/env python3
"""Exact clean-source evidence for local candidate and pretest reuse."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

SCHEMA = 1


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as stream:
        json.dump(value, stream, sort_keys=True)
        temporary = stream.name
    os.replace(temporary, path)


def identity(root, extra, sdk_tests=False):
    root = Path(root).resolve()
    def output(*args):
        return subprocess.check_output(args, cwd=root, text=True, stderr=subprocess.PIPE).strip()
    if output("git", "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("dirty source cannot reuse evidence")
    configs = {}
    directories = [parent / ".cargo" for parent in [*reversed(root.parents), root]]
    directories.append(Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))))
    for directory in directories:
        for name in ("config", "config.toml"):
            path = directory / name
            if path.is_file():
                configs[str(path)] = digest(path)
    keys = {key: value for key, value in os.environ.items()
            if key.startswith(("CARGO_", "RUST", "LC_", "PYTHON", "UV_"))
            or key in {"LANG", "PATH", "CC", "CXX", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET",
                       "DEVELOPER_DIR", "PLEXI_BUILD_TAG", "PLEXI_BUILD_TEST_CHANNEL", "PLEXI_PR_TEST_FILTERS", "PLEXI_PR_REQUIRE_SDK",
                       "PLEXI_PR_REQUIRED_PTY_FILTERS", "PLEXI_PR_REQUIRED_SCENES"}}
    # Target is artifact location, not a compiler input. A slot remains locked.
    keys.pop("CARGO_TARGET_DIR", None)
    if sdk_tests and len(extra) >= 2 and extra[1] == "scoped" and os.environ.get("PLEXI_PR_REQUIRE_SDK") != "1":
        selector = Path(__file__).with_name("pr-sdk-selection.py")
        if selector.is_file():
            sdk_tests = not output(sys.executable, str(selector), extra[0]).startswith("skip:")
    python_tests = None
    if sdk_tests and (root / "sdk/python/pyproject.toml").is_file():
        # Resolve the same uv environment as sdk-smoke, including its actual
        # interpreter and test dependency versions. A failure disables reuse.
        python_tests = output("uv", "run", "--python", "3.12", "--project", "sdk/python",
                              "--with", "pytest", "--with", "pytest-asyncio", "python", "-c",
                              "import sys,json; from importlib.metadata import distributions; "
                              "print(json.dumps([sys.executable,sys.version,sorted((d.metadata['Name'],d.version) for d in distributions())]))")
    tag = subprocess.run(["git", "describe", "--tags", "--exact-match", "HEAD"],
                         cwd=root, capture_output=True, text=True)
    driver = {name: digest(Path(__file__).with_name(name))
              for name in ("build-evidence.py", "pr-install-pretests.sh", "pr-sdk-selection.py")
              if Path(__file__).with_name(name).is_file()}
    return {"exact_tag": tag.stdout.strip() if tag.returncode == 0 else "", "driver": driver, "python_tests": python_tests, "schema": SCHEMA, "head": output("git", "rev-parse", "HEAD"),
            "source": str(root), "channel_input": digest(root / ".channel") if (root / ".channel").is_file() else None,
            "toolchain": output("rustc", "-vV"), "cargo": output("cargo", "-vV"),
            "uv": output("uv", "--version"), "configs": configs, "environment": keys, "extra": extra}


def read(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def receipt_run(arguments):
    for name in ("PLEXI_PR_REQUIRED_PTY_FILTERS", "PLEXI_PR_REQUIRED_SCENES"):
        value = os.environ.get(name, "")
        if not value.strip() or (value != "none" and "none" in value.splitlines()):
            print(f"pretest-receipt: declare {name} (selections or 'none')", file=sys.stderr)
            return 1
    root, *selection = arguments
    target = os.environ.get("CARGO_TARGET_DIR")
    path = Path(target) / ".plexi-pretests.json" if target else None
    try:
        key = identity(root, selection, sdk_tests=True)
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"pretest-receipt: unavailable: {error}", flush=True)
        key = None
    previous = read(path) if path else None
    if key and previous and previous.get("identity") == key and previous.get("result") == 0:
        log = Path(previous.get("log", ""))
        if log.is_file() and digest(log) == previous.get("log_sha256"):
            print(f"pretest-receipt: reuse exact successful execution {log}", flush=True)
            return 0
    if path:
        path.unlink(missing_ok=True)
    print("pretest-receipt: run (no matching successful execution)", flush=True)
    command = ["bash", str(Path(__file__).with_name("pr-install-pretests.sh")), *arguments]
    log = Path(target) / ".plexi-pretests.log" if target else None
    if not log:
        return subprocess.call(command)
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("wb") as stream:
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        for line in process.stdout:
            sys.stdout.buffer.write(line)
            sys.stdout.buffer.flush()
            stream.write(line)
        result = process.wait()
    if result == 0 and key:
        try:
            if identity(root, selection, sdk_tests=True) == key:
                atomic_json(path, {"identity": key, "result": 0, "log": str(log), "log_sha256": digest(log)})
        except (OSError, subprocess.CalledProcessError, ValueError) as error:
            print(f"pretest-receipt: not recorded: {error}", file=sys.stderr)
    return result


def artifact(action, root, head, channel, binary):
    path = Path(binary).parent / ".plexi-candidate.json"
    key = identity(root, ["release", "warnings-denied", channel])
    if key["head"] != head or os.environ.get("PLEXI_BUILD_TEST_CHANNEL") != channel:
        raise ValueError("candidate head or compiled channel environment mismatch")
    embedded = json.loads(subprocess.check_output([binary, "--build-info"], text=True))
    if embedded.get("test_channel") != channel:
        raise ValueError("binary compiled PR channel mismatch or unavailable")
    if embedded.get("source_commit") != head:
        raise ValueError("binary source commit mismatch")
    value = {"identity": key, "sha256": digest(binary)}
    if action == "stamp":
        atomic_json(path, value)
    elif read(path) != value:
        raise ValueError("candidate evidence missing or mismatched")
    print(f"candidate-{action}: {head} {channel} sha256={value['sha256']}")
    return 0


def main():
    try:
        action, *args = sys.argv[1:]
        if action == "pretests":
            return receipt_run(args)
        return artifact(action, *args)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"build-evidence: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
