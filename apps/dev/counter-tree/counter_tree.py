"""Counter app demonstrating L1 ComponentEvent routing.

Button clicks and input changes fire on_component_event with matching node_id.
"""
from plexi_sdk import App, State


class CounterTree(App):
    count = State(0)
    label = State("")

    def on_component_event(self, ctx, node_id, event_type, payload):
        if event_type == "click":
            if node_id == "increment":
                self.count += 1
                ctx.info(f"increment -> {self.count}")
            elif node_id == "decrement":
                self.count -= 1
                ctx.info(f"decrement -> {self.count}")
        elif event_type == "change" and node_id == "label_input":
            self.label = (payload or {}).get("value", "")

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
                        },
                        {
                            "type": "badge",
                            "label": f"{self.count}",
                        },
                        {
                            "type": "button",
                            "node_id": "increment",
                            "label": "+",
                        },
                    ],
                },
                {
                    "type": "input",
                    "node_id": "label_input",
                    "value": self.label,
                    "placeholder": "Enter a label…",
                },
                {
                    "type": "text",
                    "text": self.label or "up/down or click buttons",
                    "size": 12.0,
                    "color": "#6c7086",
                },
            ],
        })


CounterTree().run()
