"""Offline proof for the path-specific docs-only gate decision (stints 1079, 1102).

Run: uv run --no-project python scripts/test_docs_gate_scope.py
Wired into `just test-infra-scripts`.
"""

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("docs_gate_scope.py")
spec = importlib.util.spec_from_file_location("docs_gate_scope", SCRIPT)
scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scope)


def git(root, *args):
    return subprocess.run(("git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t",
                           *args), check=True, capture_output=True, text=True).stdout.strip()


def make_root(files):
    """Build a throwaway git work tree (files left untracked) and return its root."""
    root = Path(tempfile.mkdtemp(prefix="docs-gate-scope-"))
    git(root, "init", "-q")
    write(root, files)
    return root


def write(root, files):
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


def commit(root):
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", "x")
    return git(root, "rev-parse", "HEAD")


class BindingSetTests(unittest.TestCase):
    def test_computes_shipped_and_test_asserted_markdown(self):
        root = make_root({
            "src/lib.rs": (
                'const A: &str = include_str!("../docs/SHIPPED.md");\n'
                "#[cfg(test)]\n"
                "mod tests {\n"
                '    const B: &str = include_str!("../docs/ASSERTED.md");\n'
                "}\n"
            ),
            "docs/SHIPPED.md": "x",
            "docs/ASSERTED.md": "x",
        })
        bindings = scope.binding_paths(root)
        self.assertEqual(bindings["docs/SHIPPED.md"]["mode"], "shipped")
        self.assertEqual(bindings["docs/ASSERTED.md"]["mode"], "test")

    def test_ignores_non_markdown_includes(self):
        root = make_root({"src/lib.rs": 'const A: &str = include_str!("../wit/plexi.wit");\n',
                          "wit/plexi.wit": "x"})
        self.assertNotIn("wit/plexi.wit", scope.binding_paths(root))

    def test_shipped_wins_when_also_included_from_a_test(self):
        root = make_root({
            "src/lib.rs": (
                'pub const A: &str = include_str!("../docs/BOTH.md");\n'
                "#[cfg(test)]\n"
                'mod tests { const B: &str = include_str!("../docs/BOTH.md"); }\n'
            ),
            "docs/BOTH.md": "x",
        })
        self.assertEqual(scope.binding_paths(root)["docs/BOTH.md"]["mode"], "shipped")

    def test_scans_every_workspace_crate_not_only_src(self):
        root = make_root({
            "src/lib.rs": "pub fn f() {}\n",
            "tools/gen/Cargo.toml": "[package]\n",
            "tools/gen/src/main.rs": 'const A: &str = include_str!("../../../docs/GEN.md");\n',
            "docs/GEN.md": "x",
        })
        self.assertIn("docs/GEN.md", scope.binding_paths(root))


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
        root = make_root({"src/lib.rs": 'pub const A: &str = include_str!("../docs/SHIPPED.md");\n',
                          "docs/SHIPPED.md": "x"})
        self.assert_required(["docs/SHIPPED.md"], root, "docs/SHIPPED.md is bound by include_str")

    def test_test_asserted_markdown_is_required(self):
        root = make_root({
            "src/lib.rs": "#[cfg(test)]\nmod tests { const A: &str = include_str!(\"../docs/ASSERTED.md\"); }\n",
            "docs/ASSERTED.md": "x",
        })
        self.assert_required(["docs/ASSERTED.md"], root, "in test code")

    def test_new_file_in_a_doc_sync_asserted_directory_is_required(self):
        root = make_root({
            "src/lib.rs": "#[cfg(test)]\nmod tests { const A: &str = include_str!(\"../docs/ASSERTED.md\"); }\n",
            "docs/ASSERTED.md": "x",
        })
        self.assert_required(["docs/BRAND_NEW.md"], root, "doc-sync assertion")

    def test_new_include_str_moves_the_path_into_the_required_set(self):
        before = make_root({"src/lib.rs": "pub fn f() {}\n"})
        self.assertTrue(scope.decide(["docs/NEW.md"], before)[0])
        after = make_root({"src/lib.rs": 'const A: &str = include_str!("../docs/NEW.md");\n',
                           "docs/NEW.md": "x"})
        self.assert_required(["docs/NEW.md"], after, "docs/NEW.md")

    def test_include_dir_binds_every_markdown_file_it_embeds(self):
        root = make_root({
            "Cargo.toml": "[package]\n",
            "src/lib.rs": 'static APPS: Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/apps");\n',
            "apps/notes/README.md": "x",
            "apps/notes/main.py": "x",
        })
        self.assert_required(["apps/notes/README.md"], root, "apps/ is bound by include_dir")
        self.assert_required(["apps/brand-new/AGENTS.md"], root, "include_dir")

    def test_include_dir_resolves_against_its_own_crate(self):
        root = make_root({
            "crates/sdk/Cargo.toml": "[package]\n",
            "crates/sdk/src/lib.rs": 'static D: Dir = include_dir!("$CARGO_MANIFEST_DIR/pkg");\n',
            "crates/sdk/pkg/GUIDE.md": "x",
            "pkg/OTHER.md": "x",
        })
        self.assert_required(["crates/sdk/pkg/GUIDE.md"], root, "include_dir")
        self.assertTrue(scope.decide(["pkg/OTHER.md"], root)[0])

    def test_unresolvable_include_dir_fails_closed(self):
        root = make_root({"src/lib.rs": 'static D: Dir = include_dir!("$OUT_DIR/gen");\n'})
        self.assert_required(["notes/guide.md"], root, "binding set unavailable")

    def test_manifest_rooted_test_walk_binds_the_walked_directory(self):
        root = make_root({
            "src/lib.rs": (
                "#[test]\nfn skills_parse() {\n"
                '    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));\n'
                '    for d in ["skills"] { read_dir(root.join(d)); }\n'
                "}\n"
                'fn label() -> &\'static str { "docs" }\n'
            ),
            "skills/x/SKILL.md": "x",
            "docs/guide.md": "x",
        })
        self.assert_required(["skills/x/SKILL.md"], root, "skills/ is bound by manifest-read")
        # A bare word outside a manifest-rooted item is not a repository read.
        self.assertTrue(scope.decide(["docs/guide.md"], root)[0])

    def test_build_script_reads_bind_like_any_other_source(self):
        root = make_root({
            "Cargo.toml": "[package]\n",
            "build.rs": ('fn main() { let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));'
                         ' std::fs::read_dir(d.join("templates")); }\n'),
            "templates/T.md": "x",
        })
        self.assert_required(["templates/T.md"], root, "templates/ is bound by manifest-read")

    def test_exact_repository_path_literal_binds_anywhere(self):
        root = make_root({
            "src/lib.rs": 'fn parity(root: &Path) { read(root.join("docs/cli/PARITY.md")); }\n',
            "docs/cli/PARITY.md": "x",
        })
        self.assert_required(["docs/cli/PARITY.md"], root, "path-literal")

    def test_paths_in_comments_do_not_bind(self):
        root = make_root({"src/lib.rs": '// see include_dir!("$CARGO_MANIFEST_DIR/x") "docs/a/B.md"\n'
                                        "pub fn f() {}\n",
                          "docs/a/B.md": "x"})
        # include_dir!/include_str! in a comment still bind (scanned on raw text, fail-safe);
        # a bare path literal in a comment does not.
        self.assertNotIn("docs/a/B.md", scope.binding_paths(root))

    def test_empty_diff_is_not_skippable(self):
        root = make_root({"src/lib.rs": "pub fn f() {}\n"})
        self.assertFalse(scope.decide([], root)[0])


