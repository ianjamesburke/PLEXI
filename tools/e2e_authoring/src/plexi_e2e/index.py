"""Session index: one browsable table over every captured session.

Regenerates ``benchmarks/app-authoring/sessions/INDEX.md`` from the scorecards.
The index is a derived view — it reads each session's ``scorecard.json`` (building
one on the fly if absent) and never stores anything the sessions don't already
hold. A newcomer reads this table to find a session, then opens that session dir
to see what the agent was asked, did, and where it struggled.
"""

from __future__ import annotations

from pathlib import Path

from .scorecard import build_scorecard

INDEX = "INDEX.md"

_HEADER = """# Session index

One row per captured session, newest first. Regenerate with `plexi-e2e index`
(or `just e2e-baseline` after a sweep). Every row is version-stamped so scores
stay comparable across time; `dry-run` rows are structural baselines captured
without a live host (no child agent ran).

Columns: session id, fixture, difficulty, mode, outcome, CLI/SDK versions,
wall-clock seconds, parent turns, lines of code.
"""

_TABLE_HEADER = (
    "| Session | Fixture | Diff | Mode | Outcome | CLI | SDK | Wall (s) | Turns | LOC |\n"
    "|---------|---------|------|------|---------|-----|-----|----------|-------|-----|\n"
)


def _fmt(value: object) -> str:
    return "—" if value is None else str(value)


def _session_dirs(sessions_root: Path) -> list[Path]:
    return sorted(
        (p for p in sessions_root.iterdir() if p.is_dir() and (p / "manifest.json").is_file()),
        key=lambda p: p.name,
        reverse=True,
    )


def build_index(sessions_root: Path) -> str:
    """Render the full INDEX.md text from the sessions under ``sessions_root``."""
    sessions_root = Path(sessions_root)
    rows = []
    for session_dir in _session_dirs(sessions_root):
        card = build_scorecard(session_dir)
        rows.append(
            "| {sid} | {fx} | {diff} | {mode} | {outcome} | {cli} | {sdk} | {wall} | {turns} | {loc} |".format(
                sid=card.session_id,
                fx=card.fixture_id,
                diff=card.difficulty,
                mode=card.mode,
                outcome=card.outcome,
                cli=_fmt(card.versions.get("cli")),
                sdk=_fmt(card.versions.get("sdk")),
                wall=_fmt(card.wall_clock_secs),
                turns=card.parent_turns,
                loc=_fmt(card.lines_of_code),
            )
        )

    body = _TABLE_HEADER + ("\n".join(rows) + "\n" if rows else "_No sessions captured yet._\n")
    return _HEADER + "\n" + body


def write_index(sessions_root: Path) -> Path:
    """Write INDEX.md into ``sessions_root``; return its path."""
    sessions_root = Path(sessions_root)
    dest = sessions_root / INDEX
    dest.write_text(build_index(sessions_root), encoding="utf-8")
    return dest
