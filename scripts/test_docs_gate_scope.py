"""Offline proof for the path-specific docs-only gate decision (stint 1079).

Run: uv run --no-project python scripts/test_docs_gate_scope.py
Wired into `just test-infra-scripts`.
"""

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("docs_gate_scope.py")
spec = importlib.util.spec_from_file_location("docs_gate_scope", SCRIPT)
scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scope)


def make_root(files):
    """Build a throwaway src/ tree and return its root Path."""
    root = Path(tempfile.mkdtemp(prefix="docs-gate-scope-"))
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    return root


class EmbeddedSetTests(unittest.TestCase):
    def test_computes_shipped_and_test_asserted_markdown(self):
        root = make_root(
            {
                "src/lib.rs": (
                    'const A: &str = include_str!("../docs/SHIPPED.md");\n'
                    "#[cfg(test)]\n"
                    "mod tests {\n"
                    '    const B: &str = include_str!("../docs/ASSERTED.md");\n'
                    "}\n"
                ),
                "docs/SHIPPED.md": "x",
                "docs/ASSERTED.md": "x",
            }
        )
        embedded = scope.embedded_markdown(root)
        self.assertEqual(embedded.get("docs/SHIPPED.md"), "shipped")
        self.assertEqual(embedded.get("docs/ASSERTED.md"), "test")

    def test_ignores_non_markdown_includes(self):
        root = make_root(
            {
                "src/lib.rs": 'const A: &str = include_str!("../wit/plexi.wit");\n',
                "wit/plexi.wit": "x",
            }
        )
        self.assertNotIn("wit/plexi.wit", scope.embedded_markdown(root))

    def test_shipped_wins_when_also_included_from_a_test(self):
        root = make_root(
            {
                "src/lib.rs": (
                    'pub const A: &str = include_str!("../docs/BOTH.md");\n'
                    "#[cfg(test)]\n"
                    'mod tests { const B: &str = include_str!("../docs/BOTH.md"); }\n'
                ),
                "docs/BOTH.md": "x",
            }
        )
        self.assertEqual(scope.embedded_markdown(root).get("docs/BOTH.md"), "shipped")


class DecisionTests(unittest.TestCase):
    def assert_required(self, paths, root, needle):
        skippable, reasons = scope.decide(paths, root)
        self.assertFalse(skippable, reasons)
        joined = "\n".join(reasons)
        self.assertIn(needle, joined, joined)

    def test_plain_non_embedded_markdown_is_skippable(self):
        root = make_root({"src/lib.rs": "pub fn f() {}\n"})
        skippable, reasons = scope.decide(["notes/guide.md"], root)
        self.assertTrue(skippable, reasons)
        self.assertEqual(reasons, [])

    def test_non_markdown_code_path_is_required(self):
        root = make_root({"src/lib.rs": "pub fn f() {}\n"})
        self.assert_required(["notes/guide.md", "src/lib.rs"], root, "not markdown")

    def test_shipped_embedded_markdown_is_required(self):
        root = make_root(
            {
                "src/lib.rs": 'pub const A: &str = include_str!("../docs/SHIPPED.md");\n',
                "docs/SHIPPED.md": "x",
            }
        )
        skippable, reasons = scope.decide(["docs/SHIPPED.md"], root)
        self.assertFalse(skippable, reasons)
        self.assertIn("docs/SHIPPED.md", reasons[0])
        self.assertIn("shipped", reasons[0])

    def test_test_asserted_markdown_is_required(self):
        root = make_root(
            {
                "src/lib.rs": (
                    "#[cfg(test)]\n"
                    'mod tests { const A: &str = include_str!("../docs/ASSERTED.md"); }\n'
                ),
                "docs/ASSERTED.md": "x",
            }
        )
        skippable, reasons = scope.decide(["docs/ASSERTED.md"], root)
        self.assertFalse(skippable, reasons)
        self.assertIn("doc-sync assertion", reasons[0])

    def test_new_file_in_a_doc_sync_asserted_directory_is_required(self):
        root = make_root(
            {
                "src/lib.rs": (
                    "#[cfg(test)]\n"
                    'mod tests { const A: &str = include_str!("../docs/ASSERTED.md"); }\n'
                ),
                "docs/ASSERTED.md": "x",
            }
        )
        skippable, reasons = scope.decide(["docs/BRAND_NEW.md"], root)
        self.assertFalse(skippable, reasons)
        self.assertIn("docs/", reasons[0])

    def test_new_include_str_moves_the_path_into_the_required_set(self):
        before = make_root({"src/lib.rs": "pub fn f() {}\n"})
        self.assertTrue(scope.decide(["docs/NEW.md"], before)[0])
        after = make_root(
            {
                "src/lib.rs": 'const A: &str = include_str!("../docs/NEW.md");\n',
                "docs/NEW.md": "x",
            }
        )
        skippable, reasons = scope.decide(["docs/NEW.md"], after)
        self.assertFalse(skippable, "adding include_str! must not need a list edit")
        self.assertIn("docs/NEW.md", reasons[0])

    def test_empty_diff_is_not_skippable(self):
        root = make_root({"src/lib.rs": "pub fn f() {}\n"})
        self.assertFalse(scope.decide([], root)[0])


class RealRepoSanityTests(unittest.TestCase):
    """The real tree must classify its known-compiled markdown as required."""

    def test_real_repo_embedded_markdown_is_required(self):
        root = Path(__file__).resolve().parent.parent
        embedded = scope.embedded_markdown(root)
        for path in ("src/assistant/AGENT.md", "docs/CONFIG.md", "README.md"):
            self.assertIn(path, embedded)
            self.assertFalse(scope.decide([path], root)[0], path)


if __name__ == "__main__":
    unittest.main()
