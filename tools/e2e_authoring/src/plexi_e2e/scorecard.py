"""Scorecard: a structured, comparable score derived from a captured session.

A scorecard is a *projection*, never a second source of truth. It reads only the
files the 0331 runner already wrote — ``manifest.json``, ``outcome.json`` (absent
on a dry run), and ``observations.jsonl`` — and distils them into one flat
``scorecard.json`` the benchmark index and cross-time comparisons consume. Rebuild
it any time from the raw session with no live host.

Fields (stint 0215):

  outcome / stalled_at        from outcome.json (``plan-only`` for a dry run)
  wall_clock_secs             from manifest
  parent_turns                parent interventions (manifest)
  child_turns                 observation rounds that produced visible child output
  commands_used / errors      from outcome.json (ground-truth observations, not self-report)
  lines_of_code               from a ``code_metrics`` observation the live runner records
  versions                    cli / sdk / channel stamp (manifest)
  timings                     init -> first-frame -> first-interactive, where automatable

The three timings are anchored at the earliest observation and derived from
observation timestamps:

  host_ready_secs           host reached ready (init)
  first_child_output_secs   child produced its first visible output (first-frame proxy)
  first_interactive_secs    first observed input -> state round-trip; ``None`` until the
                            built app emits interactive markers into the host log

Anything not derivable from the raw session is ``None`` — never guessed.
"""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any

from .capture import MANIFEST, OBSERVATIONS, OUTCOME

SCORECARD = "scorecard.json"
SCHEMA_VERSION = 1


class ScorecardError(ValueError):
    """A session directory is missing a file the scorecard requires."""


@dataclass
class Timings:
    host_ready_secs: float | None = None
    first_child_output_secs: float | None = None
    first_interactive_secs: float | None = None


@dataclass
class Scorecard:
    schema_version: int
    session_id: str
    fixture_id: str
    difficulty: str
    mode: str  # "dry-run" | "live"
    outcome: str  # worked | partial | failed | plan-only
    stalled_at: str | None
    wall_clock_secs: float | None
    parent_turns: int
    child_turns: int
    commands_used: list[str]
    errors: list[str]
    lines_of_code: int | None
    versions: dict[str, str | None]
    timings: Timings = field(default_factory=Timings)

    def to_dict(self) -> dict[str, Any]:
        d = asdict(self)
        return d


def _parse_ts(ts: str) -> datetime | None:
    try:
        return datetime.fromisoformat(ts)
    except (ValueError, TypeError):
        return None


def _read_json(path: Path, required: bool) -> dict[str, Any]:
    if not path.is_file():
        if required:
            raise ScorecardError(f"session is missing required {path.name}: {path}")
        return {}
    return json.loads(path.read_text(encoding="utf-8"))


def _read_observations(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def _child_output_rounds(observations: list[dict[str, Any]]) -> int:
    """Observation rounds where the child produced visible output."""
    return sum(
        1
        for o in observations
        if o.get("source") == "pane_capture" and o.get("data", {}).get("lines")
    )


def _latest_loc(observations: list[dict[str, Any]]) -> int | None:
    loc: int | None = None
    for o in observations:
        if o.get("kind") == "code_metrics":
            val = o.get("data", {}).get("loc")
            if isinstance(val, int):
                loc = val
    return loc


def _timings(observations: list[dict[str, Any]]) -> Timings:
    stamped = [(o, _parse_ts(o.get("ts", ""))) for o in observations]
    stamped = [(o, ts) for o, ts in stamped if ts is not None]
    if not stamped:
        return Timings()
    anchor = min(ts for _, ts in stamped)

    def _first(predicate) -> float | None:
        for o, ts in stamped:
            if predicate(o):
                return round((ts - anchor).total_seconds(), 3)
        return None

    host_ready = _first(lambda o: o.get("kind") == "host_status" and o.get("data", {}).get("ready"))
    first_output = _first(
        lambda o: o.get("source") == "pane_capture" and o.get("data", {}).get("lines")
    )
    # A first interactive round-trip requires the built app to emit an
    # input->state marker into the host log; not yet automatable, so left None
    # rather than conflated with first output.
    return Timings(
        host_ready_secs=host_ready,
        first_child_output_secs=first_output,
        first_interactive_secs=None,
    )


def build_scorecard(session_dir: Path) -> Scorecard:
    """Derive a scorecard from a captured session directory."""
    session_dir = Path(session_dir)
    if not session_dir.is_dir():
        raise ScorecardError(f"session directory not found: {session_dir}")

    manifest = _read_json(session_dir / MANIFEST, required=True)
    dry_run = bool(manifest.get("dry_run"))
    # A live session always writes outcome.json; its absence means a corrupt or
    # aborted capture, not a valid failure — surface it rather than mint a fake
    # "failed" row with empty commands/errors. A dry run legitimately has none.
    outcome = _read_json(session_dir / OUTCOME, required=not dry_run)
    observations = _read_observations(session_dir / OBSERVATIONS)

    fixture = manifest.get("fixture", {})
    versions = manifest.get("versions") or {
        "cli": None,
        "sdk": None,
        "channel": manifest.get("channel"),
    }

    if dry_run:
        status = "plan-only"
    else:
        status = outcome.get("status", "failed")

    return Scorecard(
        schema_version=SCHEMA_VERSION,
        session_id=manifest.get("session_id", session_dir.name),
        fixture_id=fixture.get("id", "unknown"),
        difficulty=fixture.get("difficulty", "unknown"),
        mode="dry-run" if dry_run else "live",
        outcome=status,
        stalled_at=outcome.get("stalled_at"),
        wall_clock_secs=manifest.get("wall_clock_secs"),
        parent_turns=int(manifest.get("parent_turns", 0)),
        child_turns=_child_output_rounds(observations),
        commands_used=list(outcome.get("commands_observed", [])),
        errors=list(outcome.get("errors", [])),
        lines_of_code=_latest_loc(observations),
        versions=versions,
        timings=_timings(observations),
    )


def write_scorecard(session_dir: Path) -> Path:
    """Build and write ``scorecard.json`` into ``session_dir``; return its path."""
    card = build_scorecard(session_dir)
    dest = Path(session_dir) / SCORECARD
    dest.write_text(json.dumps(card.to_dict(), indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return dest
