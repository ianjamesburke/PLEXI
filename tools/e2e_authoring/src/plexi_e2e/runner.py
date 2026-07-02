"""Session orchestration: provision -> boot -> drive -> observe -> capture -> teardown.

One :meth:`E2ESession.run` call provisions an isolated session, executes a
fixture, and leaves a complete session directory behind. ``dry_run`` exercises
all provisioning, capture, and env/argv plumbing without booting a real host or
spawning a child — it records the exact plan a live run would execute, which is
what makes the pipeline testable in a headless environment.

Isolation model: the unit of host isolation is a *channel* (``host start`` binds
to the installed binary's own channel and cannot be pointed elsewhere — see
src/cli/host.rs). Each session gets a unique session directory, and
``--fresh-profile`` archives the channel's existing profile dir aside so state
never bleeds between runs. Two concurrent, non-interfering sessions therefore
require two channels (host-per-channel rule); sequential sessions on one channel
are isolated by fresh-profile + unique session dirs.
"""

from __future__ import annotations

import json
import logging
import os
import shutil
import time
import uuid
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path

from . import env as envmod
from .capture import SessionCapture
from .config import SessionConfig
from .plexi_cli import PlexiCli
from .protocol import SessionProtocol
from .scorecard import write_scorecard
from .versions import resolve_versions

PLAN = "plan.json"

# Source extensions counted toward an app's lines-of-code metric.
_CODE_EXTS = (".py", ".rs", ".toml", ".js", ".ts", ".html", ".css")


@dataclass
class PlannedStep:
    stage: str
    description: str
    argv: list[str] = field(default_factory=list)


@dataclass
class SessionResult:
    session_id: str
    session_dir: Path
    dry_run: bool
    ready: bool = False
    outcome: str | None = None


