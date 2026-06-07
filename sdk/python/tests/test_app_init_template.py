import ast
import pathlib

TEMPLATE = pathlib.Path(__file__).parent.parent / "plexi_sdk/templates/app_init.py"


def test_no_divider_false():
    """FooterKeys must use the default divider (True). divider=False hides the footer."""
    assert "divider=False" not in TEMPLATE.read_text()


def test_single_grow_spacer():
    """Exactly one Spacer(grow=True), placed after the Label to push the footer down."""
    src = TEMPLATE.read_text()
    tree = ast.parse(src)
    spacer_lines, label_lines = [], []
    for node in ast.walk(tree):
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name):
            if node.func.id == "Spacer":
                for kw in node.keywords:
                    if kw.arg == "grow" and isinstance(kw.value, ast.Constant) and kw.value.value:
                        spacer_lines.append(node.lineno)
            if node.func.id == "Label":
                label_lines.append(node.lineno)
    assert len(spacer_lines) == 1, f"Expected 1 grow Spacer, got {len(spacer_lines)}"
    assert label_lines and spacer_lines[0] > label_lines[0], \
        "Grow Spacer must appear after Label (pushes footer to bottom)"


def test_template_uses_view():
    src = TEMPLATE.read_text()
    assert "def view(self)" in src, "Template must use view() — not on_render()"
    tree = ast.parse(src)
    for node in ast.walk(tree):
        if isinstance(node, ast.FunctionDef) and node.name == "on_render":
            raise AssertionError("Template must not define on_render (that's the canvas path)")


def test_template_uses_self_state():
    src = TEMPLATE.read_text()
    assert "self.state.get" in src, "Template must use self.state.get() not ctx.load_state()"
    assert "ctx.load_state" not in src
    assert "ctx.save_state" not in src
    assert "self.state.save" in src


def test_template_on_init_no_ctx():
    src = TEMPLATE.read_text()
    assert "def on_init(self)" in src
    assert "def on_init(self, ctx" not in src


def test_template_on_key_no_ctx():
    src = TEMPLATE.read_text()
    assert "def on_key(self, key" in src or "async def on_key(self, key" in src
    assert "def on_key(self, ctx" not in src


def test_template_no_render_context_import():
    src = TEMPLATE.read_text()
    assert "RenderContext" not in src, "Template should not import RenderContext (not needed with v2 API)"
