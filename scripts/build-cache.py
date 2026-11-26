#!/usr/bin/env python3
"""Bounded persistent Cargo slots, exclusively locked for the entire operation."""
from contextlib import ExitStack
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid

MARKER = "plexi-owned-build-cache-v1"
GIB = 1024**3
# Defaults; both overridable (stint 1495). Documented in src/testing/TESTING.md.
DEFAULT_MIN_FREE_BYTES = 12 * GIB
DEFAULT_TOTAL_MAX_BYTES = 64 * GIB  # two warm lanes plus headroom
# The release binary a slot keeps through pruning.
ARTIFACT = Path("release") / "plexi"
KEPT = {"plexi"}
# Per-slot record of the sources last registered in the slot (stint 1630).
SOURCES = "sources.json"
SOURCES_SCHEMA = 1


def positive_bytes(name, default):
    """A byte-count override: unset means `default`; anything not a positive
    whole number is refused by name, never read as "off"."""
    raw = os.environ.get(name)
    if raw is None:
        return default
    try:
        value = int(raw)
    except ValueError:
        value = 0
    if value <= 0:
        raise ValueError(f"{name} must be a positive whole number of bytes, got {raw!r}")
    return value


class DiskFloorError(OSError):
    """Free space on the build volume is below the floor."""


def free_bytes(path):
    try:
        return shutil.disk_usage(path).free
    except OSError as error:
        raise DiskFloorError(f"floor check failed: could not read free space on {path}: {error}") from error


def require_free_space(path, floor, free=None):
    """Refuse to build when the volume holding `path` has less than `floor` free."""
    free = free_bytes(path) if free is None else free
    if free < floor:
        raise DiskFloorError(
            f"floor check failed: {free / GIB:.1f} GiB free on the volume holding {path}, "
            f"floor {floor / GIB:.1f} GiB (PLEXI_BUILD_MIN_FREE_BYTES). Reclaim space with "
            f"`just clean-stale-targets` (merged worktrees' target dirs), `just trim-targets` "
            f"(debug and incremental output), or a lower PLEXI_BUILD_CACHE_TOTAL_MAX_BYTES.")
    print(f"build-cache: free_bytes={free} floor_bytes={floor} volume={path}", flush=True)


def tree_bytes(path):
    """Bytes in regular files under `path`. Another lane's cargo may be writing
    there: a file or directory that vanishes mid-walk counts as nothing."""
    total = 0
    for directory, _dirs, files in os.walk(path):
        for name in files:
            try:
                info = os.lstat(os.path.join(directory, name))
            except FileNotFoundError:
                continue
            if stat.S_ISREG(info.st_mode):
                total += info.st_size
    return total


def seed_codex(source, worktree):
    """Clone a warm debug tree before launching Codex in a fresh worktree."""
    target = worktree / "target"
    if target.is_symlink() or (target.exists() and (not target.is_dir() or any(target.iterdir()))):
        print(f"build-cache: skip Codex seed: target already populated path={target}", flush=True)
        return
    fingerprints = source / ".fingerprint"
    if source.is_symlink() or fingerprints.is_symlink() or not fingerprints.is_dir():
        print(f"build-cache: skip Codex seed: no warm target path={source}", flush=True)
        return
    if (sys.platform != "darwin" or source.stat().st_dev != worktree.stat().st_dev
            or subprocess.run(["/bin/df", "-T", "apfs", str(worktree)],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode):
        print("build-cache: skip Codex seed: source and worktree need the same APFS volume", flush=True)
        return

    floor = positive_bytes("PLEXI_BUILD_MIN_FREE_BYTES", DEFAULT_MIN_FREE_BYTES)
    with ExitStack() as locks:
        for name in (".cargo-build-lock", ".cargo-artifact-lock", ".cargo-lock"):
            try:
                lock = locks.enter_context((source / name).open("rb"))
            except FileNotFoundError:
                continue
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                print(f"build-cache: skip Codex seed: warm target busy path={source}", flush=True)
                return
        size = tree_bytes(source)
        free = free_bytes(worktree)
        if free - size < floor:
            print(f"build-cache: skip Codex seed: free_bytes={free} reserve_bytes={size} "
                  f"PLEXI_BUILD_MIN_FREE_BYTES={floor}", flush=True)
            return
        target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix=".seed-", dir=target) as stage:
            debug = Path(stage) / "debug"
            subprocess.run(["/bin/cp", "-c", "-R", str(source), str(debug)], check=True)
            require_free_space(worktree, floor)

            # Cargo keys path packages by relative source paths and mtimes.
            # Force a rebuild even if a different checkout has older files.
            packages = tomllib.loads((worktree / "Cargo.lock").read_text())["package"]
            local_names = {package["name"] for package in packages if "source" not in package}
            for fingerprint in (debug / ".fingerprint").iterdir():
                if fingerprint.name.rpartition("-")[0] in local_names:
                    shutil.rmtree(fingerprint)
            if any(entry != Path(stage) for entry in target.iterdir()):
                print(f"build-cache: skip Codex seed: target already populated path={target}", flush=True)
                return
            debug.rename(target / "debug")
    print(f"build-cache: seeded Codex target source={source} path={target / 'debug'} bytes={size}", flush=True)


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
                if inner.name in KEPT and inner.is_file() and not inner.is_symlink():
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
    """Marked slots that really sit inside `root`: a symlinked namespace or slot
    that leads elsewhere is skipped, never pruned."""
    real_root = root.resolve()
    slots = []
    for slot in sorted(root.glob("*/slot-*")):
        if slot.parent.is_symlink() or slot.is_symlink() or not slot.is_dir():
            continue
        if not slot.resolve().is_relative_to(real_root):
            print(f"build-cache: skip slot outside the cache root slot={slot}", flush=True)
            continue
        if (slot / "owner").is_file() and (slot / "owner").read_text() == MARKER:
            slots.append(slot)
    return slots


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


