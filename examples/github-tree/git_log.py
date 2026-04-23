from __future__ import annotations

"""git_log — pure-function git data layer for commit_graph.py.

No SDK imports. All functions take `repo_root: str` and return plain data
structures so they can be unit-tested independently.
"""

import subprocess
import re
from typing import Optional

# ── Data shapes ───────────────────────────────────────────────────────────────

# Commit dict keys:
#   hash       str   — full 40-char SHA
#   short_hash str   — first 7 chars
#   parents    list  — list of full SHA strings
#   author     str
#   ts         int   — unix timestamp
#   subject    str
#   refs       list  — ref names from %D field (e.g. "HEAD -> main", "origin/alpha")
#   is_merge   bool  — True when len(parents) >= 2
#   added      int   — lines added (0 if stats unavailable)
#   removed    int   — lines removed (0 if stats unavailable)
#   lane       int   — assigned by lane_layout(), -1 before assignment
#   color      str   — assigned by lane_layout()
#   y          float — assigned by renderer

# Ref dict keys:
#   name       str
#   tip_hash   str   — full SHA of the tip commit
#   tip_ts     int   — unix timestamp of the tip commit
#   is_remote  bool
#   is_stale   bool  — tip_ts < now - 30*86400


# ── Subprocess helper ─────────────────────────────────────────────────────────

def _run(cmd: list[str], cwd: str, timeout: float = 15.0) -> tuple[int, str, str]:
    """Run cmd under cwd. Returns (returncode, stdout, stderr). Never raises."""
    try:
        proc = subprocess.run(
            cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout,
        )
        return proc.returncode, proc.stdout, proc.stderr
    except FileNotFoundError as e:
        return 127, "", f"command not found: {cmd[0]} ({e})"
    except subprocess.TimeoutExpired as e:
        return 124, "", f"timeout running {cmd[0]}: {e}"
    except Exception as e:
        return 1, "", f"error running {cmd[0]}: {e}"


# ── Repo detection ────────────────────────────────────────────────────────────

def find_repo_root(cwd: str) -> Optional[str]:
    """Return the git root containing `cwd`, or None."""
    rc, out, _ = _run(["git", "rev-parse", "--show-toplevel"], cwd=cwd)
    if rc != 0 or not out.strip():
        return None
    return out.strip()


def get_head_sha(repo_root: str) -> Optional[str]:
    """Return current HEAD SHA (works in detached HEAD too)."""
    rc, out, _ = _run(["git", "rev-parse", "HEAD"], cwd=repo_root)
    if rc != 0:
        return None
    return out.strip()


def get_head_branch(repo_root: str) -> Optional[str]:
    """Return current branch name, or None if detached HEAD."""
    rc, out, _ = _run(["git", "symbolic-ref", "--short", "HEAD"], cwd=repo_root)
    if rc != 0:
        return None
    return out.strip() or None


def get_origin_remote(repo_root: str) -> Optional[str]:
    """Return the URL of `origin`, or None."""
    rc, out, _ = _run(["git", "remote", "get-url", "origin"], cwd=repo_root)
    if rc != 0:
        return None
    return out.strip() or None


# ── Refs ──────────────────────────────────────────────────────────────────────

def fetch_refs(repo_root: str, now_ts: int) -> list[dict]:
    """Return a list of Ref dicts from `git for-each-ref`."""
    fmt = "%(refname:short)\t%(objectname)\t%(committerdate:unix)"
    rc, out, _ = _run(
        ["git", "for-each-ref", f"--format={fmt}", "refs/heads", "refs/remotes"],
        cwd=repo_root,
    )
    if rc != 0:
        return []

    stale_cutoff = now_ts - 30 * 86400
    refs: list[dict] = []
    for line in out.splitlines():
        line = line.strip()
        if not line:
            continue
        parts = line.split("\t", 2)
        if len(parts) < 3:
            continue
        name, tip_hash, ts_str = parts
        try:
            tip_ts = int(ts_str)
        except ValueError:
            tip_ts = 0
        is_remote = name.startswith("origin/") or "/" in name.split("/", 1)[0]
        # A name with a "/" where the first segment isn't "origin" is a
        # feature branch (e.g. "feature/foo") — not a remote ref.
        # Simple heuristic: if it contains "/" and starts with "origin" → remote.
        is_remote = name.startswith("origin/")
        refs.append({
            "name": name,
            "tip_hash": tip_hash,
            "tip_ts": tip_ts,
            "is_remote": is_remote,
            "is_stale": tip_ts < stale_cutoff,
        })
    return refs


