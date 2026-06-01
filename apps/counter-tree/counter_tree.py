"""Counter app demonstrating the component tree API (epic #1897 B3).

Button clicks route through on_component_event (issue #1904).
Keyboard shortcuts (up/down/+/-) also work.
"""
from plexi_sdk import App, State


class CounterTree(App):
    count = State(0)
    label = State("")

    def on_component_event(self, ctx, node_id, event_type, payload):
        if event_type == "click":
            if node_id == "increment":
                self.count += 1
            elif node_id == "decrement":
                self.count -= 1
        ctx.info(f"component_event node_id={node_id!r} event_type={event_type!r}")

    def on_key(self, ctx, key, mods):
        if key in ("up", "="):
            self.count += 1
        elif key in ("down", "-"):
            self.count -= 1

    def on_render(self, ctx):
        ctx.render_tree({
            "type": "stack",
            "direction": "vertical",
            "gap": 8.0,
            "children": [
                {
                    "type": "text",
                    "text": f"Count: {self.count}",
                    "size": 18.0,
                    "color": "#cdd6f4",
                },
                {
                    "type": "stack",
                    "direction": "horizontal",
                    "gap": 8.0,
                    "children": [
                        {
                            "type": "button",
                            "node_id": "decrement",
                            "label": "−",
                            "_l0": {"type": "text", "text": "−"},
                        },
                        {
                            "type": "badge",
                            "label": f"{self.count}",
                            "_l0": {"type": "text", "text": f"{self.count}"},
                        },
                        {
                            "type": "button",
                            "node_id": "increment",
                            "label": "+",
                            "_l0": {"type": "text", "text": "+"},
                        },
                    ],
                },
                {
                    "type": "input",
                    "node_id": "label_input",
                    "value": self.label,
                    "placeholder": "Enter a label…",
                    "_l0": {"type": "text", "text": self.label},
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