class ProofTests(unittest.TestCase):
    def test_proof_reads_the_named_commit_not_the_work_tree(self):
        root = make_root({"src/lib.rs": "pub fn f() {}\n", "docs/A.md": "x"})
        clean = commit(root)
        write(root, {"src/lib.rs": 'const A: &str = include_str!("../docs/A.md");\n'})
        embedded = commit(root)
        write(root, {"src/lib.rs": "pub fn g() {}\n"})  # uncommitted noise must not matter
        first = scope.proof(["docs/A.md"], root, clean)
        self.assertTrue(first["skip"], first)
        self.assertEqual(first, scope.proof(["docs/A.md"], root, clean))
        second = scope.proof(["docs/A.md"], root, embedded)
        self.assertFalse(second["skip"], second)
        self.assertNotEqual(first["binding_digest"], second["binding_digest"])

    def test_proof_of_unknown_commit_raises(self):
        root = make_root({"src/lib.rs": "pub fn f() {}\n"})
        commit(root)
        with self.assertRaises(scope.ScopeError):
            scope.proof(["docs/A.md"], root, "f" * 40)


class RealRepoSanityTests(unittest.TestCase):
    """The real tree must classify its known-compiled or test-read markdown as required."""

    def test_real_repo_binding_markdown_is_required(self):
        root = Path(__file__).resolve().parent.parent
        bindings = scope.binding_paths(root)
        for path in ("src/assistant/AGENT.md", "docs/CONFIG.md", "README.md"):
            self.assertIn(path, bindings)
        for path in ("src/assistant/AGENT.md", "docs/CONFIG.md", "README.md",
                     "apps/AGENTS.md", "apps/brand-new-app/README.md",
                     "sdk/python/plexi_sdk/templates/plugin-contract.md",
                     "sdk/python/plexi_sdk/NEW.md", "registry/NEW.md",
                     "skills/plexi-cli/SKILL.md", "docs/cli/PARITY.md"):
            with self.subTest(path=path):
                self.assertFalse(scope.decide_with([path], bindings)[0], path)


if __name__ == "__main__":
    unittest.main()
