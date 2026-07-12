from __future__ import annotations


def emit_text_input(ctx, node_id: str, value: str = "", placeholder: str = "") -> None:
    """Emit the legacy submit-only text input node through a render context."""
    ctx.render_tree({
        "type": "text_input",
        "node_id": node_id,
        "value": value,
        "placeholder": placeholder,
    })
