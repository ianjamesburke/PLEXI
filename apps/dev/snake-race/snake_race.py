#!/usr/bin/env python3
"""Snake Race — four-pane speedrun to 50 combined points with timer and personal best.

Spawns four Snake instances in a 2×2 grid, aggregates scores via pipes,
and records the time to reach 50 total points.
"""

from __future__ import annotations

import json
import pathlib
import time
from typing import Any

from plexi_sdk import App, CapabilityDeniedError, RenderContext

TOTAL_TARGET = 50
NUM_SNAKES = 4
PB_FILE = "snake-race-pb.json"

# (layout, from_snake_index_or_None)
# Produces: [Coord | S1 S3 / S2 S4]
SPAWN_PLAN = [
    ("split_h", None),  # S1: right of coordinator
    ("split_v", 0),     # S2: below S1
    ("split_h", 0),     # S3: right of S1
    ("split_h", 1),     # S4: right of S2
]


class SnakeRaceApp(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self._scores: list[int] = [0] * NUM_SNAKES
        self._pane_ids: list[int | None] = [None] * NUM_SNAKES
        self._spawned = 0
        self._start_time: float | None = None
        self._stop_time: float | None = None
        self._pb: float | None = self._load_pb()
        self.emit.info("snake-race: init")
        try:
            await self.emit.capability_request("panes.spawn")
        except CapabilityDeniedError:
            self.emit.error("snake-race: panes.spawn denied — cannot spawn snake panes")
            return
        self._spawn_next()

    def _spawn_next(self) -> None:
        i = self._spawned
        if i >= NUM_SNAKES:
            return
        layout, from_idx = SPAWN_PLAN[i]
        from_pane_id = self._pane_ids[from_idx] if from_idx is not None else None
        pipe_id = f"race-{i}"
        self.emit.pipe_open(pipe_id, mode="json", direction="in")
        self.emit.spawn_pane(
            "snake",
            layout=layout,
            pipe_id=pipe_id,
            from_pane_id=from_pane_id,
            request_id=f"s{i}",
        )
        self.emit.info(
            f"snake-race: spawning S{i + 1} layout={layout} from_pane_id={from_pane_id}"
        )

    def on_pane_spawned(self, pane_id: int, request_id: str | None = None) -> None:
        if not request_id or not request_id.startswith("s"):
            return
        try:
            idx = int(request_id[1:])
        except ValueError:
            return
        self._pane_ids[idx] = pane_id
        self._spawned += 1
        self.emit.info(f"snake-race: S{idx + 1} spawned pane_id={pane_id}")
        self._spawn_next()

    def on_pane_spawn_error(self, reason: str, request_id: str | None = None) -> None:
        self.emit.error(
            f"snake-race: spawn failed request_id={request_id!r} reason={reason!r}"
        )

    def on_pipe_message(
        self, ctx: RenderContext, pipe_id: str, payload: Any
    ) -> None:
        if not pipe_id.startswith("race-"):
            return
        try:
            idx = int(pipe_id[len("race-"):])
        except ValueError:
            return

        event = payload.get("event") if isinstance(payload, dict) else None
        score = payload.get("score", 0) if isinstance(payload, dict) else 0

        if event == "score":
            self._scores[idx] = score
            if self._start_time is None:
                self._start_time = time.monotonic()
                self.emit.info("snake-race: timer started")
            total = sum(self._scores)
            if total >= TOTAL_TARGET and self._stop_time is None:
                self._stop_time = time.monotonic()
                elapsed = self._stop_time - self._start_time
                self.emit.info(f"snake-race: finished elapsed={elapsed:.3f}s")
                self._maybe_update_pb(elapsed)
        elif event == "game_over":
            self._scores[idx] = score
            self.emit.info(f"snake-race: S{idx + 1} game_over score={score}")

    def _pb_dir(self) -> pathlib.Path:
        root = self.workspace_root
        return pathlib.Path(root) if root else pathlib.Path.home()

    def _load_pb(self) -> float | None:
        try:
            p = self._pb_dir() / PB_FILE
            if p.exists():
                data = json.loads(p.read_text())
                val = float(data.get("pb_seconds", 0))
                return val if val > 0 else None
        except Exception as e:
            self.emit.warn(f"snake-race: pb load failed: {e}")
        return None

    def _maybe_update_pb(self, elapsed: float) -> None:
        if self._pb is None or elapsed < self._pb:
            self._pb = elapsed
            try:
                p = self._pb_dir() / PB_FILE
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(json.dumps({"pb_seconds": elapsed}))
                self.emit.info(f"snake-race: new PB {elapsed:.3f}s")
            except Exception as e:
                self.emit.error(f"snake-race: pb save failed: {e}")

    def on_render(self, ctx: RenderContext) -> None:
        t = ctx.theme
        ctx.rect(0, 0, ctx.w, ctx.h, fill=t.bg)
        pad = 16.0
        y = pad

        ctx.text(pad, y, "Snake Race", size=17.0, color=t.fg, bold=True)
        ctx.text(pad, y + 22, f"First to {TOTAL_TARGET}", size=12.0, color=t.muted)
        y += 52.0

        # Per-snake scores
        for i in range(NUM_SNAKES):
            spawned = self._pane_ids[i] is not None
            color = t.fg if spawned else t.muted
            ctx.text(pad, y, f"S{i + 1}", size=13.0, color=t.muted)
            ctx.text(pad + 28, y, str(self._scores[i]), size=13.0, color=color)
            y += 22.0

        y += 6.0
        ctx.rect(pad, y, ctx.w - pad * 2, 1.0, fill=t.surface, radius=0.0)
        y += 10.0

        # Total
        total = sum(self._scores)
        bar_w = max(0.0, min(ctx.w - pad * 2, (ctx.w - pad * 2) * total / TOTAL_TARGET))
        ctx.rect(pad, y, ctx.w - pad * 2, 8.0, fill=t.surface, radius=4.0)
        ctx.rect(pad, y, bar_w, 8.0, fill=t.accent, radius=4.0)
        y += 16.0
        ctx.text(pad, y, f"{total} / {TOTAL_TARGET}", size=13.0, color=t.fg)
        y += 26.0

        # Timer
        if self._stop_time is not None:
            elapsed = self._stop_time - self._start_time  # type: ignore[operator]
            time_color = t.success
        elif self._start_time is not None:
            elapsed = time.monotonic() - self._start_time
            time_color = t.accent
        else:
            elapsed = 0.0
            time_color = t.muted

        if self._start_time is not None or self._stop_time is not None:
            mins = int(elapsed // 60)
            secs = elapsed % 60
            time_str = f"{mins}:{secs:06.3f}" if mins > 0 else f"{secs:.3f}s"
        else:
            time_str = "—"

        ctx.text(pad, y, time_str, size=22.0, color=time_color, bold=True)
        y += 34.0

        # Personal best
        if self._pb is not None:
            mins = int(self._pb // 60)
            secs = self._pb % 60
            pb_str = f"{mins}:{secs:06.3f}" if mins > 0 else f"{secs:.3f}s"
            ctx.text(pad, y, f"PB  {pb_str}", size=12.0, color=t.success)
        else:
            ctx.text(pad, y, "PB  —", size=12.0, color=t.muted)


if __name__ == "__main__":
    SnakeRaceApp().run()
