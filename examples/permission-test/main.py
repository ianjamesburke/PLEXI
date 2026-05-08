#!/usr/bin/env python3
"""permission-test — POC app for PermissionStore testing (issue #426).

Requests net.http at runtime (not declared in manifest) so the grant/deny
modal fires immediately on launch. Displays the decision result so the tester
can confirm it was received. On relaunch with a Red entry in permissions.toml
the host auto-denies without showing the modal.
"""
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext

GRAY = "#6c7086"
GREEN = "#a6e3a1"
RED = "#f38ba8"
WHITE = "#cdd6f4"

class PermissionTestApp(App):
    def __init__(self):
        self.result = None      # None = pending, True = granted, False = denied
        self.requested = False

    def on_init(self, ctx, emit):
        emit.info("permission-test: requesting net.http capability")
        granted = emit.capability_request("net.http")
        self.result = granted
        self.requested = True
        if granted:
            emit.info("permission-test: net.http GRANTED")
        else:
            emit.info("permission-test: net.http DENIED (or permanently blocked)")

    def render(self, ctx: RenderContext, emit):
        ctx.clear()
        w, h = ctx.width, ctx.height
        cx = w / 2
        cy = h / 2

        ctx.text(cx, cy - 30, "Permission Test", color=WHITE, size=18,
                 align="center", bold=True)
        ctx.text(cx, cy - 10, "Requested: net.http", color=GRAY, size=13,
                 align="center")

        if self.result is True:
            ctx.text(cx, cy + 15, "GRANTED", color=GREEN, size=16,
                     align="center", bold=True)
            ctx.text(cx, cy + 35,
                     "Check ~/.plexi-pr-860/permissions.toml for a 'green' entry.",
                     color=GRAY, size=11, align="center")
        elif self.result is False:
            ctx.text(cx, cy + 15, "DENIED / BLOCKED", color=RED, size=16,
                     align="center", bold=True)
            ctx.text(cx, cy + 35,
                     "Check ~/.plexi-pr-860/permissions.toml for a 'red' entry.",
                     color=GRAY, size=11, align="center")
        else:
            ctx.text(cx, cy + 15, "Waiting for decision...", color=GRAY,
                     size=13, align="center")

if __name__ == "__main__":
    PermissionTestApp().run()
