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
