"""V3AppRuntime: standalone runtime for module-level Plexi apps.

Replaces the V3ProcessApp(App) inheritance adapter with a composition-based
runtime that owns its own protocol transport, frame clock, and state. No
inheritance means no attribute collision bugs (the _last_render_time category).

Protocol: JSON lines on stdin/stdout. Synchronous event loop - the host drives
timing via render events; no asyncio needed since effects are fire-and-forget
and async responses (http_response, capability_decision) arrive as events.
"""
from __future__ import annotations

import importlib.util
import json
import sys
import time
import threading
import uuid
from pathlib import Path

import plexi_sdk as sdk
from plexi_sdk import _v3_state, effects, events
from plexi_sdk._keys import normalize_key as _normalize_key
from plexi_sdk._v3_state import StateSnapshot


_LOCK = threading.Lock()
_EMIT_BATCH: list[dict] | None = None


def _emit(obj: dict) -> None:
    with _LOCK:
        if _EMIT_BATCH is not None:
            _EMIT_BATCH.append(obj)
            return
        sys.stdout.write(json.dumps(obj) + "\n")
        sys.stdout.flush()


def _begin_emit_batch() -> None:
    global _EMIT_BATCH
    with _LOCK:
        if _EMIT_BATCH is not None:
            raise RuntimeError("nested protocol output batch")
        _EMIT_BATCH = []


def _finish_emit_batch() -> None:
    global _EMIT_BATCH
    with _LOCK:
        batch = _EMIT_BATCH
        _EMIT_BATCH = None
        if not batch:
            return
        sys.stdout.write("".join(json.dumps(obj) + "\n" for obj in batch))
        sys.stdout.flush()


