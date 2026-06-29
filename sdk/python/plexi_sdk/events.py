"""Input and host-result events delivered to an app's ``update(event)``.

The ProcessApp adapter converts PGAP input into these dataclasses before it
calls ``update``. Apps usually branch with ``isinstance(event, KeyEvent)`` or
``isinstance(event, UiAction)`` and return a list of effects in response.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional


@dataclass
class Modifiers:
    """Keyboard modifier state attached to ``KeyEvent``."""

    ctrl: bool = False
    shift: bool = False
    alt: bool = False
    meta: bool = False


@dataclass
class KeyEvent:
    """Keyboard input. ``key`` uses Plexi's normalized lowercase key names."""

    key: str
    modifiers: Modifiers = field(default_factory=Modifiers)
    pressed: bool = True


@dataclass
class MouseEvent:
    """Pointer input in pane coordinates, including buttons, scroll, and region."""

    x: float
    y: float
    button: Optional[str] = None
    pressed: bool = False
    scroll_x: float = 0.0
    scroll_y: float = 0.0
    region: Optional[str] = None


@dataclass
class UiAction:
    """Click/activation event from a component or host action."""

    handler_id: str
    args: list[str] = field(default_factory=list)


@dataclass
class UiValueChange:
    """Value-change event from an editable component with matching ``handler_id``."""

    handler_id: str
    value: str


@dataclass
class ListSelect:
    """Selection changed in a host-rendered list component."""

    id: str
    index: int


@dataclass
class ListActivate:
    """Item activated in a host-rendered list component."""

    id: str
    index: int


@dataclass
class Resize:
    """Pane size changed. Dimensions are logical pixels."""

    width: float
    height: float


@dataclass
class FocusGained:
    """This pane received focus."""

    pass


@dataclass
class FocusLost:
    """This pane lost focus."""

    pass


@dataclass
class FocusChanged:
    """Workspace focus telemetry delivered to apps that consume it."""

    timestamp: str
    duration_secs: int = 0
    reason: str = "focus_changed"
    pane_id: Optional[int] = None
    context_name: Optional[str] = None
    context_root: Optional[str] = None
    cwd: Optional[str] = None


@dataclass
class TimerFired:
    """A ``SetTimer`` timer fired; ``id`` echoes the scheduled timer id."""

    id: int


@dataclass
class RenderFrame:
    """Host-paced render tick for continuous or scheduled apps."""

    frame_id: int
    elapsed: float


@dataclass
class SystemStats:
    """Snapshot of host system metrics returned by ``GetSystemStats``."""

    cpu_usage_pct: float
    memory_used_bytes: int
    memory_total_bytes: int
    disk_read_bps: int
    disk_write_bps: int
    net_rx_bps: int
    net_tx_bps: int
    uptime_secs: int
    load_avg_one_min: float


@dataclass
class SystemStatsResult:
    """Result event for ``GetSystemStats``."""

    stats: SystemStats


@dataclass
class FileReadResult:
    """Result event for ``FileRead``. ``error`` is ``None`` on success."""

    content: Optional[bytes]
    error: Optional[str]


@dataclass
class FileListEntry:
    """One directory entry returned by ``FileListResult``."""

    name: str
    path: str
    is_dir: bool = False
    size_bytes: Optional[int] = None


@dataclass
class FileListResult:
    """Result event for ``FileList``. ``entries`` is ``None`` on error."""

    entries: Optional[list]
    error: Optional[str]


@dataclass
class FileWriteResult:
    """Result event for ``FileWrite``. ``error`` is ``None`` on success."""

    error: Optional[str]


@dataclass
class HttpResponse:
    """Result event for ``HttpFetch``."""

    status: int
    headers: list
    body: bytes


@dataclass
class AiStreamChunk:
    """Streaming partial response for an ``AiQuery`` request."""

    request_id: str
    delta: str
    reasoning: Optional[str]
    done: bool


@dataclass
class AiResponse:
    """Final response for an ``AiQuery`` request."""

    request_id: str
    content: Optional[str]
    tokens_in: int
    tokens_out: int
    error: Optional[str]


@dataclass
class DeclareEventStreamsResult:
    """Result event for ``DeclareEventStreams``."""

    streams: Optional[list]
    error: Optional[str]


@dataclass
class EmitEventResult:
    """Result event for ``EmitEvent``."""

    sequence: Optional[int]
    error: Optional[str]


@dataclass
class SurfaceReady:
    """GPU surface became available for advanced surface apps."""

    texture_handle: int
    width: int
    height: int


@dataclass
class SurfaceResized:
    """GPU surface size changed for advanced surface apps."""

    texture_handle: int
    width: int
    height: int


@dataclass
class PipePayload:
    """Payload carried by a typed pipe message."""

    binary: Optional[bytes] = None
    json: Optional[str] = None


@dataclass
class PipeMessage:
    """Typed pipe message delivered from another pane/app."""

    handle: int
    payload: PipePayload


@dataclass
class PipePeerConnected:
    """A peer connected to an opened typed pipe."""

    handle: int


@dataclass
class PipeClosed:
    """A typed pipe closed."""

    handle: int


@dataclass
class PipeError:
    """A typed pipe operation failed."""

    handle: int
    error: str


@dataclass
class CapabilityGranted:
    """A requested capability was granted."""

    name: str


@dataclass
class CapabilityDenied:
    """A requested capability was denied."""

    name: str


@dataclass
class PaymentComplete:
    """Payment flow completed for the app."""

    pass


@dataclass
class PaymentFailed:
    """Payment flow failed with a human-readable reason."""

    reason: str
