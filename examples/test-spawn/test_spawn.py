#!/usr/bin/env python3
from plexi_sdk import App, RenderContext

class TestApp(App):
    async def on_start(self, ctx: RenderContext):
        ctx.emit.spawn_pane("terminal", request_id="req-1")
        ctx.emit.spawn_pane("terminal", layout="split_h", request_id="req-2")

    def on_pane_spawned(self, pane_id, request_id=None):
        import sys
        print(f"[PASS] pane_spawned pane_id={pane_id} request_id={request_id}", file=sys.stderr, flush=True)

    def on_pane_spawn_error(self, reason, request_id=None):
        import sys
        print(f"[FAIL] pane_spawn_error reason={reason} request_id={request_id}", file=sys.stderr, flush=True)

TestApp().run()
