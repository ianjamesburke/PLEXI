from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional


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
