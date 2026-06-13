#!/usr/bin/env python3
"""Static markdown sample for visual styling regression.

Exercises every themed markdown surface in one frame: headings, inline
`code` spans, fenced code blocks (toml / json / python), a blockquote, a
link, and a bullet list. Used by the `markdown_demo_styling` screenshot
test to verify code-block cards, inline-code tint, and spacing.
"""
from plexi_sdk import App, RenderContext, PAD

SAMPLE = """\
# Markdown styling

Add a `timer` capability that lets apps schedule periodic wakeups without
polling or sleep loops. Configure it with the `interval_secs` field.

## Manifest

```toml
capabilities = ["timer"]
```

## Protocol

App sends:

```json
{ "type": "SetTimer", "id": "check-in", "interval_secs": 300 }
{ "type": "ClearTimer", "id": "check-in" }
```

```python
def on_timer(self, timer_id: str, emit: Emitter):
    if timer_id == "check-in":
        emit.notify(title="5-min check-in")
```

> Timers fire on the host clock, not the app process, so a sleeping app
> still wakes on schedule.

See the [timer docs](https://plexiapp.com/docs/timer) for details.

- `SetTimer` registers a repeating wakeup
- `ClearTimer` cancels it
- The host emits `Timer` events back to the app
"""


class MarkdownDemo(App):
    async def on_init(self) -> None:
        self.emit.info("markdown-demo initialized")

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(ctx.theme.bg)
        ctx.markdown(PAD, PAD, ctx.w - PAD * 2, SAMPLE, color=ctx.theme.fg)


if __name__ == "__main__":
    MarkdownDemo().run()
