#!/usr/bin/env python3
"""Notification Tester — exercise every shape of the notification system.

Kinds (how the notification renders):
  m — Message (fire-and-forget; default priority NORMAL)
  c — Choice (blocks for a pick; default priority HIGH)
  i — Input (blocks for text; default priority HIGH)
  r — Required message (no Esc; default priority CRITICAL)

Rich attachments (#74):
  p — Inline-image message (a tiny PNG generated at click time)
  q — Inline-image choice (image + 3 structured choices)
  t — Oversized inline image (post-decode > 50KB → placeholder badge)

Priority tiers (how urgently it sorts in the queue):
  1 — LOW        (0)    background info
  2 — NORMAL     (50)   standard confirmations
  3 — HIGH       (100)  needs attention soon
  4 — CRITICAL   (200)  interrupt-level

Queue demos:
  b — Burst: three messages at mixed priorities (CRITICAL / LOW / NORMAL).
      Good for verifying the live queue grows without displacing the pinned
      front-most and that dismissal picks the next-highest dynamically.
  s — Stack: fire ten LOW notifications in quick succession. Watch the
      count go live while the pinned top stays stable.

Scope (context vs global) is NOT a runtime choice. It's declared per-app
in manifest.toml::default_notification_scope. Flip this app's manifest to
"global" to test cross-context visibility.
"""
from __future__ import annotations

import threading
import time

from plexi_sdk import (
    App, RenderContext,
    PRIORITY_LOW, PRIORITY_NORMAL, PRIORITY_HIGH, PRIORITY_CRITICAL,
)
from plexi_sdk.ui import (
    Column, Card, AppBar, Section,
    KeyRow, ScrollLog, Spacer, Footer,
)


def _priority_label(p: int) -> str:
    """Map an int priority back to its tier name for log readability."""
    if p >= PRIORITY_CRITICAL: return f"CRITICAL({p})"
    if p >= PRIORITY_HIGH:     return f"HIGH({p})"
    if p >= PRIORITY_NORMAL:   return f"NORMAL({p})"
    return f"LOW({p})"


class NotificationTesterApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._log: list[str] = []
        self._burst_counter = 0
        self._stack_counter = 0
        ctx.status_summary("Notification Tester — ready")
        self.emit.info("Notification Tester started")

    def _append(self, line: str) -> None:
        ts = time.strftime("%H:%M:%S")
        self._log.append(f"{ts}  {line}")
        self.emit.schedule_render(after_ms=50)

    # ── Kinds ─────────────────────────────────────────────────────────────
    def _fire_message(self, priority: int = PRIORITY_NORMAL) -> None:
        self.emit.notify(
            title=f"Plain message · {_priority_label(priority)}",
            body="Message-kind notification. Enter or Space to acknowledge.",
            level="info",
            priority=priority,
        )
        self._append(f"sent: message @ {_priority_label(priority)}")

    def _fire_required_message(self) -> None:
        def runner():
            result = self.emit.run_sync(self.emit.notify_and_wait(
                title="Required message",
                body="You cannot dismiss this with Esc. Press Enter to acknowledge.",
                level="warn",
                priority=PRIORITY_CRITICAL,
            ))
            self._append(f"required message → {result}")
        threading.Thread(target=runner, daemon=True).start()
        self._append("sent: required @ CRITICAL(200)")

    def _fire_choice(self) -> None:
        def runner():
            options = [
                {"label": "Yes",   "value": "yes",   "shortcut": "y"},
                {"label": "No",    "value": "no",    "shortcut": "n"},
                {"label": "Maybe", "value": "maybe", "shortcut": "m"},
            ]
            result = self.emit.run_sync(self.emit.notify_choice(
                title="How's your focus right now?",
                body="Pick one. y/n/m or ↑↓ + Enter, or 1-3 to direct-select.",
                options=options,
                level="info",
                priority=PRIORITY_HIGH,
            ))
            self._append(f"choice → {result}")
        threading.Thread(target=runner, daemon=True).start()
        self._append("sent: choice @ HIGH(100)")

    def _fire_input(self) -> None:
        def runner():
            result = self.emit.run_sync(self.emit.notify_input(
                title="What are you working on?",
                prompt="Type a short description…",
                body="Enter for newline, Cmd+Enter to submit, Esc to cancel.",
                level="info",
                priority=PRIORITY_HIGH,
            ))
            self._append(
                "input → cancelled" if result == "__cancel__"
                else f"input → {result!r}"
            )
        threading.Thread(target=runner, daemon=True).start()
        self._append("sent: input @ HIGH(100)")

    # ── Rich attachments (#74) ────────────────────────────────────────────
    @staticmethod
    def _gradient_png(size: int = 64) -> bytes:
        """Build a tiny gradient PNG without a third-party dep.

        Uses Python's stdlib `zlib` + manual chunk assembly. Output is a
        well-formed PNG well under the 50 KB cap (a 64x64 grayscale
        gradient is ~700 bytes after zlib)."""
        import struct
        import zlib

        # Filter byte (0 = none) + one row of width bytes per scanline.
        raw = bytearray()
        for y in range(size):
            raw.append(0)
            for x in range(size):
                # Diagonal gradient with a tiny color shift across rows so
                # the result reads as an obvious image, not a flat field.
                v = (x + y) * 255 // (2 * (size - 1))
                # RGB triplet — same v in all channels for a gray ramp,
                # plus a green tint to make it visually unmistakable.
                raw.extend([v, min(255, v + 30), v])
        compressed = zlib.compress(bytes(raw))

        def chunk(tag: bytes, data: bytes) -> bytes:
            return (
                struct.pack(">I", len(data))
                + tag
                + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
            )

        ihdr = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)  # 8-bit RGB
        return (
            b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", compressed)
            + chunk(b"IEND", b"")
        )

    def _fire_inline_image(self) -> None:
        """Post a message-kind notification with a small inline PNG."""
        png = self._gradient_png(size=64)
        def runner():
            # Use the SDK convenience: handles base64 + 50KB cap client-side.
            self.emit.run_sync(self.emit.notify_with_image(
                title="Inline image attached",
                body=f"64x64 gradient PNG, {len(png)} bytes pre-encode.",
                image_bytes=png,
                mime="image/png",
                priority=PRIORITY_NORMAL,
            ))
        threading.Thread(target=runner, daemon=True).start()
        self._append(f"sent: inline-image message ({len(png)}-byte PNG)")

    def _fire_inline_image_choice(self) -> None:
        """Choice notification with image + 3 structured choices.

        This is the issue #74 hero flow: the agent renders both UI options
        and asks the user to pick. We use a single image here for brevity;
        in production the agent would render two and stack them."""
        def runner():
            png = self._gradient_png(size=96)
            options = [
                {"label": "Sidebar layout",    "value": "sidebar",   "shortcut": "1"},
                {"label": "Full-width layout", "value": "fullwidth", "shortcut": "2"},
                {"label": "Neither — try again", "value": "retry",   "shortcut": "n"},
            ]
            result = self.emit.run_sync(self.emit.notify_with_image(
                title="Which UI approach?",
                body="Rendered the gradient option above. Pick one to continue.",
                image_bytes=png,
                mime="image/png",
                priority=PRIORITY_HIGH,
                choices=options,
            ))
            self._append(f"image-choice → {result}")
        threading.Thread(target=runner, daemon=True).start()
        self._append("sent: image-choice @ HIGH(100)")

    def _fire_oversized_image(self) -> None:
        """100KB image — host should render the placeholder badge.

        We send via the lower-level Emitter.notify (NOT notify_with_image)
        because notify_with_image enforces the 50KB cap client-side; the
        purpose of THIS button is to verify the host-side cap fires."""
        import base64
        big = bytes(100 * 1024)  # 100 KB of zeros — exceeds 50 KB cap
        self.emit.notify(
            title="Oversized inline image",
            body="Decoded payload exceeds 50 KB cap — should render placeholder.",
            level="warn",
            priority=PRIORITY_NORMAL,
            image_inline={
                "mime": "image/png",
                "base64": base64.standard_b64encode(big).decode("ascii"),
            },
        )
        self._append(f"sent: oversized image ({len(big)} bytes)")

    # ── Queue demos ───────────────────────────────────────────────────────
    def _fire_burst(self) -> None:
        """3 notifications at mixed priorities in one sync pass.

        If the modal is already open, the CRITICAL one should go to the
        top of the sort order without displacing the currently-pinned
        notification. The user sees the count tick up; on dismiss the
        CRITICAL one becomes front-most."""
        self._burst_counter += 1
        tag = self._burst_counter
        priorities = [PRIORITY_CRITICAL, PRIORITY_LOW, PRIORITY_NORMAL]
        for i, pri in enumerate(priorities, 1):
            self.emit.notify(
                title=f"Burst {tag} · {i}/3 · {_priority_label(pri)}",
                body="Cmd+] / Cmd+[ to preview. Dismiss to watch the next one "
                     "come from priority order, not list order.",
                level="info",
                priority=pri,
            )
        self._append(f"burst {tag}: CRITICAL / LOW / NORMAL")

    def _fire_stack(self) -> None:
        """10 LOW-priority notifications rapidly. Watch the count go live
        while your pinned view stays stable (never yanks to something new)."""
        self._stack_counter += 1
        tag = self._stack_counter
        for i in range(1, 11):
            self.emit.notify(
                title=f"Stack {tag} · {i}/10 · LOW",
                body="All LOW priority, arrival-ordered. Count grows live.",
                level="info",
                priority=PRIORITY_LOW,
            )
        self._append(f"stack {tag}: 10 @ LOW")

    # ── Input routing ─────────────────────────────────────────────────────
    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        k = key.lower()
        # Kinds
        if   k == "m": self._fire_message(PRIORITY_NORMAL)
        elif k == "c": self._fire_choice()
        elif k == "i": self._fire_input()
        elif k == "r": self._fire_required_message()
        # Priority tiers — fire a plain message at the tier so users can
        # verify sort order + front-pinning behaviour.
        elif k == "1": self._fire_message(PRIORITY_LOW)
        elif k == "2": self._fire_message(PRIORITY_NORMAL)
        elif k == "3": self._fire_message(PRIORITY_HIGH)
        elif k == "4": self._fire_message(PRIORITY_CRITICAL)
        # Rich attachments (#74)
        elif k == "p": self._fire_inline_image()
        elif k == "q": self._fire_inline_image_choice()
        elif k == "t": self._fire_oversized_image()
        # Queue demos
        elif k == "b": self._fire_burst()
        elif k == "s": self._fire_stack()

    def on_render(self, ctx: RenderContext) -> None:
        ctx.render(Column([
            AppBar(title="Notification Tester"),
            Section("Kinds"),
            Card([
                KeyRow("m", "Message (NORMAL)"),
                KeyRow("c", "Choice (HIGH, blocks for pick)"),
                KeyRow("i", "Input (HIGH, blocks for text)"),
                KeyRow("r", "Required message (CRITICAL, no Esc)"),
            ]),
            Section("Priority tiers"),
            Card([
                KeyRow("1", "LOW (0) — background info"),
                KeyRow("2", "NORMAL (50) — standard confirmation"),
                KeyRow("3", "HIGH (100) — attention soon"),
                KeyRow("4", "CRITICAL (200) — interrupt-level"),
            ]),
            Section("Rich attachments (#74)"),
            Card([
                KeyRow("p", "Inline PNG message — 64x64 gradient"),
                KeyRow("q", "Inline PNG choice — 3 structured choices"),
                KeyRow("t", "Oversized inline image — placeholder fires at 50 KB"),
            ]),
            Section("Queue demos"),
            Card([
                KeyRow("b", "Burst — 3 mixed-priority at once"),
                KeyRow("s", "Stack — 10 LOW in quick succession"),
            ]),
            Section("Event log"),
            ScrollLog(lines=self._log, empty_text="no events yet"),
            Spacer(grow=True),
            Footer(
                "Modal auto-opens on send · ⌘⇧A re-opens · ⌘] / ⌘[ cycle the queue"
            ),
        ]))


NotificationTesterApp().run()
