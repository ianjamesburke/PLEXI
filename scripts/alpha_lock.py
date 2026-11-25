"""Serialize alpha writers across all worktrees of a repository."""
from contextlib import contextmanager
import fcntl
import os
from pathlib import Path
import subprocess
import sys
import time

OWNER = "PLEXI_ALPHA_LOCK_OWNER"
FD = "PLEXI_ALPHA_LOCK_FD"


def lock_path():
    common = subprocess.check_output(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], text=True
    ).strip()
    return Path(common) / "plexi-alpha-writer.lock"


def try_lock(fd):
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        return False
    return True


def inherited_fd(probe):
    try:
        owner, fd = int(os.environ[OWNER]), int(os.environ[FD])
        if fd > 2 and os.path.samestat(os.fstat(fd), os.fstat(probe.fileno())):
            # A separately opened descriptor cannot take this flock while the
            # independent probe is contending. Only the inherited open file
            # description can reacquire it; the PID must match the holder record.
            if owner > 1 and probe.read().strip() == str(owner) and try_lock(fd):
                return fd
    except (KeyError, ValueError, OSError):
        pass
    return None


@contextmanager
def writer_lock(path, timeout=60):
    # Keep the inode in place. Unlinking would let two writers lock different inodes.
    with path.open("a+", encoding="utf-8") as probe:
        deadline = time.monotonic() + timeout
        while not try_lock(probe.fileno()):
            probe.seek(0)
            fd = inherited_fd(probe)
            if fd is not None:
                yield fd
                return
            if time.monotonic() >= deadline:
                probe.seek(0)
                raise TimeoutError(f"alpha writer lock timeout at {path}; holder pid={probe.read().strip() or 'unknown'}")
            time.sleep(0.05)
        probe.seek(0)
        probe.truncate()
        probe.write(f"{os.getpid()}\n")
        probe.flush()
        previous = {key: os.environ.get(key) for key in (OWNER, FD)}
        os.environ.update({OWNER: str(os.getpid()), FD: str(probe.fileno())})
        try:
            yield probe.fileno()
        finally:
            for key, value in previous.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value


def main():
    try:
        if sys.argv[1:] == ["--verify"]:
            with lock_path().open("r", encoding="utf-8") as probe:
                if try_lock(probe.fileno()) or inherited_fd(probe) is None:
                    raise RuntimeError("missing verified inherited alpha writer lock")
            return 0
        with writer_lock(lock_path()) as fd:
            return subprocess.call(sys.argv[1:], pass_fds=(fd,))
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"alpha-lock: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
