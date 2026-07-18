from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional


@dataclass
class SetState:
    data: dict


@dataclass
class PersistState:
    data: dict


@dataclass
class SetSchedulerMode:
    mode: str
    fps: int | None = None


@dataclass
class FileRead:
    path: str


@dataclass
class FileWrite:
    path: str
    content: bytes


@dataclass
class ReadHostLog:
    """Request the tail of the Plexi host channel log (`logs.read` capability).

    The host owns path resolution — it tails its own `~/.plexi-<channel>/plexi.log`
    and replies with an `events.HostLogResult`. The app never names or opens the
    file, so the request carries only how many trailing bytes to read.
    """
    max_bytes: int = 256 * 1024


@dataclass
class HttpFetch:
    url: str
    method: str = "GET"
    headers: dict = field(default_factory=dict)
    body: Optional[bytes] = None


@dataclass
class AiMessage:
    role: str
    content: str


@dataclass
class AiQuery:
    request_id: str
    model_tier: str
    system: str
    messages: list


@dataclass
class AiTool:
    name: str
    description: str
    input_schema: dict[str, Any]
    output_schema: dict[str, Any]
    timeout_ms: Optional[int] = None
    read_only: bool = False


@dataclass
class ExposeTools:
    tools: list[AiTool]


@dataclass
class ToolResult:
    call_id: str
    output_json: Optional[str] = None
    error: Optional[str] = None


@dataclass
class SetTimer:
    id: int
    delay_ms: int
    repeat: bool = False


@dataclass
class CancelTimer:
    id: int


@dataclass
class GetSystemStats:
    pass


@dataclass
class SetTitle:
    title: str


@dataclass
class SetStatus:
    text: str


@dataclass
class CloseSelf:
    pass


@dataclass
class RequestCapability:
    name: str


@dataclass
class EventStreamDecl:
    name: str
    schema_json: str
    description: Optional[str] = None


@dataclass
class DeclareEventStreams:
    streams: list


@dataclass
class EmitEvent:
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


@dataclass
class SubscribeEventStreams:
    request_id: str
    app_id: str
    event_names: list[str]
    payload_mode: str = "full"
    trigger_mode: str = "conversation"
    resource_id: Optional[str] = None


@dataclass
class UnsubscribeEventStreams:
    request_id: str
    subscription_id: str
