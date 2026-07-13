from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional


@dataclass
class Modifiers:
    ctrl: bool = False
    shift: bool = False
    alt: bool = False
    meta: bool = False


@dataclass
class KeyEvent:
    key: str
    modifiers: Modifiers = field(default_factory=Modifiers)
    pressed: bool = True


@dataclass
class MouseEvent:
    x: float
    y: float
    button: Optional[str] = None
    pressed: bool = False
    scroll_x: float = 0.0
    scroll_y: float = 0.0


@dataclass
class UiAction:
    handler_id: str


@dataclass
class UiValueChange:
    handler_id: str
    value: str


@dataclass
class Resize:
    width: float
    height: float


@dataclass
class FocusGained:
    pass


@dataclass
class FocusLost:
    pass


@dataclass
class FocusChanged:
    timestamp: str
    duration_secs: int = 0
    reason: str = "focus_changed"
    pane_id: Optional[int] = None
    context_name: Optional[str] = None
    context_root: Optional[str] = None
    cwd: Optional[str] = None


@dataclass
class TimerFired:
    id: int


@dataclass
class RenderFrame:
    frame_id: int
    elapsed: float


@dataclass
class SystemStats:
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
    stats: SystemStats


@dataclass
class FileReadResult:
    content: Optional[bytes]
    error: Optional[str]


@dataclass
class FileWriteResult:
    error: Optional[str]


@dataclass
class HttpResponse:
    status: int
    headers: list
    body: bytes


@dataclass
class AiStreamChunk:
    request_id: str
    delta: str
    reasoning: Optional[str]
    done: bool


@dataclass
class AiResponse:
    request_id: str
    content: Optional[str]
    tokens_in: int
    tokens_out: int
    error: Optional[str]


@dataclass
class ToolCall:
    call_id: str
    name: str
    input_json: str
    caller_id: str


@dataclass
class EventSubscriptionResult:
    request_id: str
    subscription_id: Optional[str]
    error: Optional[str]


@dataclass
class EventUnsubscriptionResult:
    request_id: str
    removed: bool
    error: Optional[str]


@dataclass
class AppEvent:
    subscription_id: str
    app_id: str
    event: str
    event_id: int
    resource_id: str
    trigger_mode: str
    summary: Optional[str]
    payload_json: Optional[str]
    state_ref: Optional[str]
    created_at: str


@dataclass
class DeclareEventStreamsResult:
    streams: Optional[list]
    error: Optional[str]


@dataclass
class EmitEventResult:
    sequence: Optional[int]
    error: Optional[str]


@dataclass
class SurfaceReady:
    texture_handle: int
    width: int
    height: int


@dataclass
class SurfaceResized:
    texture_handle: int
    width: int
    height: int


@dataclass
class PipePayload:
    binary: Optional[bytes] = None
    json: Optional[str] = None


@dataclass
class PipeMessage:
    handle: int
    payload: PipePayload


@dataclass
class PipePeerConnected:
    handle: int


@dataclass
class PipeClosed:
    handle: int


@dataclass
class PipeError:
    handle: int
    error: str


@dataclass
class CapabilityGranted:
    name: str


@dataclass
class CapabilityDenied:
    name: str


@dataclass
class PaymentComplete:
    pass


@dataclass
class PaymentFailed:
    reason: str