# ── Commit log ────────────────────────────────────────────────────────────────

_LOG_SEP = "\x00"  # NUL-delimited fields within a record
_LOG_RS  = "\x01"  # record separator between commits

_LOG_FORMAT = (
    "%H" + _LOG_SEP +  # 0 full hash
    "%P" + _LOG_SEP +  # 1 parent hashes (space-separated)
    "%an" + _LOG_SEP + # 2 author name
    "%ct" + _LOG_SEP + # 3 commit timestamp (unix)
    "%D" + _LOG_SEP +  # 4 ref names (HEAD -> main, origin/main, …)
    "%s" + _LOG_RS     # 5 subject
)


def fetch_commits(repo_root: str, since_ts: int, until_ts: int) -> list[dict]:
    """Return commits (newest-first) in the [since_ts, until_ts] window.

    `git log` does NOT accept a bare unix timestamp for `--since`/`--until` —
    it tries to parse the value as a relative/absolute date string and
    silently returns zero results if it can't. Use the `@<seconds>` form,
    which is git's documented unix-epoch date syntax (see gitrevisions(7)).
    """
    rc, out, _ = _run(
        [
            "git", "log", "--all", "--date-order",
            f"--format={_LOG_FORMAT}",
            f"--since=@{since_ts}",
            f"--until=@{until_ts}",
        ],
        cwd=repo_root,
        timeout=20.0,
    )
    if rc != 0:
        return []
    return _parse_commits(out)


def _parse_commits(raw: str) -> list[dict]:
    """Parse raw git log output into Commit dicts. Pure function."""
    commits: list[dict] = []
    for record in raw.split(_LOG_RS):
        record = record.strip()
        if not record:
            continue
        fields = record.split(_LOG_SEP)
        if len(fields) < 6:
            continue
        h, parents_raw, author, ts_str, refs_raw, subject = fields[:6]
        h = h.strip()
        if not h:
            continue
        parents = [p for p in parents_raw.strip().split() if p]
        try:
            ts = int(ts_str.strip())
        except ValueError:
            ts = 0
        ref_list = [r.strip() for r in refs_raw.split(",") if r.strip()]
        commits.append({
            "hash": h,
            "short_hash": h[:7],
            "parents": parents,
            "author": author.strip(),
            "ts": ts,
            "subject": subject.strip(),
            "refs": ref_list,
            "is_merge": len(parents) >= 2,
            "added": 0,
            "removed": 0,
            "lane": -1,
            "color": "#6c7086",
            "y": 0.0,
        })
    return commits


# Public alias for unit tests
parse_commits = _parse_commits


# ── Numstats ──────────────────────────────────────────────────────────────────

_NUMSTAT_COMMIT_MARKER = "__COMMIT__"
_MAX_COMMITS_FOR_STATS = 2000


def fetch_numstats(
    repo_root: str, since_ts: int, until_ts: int, commit_count: int
) -> dict[str, tuple[int, int]]:
    """Return {hash: (added, removed)} for each commit in the window.

    Returns an empty dict if commit_count > _MAX_COMMITS_FOR_STATS (too slow).
    """
    if commit_count > _MAX_COMMITS_FOR_STATS:
        return {}

    fmt = f"--format={_NUMSTAT_COMMIT_MARKER}%H"
    rc, out, _ = _run(
        [
            "git", "log", "--all", "--numstat", fmt,
            f"--since=@{since_ts}", f"--until=@{until_ts}",
        ],
        cwd=repo_root,
        timeout=30.0,
    )
    if rc != 0:
        return {}
    return _parse_numstats(out)


def _parse_numstats(raw: str) -> dict[str, tuple[int, int]]:
    """Parse `git log --numstat` output. Pure function."""
    result: dict[str, tuple[int, int]] = {}
    current_hash: Optional[str] = None
    added = removed = 0

    for line in raw.splitlines():
        if line.startswith(_NUMSTAT_COMMIT_MARKER):
            if current_hash is not None:
                result[current_hash] = (added, removed)
            current_hash = line[len(_NUMSTAT_COMMIT_MARKER):].strip()
            added = removed = 0
            continue
        if current_hash is None:
            continue
        parts = line.split("\t", 2)
        if len(parts) < 2:
            continue
        a_str, r_str = parts[0], parts[1]
        # Binary files show "-" instead of a number
        try:
            added += int(a_str)
        except ValueError:
            pass
        try:
            removed += int(r_str)
        except ValueError:
            pass

    if current_hash is not None:
        result[current_hash] = (added, removed)

    return result


