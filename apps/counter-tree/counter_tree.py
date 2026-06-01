"""Counter app demonstrating the component tree API (epic #1897 B3)."""
from plexi_sdk import App, State


class CounterTree(App):
    count = State(0)

    def on_key(self, ctx, key, mods):
        if key in ("up", "="):
            self.count += 1
        elif key in ("down", "-"):
            self.count -= 1

    def on_render(self, ctx):
        ctx.render_tree({
            "type": "stack",
            "direction": "vertical",
            "children": [
                {
                    "type": "text",
                    "text": f"Count: {self.count}",
                    "size": 18.0,
                    "color": "#cdd6f4",
                },
                {
                    "type": "text",
                    "text": "up/down or +/- to change",
                    "size": 12.0,
                    "color": "#6c7086",
                },
            ],
        })


CounterTree().run()
