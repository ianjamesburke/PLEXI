#!/usr/bin/env python3
"""Agent Tester — POC for #338 (agent-as-app, part 2 of #285).

Demonstrates the full agent-as-app loop end-to-end without writing
turn-loop boilerplate. The Plexi host renders the conversation UI;
this app receives `PlexiEvent::AgentInit` (system prompt) once at
startup and `PlexiEvent::UserMessage` per submit, then replies via
the `iq.query` broker.

Build: ~10 lines of substance. The `Agent` SDK base class handles
history mirroring, append helpers, and event wiring.
"""
from __future__ import annotations

from plexi_sdk import Agent


class AgentTester(Agent):
    async def respond(self, text: str) -> str:
        # `self.history` is auto-populated by `append_user_message` (called
        # by the SDK before this override runs). `system_prompt` is set by
        # the host's AgentInit forwarded from the manifest.
        del text  # captured by `self.history[-1]`
        response = await self.emit.iq_query(
            model_tier="medium",
            system=self.system_prompt or "",
            messages=self.history,
        )
        return response.content


if __name__ == "__main__":
    AgentTester().run()
