#!/usr/bin/env python3
"""
Notification Tester — demonstrates all notification methods including the
non-blocking async variants added in #310.

Blocking: notify_and_wait, notify_choice, notify_input (require async def or
  worker thread — buttons here schedule them as background tasks).
Non-blocking: notify_async, notify_and_wait_async, notify_choice_async,
  notify_input_async (return immediately; callback fires on event thread).
"""
from plexi_sdk import App, RenderContext, PRIORITY_NORMAL, PRIORITY_HIGH
from plexi_sdk.ui import (
    AppBar,
    ButtonRow,
    Card,
    Column,
    FooterKeys,
    Label,
    Section,
    Spacer,
)


class NotificationTester(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self._log: list[str] = []
        self._btn_async = ButtonRow("async_msg", "notify_async (callback)")
        self._btn_choice_async = ButtonRow("async_choice", "notify_choice_async (callback)")
        self._btn_input_async = ButtonRow("async_input", "notify_input_async (callback)")
        self._btn_wait_async = ButtonRow("async_wait", "notify_and_wait_async (callback)")
        self._btn_blocking = ButtonRow("blocking", "notify_and_wait (blocking)")
        ctx.status_summary("Ready")
        self.emit.info("notification-tester: initialized")

    def _log_result(self, label: str, result: str) -> None:
        entry = f"{label}: {result!r}"
        self._log = [entry] + self._log[:4]
        self.emit.info(f"notification-tester: {entry}")

    def on_render(self, ctx: RenderContext) -> None:
        if self._btn_async.clicked:
            ctx.notify_async(
                title="Async message",
                body="This returns immediately. Callback will log the result.",
                priority=PRIORITY_NORMAL,
                on_response=lambda r: self._log_result("notify_async", r),
            )

        if self._btn_choice_async.clicked:
            ctx.notify_choice_async(
                title="Async choice",
                options=[
                    {"label": "Yes", "value": "yes", "shortcut": "y"},
                    {"label": "No", "value": "no", "shortcut": "n"},
                ],
                priority=PRIORITY_HIGH,
                body="Pick without blocking on_render.",
                on_response=lambda r: self._log_result("notify_choice_async", r),
            )

        if self._btn_input_async.clicked:
            ctx.notify_input_async(
                title="Async input",
                prompt="Type something:",
                priority=PRIORITY_NORMAL,
                on_response=lambda r: self._log_result("notify_input_async", r),
            )

        if self._btn_wait_async.clicked:
            ctx.notify_and_wait_async(
                title="Async wait",
                body="Acknowledge or cancel — callback fires either way.",
                priority=PRIORITY_NORMAL,
                on_response=lambda r: self._log_result("notify_and_wait_async", r),
            )

        if self._btn_blocking.clicked:
            self.emit.schedule_task(self._blocking_demo())

        log_labels = self._log if self._log else ["(no results yet)"]

        ctx.render(Column([
            AppBar(title="Notification Tester", subtitle="#310 — async notify variants"),
            Section("NON-BLOCKING (callback-based)"),
            Card([
                self._btn_async,
                self._btn_choice_async,
                self._btn_input_async,
                self._btn_wait_async,
            ]),
            Section("BLOCKING (for comparison)"),
            Card([self._btn_blocking]),
            Section("RESULTS"),
            Card([Label(line, tone="caption") for line in log_labels]),
            Spacer(grow=True),
            FooterKeys([("q", "quit")]),
        ]))

    async def _blocking_demo(self) -> None:
        result = await self.emit.notify_and_wait(
            title="Blocking wait",
            body="This awaits on a background task — on_render stays free.",
            priority=PRIORITY_NORMAL,
        )
        self._log_result("notify_and_wait (blocking)", result)


NotificationTester().run()
