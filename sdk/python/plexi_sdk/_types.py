from __future__ import annotations

from dataclasses import dataclass


class CapabilityDeniedError(RuntimeError):
    """Raised when the host rejects a brokered call because the app's manifest
    didn't declare the required capability. Distinct from generic RuntimeError
    so apps can catch the gate-denial path explicitly."""


# ── Video substrate (#345) types ──────────────────────────────────────────────

@dataclass
class VideoHandle:
    """Result of `Emitter.open_video`. `handle_id` is opaque — pass it back to
    `set_video_state` and `close_video`. The associated `Pipe` delivers
    decoded RGBA8 frames (one frame per `pipe.read_frame()`) of length
    `width * height * 4`."""
    handle_id: int
    width: int
    height: int
    fps: float
    duration_ms: int
    pipe: "object"  # Pipe — avoid circular import; typed as object here


# ── agents.list (#286) types ──────────────────────────────────────────────────

@dataclass
class AgentInfo:
    """One row of the agent roster returned by `Emitter.agent_roster`.

    `pane_id` is the host's stable id for the agent pane; pass it to
    `Emitter.pipe_open_directed(pipe_id, pane_id)` to wire an inter-agent
    channel. `app_id` is the manifest id for subprocess agents (or the
    sentinel `"iq"` for the legacy in-process Cmd+I agent backend).
    """
    pane_id: int
    app_id: str
    name: str