# Cargo decides whether a path package (the host, crates/*, vendored deps/*) is
# fresh by comparing source mtimes with the slot's outputs, and it records those
# sources relative to the package root. A slot serves many worktrees, so a tree
# whose differing file is older on disk than the slot's last build would reuse
# another tree's artifact (PR #3245). Every Cargo call in a slot therefore first
# registers its tree: a file whose content differs from what the slot last saw
# is stamped "now", an unchanged file keeps the stamp of when that content first
# arrived, and no file's mtime is ever moved back (only raised to its stamp), so
# no other target dir can be fooled either. Unchanged crates stay cache hits.


def tree_sources(tree):
    """sha256 of every file Cargo can see in `tree`: tracked plus untracked,
    unignored files (the same set build.rs hashes into the build identity)."""
    listed = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
                            cwd=tree, capture_output=True, check=False)
    if listed.returncode:
        raise ValueError(f"sources: git ls-files failed in {tree}: {listed.stderr.decode(errors='replace').strip()}")
    files = {}
    for raw in sorted(set(listed.stdout.split(b"\0"))):
        name = raw.decode()
        if not name:
            continue
        path = tree / name
        # A deleted tracked file, a symlink or a submodule carries no content
        # of its own; Cargo treats a missing dependency as stale anyway.
        if path.is_symlink() or not path.is_file():
            continue
        files[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    return files


def load_sources(slot):
    path = slot / SOURCES
    try:
        record = json.loads(path.read_text())
    except FileNotFoundError:
        return None
    except (OSError, ValueError) as error:
        print(f"build-cache: sources record unreadable at {path} ({error}); treating every file as changed",
              file=sys.stderr, flush=True)
        return None
    if not isinstance(record, dict) or record.get("schema") != SOURCES_SCHEMA:
        print(f"build-cache: sources record at {path} has another schema; treating every file as changed",
              file=sys.stderr, flush=True)
        return None
    return record


def save_sources(slot, record):
    temporary = slot / f"{SOURCES}.{os.getpid()}.tmp"
    temporary.write_text(json.dumps(record, sort_keys=True))
    os.replace(temporary, slot / SOURCES)


def forget_drift(record):
    """Drop entries whose file changed in the last registered tree after it was
    registered: a compile in between may have read either content."""
    tree = Path(record["tree"])
    if not tree.is_dir():
        return 0
    current = tree_sources(tree)
    drifted = [name for name, (digest, _stamp) in record["files"].items() if current.get(name) != digest]
    for name in drifted:
        del record["files"][name]
    return len(drifted)


def sync_sources(slot, tree, run):
    """Register `tree` as the sources the next Cargo call in `slot` compiles.
    Returns how many files were stamped as changed."""
    tree = Path(tree).resolve()
    with (slot / "sources.lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        record = load_sources(slot)
        if record is not None and record["run"] != run and not record["settled"]:
            # An earlier run ended without settling (killed): its last tree may
            # have changed under a compile. Nothing it recorded can be trusted.
            print(f"build-cache: sources from unsettled run {record['run']} discarded", file=sys.stderr, flush=True)
            record = None
        if record is not None and record["run"] == run and record["tree"] != str(tree):
            forget_drift(record)
        known = record["files"] if record is not None else {}
        now = time.time_ns()
        files, changed, raised = {}, 0, 0
        for name, digest in tree_sources(tree).items():
            entry = known.get(name)
            stamp = entry[1] if entry is not None and entry[0] == digest else now
            changed += stamp == now
            files[name] = [digest, stamp]
            path = tree / name
            info = path.stat()
            if info.st_mtime_ns < stamp:
                os.utime(path, ns=(info.st_atime_ns, stamp))
                raised += 1
        save_sources(slot, {"schema": SOURCES_SCHEMA, "run": run, "settled": False, "tree": str(tree),
                            "files": files})
    print(f"build-cache: sources tree={tree} files={len(files)} changed={changed} raised={raised} "
          f"(changed files rebuild; unchanged files keep their stamp and stay cache hits)", file=sys.stderr, flush=True)
    return changed


def settle_sources(slot, run):
    """Close a slot run. A run that compiled without registering (a tree whose
    lease wrapper predates registration) leaves outputs the record cannot
    describe, so the record is dropped and the next tree is stamped afresh."""
    with (slot / "sources.lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        record = load_sources(slot)
        if record is None:
            return
        if record["run"] != run:
            (slot / SOURCES).unlink()
            print(f"build-cache: no sources registered during this run; slot record reset, the next build "
                  f"stamps every file as changed", flush=True)
            return
        drifted = forget_drift(record)
        record["settled"] = True
        save_sources(slot, record)
        print(f"build-cache: sources settled tree={record['tree']} drifted={drifted}", flush=True)


def sync_from_environment():
    """`build-cache.py sync-sources`: register the current git tree with the
    slot named by the enclosing `build-cache.py <channel>` run."""
    slot, run = os.environ.get("PLEXI_BUILD_CACHE_SLOT"), os.environ.get("PLEXI_BUILD_CACHE_RUN")
    if not slot or not run:
        raise ValueError("sync-sources runs only inside a build-cache.py slot run "
                         "(PLEXI_BUILD_CACHE_SLOT and PLEXI_BUILD_CACHE_RUN are set by scripts/build-cache.py)")
    slot = Path(slot)
    marker = slot / "owner"
    if slot.is_symlink() or not marker.is_file() or marker.read_text() != MARKER:
        raise ValueError(f"sync-sources: {slot} is not an owned build-cache slot")
    top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=False)
    if top.returncode:
        raise ValueError(f"sync-sources: Cargo in slot {slot} from {os.getcwd()}, which is not inside a git "
                         f"work tree; its sources cannot be registered")
    sync_sources(slot, Path(top.stdout.strip()), run)
    return 0


def run(channel, command):
    if not any(channel.startswith(prefix) and channel[len(prefix):].isascii() and channel[len(prefix):].isdigit() for prefix in ("pr-", "task-")):
        raise ValueError("expected pr-<number> or task-<number>")
    # Read both overrides first so a bad value stops before anything changes.
    floor = positive_bytes("PLEXI_BUILD_MIN_FREE_BYTES", DEFAULT_MIN_FREE_BYTES)
    total_cap = positive_bytes("PLEXI_BUILD_CACHE_TOTAL_MAX_BYTES", DEFAULT_TOTAL_MAX_BYTES)
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
                    prune_to_cap(base.parent, total_cap, keep=slot)
                    require_free_space(base, floor)
                    (slot / "last-channel").write_text(owner)
                    run_id = uuid.uuid4().hex
                    env = {**os.environ, "CARGO_TARGET_DIR": str(target),
                           "PLEXI_BUILD_TELEMETRY": str(slot / "telemetry.jsonl"),
                           "PLEXI_BUILD_CACHE_SLOT": str(slot), "PLEXI_BUILD_CACHE_RUN": run_id}
                    if channel.startswith("pr-"):
                        env["PLEXI_BUILD_TEST_CHANNEL"] = channel
                    else:
                        env.pop("PLEXI_BUILD_TEST_CHANNEL", None)
                    print(f"build-cache: slot={slot} retained_bytes={total} wait_seconds={time.monotonic()-started:.3f}", flush=True)
                    try:
                        return subprocess.call(command, env=env)
                    finally:
                        settle_sources(slot, run_id)
                finally:
                    lock.close()
            if time.monotonic() >= deadline:
                raise TimeoutError("all persistent build slots busy")
            time.sleep(0.2)


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["sync-sources"]:
            sys.exit(sync_from_environment())
        if len(sys.argv) == 4 and sys.argv[1] == "seed-codex":
            seed_codex(Path(sys.argv[2]), Path(sys.argv[3]))
            sys.exit(0)
        sys.exit(run(sys.argv[1], sys.argv[2:]))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"build-cache: {error}", file=sys.stderr)
        sys.exit(1)