class E2ESession:
    def __init__(self, config: SessionConfig, logger: logging.Logger | None = None) -> None:
        self.cfg = config
        self.log = logger or logging.getLogger("plexi_e2e.runner")
        self.protocol = SessionProtocol(config.fixture)
        self.cli = PlexiCli(
            binary=config.binary,
            channel=config.channel,
            home=config.home,
            logger=self.log.getChild("cli"),
        )
        self._plan: list[PlannedStep] = []
        self._archived_profile: Path | None = None

    # -- public ----------------------------------------------------------

    def run(self) -> SessionResult:
        capture, started = self._provision()
        try:
            if self.cfg.dry_run:
                return self._run_dry(capture)
            return self._run_live(capture)
        finally:
            self._finalize_manifest(capture, started)

    # -- provisioning ----------------------------------------------------

    def _session_id(self) -> str:
        ts = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        return f"{ts}_{self.cfg.fixture.id}_{uuid.uuid4().hex[:6]}"

    def _provision(self) -> tuple[SessionCapture, float]:
        session_id = self._session_id()
        session_dir = self.cfg.sessions_root / session_id
        capture = SessionCapture(session_dir, logger=self.log.getChild("capture"))
        if self.cfg.fixture.source_path is not None:
            capture.copy_fixture(self.cfg.fixture.source_path)
        self.log.info(
            "provisioned session %s (channel=%s dry_run=%s)",
            session_id, self.cfg.channel, self.cfg.dry_run,
        )
        return capture, time.monotonic()

    def _plan_step(self, capture: SessionCapture, step: PlannedStep) -> None:
        self._plan.append(step)
        capture.append_observation("plan", "runner", asdict(step))

    def _seed_specs(self) -> list[tuple[str, str | None]]:
        """Seed panes with `~` expanded — `host start --pane cwd=...` writes the
        string verbatim into the spawn queue and the host resolves it as a
        PathBuf with no shell expansion, so a literal `~` would be a bad path."""
        fx = self.cfg.fixture
        specs = [(p.cwd, p.cmd) for p in fx.seed_panes] or [(fx.child_cwd, None)]
        return [(os.path.expanduser(cwd), cmd) for cwd, cmd in specs]

    # -- dry run ---------------------------------------------------------

    def _run_dry(self, capture: SessionCapture) -> SessionResult:
        """Record the full plan without touching a host or child."""
        fx = self.cfg.fixture
        seed = self._seed_specs()

        self._plan_step(capture, PlannedStep(
            "preflight", "verify binary installed and no host already running",
            [self.cfg.binary, "host", "status", "--json"],
        ))
        self._plan_step(capture, PlannedStep(
            "boot", "start isolated host with seeded workspace pane(s)",
            [self.cfg.binary, "host", "start", "--timeout-secs", str(self.cfg.boot_timeout_secs)]
            + [arg for cwd, cmd in seed for arg in ("--pane", f"cwd={cwd}" + (f",cmd={cmd}" if cmd else ""))],
        ))
        self._plan_step(capture, PlannedStep(
            "boot", "poll readiness", [self.cfg.binary, "host", "status", "--json"],
        ))
        self._plan_step(capture, PlannedStep(
            "drive", "list panes to find the workspace pane", [self.cfg.binary, "pane", "list"],
        ))
        self._plan_step(capture, PlannedStep(
            "drive", f"launch child agent ({fx.child_launch}) in workspace pane",
            [self.cfg.binary, "pane", "send", "<workspace_pane_id>", fx.child_launch + "\\n"],
        ))

        prompt = self.protocol.initial_prompt()
        capture.append_transcript(f"## parent (turn {prompt.turn}, prompt)\n\n{prompt.text}\n")
        capture.append_observation("intervention", "protocol", asdict(prompt))
        self._plan_step(capture, PlannedStep(
            "drive", "deliver the initial user prompt to the child",
            [self.cfg.binary, "pane", "send", "<child_pane_id>", "<prompt>\\n"],
        ))

        for i in range(self.cfg.observe_rounds):
            self._plan_step(capture, PlannedStep(
                "observe", f"round {i + 1}: capture child transcript",
                [self.cfg.binary, "pane", "capture", "<child_pane_id>", "--lines", "80"],
            ))
            self._plan_step(capture, PlannedStep(
                "observe", f"round {i + 1}: tail host log for ground truth",
                ["tail", "-100", str(envmod.log_path(self.cfg.channel, self.cfg.home))],
            ))
        self._plan_step(capture, PlannedStep(
            "observe", "if an app pane opened, snapshot its L1 render tree",
            [self.cfg.binary, "pane", "state", "<app_pane_id>"],
        ))
        self._plan_step(capture, PlannedStep(
            "teardown", "stop host and confirm process gone",
            [self.cfg.binary, "host", "stop"],
        ))

        (capture.dir / PLAN).write_text(
            json.dumps([asdict(s) for s in self._plan], indent=2) + "\n", encoding="utf-8"
        )
        capture.append_friction(
            "DRY RUN — no live child executed. Live pilot needs a display (host GUI) "
            "and child-agent credentials. This plan.json lists the exact steps a live run runs."
        )
        self.log.info("dry-run plan written: %d steps", len(self._plan))
        return SessionResult(capture.session_id, capture.dir, dry_run=True)

    # -- live run --------------------------------------------------------

    def _run_live(self, capture: SessionCapture) -> SessionResult:
        if shutil.which(self.cfg.binary) is None:
            raise RuntimeError(
                f"binary {self.cfg.binary!r} not found on PATH — install the "
                f"'{self.cfg.channel}' channel first (e.g. `just channel-install {self.cfg.channel}`)"
            )
        self._preflight()
        if self.cfg.fresh_profile:
            self._archive_profile()

        self.cli.host_start(self._seed_specs(), self.cfg.boot_timeout_secs)
        ready = self._await_ready(capture)
        result = SessionResult(capture.session_id, capture.dir, dry_run=False, ready=ready)
        if not ready:
            capture.write_outcome("failed", "boot", 0, [], ["host did not reach ready state"])
            result.outcome = "failed"
            self._teardown()
            return result

        try:
            self._drive_and_observe(capture)
            # Outcome classification from ground truth is left to the reviewer /
            # 0215 scorecard; the parent records status "partial" pending human
            # confirmation rather than guessing worked/failed from a transcript.
            capture.write_outcome(
                "partial", None, self.protocol.turn,
                self._observed_commands(capture),
                self._observed_errors(capture),
                notes="Live session captured. Confirm final outcome from outcome/host.log.",
            )
            result.outcome = "partial"
        except Exception as exc:
            # A drive/observe failure must still leave a complete, honest session
            # dir: record it as a failed outcome rather than an incomplete dir.
            self.log.error("drive/observe failed: %s", exc)
            capture.write_outcome(
                "failed", "drive", self.protocol.turn,
                self._observed_commands(capture),
                self._observed_errors(capture) + [f"{type(exc).__name__}: {exc}"],
                notes="Session aborted by an exception during drive/observe.",
            )
            result.outcome = "failed"
            raise
        finally:
            self._capture_code_metrics(capture)
            self._capture_log_slice(capture)
            self._teardown()
        return result

    def _preflight(self) -> None:
        status = self.cli.host_status()
        if status.get("ready"):
            raise RuntimeError(
                f"a host for channel '{self.cfg.channel}' is already running "
                f"(pid {status.get('pid')}); stop it before starting a session"
            )

    def _await_ready(self, capture: SessionCapture) -> bool:
        deadline = time.monotonic() + self.cfg.boot_timeout_secs
        while time.monotonic() < deadline:
            status = self.cli.host_status()
            capture.append_observation("host_status", "cli", status)
            if status.get("ready"):
                return True
            time.sleep(1.0)
        return False

    def _drive_and_observe(self, capture: SessionCapture) -> None:
        fx = self.cfg.fixture
        panes = self.cli.pane_list()
        capture.append_observation("pane_list", "cli", panes)
        workspace = self._pick_workspace_pane(panes)

        self.cli.pane_send(workspace, fx.child_launch + "\n")
        capture.append_observation("child_launch", "cli", {"pane": workspace, "cmd": fx.child_launch})
        time.sleep(self.cfg.observe_interval_secs)

        prompt = self.protocol.initial_prompt()
        self.cli.pane_send(workspace, prompt.text.replace("\n", " ") + "\n")
        capture.append_transcript(f"## parent (turn {prompt.turn}, prompt)\n\n{prompt.text}\n")
        capture.append_observation("intervention", "protocol", {"turn": prompt.turn, "text": prompt.text})

        cursor: int | None = None
        for i in range(self.cfg.observe_rounds):
            time.sleep(self.cfg.observe_interval_secs)
            cap = self.cli.pane_capture(workspace, lines=120, from_cursor=cursor)
            lines, cursor = self._capture_lines(cap, cursor)
            if lines:
                capture.append_transcript(f"## child (observe round {i + 1})\n\n" + "\n".join(lines))
            capture.append_observation("pane_capture", "pane_capture", {"round": i + 1, "lines": lines})

    def _pick_workspace_pane(self, panes: object) -> int:
        if not isinstance(panes, list) or not panes:
            raise RuntimeError("pane list is empty — host booted with no seeded pane")
        first = panes[0]
        pane_id = first.get("id") if isinstance(first, dict) else None
        if pane_id is None:
            raise RuntimeError(f"could not read pane id from pane list entry: {first!r}")
        return int(pane_id)

    @staticmethod
    def _capture_lines(cap: object, cursor: int | None) -> tuple[list[str], int | None]:
        if isinstance(cap, dict):
            return list(cap.get("lines", [])), cap.get("cursor", cursor)
        if isinstance(cap, list):
            return [str(x) for x in cap], cursor
        return [], cursor

    def _observed_commands(self, capture: SessionCapture) -> list[str]:
        cmds = []
        for obs in capture.read_observations():
            if obs.get("kind") == "child_launch":
                cmds.append(obs["data"].get("cmd", ""))
        return cmds

    def _observed_errors(self, capture: SessionCapture) -> list[str]:
        errors = []
        for obs in capture.read_observations():
            if obs.get("source") == "pane_capture":
                for line in obs.get("data", {}).get("lines", []):
                    if "error" in str(line).lower() or "traceback" in str(line).lower():
                        errors.append(str(line))
        return errors

    def _capture_code_metrics(self, capture: SessionCapture) -> None:
        """Count the lines of code the child produced in its workspace.

        Ground-truth size signal for the scorecard. Best-effort: if the workspace
        dir does not exist (child never scaffolded anything) it records nothing.
        """
        root = Path(os.path.expanduser(self.cfg.fixture.child_cwd))
        if not root.is_dir():
            return
        loc = 0
        files = 0
        for path in root.rglob("*"):
            if not path.is_file() or path.suffix not in _CODE_EXTS:
                continue
            if any(part in {".venv", "__pycache__", ".git", "node_modules"} for part in path.parts):
                continue
            try:
                loc += sum(1 for _ in path.open("r", encoding="utf-8", errors="ignore"))
                files += 1
            except OSError as exc:
                self.log.info("could not read %s for LOC: %s", path, exc)
        capture.append_observation(
            "code_metrics", "cli", {"loc": loc, "files": files, "root": str(root)}
        )

    def _capture_log_slice(self, capture: SessionCapture) -> None:
        log_file = envmod.log_path(self.cfg.channel, self.cfg.home)
        if log_file.is_file():
            text = log_file.read_text(encoding="utf-8", errors="replace")
            capture.record_log_slice("\n".join(text.splitlines()[-500:]))

    # -- profile isolation / teardown ------------------------------------

    def _archive_profile(self) -> None:
        profile = envmod.profile_dir(self.cfg.channel, self.cfg.home)
        if profile.is_dir():
            stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
            dest = profile.with_name(profile.name + f".bak-{stamp}")
            shutil.move(str(profile), str(dest))
            self._archived_profile = dest
            self.log.info("archived existing profile %s -> %s", profile, dest)

    def _teardown(self) -> None:
        self.cli.host_stop()
        status = self.cli.host_status()
        if status.get("ready"):
            self.log.error("host still running after stop for channel %s", self.cfg.channel)

    # -- manifest --------------------------------------------------------

    def _finalize_manifest(self, capture: SessionCapture, started: float) -> None:
        fx = self.cfg.fixture
        manifest = {
            "schema_version": 1,
            "session_id": capture.session_id,
            "channel": self.cfg.channel,
            "binary": self.cfg.binary,
            "dry_run": self.cfg.dry_run,
            "fresh_profile": self.cfg.fresh_profile,
            "fixture": {
                "id": fx.id,
                "difficulty": fx.difficulty,
                "description": fx.description,
            },
            "versions": resolve_versions(
                self.cfg.binary, self.cfg.channel, probe_cli=not self.cfg.dry_run
            ),
            "profile_dir": str(envmod.profile_dir(self.cfg.channel, self.cfg.home)),
            "archived_profile": str(self._archived_profile) if self._archived_profile else None,
            "wall_clock_secs": round(time.monotonic() - started, 3),
            "parent_turns": self.protocol.turn,
            "created_at": datetime.now(timezone.utc).isoformat(),
        }
        capture.write_manifest(manifest)
        # Scorecard is a projection over the just-written manifest + observations;
        # every session ends with one so the index and comparisons never re-run a host.
        write_scorecard(capture.dir)
