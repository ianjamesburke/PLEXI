"""TextEdit demo: single-line and multiline text input via the component tree.

Shows a single-line input (Enter submits) and a multiline editor (Cmd+Enter
submits). Submitted values are logged below each field.
"""
from plexi_sdk import App, State, TextEdit


class TextEditDemo(App):
    single_value = State("")
    multi_value = State("")
    single_log = State("")
    multi_log = State("")

    def on_component_event(self, ctx, node_id, event_type, payload):
        value = (payload or {}).get("value", "")
        if event_type == "change":
            if node_id == "single":
                self.single_value = value
            elif node_id == "multi":
                self.multi_value = value
        elif event_type == "submit":
            if node_id == "single":
                self.single_log = f"Submitted: {value}"
                ctx.info(f"single submit: {value}")
            elif node_id == "multi":
                self.multi_log = f"Submitted: {value}"
                ctx.info(f"multi submit: {value}")

    def on_render(self, ctx):
        ctx.render_tree({
            "type": "stack",
            "direction": "vertical",
            "gap": 12.0,
            "padding": {"top": 12.0, "right": 16.0, "bottom": 16.0, "left": 16.0},
            "children": [
                {"type": "app_bar", "title": "TextEdit Demo", "subtitle": ""},
                {"type": "section", "title": "Single-line"},
                TextEdit("single", placeholder="Type and press Enter...", value=self.single_value).to_node(),
                {"type": "label", "text": self.single_log or "No submission yet", "size": 0.0, "color": "", "tone": "hint", "bold": False, "monospace": False, "max_lines": 1},
                {"type": "section", "title": "Multiline (Cmd+Enter to submit)"},
                TextEdit("multi", placeholder="Type here...\nCmd+Enter to submit", value=self.multi_value, multiline=True).to_node(),
                {"type": "label", "text": self.multi_log or "No submission yet", "size": 0.0, "color": "", "tone": "hint", "bold": False, "monospace": False, "max_lines": 3},
            ],
        })


TextEditDemo().run()
