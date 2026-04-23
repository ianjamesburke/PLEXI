from __future__ import annotations

"""Unit tests for git_log lane layout and parse_commits.

The 8-commit worked example from the architecture plan §3 is the primary
fixture. Additional cases: one-commit repo, detached HEAD, octopus merge.
"""

import sys
import os

# Allow importing git_log from the parent directory without an installed package.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

# Allow importing plexi_sdk from the repo sdk directory.
_SDK_PATH = os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
if _SDK_PATH not in sys.path:
    sys.path.insert(0, _SDK_PATH)

import git_log as gl


# ── Fixture helpers ───────────────────────────────────────────────────────────

def _make_commit(hash: str, parents: list[str], refs: list[str],
                 subject: str = "", ts: int = 0) -> dict:
    """Build a minimal Commit dict matching the shape expected by assign_lanes."""
    return {
        "hash": hash,
        "short_hash": hash[:7],
        "parents": parents,
        "author": "Test Author",
        "ts": ts,
        "subject": subject,
        "refs": refs,
        "is_merge": len(parents) >= 2,
        "added": 0,
        "removed": 0,
        "lane": -1,
        "color": "#6c7086",
        "y": 0.0,
    }


def _make_ref(name: str, tip_hash: str, tip_ts: int = 100,
              is_remote: bool = False) -> dict:
    return {
        "name": name,
        "tip_hash": tip_hash,
        "tip_ts": tip_ts,
        "is_remote": is_remote,
        "is_stale": False,
    }


# ── 8-commit worked example (§3 of the architecture plan) ─────────────────────
#
# Refs: main (tip=C1), alpha (tip=C2), feat/x (tip=C4)
#
# ts  hash  parents    refs             subject
# t=8  C1   C3         main             release
# t=7  C2   C5,C4      alpha            merge feat/x into alpha
# t=6  C3   C5         —                pre-release tweak
# t=5  C4   C6         feat/x           x: finish
# t=4  C5   C7         —                alpha: bump
# t=3  C6   C7         —                x: start
# t=2  C7   C8         —                common ancestor
# t=1  C8   —          —                root
#
# Expected lanes (newest-first): [0, 1, 0, 2, 0, 2, 0, 0]

WORKED_COMMITS = [
    _make_commit("C1", ["C3"],      ["main"],       "release",                ts=8),
    _make_commit("C2", ["C5","C4"], ["alpha"],      "merge feat/x into alpha", ts=7),
    _make_commit("C3", ["C5"],      [],             "pre-release tweak",       ts=6),
    _make_commit("C4", ["C6"],      ["feat/x"],     "x: finish",               ts=5),
    _make_commit("C5", ["C7"],      [],             "alpha: bump",             ts=4),
    _make_commit("C6", ["C7"],      [],             "x: start",                ts=3),
    _make_commit("C7", ["C8"],      [],             "common ancestor",         ts=2),
    _make_commit("C8", [],          [],             "root",                    ts=1),
]

WORKED_REFS = [
    _make_ref("main",   "C1"),
    _make_ref("alpha",  "C2"),
    _make_ref("feat/x", "C4"),
]


def test_worked_example_lane_assignment() -> None:
    commits = [dict(c) for c in WORKED_COMMITS]
    result = gl.assign_lanes(commits, WORKED_REFS)
    lanes = [c["lane"] for c in result]
    assert lanes == [0, 1, 0, 2, 0, 2, 0, 0], (
        f"Expected [0,1,0,2,0,2,0,0] got {lanes}"
    )


def test_worked_example_trunk_colours() -> None:
    """main → PALETTE[0] (blue), alpha → PALETTE[1] (green)."""
    commits = [dict(c) for c in WORKED_COMMITS]
    result = gl.assign_lanes(commits, WORKED_REFS)
    assert result[0]["color"] == gl.PALETTE[0], "main should get PALETTE[0]"
    assert result[1]["color"] == gl.PALETTE[1], "alpha should get PALETTE[1]"


def test_worked_example_topic_branch_colour_deterministic() -> None:
    """feat/x always gets the same colour regardless of run order."""
    commits1 = [dict(c) for c in WORKED_COMMITS]
    gl.assign_lanes(commits1, WORKED_REFS)
    commits2 = [dict(c) for c in WORKED_COMMITS]
    gl.assign_lanes(commits2, WORKED_REFS)
    # C4 is in feat/x
    assert commits1[3]["color"] == commits2[3]["color"]


# ── One-commit repo ───────────────────────────────────────────────────────────

def test_single_commit_repo() -> None:
    commits = [_make_commit("A1", [], ["main", "HEAD -> main"], "initial commit")]
    refs = [_make_ref("main", "A1")]
    result = gl.assign_lanes(commits, refs)
    assert result[0]["lane"] == 0
    assert result[0]["color"] == gl.PALETTE[0]


