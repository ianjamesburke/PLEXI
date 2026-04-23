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

import sys

def _diag(msg: str) -> None:
    """Write a diagnostic line to stderr. The Plexi host forwards subprocess
    stderr into the main log tagged `app::<app_id>`, so this is visible
    regardless of the SDK-side log level and regardless of whether the app
    has wired `ctx.debug` through to the code path."""
    try:
        print(f"[git_log] {msg}", file=sys.stderr, flush=True)
    except Exception:
        pass


def _run(cmd: list[str], cwd: str, timeout: float = 15.0) -> tuple[int, str, str]:
    """Run cmd under cwd. Returns (returncode, stdout, stderr). Never raises."""
    try:
        proc = subprocess.run(
            cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout,
        )
        _diag(
            f"cmd={cmd!r} cwd={cwd!r} rc={proc.returncode} "
            f"stdout_bytes={len(proc.stdout)} stderr_bytes={len(proc.stderr)}"
        )
        if proc.returncode != 0 and proc.stderr:
            _diag(f"  stderr: {proc.stderr.strip()[:500]}")
        return proc.returncode, proc.stdout, proc.stderr
    except FileNotFoundError as e:
        _diag(f"cmd={cmd!r} FileNotFoundError: {e}")
        return 127, "", f"command not found: {cmd[0]} ({e})"
    except subprocess.TimeoutExpired as e:
        _diag(f"cmd={cmd!r} TimeoutExpired: {e}")
        return 124, "", f"timeout running {cmd[0]}: {e}"
    except Exception as e:
        _diag(f"cmd={cmd!r} error: {e}")
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

_LOG_SEP = "\x00"  # NUL-delimited fields within a record (in OUTPUT)
_LOG_RS  = "\x01"  # record separator between commits (in OUTPUT)