# ── Lane layout ───────────────────────────────────────────────────────────────

# Colour palette: 12 colours (Catppuccin Mocha-compatible)
PALETTE = [
    "#89b4fa",  # 0 blue       (reserved: main / first trunk)
    "#a6e3a1",  # 1 green      (reserved: alpha if present)
    "#f9e2af",  # 2 yellow     (reserved: beta if present)
    "#f5c2e7",  # 3 pink
    "#cba6f7",  # 4 mauve
    "#fab387",  # 5 peach
    "#94e2d5",  # 6 teal
    "#eba0ac",  # 7 maroon
    "#f38ba8",  # 8 red
    "#74c7ec",  # 9 sapphire
    "#b4befe",  # 10 lavender
    "#a6adc8",  # 11 subtext
]

_TRUNK_PATTERN = re.compile(
    r"^(main|master|alpha|beta|develop|release/.+)$"
)


def _fnv1a(s: str) -> int:
    """FNV-1a 32-bit hash for deterministic colour assignment."""
    h = 0x811c9dc5
    for ch in s.encode():
        h ^= ch
        h = (h * 0x01000193) & 0xFFFFFFFF
    return h


def _branch_name_from_refs(refs: list[str]) -> Optional[str]:
    """Extract a local branch name from a commit's ref list, if any."""
    for r in refs:
        # "HEAD -> main" → "main"
        if r.startswith("HEAD -> "):
            return r[8:]
        # skip remote refs and tags
        if r.startswith("origin/") or r.startswith("tag:"):
            continue
        return r
    return None