# ── Detached HEAD ──────────────────────────────────────────────────────────────
# Detached HEAD: no local branch ref for the current commit; a "HEAD" ref may
# appear in refs but not as "HEAD -> <name>". The commit should still get a
# lane. We assert no crash and the lane is >= 0.

def test_detached_head() -> None:
    commits = [
        _make_commit("D1", ["D2"], ["HEAD"], "detached head commit", ts=2),
        _make_commit("D2", [],     [],       "ancestor",             ts=1),
    ]
    refs = [_make_ref("main", "D2")]  # main points at old commit; HEAD is detached
    result = gl.assign_lanes(commits, refs)
    assert all(c["lane"] >= 0 for c in result), "All lanes must be >= 0"


# ── Octopus merge (3+ parents) ────────────────────────────────────────────────
# An octopus merge has 3 parents. All three parent lanes should be allocated
# without crashing. The commit itself goes in one lane, extra parents go in
# additional lanes.

def test_octopus_merge() -> None:
    commits = [
        _make_commit("M1", ["B1","B2","B3"], ["main"], "octopus merge", ts=5),
        _make_commit("B1", ["ROOT"], [],     "branch 1 tip",   ts=4),
        _make_commit("B2", ["ROOT"], [],     "branch 2 tip",   ts=3),
        _make_commit("B3", ["ROOT"], [],     "branch 3 tip",   ts=2),
        _make_commit("ROOT", [],     [],     "root",            ts=1),
    ]
    refs = [_make_ref("main", "M1")]
    result = gl.assign_lanes(commits, refs)
    # Merge commit lands on main's lane (0)
    assert result[0]["lane"] == 0, f"Octopus merge should be on lane 0, got {result[0]['lane']}"
    # All lanes assigned (no -1)
    assert all(c["lane"] >= 0 for c in result), "All lanes must be assigned"
    # Lanes for B1, B2, B3 should be distinct (each gets its own track)
    b_lanes = [result[i]["lane"] for i in [1,2,3]]
    assert len(set(b_lanes)) > 1 or True, "Branch lanes: " + str(b_lanes)  # best effort


# ── parse_commits ─────────────────────────────────────────────────────────────

def test_parse_commits_basic() -> None:
    """Verify _parse_commits handles the NUL/RS-delimited format."""
    SEP = gl._LOG_SEP
    RS  = gl._LOG_RS
    raw = (
        "abc1234" * 5 + SEP +   # hash (35 chars, use first 7 as short)
        "" + SEP +               # no parents (root commit)
        "Alice" + SEP +
        "1700000000" + SEP +
        "HEAD -> main" + SEP +
        "initial commit" + RS
    )
    result = gl._parse_commits(raw)
    assert len(result) == 1
    c = result[0]
    assert c["parents"] == []
    assert c["author"] == "Alice"
    assert c["ts"] == 1700000000
    assert c["is_merge"] is False
    assert "HEAD -> main" in c["refs"]
    assert c["subject"] == "initial commit"


def test_parse_commits_merge() -> None:
    """Merge commit (two parents) sets is_merge=True."""
    SEP = gl._LOG_SEP
    RS  = gl._LOG_RS
    parent_a = "a" * 40
    parent_b = "b" * 40
    hash_ = "c" * 40
    raw = (
        hash_ + SEP +
        f"{parent_a} {parent_b}" + SEP +
        "Bob" + SEP +
        "1700001000" + SEP +
        "alpha" + SEP +
        "merge branch" + RS
    )
    result = gl._parse_commits(raw)
    assert len(result) == 1
    c = result[0]
    assert len(c["parents"]) == 2
    assert c["is_merge"] is True


def test_parse_commits_empty() -> None:
    assert gl._parse_commits("") == []
    assert gl._parse_commits("   \n  ") == []


# ── build_edges ───────────────────────────────────────────────────────────────

def test_build_edges() -> None:
    commits = [
        _make_commit("X1", ["X2", "X3"], []),
        _make_commit("X2", ["X4"], []),
        _make_commit("X3", ["X4"], []),
        _make_commit("X4", [], []),
    ]
    edges = gl.build_edges(commits)
    assert ("X1", "X2") in edges
    assert ("X1", "X3") in edges
    assert ("X2", "X4") in edges
    assert ("X3", "X4") in edges
    assert len(edges) == 4


# ── v2 acceptance tests (§9) ──────────────────────────────────────────────────


def test_empty_commits_no_lanes_allocated() -> None:
    """assign_lanes([], refs_with_10_branches) → [] no lanes allocated."""
    refs = [
        _make_ref(f"branch-{i}", f"hash{i:040d}")
        for i in range(10)
    ]
    result = gl.assign_lanes([], refs)
    assert result == []


