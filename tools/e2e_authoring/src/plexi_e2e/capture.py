"""Session capture: one directory per session, the benchmark interchange format.

Layout (stint 0331 capture format; stint 0215 accumulates these):

    <session-id>/
      manifest.json      session id, channel, binary, versions, fixture ref, timing
      prompt.toml        verbatim copy of the fixture the child was given
      transcript.md      the child's terminal transcript (pane captures over time)
      observations.jsonl parent ground-truth events (pane state/capture, log, events, interventions)
      friction.md        rigorous friction notes (what the scaffold should have told the child)
      outcome.json       structured outcome: worked | partial | failed, where it stalled
      host.log           the slice of plexi.log covering the session window

Every observation is appended as one JSON object per line so a run can stream and
a reviewer (or the 0215 scorecard) can parse without loading the whole file.
"""

from __future__ import annotations

import json
import logging
import shutil
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

MANIFEST = "manifest.json"
PROMPT = "prompt.toml"
TRANSCRIPT = "transcript.md"
OBSERVATIONS = "observations.jsonl"
FRICTION = "friction.md"
OUTCOME = "outcome.json"
HOST_LOG = "host.log"

OUTCOME_STATES = ("worked", "partial", "failed")


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


class SessionCapture:
    """Writes and owns a single session directory."""

    def __init__(self, session_dir: Path, logger: logging.Logger | None = None) -> None:
        self.dir = Path(session_dir)
        self.log = logger or logging.getLogger("plexi_e2e.capture")
        self.dir.mkdir(parents=True, exist_ok=False)
        self.log.info("session dir created: %s", self.dir)
        (self.dir / TRANSCRIPT).write_text("# Child transcript\n\n", encoding="utf-8")
        (self.dir / OBSERVATIONS).touch()
        (self.dir / FRICTION).write_text(
            "# Friction notes\n\n"
            "_Where the child struggled, guessed, or was misled by the scaffold._\n\n",
            encoding="utf-8",
        )

    @property
    def session_id(self) -> str:
        return self.dir.name

    def copy_fixture(self, fixture_path: Path) -> None:
        shutil.copyfile(fixture_path, self.dir / PROMPT)

    def write_manifest(self, manifest: dict[str, Any]) -> None:
        (self.dir / MANIFEST).write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )

    def append_observation(self, kind: str, source: str, data: Any) -> None:
        """Append one parent observation. ``source`` is the ground-truth channel
        (``pane_state`` / ``pane_capture`` / ``host_log`` / ``events`` / ``cli``),
        ``kind`` the semantic event."""
        record = {"ts": _utc_now(), "kind": kind, "source": source, "data": data}
        with (self.dir / OBSERVATIONS).open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(record) + "\n")

    def append_transcript(self, text: str) -> None:
        with (self.dir / TRANSCRIPT).open("a", encoding="utf-8") as fh:
            fh.write(text.rstrip("\n") + "\n")

    def append_friction(self, note: str) -> None:
        with (self.dir / FRICTION).open("a", encoding="utf-8") as fh:
            fh.write(f"- {note}\n")

    def record_log_slice(self, text: str) -> None:
        (self.dir / HOST_LOG).write_text(text, encoding="utf-8")

    def write_outcome(
        self,
        status: str,
        stalled_at: str | None,
        turns: int,
        commands_observed: list[str],
        errors: list[str],
        notes: str = "",
    ) -> None:
        if status not in OUTCOME_STATES:
            raise ValueError(f"outcome status must be one of {OUTCOME_STATES}, got {status!r}")
        outcome = {
            "status": status,
            "stalled_at": stalled_at,
            "turns": turns,
            "commands_observed": commands_observed,
            "errors": errors,
            "notes": notes,
            "recorded_at": _utc_now(),
        }
        (self.dir / OUTCOME).write_text(
            json.dumps(outcome, indent=2) + "\n", encoding="utf-8"
        )

    def read_observations(self) -> list[dict]:
        text = (self.dir / OBSERVATIONS).read_text(encoding="utf-8")
        return [json.loads(line) for line in text.splitlines() if line.strip()]
