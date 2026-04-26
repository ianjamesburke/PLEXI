#!/usr/bin/env python3
"""Agent Worker — POC for #286 (agent roster + inter-agent pipes).

A simple worker agent that listens for delegated requests on a directed
JSON pipe opened by a coordinator agent. The worker:

  1. Receives `PipeMessage { request_id, prompt }` on the pipe.
  2. Calls `iq.query` with the prompt as the user message.
  3. Sends `{ request_id, content }` back on the same pipe.
  4. Logs the exchange in its own conversation history surface so the
     user sees the round-trip happen.

Both pipe-receive and direct-user paths run their `iq.query` calls on
a background thread so the SDK stdin loop stays free.
"""
from __future__ import annotations

import threading

from plexi_sdk import Agent


class AgentWorker(Agent):
    def respond(self, text: str) -> str | None:
        # Direct user messages — handle off the stdin loop.
        del text  # captured by self.history[-1]

        def runner() -> None:
            try:
                response = self.emit.iq_query(
                    model_tier="low",
                    system=self.system_prompt or "",
                    messages=self.history,
                )
                self.append_assistant_message(response.content)
            except Exception as e:  # noqa: BLE001
                self.append_system_message(f"Worker iq_query failed: {e}")

        threading.Thread(target=runner, daemon=True).start()
        return None  # Manual-append mode.

    def on_pipe_message(self, ctx, pipe_id: str, payload):  # noqa: ANN001
        del ctx
        if not isinstance(payload, dict):
            self.append_system_message(
                f"Worker: ignoring non-dict payload on pipe '{pipe_id}'"
            )
            return
        request_id = str(payload.get("request_id", ""))
        prompt = str(payload.get("prompt", ""))
        if not prompt:
            self.append_system_message(
                f"Worker: empty prompt on pipe '{pipe_id}' — ignoring"
            )
            return
        self.append_tool_message(f"received delegated request: {prompt[:80]}")

        # Run the iq.query off the stdin loop so further pipe messages can
        # land while we're waiting on the LLM. The reply goes back on the
        # same pipe — host's directed-pipe table scopes the route.
        def runner() -> None:
            try:
                response = self.emit.iq_query(
                    model_tier="low",
                    system=self.system_prompt or "",
                    messages=[{"role": "user", "content": prompt}],
                )
                self.append_assistant_message(response.content)
                self.emit.pipe_send(pipe_id, {
                    "request_id": request_id,
                    "content": response.content,
                })
            except Exception as e:  # noqa: BLE001
                self.append_system_message(f"Worker iq_query failed: {e}")

        threading.Thread(target=runner, daemon=True).start()


if __name__ == "__main__":
    AgentWorker().run()
