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


def _emit(obj: dict) -> None:
    with _LOCK:
        sys.stdout.write(json.dumps(obj) + "\n")
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
        self._repeating_timers: dict[int, int] = {}
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
            self._handle(ev)

    def _handle(self, ev: dict) -> None:
        t = ev.get("type", "")

        if t == "init":
            self._handle_init(ev)
        elif t == "render":
            self._handle_render(ev)
        elif t == "key":
            self._handle_key(ev)
        elif t == "timer":
            self._handle_timer(ev)
        elif t == "component_event":
            self._handle_component_event(ev)
        elif t == "http_response":
            self._handle_http_response(ev)
        elif t == "file_list_result":
            self._handle_file_list_result(ev)
        elif t == "file_read_result":
            self._handle_file_read_result(ev)
        elif t == "file_write_result":
            self._handle_file_write_result(ev)
        elif t == "capability_decision":
            self._handle_capability_decision(ev)
        elif t == "ui_action":
            self._handle_ui_action(ev)
        elif t == "list_select":
            self._handle_list_select(ev)
        elif t == "list_activate":
            self._handle_list_activate(ev)
        elif t == "focus_changed":
            self._handle_focus_changed(ev)
        elif t == "capability_granted":
            self._dispatch(events.CapabilityGranted(name=ev.get("name", "")))
        elif t == "capability_denied":
            self._dispatch(events.CapabilityDenied(name=ev.get("name", "")))
        elif t == "click":
            self._handle_click(ev)
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
        sdk._workspace_root = self._workspace_root
        w = float(ev.get("width", 0))
        h = float(ev.get("height", 0))
        sdk.pane_width = w
        sdk.pane_height = h
        sdk.canvas_width = 0.0
        sdk.canvas_height = 0.0
        sdk.keys_held = set()
        self._capabilities = ev.get("capabilities", [])

        from ._theme import theme as _theme
        _theme.update_from(ev.get("theme"))

        init_state = ev.get("state")
        if init_state and isinstance(init_state, dict):
            self._values = dict(init_state)

        _emit({
            "type": "ready",
            "sdk": sdk.SDK_ID,
            "protocol_version": sdk.PROTOCOL_VERSION,
            "features_used": [],
        })

        self._set_state(in_view=False)
        init_effects = self._module.init(
            (w, h),
            self._launch_args,
        )
        self._apply_effects(init_effects)

    def _handle_render(self, ev: dict) -> None:
        cw = float(ev.get("canvas_width", 0.0))
        ch = float(ev.get("canvas_height", 0.0))
        if cw > 0.0 and ch > 0.0:
            prev_cw, prev_ch = sdk.canvas_width, sdk.canvas_height
            sdk.canvas_width = cw
            sdk.canvas_height = ch
            if (prev_cw == 0.0 or abs(cw - prev_cw) > 1.0 or abs(ch - prev_ch) > 1.0):
                self._dispatch(
                    events.Resize(width=sdk.pane_width, height=sdk.pane_height),
                    schedule_render=False,
                )
        now = time.monotonic()
        elapsed = 0.0 if self._last_render_time is None else now - self._last_render_time
        self._last_render_time = now
        self._frame_id += 1

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
            node = root.to_node() if hasattr(root, "to_node") else root
            _emit({"type": "component_tree", "root": node})

        frame_id = ev.get("frame_id", self._frame_id)
        _emit({"type": "frame_done", "frame_id": frame_id})

    def _handle_key(self, ev: dict) -> None:
        key = _normalize_key(ev.get("key", ""))
        pressed = ev.get("pressed", True)
        mods = ev.get("modifiers", {})
        modifiers = events.Modifiers(
            ctrl=bool(mods.get("ctrl", False)),
            shift=bool(mods.get("shift", False)),
            alt=bool(mods.get("alt", False)),
            meta=bool(mods.get("meta", False)),
        )
        if pressed:
            sdk.keys_held.add(key)
        else:
            sdk.keys_held.discard(key)
        self._dispatch(events.KeyEvent(key=key, modifiers=modifiers, pressed=bool(pressed)))

    def _handle_timer(self, ev: dict) -> None:
        timer_id_str = ev.get("timer_id", "")
        try:
            parsed_id = int(timer_id_str)
        except (ValueError, TypeError):
            return
        self._dispatch(events.TimerFired(id=parsed_id))
        repeat_ms = self._repeating_timers.get(parsed_id)
        if repeat_ms is not None:
            _emit({"type": "set_timer", "timer_id": str(parsed_id), "after_ms": repeat_ms})

    def _handle_component_event(self, ev: dict) -> None:
        node_id = ev.get("node_id", "")
        event_type = ev.get("event_type", "")
        payload = ev.get("payload") or {}
        if event_type == "click" and node_id:
            self._dispatch(events.UiAction(handler_id=node_id))
        elif event_type == "change" and node_id:
            value = payload.get("value", "") if isinstance(payload, dict) else ""
            self._dispatch(events.UiValueChange(handler_id=node_id, value=value))
        elif event_type == "submit" and node_id:
            value = payload.get("value", "") if isinstance(payload, dict) else ""
            self._dispatch(events.UiValueChange(handler_id=node_id, value=value))
            self._dispatch(events.UiAction(handler_id=node_id))

    def _handle_ui_action(self, ev: dict) -> None:
        handler_id = ev.get("handler_id", "")
        if handler_id:
            self._dispatch(events.UiAction(handler_id=handler_id))

    def _handle_list_select(self, ev: dict) -> None:
        list_id = ev.get("id", "")
        if list_id:
            self._dispatch(events.ListSelect(id=list_id, index=int(ev.get("index", 0))))

    def _handle_list_activate(self, ev: dict) -> None:
        list_id = ev.get("id", "")
        if list_id:
            self._dispatch(events.ListActivate(id=list_id, index=int(ev.get("index", 0))))

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

    def _handle_http_response(self, ev: dict) -> None:
        error = ev.get("error")
        if error:
            body = error.encode("utf-8")
            status = 0
        else:
            body = ev.get("body", "").encode("utf-8")
            status = 200
        self._dispatch(events.HttpResponse(status=status, headers=[], body=body))

    def _handle_file_list_result(self, ev: dict) -> None:
        entries = ev.get("entries")
        if isinstance(entries, list):
            entries = [
                events.FileListEntry(
                    name=str(item.get("name", "")),
                    path=str(item.get("path", "")),
                    is_dir=bool(item.get("is_dir", False)),
                    size_bytes=item.get("size_bytes"),
                )
                for item in entries
                if isinstance(item, dict)
            ]
        else:
            entries = None
        self._dispatch(events.FileListResult(entries=entries, error=ev.get("error")))

    def _handle_file_read_result(self, ev: dict) -> None:
        content = ev.get("content")
        if isinstance(content, list):
            content = bytes(content)
        elif isinstance(content, str):
            content = content.encode("utf-8")
        else:
            content = None
        self._dispatch(events.FileReadResult(content=content, error=ev.get("error")))

    def _handle_file_write_result(self, ev: dict) -> None:
        self._dispatch(events.FileWriteResult(error=ev.get("error")))

    def _handle_capability_decision(self, ev: dict) -> None:
        granted = ev.get("granted", False)
        capability = ev.get("capability", "")
        if granted:
            if capability and capability not in self._capabilities:
                self._capabilities.append(capability)
            self._dispatch(events.CapabilityGranted(name=capability))
        else:
            self._dispatch(events.CapabilityDenied(name=capability))

    def _handle_click(self, ev: dict) -> None:
        x = float(ev.get("x", 0.0))
        y = float(ev.get("y", 0.0))
        button = ev.get("button", "primary")
        region = ev.get("region")
        self._dispatch(events.MouseEvent(x=x, y=y, button=button, pressed=True, region=region))

    def _handle_resize(self, ev: dict) -> None:
        w = float(ev.get("width", 0.0))
        h = float(ev.get("height", 0.0))
        sdk.pane_width = w
        sdk.pane_height = h
        self._dispatch(events.Resize(width=w, height=h))

    def _dispatch(self, event, *, schedule_render: bool = True) -> None:
        self._set_state(in_view=False)
        app_effects = self._module.update(event)
        self._apply_effects(app_effects)
        if schedule_render:
            _emit({"type": "schedule_render", "after_ms": 16})

    def _set_state(self, in_view: bool) -> None:
        snapshot = StateSnapshot(dict(self._values), {})
        sdk._state = snapshot
        sdk._in_view = in_view
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
                })
                if effect.repeat:
                    self._repeating_timers[int(effect.id)] = int(effect.delay_ms)
                else:
                    self._repeating_timers.pop(int(effect.id), None)
            elif isinstance(effect, effects.CancelTimer):
                self._repeating_timers.pop(int(effect.id), None)
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
            elif isinstance(effect, effects.OpenUrl):
                _emit({"type": "open_url", "url": effect.url})
            elif isinstance(effect, effects.FileList):
                _emit({
                    "type": "file_list",
                    "path": effect.path,
                    "extensions": list(effect.extensions or []),
                })
            elif isinstance(effect, effects.FileRead):
                _emit({"type": "file_read", "path": effect.path})
            elif isinstance(effect, effects.FileWrite):
                _emit({
                    "type": "file_write",
                    "path": effect.path,
                    "content": list(effect.content),
                })
            elif isinstance(effect, effects.RequestCapability):
                _emit({
                    "type": "capability_request",
                    "request_id": str(uuid.uuid4()),
                    "capability": effect.name,
                })
            elif isinstance(effect, effects.CloseSelf):
                _emit({"type": "close_self"})
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