# Git's own escapes for control bytes in --format output. We cannot put
# literal \x00 into the --format argv string because Python's subprocess
# rejects arguments containing embedded NUL bytes (fetch_commits silently
# returned zero commits before this fix — git never ran). `%x00` / `%x01`
# tell git to emit those bytes in its output; argv stays ASCII-clean.
_LOG_FORMAT = (
    "%H" + "%x00" +  # 0 full hash
    "%P" + "%x00" +  # 1 parent hashes (space-separated)
    "%an" + "%x00" + # 2 author name
    "%ct" + "%x00" + # 3 commit timestamp (unix)
    "%D" + "%x00" +  # 4 ref names (HEAD -> main, origin/main, …)
    "%s" + "%x01"    # 5 subject
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

MUTED_COLOR = "#6c7086"  # Catppuccin MUTED — used for OTHER_LANE commits
MAX_LANES   = 5         # lanes 0–4 render normally
OTHER_LANE  = 5         # collapse index for overflow refs


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

    v2 algorithm: viewport-scoped lanes. A ref earns a lane only if at least
    one commit it reaches appears in `commits` (the visible window). Hard cap
    of 5 rendered lanes; any overflow collapses to lane index 5 (OTHER_LANE)
    drawn in MUTED.

    Walk order: newest→oldest (commits are already sorted that way).
    1. Walk commits; when a commit's hash matches a ref tip, that ref is
       "seen". Collect seen_refs in commit-recency order.
    2. Allocate lanes 0..4 from seen_refs, with reserved-colour overrides:
       main → blue (0), alpha → green (1), beta → yellow (2) — but ONLY if
       that branch has a commit in the window.
    3. Each commit is assigned to the lane of the first ref whose tip_hash
       chain contains it (walk child→parent via the in-window commit graph).
       If no ref contains it → OTHER_LANE with MUTED.
    """
    if not commits:
        return commits

    # MAX_LANES and OTHER_LANE are module-level constants (gl.OTHER_LANE usable in tests)

    # ── Reserved colour map ───────────────────────────────────────────────────
    # main→0 blue, alpha→1 green, beta→2 yellow (only when seen in window).
    RESERVED: dict[str, int] = {"main": 0, "master": 0, "alpha": 1, "beta": 2}
    RESERVED_REMOTE: dict[str, int] = {
        "origin/main": 0, "origin/master": 0,
        "origin/alpha": 1, "origin/beta": 2,
    }

    # ── Build tip_hash → ref name map ─────────────────────────────────────────
    tip_to_refs: dict[str, list[str]] = {}
    for ref in refs:
        tip_to_refs.setdefault(ref["tip_hash"], []).append(ref["name"])

    # Build a quick lookup: ref name → ref dict
    ref_by_name: dict[str, dict] = {r["name"]: r for r in refs}

    # ── Step 1: collect seen_refs in commit-recency order ─────────────────────
    # A ref is "seen" when we encounter its tip_hash while walking newest→oldest.
    commit_hashes: set[str] = {c["hash"] for c in commits}
    seen_refs: list[str] = []           # ordered by first appearance
    seen_ref_set: set[str] = set()

    for c in commits:
        for rname in tip_to_refs.get(c["hash"], []):
            if rname not in seen_ref_set:
                seen_refs.append(rname)
                seen_ref_set.add(rname)

    # ── Step 2: allocate lanes with reserved-colour overrides ─────────────────
    # We want at most MAX_LANES distinct rendered lanes (0..4); additional refs
    # collapse to OTHER_LANE.
    # Priority: reserved refs come first at their fixed indices; then remaining
    # seen_refs fill gaps in recency order.

    ref_to_lane: dict[str, int] = {}   # ref name → lane index (0-5)

    # Place reserved refs first (only if seen)
    all_reserved = dict(RESERVED)
    all_reserved.update(RESERVED_REMOTE)
    reserved_lanes_used: set[int] = set()

    for rname in seen_refs:
        if rname in all_reserved:
            lane = all_reserved[rname]
            if lane not in reserved_lanes_used:
                ref_to_lane[rname] = lane
                reserved_lanes_used.add(lane)

    # Fill remaining seen_refs into free slots 0..MAX_LANES-1, in recency order
    next_free = 0
    for rname in seen_refs:
        if rname in ref_to_lane:
            continue
        # Find next free slot not reserved
        while next_free < MAX_LANES and next_free in reserved_lanes_used:
            next_free += 1
        if next_free < MAX_LANES:
            ref_to_lane[rname] = next_free
            next_free += 1
        else:
            ref_to_lane[rname] = OTHER_LANE

    # ── Step 3: assign each commit to a lane ──────────────────────────────────
    # Build parent→children adjacency for the in-window graph so we can walk
    # child→parent to find the first containing ref.
    # Strategy: for each commit, check all seen_refs. The ref whose tip is
    # "reachable from" this commit (i.e., the commit is an ancestor of the tip
    # within the window) owns it.
    # Cheap approximation within the viewport: walk the child edge graph
    # forward — a commit belongs to a ref if the ref's tip_hash descends from
    # it within the commits list.
    #
    # Implementation: build child→[parents] so we can propagate ref ownership
    # downward (newest→oldest). Each commit gets the lane of the first ref
    # tip that reaches it via the in-window parent chain.

    hash_to_idx: dict[str, int] = {c["hash"]: i for i, c in enumerate(commits)}

    # For each commit, store which ref "owns" it (first ref tip that is a
    # descendant in the window).
    commit_ref: list[Optional[str]] = [None] * len(commits)

    # Walk newest→oldest. When we hit a ref tip, propagate ownership down the
    # first-parent chain until we hit a commit already owned or a gap.
    # This is O(n * refs) in the worst case but n is capped at ~168 commits/week.

    # Build parent chains: for each commit, track its first parent index
    first_parent_idx: list[Optional[int]] = []
    for c in commits:
        p0 = c["parents"][0] if c["parents"] else None
        first_parent_idx.append(hash_to_idx.get(p0) if p0 else None)

    # Process refs in lane-priority order (lowest lane first)
    sorted_refs = sorted(ref_to_lane.items(), key=lambda kv: kv[1])
    for rname, lane in sorted_refs:
        ref = ref_by_name.get(rname)
        if ref is None:
            continue
        tip = ref["tip_hash"]
        start_idx = hash_to_idx.get(tip)
        if start_idx is None:
            continue
        # Walk from tip down the first-parent chain, assigning ownership
        idx: Optional[int] = start_idx
        while idx is not None:
            if commit_ref[idx] is not None:
                break   # already owned by a higher-priority (lower lane) ref
            commit_ref[idx] = rname
            idx = first_parent_idx[idx]

    # ── Step 4: assign lane + colour to each commit ───────────────────────────
    def _lane_color(lane: int, rname: Optional[str]) -> str:
        if lane < 3:
            return PALETTE[lane]
        if lane == OTHER_LANE or lane >= MAX_LANES:
            return MUTED_COLOR
        if rname:
            idx = 3 + (_fnv1a(rname) % (len(PALETTE) - 3))
        else:
            idx = 3 + (lane % (len(PALETTE) - 3))
        return PALETTE[min(idx, len(PALETTE) - 1)]

    for i, c in enumerate(commits):
        rname = commit_ref[i]
        if rname is not None:
            lane = ref_to_lane.get(rname, OTHER_LANE)
        else:
            # Not reachable from any ref tip in window — collapse to other
            lane = OTHER_LANE
        c["lane"] = lane
        c["color"] = _lane_color(lane, rname)
        # Stash the owning ref name for edge colouring
        c["_ref"] = rname

    return commits


def build_edges(commits: list[dict]) -> list[tuple[str, str]]:
    """Return (child_hash, parent_hash) pairs for all commit edges."""
    edges: list[tuple[str, str]] = []
    for c in commits:
        for p in c["parents"]:
            edges.append((c["hash"], p))
    return edges
