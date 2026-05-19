#!/usr/bin/env python3
"""Webcam Viewer — POC for #1505.

Streams live camera frames from camera://0 over the video binary pipe and
renders each RGBA8 frame as a full-pane image via ctx.image().

Requires video.capture + pipe.open in manifest.toml.
"""
from __future__ import annotations

import base64
import threading
from typing import TYPE_CHECKING, cast

from plexi_sdk import App, RenderContext

if TYPE_CHECKING:
    from plexi_sdk._types import VideoHandle
    from plexi_sdk._pipe import Pipe

PIPE_ID = "webcam-viewer-feed"


class WebcamViewerApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self._handle: VideoHandle | None = None
        self._reader_thread: threading.Thread | None = None
        self._reader_stop = threading.Event()
        self._last_frame_b64: str | None = None
        self._width: int = 640
        self._height: int = 480
        self._status: str = "Opening camera..."

        self.emit.info("webcam-viewer: enumerating cameras")
        try:
            cam_list = await self.emit.list_camera_devices()  # type: ignore[attr-defined]
            count = len(cam_list.cameras)
            self.emit.info(f"webcam-viewer: found {count} camera(s)")
            if count == 0:
                self._status = "No cameras found."
                ctx.status_summary("Webcam Viewer — no cameras")
                return
            for c in cam_list.cameras:
                self.emit.info(f"  camera id={c.id!r} name={c.name!r}")
        except Exception as e:
            self._status = f"Camera list failed: {e}"
            self.emit.error(f"webcam-viewer: list failed: {e}")
            ctx.status_summary("Webcam Viewer — error")
            return

        try:
            self._handle = await self.emit.open_video("camera://0", PIPE_ID)
            self._width = self._handle.width
            self._height = self._handle.height
            self._status = f"Live — {self._width}x{self._height} @ {self._handle.fps:.0f}fps"
            self.emit.info(
                f"webcam-viewer: camera open handle_id={self._handle.handle_id} "
                f"{self._width}x{self._height} @ {self._handle.fps}fps"
            )
        except Exception as e:
            self._status = f"Camera open failed: {e}"
            self.emit.error(f"webcam-viewer: open_video failed: {e}")
            ctx.status_summary("Webcam Viewer — open failed")
            return

        # Start background reader thread.
        self._reader_stop.clear()
        self._reader_thread = threading.Thread(
            target=self._read_frames,
            daemon=True,
            name="webcam-reader",
        )
        self._reader_thread.start()

        ctx.status_summary("Webcam Viewer")

    def _read_frames(self) -> None:
        """Background thread: read RGBA8 frames from the binary pipe."""
        handle = self._handle
        if handle is None or handle.pipe is None:
            self.emit.warn("webcam-viewer: no pipe to read")
            return

        pipe = cast("Pipe", handle.pipe)
        if not pipe.connect(timeout=10.0):
            self.emit.warn("webcam-viewer: pipe connect timed out")
            return

        self.emit.info("webcam-viewer: pipe connected, reading frames")

        while not self._reader_stop.is_set():
            try:
                frame = pipe.read_frame()
            except Exception as e:
                self.emit.warn(f"webcam-viewer: read_frame error: {e}")
                break
            if not frame:
                continue
            # Encode as a data URL the host's image loader understands.
            # The host uses width * height * 4 = expected frame size; it reads
            # the raw RGBA bytes directly. We base64-encode for the JSON wire.
            b64 = base64.b64encode(frame).decode()
            # Store as data URL with dimensions embedded in a custom scheme
            # that the SDK's ctx.image() passes through unchanged to the host.
            self._last_frame_b64 = (
                f"data:image/rgba8;width={self._width};height={self._height};base64,{b64}"
            )

        self.emit.info("webcam-viewer: frame reader stopped")

    async def on_frame(self, ctx: RenderContext) -> None:
        ctx.clear("#000000")

        if self._last_frame_b64:
            ctx.image(
                src=self._last_frame_b64,
                x=0.0,
                y=0.0,
                w=ctx.w,
                h=ctx.h,
                fit="contain",
            )
        else:
            # Show status while waiting for first frame.
            ctx.text(
                x=ctx.w / 2,
                y=ctx.h / 2,
                text=self._status,
                size=14.0,
                color="#ffffff",
                align="center",
            )

    async def on_close(self, _ctx: RenderContext) -> None:
        self._reader_stop.set()
        if self._handle is not None:
            self.emit.close_video(self._handle.handle_id)
        if self._reader_thread is not None:
            self._reader_thread.join(timeout=2.0)


if __name__ == "__main__":
    WebcamViewerApp().run()
