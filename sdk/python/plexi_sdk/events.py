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
class TimerFired:
    id: int


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
