#!/usr/bin/env python3
"""Agent Coordinator — POC for #286 (agent roster + inter-agent pipes).

Demonstrates the full delegation loop end-to-end:

  1. User submits a message in the coordinator's pane.
  2. Coordinator calls `list_agents()` to find a `agent-worker` peer.
  3. Coordinator opens a directed JSON pipe to the worker's pane.
  4. Coordinator sends `{ request_id, prompt }` over the pipe.
  5. Worker receives it, calls `iq.query`, replies on the same pipe.
  6. Coordinator's `on_pipe_message` handler unblocks the waiting queue;
     the response surfaces in the coordinator's conversation history.

Both panes log the exchange in their own surfaces — open them side by
side to watch the round-trip happen.

The delegation runs on a background thread so the SDK's stdin event
loop stays free to dispatch the worker's `pipe_message` reply back into
`on_pipe_message`. `respond` returns `None` (manual append mode) and
the worker thread does the final `append_assistant_message` itself.
"""
from __future__ import annotations

import queue
import threading
import uuid

from plexi_sdk import Agent


WORKER_APP_ID = "agent-worker"
PIPE_REPLY_TIMEOUT_S = 30.0


class AgentCoordinator(Agent):
    def __init__(self) -> None:
        super().__init__()
        # Open pipe handles keyed on pipe_id so we can re-use one channel
        # across multiple delegations to the same worker.
        self._open_pipes: dict[str, object] = {}
        # Pending replies keyed on (pipe_id, request_id) — the worker
        # echoes our request_id back so we correlate concurrent
        # delegations on the same pipe.
        self._pending_replies: dict[tuple[str, str], queue.Queue] = {}

    # ── Delegation core ─────────────────────────────────────────────────────
    def respond(self, text: str) -> str | None:
        # Run the full delegation off the stdin loop so `on_pipe_message`
        # callbacks can fire while we wait for the worker's reply.
        threading.Thread(
            target=self._delegate, args=(text,), daemon=True
        ).start()
        # Manual-append mode — the thread will append the assistant row.
        return None

    def _delegate(self, text: str) -> None:
        try:
            roster = self.list_agents()
            worker = next((a for a in roster if a.app_id == WORKER_APP_ID), None)
            if worker is None:
                self.append_tool_message(
                    f"no '{WORKER_APP_ID}' in roster — open one in another pane and try again. "
                    f"roster={[a.app_id for a in roster]}"
                )
                self.append_assistant_message(
                    f"I couldn't find an `{WORKER_APP_ID}` agent in the workspace. "
                    f"Open one in another pane and try again."
                )
                return

            self.append_tool_message(
                f"found {worker.app_id} on pane {worker.pane_id} — opening directed pipe"
            )

            pipe_id = f"coord-{worker.pane_id}"
            if pipe_id not in self._open_pipes:
                self._open_pipes[pipe_id] = self.open_pipe_to(
                    worker.pane_id, pipe_id=pipe_id
                )
                self.append_tool_message(f"opened pipe '{pipe_id}'")

            request_id = str(uuid.uuid4())
            reply_q: queue.Queue = queue.Queue()
            self._pending_replies[(pipe_id, request_id)] = reply_q

            self._open_pipes[pipe_id].send(  # type: ignore[attr-defined]
                {"request_id": request_id, "prompt": text}
            )
            self.append_tool_message(f"delegated to {worker.app_id}: {text[:80]}")

            try:
                content = reply_q.get(timeout=PIPE_REPLY_TIMEOUT_S)
            except queue.Empty:
                self._pending_replies.pop((pipe_id, request_id), None)
                self.append_assistant_message(
                    f"Worker timed out after {PIPE_REPLY_TIMEOUT_S:.0f}s."
                )
                return

            self.append_assistant_message(f"Worker replied: {content}")
        except Exception as e:  # noqa: BLE001
            self.append_system_message(f"Coordinator: delegation failed: {e}")

    # ── Wire-up ─────────────────────────────────────────────────────────────
    def on_pipe_message(self, ctx, pipe_id: str, payload):  # noqa: ANN001
        del ctx
        if not isinstance(payload, dict):
            self.append_system_message(
                f"Coordinator: ignoring non-dict payload on pipe '{pipe_id}'"
            )
            return
        request_id = str(payload.get("request_id", ""))
        content = str(payload.get("content", ""))
        q = self._pending_replies.pop((pipe_id, request_id), None)
        if q is not None:
            q.put(content)
        else:
            self.append_system_message(
                f"Coordinator: stray pipe message on '{pipe_id}' "
                f"(request_id={request_id!r}, no waiter)"
            )


if __name__ == "__main__":
    AgentCoordinator().run()