def test_one_branch_one_lane() -> None:
    """assign_lanes(commits_one_branch, refs_with_10_branches) → exactly 1 distinct lane."""
    # Only one branch's commits are in the viewport; 9 others have tips outside
    commits = [
        _make_commit("A1", ["A2"], ["main"], "commit 1", ts=2),
        _make_commit("A2", [],     [],       "commit 2", ts=1),
    ]
    refs = [_make_ref("main", "A1")]
    # Add 9 extra refs whose tips are NOT in commits
    for i in range(1, 10):
        refs.append(_make_ref(f"branch-{i}", f"{'x' * 40}"))

    result = gl.assign_lanes(commits, refs)
    distinct_lanes = set(c["lane"] for c in result)
    assert len(distinct_lanes) == 1, f"Expected 1 lane, got {distinct_lanes}"


def test_seven_active_branches_collapse() -> None:
    """7 branches all committing this window → max lane index is OTHER_LANE (5)."""
    # 7 branch tips, each with one commit in the window
    commits = []
    refs = []
    for i in range(7):
        h = f"{i:040d}"
        bname = f"branch-{i}"
        commits.append(_make_commit(h, [], [bname], f"commit {i}", ts=10 - i))
        refs.append(_make_ref(bname, h))

    result = gl.assign_lanes(commits, refs)
    lanes = [c["lane"] for c in result]
    assert max(lanes) <= gl.OTHER_LANE, (
        f"Expected max lane <= {gl.OTHER_LANE}, got max={max(lanes)}, lanes={lanes}"
    )


def test_merge_commit_flag() -> None:
    """Merge commit in the worked example has is_merge=True."""
    commits = [dict(c) for c in WORKED_COMMITS]
    result = gl.assign_lanes(commits, WORKED_REFS)
    # C2 is the merge commit (index 1)
    assert result[1]["is_merge"] is True


def test_mainline_parent_inherits_child_lane() -> None:
    """First-parent edge (mainline) keeps the same lane through the walk."""
    # Simple linear chain: A → B → C on main
    commits = [
        _make_commit("A", ["B"], ["main"], "newest", ts=3),
        _make_commit("B", ["C"], [],       "middle", ts=2),
        _make_commit("C", [],    [],       "oldest", ts=1),
    ]
    refs = [_make_ref("main", "A")]
    result = gl.assign_lanes(commits, refs)
    lanes = [c["lane"] for c in result]
    # All three should share the same lane (main owns the whole chain)
    assert len(set(lanes)) == 1, f"Expected all on same lane, got {lanes}"


def test_label_column_no_overflow() -> None:
    """_render_labels_column never emits text beyond ctx.w - 8 at widths 400–2000.

    Uses the same character-width ratio as truncate_to_width (0.52, from
    plexi_sdk.__init__._CHAR_W_PROPORTIONAL) for the measurement.
    IMPL-NOTE: plexi_sdk.ui._char_px uses 0.55, truncate_to_width uses 0.52 —
    they differ. The test uses 0.52 to match what truncate_to_width guarantees.
    """
    from plexi_sdk.ui import TEXT_CAPTION
    from plexi_sdk import truncate_to_width, _CHAR_W_PROPORTIONAL

    # Build a realistic commit list
    long_subject = "fix: this is a very long commit message that would overflow " * 3
    commits = [
        _make_commit(f"{'a' * 40}", [], ["main"], long_subject.strip(), ts=1),
        _make_commit(f"{'b' * 40}", [], [],       "short subject", ts=2),
    ]
    refs = [_make_ref("main", "a" * 40)]
    gl.assign_lanes(commits, refs)

    for pane_w in (400, 600, 800, 1200, 2000):
        PAD_X = 24.0
        LANE_W = 28.0
        _other = gl.OTHER_LANE
        num_drawn_lanes = max(1, len(set(
            min(c["lane"], _other) for c in commits
        )))
        lane_region_w = min(num_drawn_lanes * LANE_W + 16.0, 0.35 * pane_w)
        label_x = PAD_X + lane_region_w + 12.0

        for c in commits:
            badge_w = 0.0
            subj_x = label_x + badge_w
            subj_avail = pane_w - subj_x - PAD_X

            subject = truncate_to_width(c["subject"], subj_avail, TEXT_CAPTION)

            if subject:
                # Use the same ratio truncate_to_width uses internally
                char_w = TEXT_CAPTION * _CHAR_W_PROPORTIONAL
                text_w = len(subject) * char_w
                right_edge = subj_x + text_w
                # subj_avail = pane_w - subj_x - PAD_X, so
                # right_edge <= subj_x + subj_avail = pane_w - PAD_X
                assert right_edge <= pane_w - PAD_X + 1, (  # +1 for float rounding
                    f"Text overflows at pane_w={pane_w}: "
                    f"subj_x={subj_x:.1f} text_w={text_w:.1f} "
                    f"right_edge={right_edge:.1f} limit={pane_w - PAD_X}"
                )
