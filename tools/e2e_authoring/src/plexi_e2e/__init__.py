"""Agent-drives-agent E2E runner for Plexi app-authoring sessions.

A parent process plays the user: it boots an isolated Plexi host, spawns a child
coding agent in a terminal pane, delivers a user-realistic prompt, observes
ground truth through the host (pane state, capture, logs, events), and records
the whole session in the capture format under ``benchmarks/app-authoring/``.

Stint 0331. The capture format here is the interchange the app-authoring
benchmark (stint 0215) accumulates.
"""

from .config import Fixture, SessionConfig
from .capture import SessionCapture
from .plexi_cli import PlexiCli, PlexiCliError
from .protocol import SessionProtocol
from .runner import E2ESession

__all__ = [
    "Fixture",
    "SessionConfig",
    "SessionCapture",
    "PlexiCli",
    "PlexiCliError",
    "SessionProtocol",
    "E2ESession",
]
