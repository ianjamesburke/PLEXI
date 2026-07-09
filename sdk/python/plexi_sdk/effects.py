"""Effect dataclasses returned from a Plexi app's ``init()`` or ``update()``.

Effects describe work for the host to perform after the lifecycle function
returns. They are data objects, not imperative calls: return
``[SetTitle("Demo"), SetState({"count": 1})]`` instead of mutating host state
directly. The adapter matches each class name to the PGAP host effect with the
same meaning.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Literal, Optional

PipStatus = Literal["green", "yellow", "red"]
"""Traffic-light health an app reports for its own activity dot."""


@dataclass
class SetState:
    """Update process-local runtime state before the next ``view()`` call.

    ``data`` should be a JSON-shaped dict. Values are merged into the runtime
    snapshot and are not written to durable app storage.
    """

    data: dict


@dataclass
class PersistState:
    """Update runtime state and request a durable app-state save."""

    data: dict


@dataclass
class SetSchedulerMode:
    """Control host-paced render scheduling.

    ``mode`` is ``"idle"``, ``"scheduled"``, or ``"continuous"``. Continuous
    mode sends ``RenderFrame`` events at ``fps`` for games and animations.
    """

    mode: str
    fps: int | None = None


@dataclass
class SetMouseTracking:
    """Enable or disable mouse-motion events for the pane."""

    enabled: bool


@dataclass
class FileRead:
    """Read a workspace-scoped file and receive a ``FileReadResult`` event."""

    path: str


@dataclass
class FileList:
    """List a workspace-scoped directory and receive a ``FileListResult`` event."""

    path: str
    extensions: list = field(default_factory=list)


@dataclass
class FileWrite:
    """Write bytes to a workspace-scoped file through the host."""

    path: str
    content: bytes


@dataclass
class HttpFetch:
    """Request an HTTP(S) fetch through the host capability gate."""

    url: str
    method: str = "GET"
    headers: dict = field(default_factory=dict)
    body: Optional[bytes] = None


@dataclass
class OpenUrl:
    """Ask the host to open an HTTP(S) URL in the default browser."""

    url: str


@dataclass
class AiMessage:
    """One chat message in an ``AiQuery`` request."""

    role: str
    content: str


@dataclass
class AiQuery:
    """Send a managed AI request and receive chunks/results by ``request_id``."""

    request_id: str
    model_tier: str
    system: str
    messages: list


@dataclass
class SetTimer:
    """Schedule a timer; the host replies with ``TimerFired(id)``."""

    id: int
    delay_ms: int
    repeat: bool = False


@dataclass
class CancelTimer:
    """Cancel a timer previously scheduled with ``SetTimer``."""

    id: int


@dataclass
class GetSystemStats:
    """Request host system metrics and receive ``SystemStatsResult``."""

    pass


@dataclass
class SetTitle:
    """Set the pane title shown by Plexi chrome."""

    title: str


@dataclass
class SetStatus:
    """Set the pane status text shown by Plexi chrome."""

    text: str


@dataclass
class SetPipStatus:
    """Report this app's own pip status — a red/yellow/green activity dot.

    Takes priority over the host's derived activity until the app reports a
    new status. The host stamps the pane id, so an app can only ever set its
    own pip. ``status`` must be ``"green"``, ``"yellow"``, or ``"red"``.
    """

    status: PipStatus


@dataclass
class CloseSelf:
    """Ask the host to close this app pane."""

    pass


@dataclass
class RequestCapability:
    """Prompt for a named capability grant at runtime."""

    name: str


@dataclass
class EventStreamDecl:
    """Declare one structured event stream emitted by the app."""

    name: str
    schema_json: str
    description: Optional[str] = None


@dataclass
class DeclareEventStreams:
    """Register app event streams before emitting events into them."""

    streams: list


@dataclass
class EmitEvent:
    """Emit one structured app event for host/audit consumers."""

    event: str
    actor: str
    summary: str
    resource_id: str
    revision_after: str
    actor_id: Optional[str] = None
    caused_by: Optional[str] = None
    resource_scope: Optional[str] = None
    payload_json: Optional[str] = None
    state_ref: Optional[str] = None
    revision_before: Optional[str] = None
    rollback_token: Optional[str] = None
    changed_resources: list = field(default_factory=list)
    suggested_trigger: Optional[str] = None
