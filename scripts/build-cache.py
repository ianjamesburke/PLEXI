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
GIB = 1024**3
# Defaults; both overridable (stint 1495). Documented in src/testing/TESTING.md.
DEFAULT_MIN_FREE_BYTES = 12 * GIB
DEFAULT_TOTAL_MAX_BYTES = 40 * GIB
# The release binary a stamped candidate hands to its install (stints 1443, 0637).
ARTIFACT = Path("release") / "plexi"


class DiskFloorError(OSError):
    """Free space on the build volume is below the floor."""


def free_bytes(path):
    return shutil.disk_usage(path).free


def require_free_space(path, floor, free=None):
    """Refuse to build when the volume holding `path` has less than `floor` free."""
    free = free_bytes(path) if free is None else free
    if free < floor:
        raise DiskFloorError(
            f"refusing to build: {free / GIB:.1f} GiB free on the volume holding {path}, "
            f"floor {floor / GIB:.1f} GiB (PLEXI_BUILD_MIN_FREE_BYTES). Reclaim space with "
            f"`just clean-stale-targets` (merged worktrees' target dirs), `just trim-targets` "
            f"(debug and incremental output), or a lower PLEXI_BUILD_CACHE_TOTAL_MAX_BYTES.")
    print(f"build-cache: free_bytes={free} floor_bytes={floor} volume={path}", flush=True)


def tree_bytes(path):
    if not path.exists():
        return 0
    return sum(p.stat().st_size for p in path.rglob("*") if p.is_file() and not p.is_symlink())


def prune_slot(slot):
    """Remove a slot's build output but keep its release artifact. Returns bytes freed."""
    target = slot / "target"
    if target.is_symlink() or not target.exists():
        return 0
    before = tree_bytes(target)
    keep = target / ARTIFACT
    for child in target.iterdir():
        if child.is_symlink() or child.is_file():
            child.unlink()
        elif child.name == ARTIFACT.parts[0] and keep.is_file():
            for inner in child.iterdir():
                if inner == keep:
                    continue
                if inner.is_dir() and not inner.is_symlink():
                    shutil.rmtree(inner)
                else:
                    inner.unlink()
        else:
            shutil.rmtree(child)
    freed = before - tree_bytes(target)
    print(f"build-cache: pruned slot={slot} freed_bytes={freed} kept={ARTIFACT if keep.is_file() else 'nothing'}",
          flush=True)
    return freed


def owned_slots(root):
    return [slot for slot in sorted(root.glob("*/slot-*"))
            if slot.is_dir() and not slot.is_symlink()
            and (slot / "owner").is_file() and (slot / "owner").read_text() == MARKER]


def prune_to_cap(root, cap, keep):
    """Prune slots, oldest use first, until every slot together fits `cap`.

    A slot whose lock another build holds is skipped; `keep` (the slot this
    build is about to use) is pruned last. Release artifacts always survive.
    """
    slots = owned_slots(root)
    sizes = {slot: tree_bytes(slot / "target") for slot in slots}
    total = sum(sizes.values())
    if total <= cap:
        return total
    print(f"build-cache: total_bytes={total} over cap_bytes={cap}", flush=True)

    def last_used(slot):
        record = slot / "last-channel"
        return record.stat().st_mtime if record.exists() else 0

    order = sorted((slot for slot in slots if slot != keep), key=last_used)
    order += [slot for slot in slots if slot == keep]
    for slot in order:
        if total <= cap:
            break
        if slot != keep:
            lock = (slot / "lock").open("a+")
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                lock.close()
                print(f"build-cache: skip leased slot={slot}", flush=True)
                continue
            try:
                total -= prune_slot(slot)
            finally:
                lock.close()
        else:
            total -= prune_slot(slot)
    print(f"build-cache: total_bytes={total} after prune", flush=True)
    return total


def run(channel, command):
    if not any(channel.startswith(prefix) and channel[len(prefix):].isascii() and channel[len(prefix):].isdigit() for prefix in ("pr-", "task-")):
        raise ValueError("expected pr-<number> or task-<number>")
    owner = os.environ.get("PLEXI_BUILD_CACHE_OWNER", channel)
    if not any(owner.startswith(prefix) and owner[len(prefix):].isascii() and owner[len(prefix):].isdigit() for prefix in ("pr-", "task-")):
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
                    maximum = int(os.environ.get("PLEXI_BUILD_CACHE_MAX_BYTES", str(40 * GIB)))
                    total = tree_bytes(target)
                    if total > maximum:
                        print(f"build-cache: prune owned oversized slot bytes={total}", flush=True)
                        total -= prune_slot(slot)
                    target.mkdir(exist_ok=True)
                    prune_to_cap(base.parent, int(os.environ.get(
                        "PLEXI_BUILD_CACHE_TOTAL_MAX_BYTES", str(DEFAULT_TOTAL_MAX_BYTES))), keep=slot)
                    require_free_space(base, int(os.environ.get(
                        "PLEXI_BUILD_MIN_FREE_BYTES", str(DEFAULT_MIN_FREE_BYTES))))
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
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"build-cache: {error}", file=sys.stderr)
        sys.exit(1)