def _load_module(app_path: Path):
    parent = str(app_path.parent)
    if parent not in sys.path:
        sys.path.insert(0, parent)
    spec = importlib.util.spec_from_file_location("plexi_v3_app", app_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load app module from {app_path!s}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["plexi_v3_app"] = module
    spec.loader.exec_module(module)
    return module


class V3AppRuntime:
    """Purpose-built runtime for SDK v3 module-level apps (init/update/view)."""

    def __init__(self, app_path: Path, launch_args: list[str] | None = None) -> None:
        self._module = _load_module(app_path)
        self._launch_args = launch_args or []
        self._values: dict = {}
        self._last_render_time: float | None = None
        self._frame_id = 0
        self._app_id = ""
        self._workspace_root = ""
        self._capabilities: list[str] = []
        self._running = True

    def run(self) -> None:
        while self._running:
            raw = sys.stdin.readline()
            if not raw:
                break
            raw = raw.strip()
            if not raw:
                continue
            try:
                ev = json.loads(raw)
            except json.JSONDecodeError:
                continue
            _begin_emit_batch()
            try:
                self._handle(ev)
            finally:
                _finish_emit_batch()

    def _handle(self, ev: dict) -> None:
        t = ev.get("type", "")

        if t == "init":
            self._handle_init(ev)
        elif t == "render":
            self._handle_render(ev)
        elif t == "key":
            self._handle_key(ev)
        elif t == "key_events":
            self._handle_key_events(ev)
        elif t == "mouse":
            self._handle_mouse(ev)
        elif t == "timer":
            self._handle_timer(ev)
        elif t == "component_event":
            self._handle_component_event(ev)
        elif t == "text_submitted":
            self._handle_text_submitted(ev)
        elif t == "http_response":
            self._handle_http_response(ev)
        elif t == "tool_call":
            self._dispatch(events.ToolCall(
                call_id=ev.get("call_id", ""),
                name=ev.get("name", ""),
                input_json=ev.get("input_json", ""),
                caller_id=ev.get("caller_id", ""),
            ))
        elif t == "app_events_subscribed":
            self._dispatch(events.EventSubscriptionResult(
                request_id=ev.get("request_id", ""),
                subscription_id=ev.get("subscription_id"),
                error=ev.get("error"),
            ))
        elif t == "app_events_unsubscribed":
            self._dispatch(events.EventUnsubscriptionResult(
                request_id=ev.get("request_id", ""),
                removed=bool(ev.get("removed", False)),
                error=ev.get("error"),
            ))
        elif t == "app_event":
            payload = ev.get("payload")
            self._dispatch(events.AppEvent(
                subscription_id=ev.get("subscription_id", ""),
                app_id=ev.get("app_id", ""),
                event=ev.get("event", ""),
                event_id=int(ev.get("event_id", 0)),
                resource_id=ev.get("resource_id", ""),
                trigger_mode=ev.get("trigger_mode", ""),
                summary=ev.get("summary"),
                payload_json=json.dumps(payload) if payload is not None else None,
                state_ref=ev.get("state_ref"),
                created_at=ev.get("created_at", ""),
            ))
        elif t == "declare_event_streams_result":
            self._dispatch(events.DeclareEventStreamsResult(
                streams=ev.get("streams"),
                error=ev.get("error"),
            ))
        elif t == "emit_event_result":
            self._dispatch(events.EmitEventResult(
                sequence=ev.get("sequence"),
                error=ev.get("error"),
            ))
        elif t == "file_read_result":
            content = ev.get("content")
            self._dispatch(events.FileReadResult(
                content=content.encode("utf-8") if isinstance(content, str) else None,
                error=ev.get("error"),
            ))
        elif t == "file_write_result":
            self._dispatch(events.FileWriteResult(error=ev.get("error")))
        elif t == "host_log_result":
            content = ev.get("content")
            self._dispatch(events.HostLogResult(
                content=content.encode("utf-8") if isinstance(content, str) else None,
                path=ev.get("path"),
                error=ev.get("error"),
            ))
        elif t == "capability_decision":
            self._handle_capability_decision(ev)
        elif t == "ui_action":
            self._handle_ui_action(ev)
        elif t == "focus_changed":
            self._handle_focus_changed(ev)
        elif t == "capability_granted":
            self._dispatch(events.CapabilityGranted(name=ev.get("name", "")))
        elif t == "capability_denied":
            self._dispatch(events.CapabilityDenied(name=ev.get("name", "")))
        elif t == "resize":
            self._handle_resize(ev)
        elif t == "shutdown":
            self._running = False
        elif t == "theme":
            from ._theme import theme as _theme
            _theme.update_from(ev.get("colors"))
        elif t == "inject_state":
            payload = ev.get("payload") or {}
            if isinstance(payload, dict):
                self._values.update(payload)
                self._set_state(in_view=False)

    def _handle_init(self, ev: dict) -> None:
        self._app_id = ev.get("app_id", "")
        self._workspace_root = ev.get("workspace_root", "")
        self._capabilities = ev.get("capabilities", [])

        from ._theme import theme as _theme
        _theme.update_from(ev.get("theme"))

        init_state = ev.get("state")
        if init_state and isinstance(init_state, dict):
            self._values = dict(init_state)

        _emit({
            "type": "ready",
            "sdk": sdk.SDK_ID,
            "features_used": [],
        })

        self._set_state(in_view=False)
        size = ev.get("size")
        if not isinstance(size, (list, tuple)) or len(size) != 2:
            size = [ev.get("width", 0.0), ev.get("height", 0.0)]
        init_effects = self._module.init(tuple(size), self._launch_args)
        self._apply_effects(init_effects)

    def _handle_render(self, ev: dict) -> None:
        now = time.monotonic()
        elapsed = 0.0 if self._last_render_time is None else now - self._last_render_time
        self._last_render_time = now
        self._frame_id += 1
        frame_id = ev.get("frame_id", self._frame_id)

        timer_ids = ev.get("timer_ids", [])
        if isinstance(timer_ids, list):
            for timer_id in timer_ids:
                self._dispatch_timer(timer_id, schedule_render=False)

        self._dispatch(
            events.RenderFrame(frame_id=self._frame_id, elapsed=elapsed),
            schedule_render=False,
        )

        self._set_state(in_view=True)
        try:
            root = self._module.view()
        finally:
            self._set_state(in_view=False)

        if root is not None:
            from ._adapter import _encode_uitree
            _emit({
                "type": "component_tree",
                "frame_id": frame_id,
                "tree": _encode_uitree(root),
            })

        _emit({"type": "frame_done", "frame_id": frame_id})

    def _handle_key(self, ev: dict, *, schedule_render: bool = True) -> None:
        key = _normalize_key(ev.get("key", ""))
        mods = ev.get("modifiers", {})
        modifiers = events.Modifiers(
            ctrl=bool(mods.get("ctrl", False)),
            shift=bool(mods.get("shift", False)),
            alt=bool(mods.get("alt", False)),
            meta=bool(mods.get("meta", False)),
        )
        self._dispatch(events.KeyEvent(
            key=key,
            modifiers=modifiers,
            pressed=bool(ev.get("pressed", True)),
        ), schedule_render=schedule_render)

    def _handle_key_events(self, ev: dict) -> None:
        key_events = ev.get("events", [])
        if not isinstance(key_events, list):
            return
        handled = False
        for key_event in key_events:
            if isinstance(key_event, dict):
                self._handle_key(key_event, schedule_render=False)
                handled = True
        if handled:
            _emit({"type": "schedule_render", "after_ms": 16})

    def _handle_mouse(self, ev: dict) -> None:
        self._dispatch(events.MouseEvent(
            x=float(ev.get("x", 0.0)),
            y=float(ev.get("y", 0.0)),
            button=ev.get("button"),
            pressed=bool(ev.get("pressed", False)),
            scroll_x=float(ev.get("scroll_x", 0.0)),
            scroll_y=float(ev.get("scroll_y", 0.0)),
        ))

    def _handle_timer(self, ev: dict) -> None:
        self._dispatch_timer(
            ev.get("timer_id", ""),
            schedule_render=False,
        )

    def _dispatch_timer(self, timer_id, *, schedule_render: bool) -> None:
        try:
            parsed_id = int(timer_id)
        except (ValueError, TypeError):
            return
        self._dispatch(
            events.TimerFired(id=parsed_id),
            schedule_render=schedule_render,
        )

    def _handle_component_event(self, ev: dict) -> None:
        node_id = ev.get("node_id", "")
        event_type = ev.get("event_type", "")
        if event_type == "click" and node_id:
            self._dispatch(events.UiAction(handler_id=node_id))

    def _handle_ui_action(self, ev: dict) -> None:
        handler_id = ev.get("handler_id", "")
        if handler_id:
            self._dispatch(events.UiAction(handler_id=handler_id))

    def _handle_focus_changed(self, ev: dict) -> None:
        self._dispatch(events.FocusChanged(
            timestamp=ev.get("timestamp", ""),
            duration_secs=int(ev.get("duration_secs", 0)),
            reason=ev.get("reason", "focus_changed"),
            pane_id=ev.get("pane_id"),
            context_name=ev.get("context_name"),
            context_root=ev.get("context_root"),
            cwd=ev.get("cwd"),
        ))

    def _handle_text_submitted(self, ev: dict) -> None:
        tid = ev.get("id", "")
        value = ev.get("value", "")
        if tid:
            self._dispatch(events.UiValueChange(handler_id=tid, value=value))

    def _handle_http_response(self, ev: dict) -> None:
        error = ev.get("error")
        if error:
            body = error.encode("utf-8")
            status = 0
        else:
            body = ev.get("body", "").encode("utf-8")
            status = 200
        self._dispatch(events.HttpResponse(status=status, headers=[], body=body))

    def _handle_capability_decision(self, ev: dict) -> None:
        granted = ev.get("granted", False)
        capability = ev.get("capability", "")
        if granted:
            if capability and capability not in self._capabilities:
                self._capabilities.append(capability)
            self._dispatch(events.CapabilityGranted(name=capability))
        else:
            self._dispatch(events.CapabilityDenied(name=capability))

    def _handle_resize(self, ev: dict) -> None:
        w = float(ev.get("width", 0.0))
        h = float(ev.get("height", 0.0))
        self._dispatch(events.Resize(width=w, height=h))

    def _dispatch(self, event, *, schedule_render: bool = True) -> None:
        self._set_state(in_view=False)
        app_effects = self._module.update(event)
        self._apply_effects(app_effects)
        if schedule_render:
            _emit({"type": "schedule_render", "after_ms": 16})

    def _set_state(self, in_view: bool) -> None:
        snapshot = StateSnapshot(dict(self._values), {})
        setattr(sdk, "_state", snapshot)
        setattr(sdk, "_in_view", in_view)
        _v3_state._state = snapshot
        _v3_state._in_view = in_view

    def _apply_effects(self, app_effects) -> None:
        state_changed = False
        for effect in app_effects or []:
            if isinstance(effect, effects.SetState):
                self._values.update(effect.data)
                state_changed = True
            elif isinstance(effect, effects.PersistState):
                self._values.update(effect.data)
                state_changed = True
                _emit({"type": "save_app_state", "payload": dict(self._values)})
            elif isinstance(effect, effects.SetStatus):
                _emit({"type": "status_summary", "text": effect.text})
            elif isinstance(effect, effects.SetTitle):
                _emit({"type": "set_title", "title": effect.title})
            elif isinstance(effect, effects.SetTimer):
                _emit({
                    "type": "set_timer",
                    "timer_id": str(effect.id),
                    "after_ms": int(effect.delay_ms),
                    "repeat": bool(effect.repeat),
                })
            elif isinstance(effect, effects.CancelTimer):
                _emit({"type": "cancel_timer", "timer_id": str(effect.id)})
            elif isinstance(effect, effects.SetSchedulerMode):
                payload: dict = {"type": "set_scheduler_mode", "mode": effect.mode}
                if effect.fps is not None:
                    payload["fps"] = int(effect.fps)
                _emit(payload)
            elif isinstance(effect, effects.HttpFetch):
                payload = {
                    "type": "http_request",
                    "request_id": str(uuid.uuid4()),
                    "method": effect.method,
                    "url": effect.url,
                }
                if effect.headers:
                    payload["headers"] = effect.headers
                if effect.body is not None:
                    payload["body"] = effect.body.decode("utf-8")
                _emit(payload)
            elif isinstance(effect, effects.FileRead):
                _emit({"type": "file_read", "path": effect.path})
            elif isinstance(effect, effects.FileWrite):
                _emit({
                    "type": "file_write",
                    "path": effect.path,
                    "content": effect.content.decode("utf-8"),
                })
            elif isinstance(effect, effects.ReadHostLog):
                _emit({"type": "read_host_log", "max_bytes": int(effect.max_bytes)})
            elif isinstance(effect, effects.RequestCapability):
                _emit({
                    "type": "capability_request",
                    "request_id": str(uuid.uuid4()),
                    "capability": effect.name,
                })
            elif isinstance(effect, effects.CloseSelf):
                _emit({"type": "close_self"})
            elif isinstance(effect, effects.ExposeTools):
                _emit({
                    "type": "expose_tools",
                    "tools": [
                        {
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": tool.input_schema,
                            "output_schema": tool.output_schema,
                            "timeout_ms": tool.timeout_ms,
                            "read_only": tool.read_only,
                        }
                        for tool in effect.tools
                    ],
                })
            elif isinstance(effect, effects.ToolResult):
                payload = {"type": "tool_result", "call_id": effect.call_id}
                if effect.output_json is not None:
                    payload["output_json"] = effect.output_json
                if effect.error is not None:
                    payload["error"] = effect.error
                _emit(payload)
            elif isinstance(effect, effects.DeclareEventStreams):
                _emit({
                    "type": "declare_event_streams",
                    "streams": [
                        {
                            "name": stream.name,
                            "schema_json": stream.schema_json,
                            "description": stream.description,
                        }
                        for stream in effect.streams
                    ],
                })
            elif isinstance(effect, effects.EmitEvent):
                _emit({
                    "type": "emit_event",
                    "event": effect.event,
                    "actor": effect.actor,
                    "actor_id": effect.actor_id,
                    "caused_by": effect.caused_by,
                    "summary": effect.summary,
                    "resource_id": effect.resource_id,
                    "resource_scope": effect.resource_scope,
                    "revision_after": effect.revision_after,
                    "payload_json": effect.payload_json,
                    "state_ref": effect.state_ref,
                    "revision_before": effect.revision_before,
                    "rollback_token": effect.rollback_token,
                    "changed_resources": effect.changed_resources,
                    "suggested_trigger": effect.suggested_trigger,
                })
            elif isinstance(effect, effects.SubscribeEventStreams):
                _emit({
                    "type": "subscribe_event_streams",
                    "request_id": effect.request_id,
                    "app_id": effect.app_id,
                    "event_names": effect.event_names,
                    "payload_mode": effect.payload_mode,
                    "trigger_mode": effect.trigger_mode,
                    "resource_id": effect.resource_id,
                })
            elif isinstance(effect, effects.UnsubscribeEventStreams):
                _emit({
                    "type": "unsubscribe_event_streams",
                    "request_id": effect.request_id,
                    "subscription_id": effect.subscription_id,
                })
        if state_changed:
            self._set_state(in_view=False)


def _host_log(level: str, msg: str) -> None:
    _emit({"type": "log", "level": level, "message": msg})


def main(argv: list[str] | None = None) -> None:
    args = list(sys.argv[1:] if argv is None else argv)
    if not args:
        raise SystemExit("usage: python -m plexi_sdk._v3_runtime <app.py> [args...]")
    app_path = Path(args[0]).resolve()
    launch_args = args[1:]
    _v3_state._host_log = _host_log
    runtime = V3AppRuntime(app_path, launch_args)
    runtime.run()


if __name__ == "__main__":
    main()
