"""Session configuration and prompt fixtures.

Required fields have no default and throw when missing. A fixture is a repeatable,
committed description of one user-realistic app-building request: the initial
prompt, the user-level answers the parent may give when the child asks
questions, and the child agent launch. Prompts never name commands, files, or SDK
symbols — that is the whole point of the parent/child split.
"""

from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path


class FixtureError(ValueError):
    """A fixture file is missing a required field or is malformed."""


@dataclass(frozen=True)
class SeedPane:
    """A terminal pane seeded into the host on boot."""

    cwd: str
    cmd: str | None = None


@dataclass(frozen=True)
class Fixture:
    """A user-realistic app-building request, loaded from a TOML fixture file."""

    id: str
    difficulty: str
    description: str
    prompt: str
    # intent -> user-level answer the parent may give when the child asks.
    answers: dict[str, str]
    # Command that launches the child coding agent in its workspace pane.
    child_launch: str
    child_cwd: str
    seed_panes: tuple[SeedPane, ...]
    source_path: Path | None = None

    @staticmethod
    def load(path: Path) -> "Fixture":
        if not path.is_file():
            raise FixtureError(f"fixture file not found: {path}")
        with path.open("rb") as fh:
            raw = tomllib.load(fh)

        fx = _require_table(raw, "fixture", path)
        child = _require_table(raw, "child", path)

        answers_raw = fx.get("answers", {})
        if not isinstance(answers_raw, dict):
            raise FixtureError(f"{path}: [fixture] answers must be a table")

        seed_panes = tuple(
            SeedPane(
                cwd=_require_str(_as_table(p, path, "seed_pane"), "cwd", path, ctx="seed_pane"),
                cmd=_opt_str(_as_table(p, path, "seed_pane").get("cmd"), path, "seed_pane.cmd"),
            )
            for p in raw.get("seed_pane", [])
        )

        return Fixture(
            id=_require_str(fx, "id", path, ctx="fixture"),
            difficulty=_require_str(fx, "difficulty", path, ctx="fixture"),
            description=_require_str(fx, "description", path, ctx="fixture"),
            prompt=_require_str(fx, "prompt", path, ctx="fixture").strip(),
            answers={str(k): str(v) for k, v in answers_raw.items()},
            child_launch=_require_str(child, "launch", path, ctx="child"),
            child_cwd=_require_str(child, "cwd", path, ctx="child"),
            seed_panes=seed_panes,
            source_path=path,
        )


@dataclass
class SessionConfig:
    """Everything one runner invocation needs. Required fields throw when absent."""

    channel: str
    fixture: Fixture
    sessions_root: Path
    binary: str
    dry_run: bool = False
    fresh_profile: bool = False
    boot_timeout_secs: int = 30
    observe_rounds: int = 6
    observe_interval_secs: float = 5.0
    home: Path | None = None

    def __post_init__(self) -> None:
        if not self.channel:
            raise ValueError("SessionConfig.channel is required")
        if not self.binary:
            raise ValueError("SessionConfig.binary is required")
        if self.fixture is None:
            raise ValueError("SessionConfig.fixture is required")
        self.sessions_root = Path(self.sessions_root)


def default_binary_for(channel: str) -> str:
    """The installed CLI binary name for ``channel`` (``plexi-<channel>``)."""
    if not channel:
        raise ValueError("channel is required")
    return f"plexi-{channel}"


def _require_str(table: dict, key: str, path: Path, ctx: str = "") -> str:
    where = f"[{ctx}] " if ctx else ""
    if key not in table:
        raise FixtureError(f"{path}: missing required {where}field '{key}'")
    val = table[key]
    if not isinstance(val, str):
        raise FixtureError(f"{path}: {where}field '{key}' must be a string, got {type(val).__name__}")
    if not val.strip():
        raise FixtureError(f"{path}: {where}field '{key}' must not be empty")
    return val


def _require_table(raw: dict, key: str, path: Path) -> dict:
    if key not in raw:
        raise FixtureError(f"{path}: missing required section '[{key}]'")
    val = raw[key]
    if not isinstance(val, dict):
        raise FixtureError(f"{path}: '[{key}]' must be a table")
    return val


def _as_table(val: object, path: Path, ctx: str) -> dict:
    if not isinstance(val, dict):
        raise FixtureError(f"{path}: each [[{ctx}]] entry must be a table")
    return val


def _opt_str(val: object, path: Path, field: str) -> str | None:
    if val is None:
        return None
    if not isinstance(val, str):
        raise FixtureError(f"{path}: {field} must be a string, got {type(val).__name__}")
    return val
