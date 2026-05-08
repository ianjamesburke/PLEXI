"""Tests for blocking emit guard on sync hooks (issue #869)."""
import asyncio
import threading

import pytest


def _make_app():
    from plexi_sdk._app import App
    app = App()
    return app


def test_capability_request_raises_from_sync_hook():
    """capability_request() called inside a sync-hook scope raises TypeError."""
    from plexi_sdk._emitter import Emitter, _sync_hook_scope
    emitter = Emitter(_make_app())
    with _sync_hook_scope():
        with pytest.raises(TypeError, match="async def"):
            emitter.capability_request("net.http")


def test_secret_get_raises_from_sync_hook():
    from plexi_sdk._emitter import Emitter, _sync_hook_scope
    emitter = Emitter(_make_app())
    with _sync_hook_scope():
        with pytest.raises(TypeError, match="await"):
            emitter.secret_get("MY_KEY")


def test_list_audio_devices_raises_from_sync_hook():
    from plexi_sdk._emitter import Emitter, _sync_hook_scope
    emitter = Emitter(_make_app())
    with _sync_hook_scope():
        with pytest.raises(TypeError):
            emitter.list_audio_devices()


def test_blocking_emit_returns_coroutine_outside_sync_hook():
    """Outside a sync-hook scope, blocking emit returns a coroutine (no raise)."""
    from plexi_sdk._emitter import Emitter
    emitter = Emitter(_make_app())
    coro = emitter.capability_request("net.http")
    assert asyncio.iscoroutine(coro)
    coro.close()


def test_sentinel_is_thread_local():
    """Sentinel set on event-loop thread does not affect background threads."""
    from plexi_sdk._emitter import Emitter, _sync_hook_scope
    emitter = Emitter(_make_app())
    raised = []

    def _bg():
        try:
            coro = emitter.capability_request("net.http")
            coro.close()
        except TypeError:
            raised.append(True)

    with _sync_hook_scope():
        t = threading.Thread(target=_bg)
        t.start()
        t.join()

    assert not raised, "sentinel on event-loop thread must not affect background threads"


def test_error_message_is_actionable():
    """TypeError message includes 'await', 'async def', and the method name."""
    from plexi_sdk._emitter import Emitter, _sync_hook_scope
    emitter = Emitter(_make_app())
    with _sync_hook_scope():
        with pytest.raises(TypeError) as exc_info:
            emitter.capability_request("net.http")
    msg = str(exc_info.value)
    assert "await" in msg
    assert "async def" in msg
    assert "capability_request" in msg


def test_sync_hook_dispatch_raises_for_blocking_emit():
    """_dispatch_hook with a sync hook that calls capability_request raises TypeError."""
    from plexi_sdk._app import App

    class BadApp(App):
        def on_init(self, ctx):
            self.emit.capability_request("net.http")

    app = BadApp()

    async def _run():
        with pytest.raises(TypeError, match="async def"):
            await app._dispatch_hook(app.on_init, None)

    asyncio.run(_run())
