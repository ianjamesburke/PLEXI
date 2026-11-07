#!/usr/bin/env python3
"""Bounded persistent Cargo slots, exclusively locked for the entire operation."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

MARKER = "plexi-owned-build-cache-v1"


def run(channel, command):
    if not any(channel.startswith(prefix) and channel[len(prefix):].isdigit() for prefix in ("pr-", "task-")):
        raise ValueError("expected pr-<number> or task-<number>")
    owner = os.environ.get("PLEXI_BUILD_CACHE_OWNER", channel)
    if not any(owner.startswith(prefix) and owner[len(prefix):].isdigit() for prefix in ("pr-", "task-")):
        raise ValueError("invalid cache owner")
    if channel.startswith("task-") and owner != channel:
        raise ValueError("task cache owner must match task")
    common = subprocess.check_output(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], text=True).strip()
    namespace = hashlib.sha256(common.encode()).hexdigest()[:16]
    base = Path(os.environ.get("PLEXI_BUILD_CACHE_ROOT", str(Path.home() / ".cache/plexi-build"))) / namespace
    base.mkdir(parents=True, exist_ok=True)
    if base.is_symlink():
        raise ValueError("cache namespace must not be a symlink")
    # One PR may have both a worker and a tester waiting. Serialize that channel
    # before choosing a slot so they cannot mutate the same source/install via
    # different free slots.
    # Serialize both channel and owner; sorted acquisition avoids lock inversion.
    from contextlib import ExitStack
    with ExitStack() as stack:
        locks = [stack.enter_context((base / f"{name}.lock").open("a+")) for name in sorted({channel, owner})]
        deadline = time.monotonic() + float(os.environ.get("PLEXI_BUILD_CACHE_TIMEOUT", "900"))
        for channel_lock in locks:
            while True:
                try:
                    fcntl.flock(channel_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        raise TimeoutError("cache owner or channel operation already active")
                    time.sleep(0.2)
        slots = [base / f"slot-{index}" for index in range(2)]
        for slot in slots:
            slot.mkdir(exist_ok=True)
            if slot.is_symlink():
                raise ValueError("cache slot must not be a symlink")
            marker = slot / "owner"
            if marker.exists():
                if marker.read_text() != MARKER:
                    raise ValueError("unknown cache ownership")
            elif any(slot.iterdir()):
                raise ValueError("refusing populated unowned cache")
            else:
                marker.write_text(MARKER)
        # Prefer the prior channel slot; otherwise least recently used.
        def rank(slot):
            record = slot / "last-channel"
            matching = record.exists() and record.read_text() == owner
            modified = record.stat().st_mtime if record.exists() else 0
            return (0 if matching else 1, -modified if matching else modified)
        slots.sort(key=rank)
        started = time.monotonic()
        deadline = started + float(os.environ.get("PLEXI_BUILD_CACHE_TIMEOUT", "900"))
        while True:
            for slot in slots:
                lock = (slot / "lock").open("a+")
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    lock.close()
                    continue
                try:
                    target = slot / "target"
                    if target.is_symlink():
                        raise ValueError("target must not be a symlink")
                    # Limit retained data before reuse, only inside our marked and
                    # locked slot. Never traverse an arbitrary caller target tree.
                    maximum = int(os.environ.get("PLEXI_BUILD_CACHE_MAX_BYTES", str(40 * 1024**3)))
                    total = sum(p.stat().st_size for p in target.rglob("*") if p.is_file() and not p.is_symlink()) if target.exists() else 0
                    if total > maximum:
                        print(f"build-cache: discard owned oversized slot bytes={total}", flush=True)
                        shutil.rmtree(target)
                    target.mkdir(exist_ok=True)
                    (slot / "last-channel").write_text(owner)
                    env = {**os.environ, "CARGO_TARGET_DIR": str(target),
                           "PLEXI_BUILD_TELEMETRY": str(slot / "telemetry.jsonl")}
                    if channel.startswith("pr-"):
                        env["PLEXI_BUILD_TEST_CHANNEL"] = channel
                    else:
                        env.pop("PLEXI_BUILD_TEST_CHANNEL", None)
                    print(f"build-cache: slot={slot} retained_bytes={total} wait_seconds={time.monotonic()-started:.3f}", flush=True)
                    return subprocess.call(command, env=env)
                finally:
                    lock.close()
            if time.monotonic() >= deadline:
                raise TimeoutError("all persistent build slots busy")
            time.sleep(0.2)


if __name__ == "__main__":
    try:
        sys.exit(run(sys.argv[1], sys.argv[2:]))
    except (OSError, ValueError, subprocess.CalledProcessError, TimeoutError) as error:
        print(f"build-cache: {error}", file=sys.stderr)
        sys.exit(1)
