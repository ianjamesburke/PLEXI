from __future__ import annotations

import asyncio
import importlib.util
import sys
from pathlib import Path

import plexi_sdk as sdk
from plexi_sdk import App
from plexi_sdk import _v3_state
from plexi_sdk import effects
from plexi_sdk import events
from plexi_sdk._v3_state import StateSnapshot


def _load_module(app_path: Path):
    spec = importlib.util.spec_from_file_location("plexi_sdk_v3_process_app", app_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load app module from {app_path!s}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["plexi_sdk_v3_process_app"] = module
    spec.loader.exec_module(module)
    return module


class V3ProcessApp(App):
    def __init__(self, app_path: Path) -> None:
        super().__init__()
        self._module = _load_module(app_path)
        self._v3_values: dict = {}
        self._repeating_timers: dict[int, int] = {}

    def on_init(self):
        self._v3_values = dict(self.state.all())
        self._set_v3_state(in_view=False)
        self._apply_effects(self._module.init((0.0, 0.0), self.launch_args))

    def view(self):
        self._set_v3_state(in_view=True)
        try:
            return self._module.view()
        finally:
            self._set_v3_state(in_view=False)

    def on_key(self, key, mods):
        modifiers = events.Modifiers(
            ctrl=bool(mods.get("ctrl", False)),
            shift=bool(mods.get("shift", False)),
            alt=bool(mods.get("alt", False)),
            meta=bool(mods.get("meta", False)),
        )
        self._dispatch_v3(events.KeyEvent(key=key, modifiers=modifiers))

    def on_timer(self, timer_id: str):
        try:
            parsed_id = int(timer_id)
        except ValueError:
            return
        self._dispatch_v3(events.TimerFired(id=parsed_id))
        repeat_ms = self._repeating_timers.get(parsed_id)
        if repeat_ms is not None:
            self.emit.set_timer(str(parsed_id), repeat_ms)

    def _dispatch_v3(self, event) -> None:
        self._set_v3_state(in_view=False)
        self._apply_effects(self._module.update(event))
        self.emit.schedule_render()

    def _set_v3_state(self, in_view: bool) -> None:
        snapshot = StateSnapshot(dict(self._v3_values), {})
        sdk._state = snapshot
        sdk._in_view = in_view
        _v3_state._state = snapshot
        _v3_state._in_view = in_view

    def _apply_effects(self, app_effects) -> None:
        state_changed = False
        for effect in app_effects or []:
            if isinstance(effect, effects.SetState):
                self._v3_values.update(effect.data)
                state_changed = True
            elif isinstance(effect, effects.SetStatus):
                self.emit.status_summary(effect.text)
            elif isinstance(effect, effects.SetTimer):
                self.emit.set_timer(str(effect.id), int(effect.delay_ms))
                if effect.repeat:
                    self._repeating_timers[int(effect.id)] = int(effect.delay_ms)
            elif isinstance(effect, effects.CancelTimer):
                self._repeating_timers.pop(int(effect.id), None)
                self.emit.cancel_timer(str(effect.id))
            elif isinstance(effect, effects.HttpFetch):
                asyncio.create_task(self._run_http_fetch(effect))
            elif isinstance(effect, effects.RequestCapability):
                asyncio.create_task(self._request_capability(effect.name))
            elif isinstance(effect, effects.CloseSelf):
                self.emit.close_self()
            elif isinstance(effect, effects.SetTitle):
                continue
            else:
                self.emit.info(f"sdk_v3_process: ignored effect {type(effect).__name__}")
        if state_changed:
            self.state.save(self._v3_values)

    async def _request_capability(self, capability: str) -> None:
        try:
            await self.emit.capability_request(capability)
        except Exception as exc:
            self.emit.info(f"sdk_v3_process: capability {capability!r} denied: {exc}")

    async def _run_http_fetch(self, effect: effects.HttpFetch) -> None:
        try:
            body = await self.emit.http_request(
                effect.url,
                method=effect.method,
                headers=effect.headers,
                body=effect.body.decode("utf-8") if effect.body is not None else None,
            )
            response = events.HttpResponse(status=200, headers=[], body=body.encode("utf-8"))
        except Exception as exc:
            response = events.HttpResponse(
                status=0,
                headers=[],
                body=str(exc).encode("utf-8"),
            )
        self._dispatch_v3(response)


def _host_log(level: str, msg: str) -> None:
    app = getattr(V3ProcessApp, "current", None)
    if app is not None:
        app.emit.info(f"{level}: {msg}")
        return
    print(f"[{level}] {msg}", file=sys.stderr)


def main(argv: list[str] | None = None) -> None:
    args = list(sys.argv[1:] if argv is None else argv)
    if not args:
        raise SystemExit("usage: python -m plexi_sdk._v3_process <app.py>")
    app = V3ProcessApp(Path(args[0]).resolve())
    V3ProcessApp.current = app
    _v3_state._host_log = _host_log
    app.run()


if __name__ == "__main__":
    main()