def assign_lanes(commits: list[dict], refs: list[dict]) -> list[dict]:
    """Assign lane and color to each commit (mutates and returns the list).

    Implements the pre-pass + main-pass algorithm from the architecture plan.
    """
    if not commits:
        return commits

    # ── Pre-pass: identify trunk branches and assign reserved lanes ───────────
    # Collect local trunk names in priority order
    trunk_names: list[str] = []
    for priority_name in ("main", "master", "alpha", "beta", "develop"):
        for ref in refs:
            if not ref["is_remote"] and ref["name"] == priority_name:
                if priority_name not in trunk_names:
                    trunk_names.append(priority_name)
    # Add any remaining local trunks matching the pattern
    for ref in refs:
        if not ref["is_remote"] and _TRUNK_PATTERN.match(ref["name"]):
            if ref["name"] not in trunk_names:
                trunk_names.append(ref["name"])

    # Map trunk name → reserved lane index
    trunk_lane: dict[str, int] = {}
    for i, name in enumerate(trunk_names):
        trunk_lane[name] = i
    # Also map remote counterparts to the same lane
    for name, lane in list(trunk_lane.items()):
        trunk_lane[f"origin/{name}"] = lane

    num_reserved = len(trunk_names)

    # Map tip_hash → list of branch names for that tip
    tip_to_names: dict[str, list[str]] = {}
    for ref in refs:
        tip = ref["tip_hash"]
        tip_to_names.setdefault(tip, []).append(ref["name"])

    # ── Colour assignment helper ──────────────────────────────────────────────

    def lane_color(lane: int, branch_hint: Optional[str] = None) -> str:
        if lane < 3 and lane < len(PALETTE):
            return PALETTE[lane]
        if branch_hint:
            idx = 3 + (_fnv1a(branch_hint) % 9)
        else:
            idx = 3 + ((lane - num_reserved) % 9)
        return PALETTE[min(idx, len(PALETTE) - 1)]

    # ── Main pass ─────────────────────────────────────────────────────────────
    # active_lanes[i] = hash that lane i is currently "expecting" (the child
    # that is waiting for its parent to appear as we walk newest→oldest).
    # None means the lane slot is free.

    # Seed with trunk tips
    # Build: tip_hash for each trunk lane
    trunk_tip_by_lane: dict[int, str] = {}
    for ref in refs:
        name = ref["name"]
        if name in trunk_lane and not ref["is_remote"]:
            lane_idx = trunk_lane[name]
            # Prefer the first ref encountered; keep the most-recent tip
            if lane_idx not in trunk_tip_by_lane:
                trunk_tip_by_lane[lane_idx] = ref["tip_hash"]

    # Also seed from remotes when no local counterpart found
    for ref in refs:
        name = ref["name"]
        if name in trunk_lane and ref["is_remote"]:
            lane_idx = trunk_lane[name]
            if lane_idx not in trunk_tip_by_lane:
                trunk_tip_by_lane[lane_idx] = ref["tip_hash"]

    # Initialise active_lanes with num_reserved slots
    active_lanes: list[Optional[str]] = [None] * num_reserved
    for lane_idx, tip in trunk_tip_by_lane.items():
        if lane_idx < num_reserved:
            active_lanes[lane_idx] = tip

    # Track which lane index "owns" each branch name (for topic branches)
    branch_lane: dict[str, int] = dict(trunk_lane)

    def first_free_non_reserved() -> int:
        """Return the leftmost free non-reserved slot, or append a new one."""
        for i in range(num_reserved, len(active_lanes)):
            if active_lanes[i] is None:
                return i
        active_lanes.append(None)
        return len(active_lanes) - 1

    def first_free_after(min_lane: int) -> int:
        """Return leftmost free slot >= min_lane (for merge parent diverge)."""
        for i in range(min_lane, len(active_lanes)):
            if active_lanes[i] is None:
                return i
        active_lanes.append(None)
        return len(active_lanes) - 1

    # Build hash → set(lane_indices that are expecting this hash)
    def build_expecting() -> dict[str, list[int]]:
        exp: dict[str, list[int]] = {}
        for i, h in enumerate(active_lanes):
            if h is not None:
                exp.setdefault(h, []).append(i)
        return exp

    for commit in commits:
        h = commit["hash"]
        parents = commit["parents"]
        refs_list = commit["refs"]

        # Determine if this commit belongs to a named branch
        branch_hint = _branch_name_from_refs(refs_list)

        # ── Step 1: pick this commit's lane ──────────────────────────────────
        expecting = build_expecting()
        if h in expecting:
            # Use the leftmost lane already expecting this hash
            chosen_lane = expecting[h][0]
        else:
            # Not expected by any active lane — either it's outside the
            # current viewport's seen tips or a new topic branch appearing
            if branch_hint and branch_hint in branch_lane:
                chosen_lane = branch_lane[branch_hint]
            else:
                chosen_lane = first_free_non_reserved()
                if branch_hint:
                    branch_lane[branch_hint] = chosen_lane

        # Ensure chosen_lane is within active_lanes
        while len(active_lanes) <= chosen_lane:
            active_lanes.append(None)

        # ── Step 2: record lane and colour ───────────────────────────────────
        commit["lane"] = chosen_lane
        commit["color"] = lane_color(chosen_lane, branch_hint)

        # ── Step 3: clear this commit from all lanes expecting it ────────────
        for i, slot in enumerate(active_lanes):
            if slot == h:
                active_lanes[i] = None

        # ── Step 4: place parents into lanes ─────────────────────────────────
        if not parents:
            pass  # root commit — nothing to do
        elif len(parents) == 1:
            # Non-merge: first parent stays in chosen_lane
            active_lanes[chosen_lane] = parents[0]
        else:
            # Merge: first parent stays in chosen_lane (mainline-first)
            active_lanes[chosen_lane] = parents[0]
            # Additional parents each get a new lane, allocated right of chosen_lane
            for extra_parent in parents[1:]:
                # Check if this parent is already expected somewhere
                exp2 = build_expecting()
                if extra_parent in exp2:
                    # Already tracked — don't allocate a second lane for it
                    pass
                else:
                    new_lane = first_free_after(chosen_lane + 1)
                    while len(active_lanes) <= new_lane:
                        active_lanes.append(None)
                    active_lanes[new_lane] = extra_parent

    return commits


def build_edges(commits: list[dict]) -> list[tuple[str, str]]:
    """Return (child_hash, parent_hash) pairs for all commit edges."""
    edges: list[tuple[str, str]] = []
    for c in commits:
        for p in c["parents"]:
            edges.append((c["hash"], p))
    return edges
