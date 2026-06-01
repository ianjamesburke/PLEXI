"""Counter app demonstrating the component tree API (epic #1897 B3).

Button/Input nodes are included for visual Style O verification only.
ComponentEvent routing to Python apps is not yet in the SDK — button
clicks and input changes have no effect until that is wired up.
"""
from plexi_sdk import App, State


class CounterTree(App):
    count = State(0)
    label = State("")

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
                    "text": "up/down or +/- to change",
                    "size": 12.0,
                    "color": "#6c7086",
                },
            ],
        })


CounterTree().run()
