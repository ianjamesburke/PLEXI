#!/usr/bin/env python3
"""Static markdown sample for visual styling regression.

Exercises the same fenced TOML / JSON / Python markdown body used as the
visual fixture for issue #293. Used by the `markdown_demo_styling`
screenshot test to verify code-block cards, syntax colour, and spacing.
"""
from plexi_sdk import App, RenderContext, PAD

SAMPLE = """\
---
touches: []
clarification_needed: []
---
## Summary

Add a `timer` capability that lets apps schedule periodic wakeups via the PGAP protocol, without polling or sleep loops in the app process.

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

Host fires back:
```json
{ "type": "Timer", "id": "check-in" }
```

Python SDK:
```python
def on_timer(self, timer_id: str, emit: Emitter):
    if timer_id == "check-in":
        emit.notify(title="5-min check-in", ...)
```

## Motivation

Enables proactive/periodic behavior without busy-looping. A productivity partner that checks in every 5 minutes, a watchdog that polls a file, a standup bot - all need this. Pairs with `background = true` (#292) and `emit.notify()` (#291).

## Related

- #292 - background = true
- #291 - emit.notify()
"""


class MarkdownDemo(App):
    async def on_init(self) -> None:
        self.emit.info("markdown-demo initialized")

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(ctx.theme.bg)
        ctx.markdown(PAD, PAD, ctx.w - PAD * 2, SAMPLE, color=ctx.theme.fg)


if __name__ == "__main__":
    MarkdownDemo().run()
