"""Offline proof for the path-specific docs-only gate decision (stints 1079, 1102).

Run: uv run --no-project python scripts/test_docs_gate_scope.py
Wired into `just test-infra-scripts`.
"""

import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

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

    def test_file_level_include_binds_the_file_not_its_directory(self):
        root = make_root({
            "src/lib.rs": ("#[cfg(test)]\nmod tests {\n"
                           '    const R: &str = include_str!("../README.md");\n'
                           '    const C: &str = include_str!("../docs/CONFIG.md");\n}\n'),
            "README.md": "x", "docs/CONFIG.md": "x", "NOTES.md": "x", "docs/OTHER.md": "x",
        })
        self.assert_required(["README.md"], root, "README.md is bound by include_str")
        self.assert_required(["docs/CONFIG.md"], root, "docs/CONFIG.md is bound by include_str")
        for sibling in ("NOTES.md", "docs/OTHER.md", "docs/BRAND_NEW.md"):
            with self.subTest(sibling=sibling):
                self.assertTrue(scope.decide([sibling], root)[0])

    RS_WALK = (
        "fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {\n"
        "    for entry in read_dir(dir).unwrap().flatten() {\n"
        "        let path = entry.path();\n"
        "        if path.is_dir() { rs_files(&path, out); }\n"
        '        else if path.extension().is_some_and(|e| e == "rs") { out.push(path); }\n'
        "    }\n}\n"
        "#[test]\nfn guard() {\n"
        '    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");\n'
        "    let mut files = Vec::new();\n    rs_files(&root, &mut files);\n"
        "    for f in files { read_to_string(f); }\n}\n"
    )

    def test_extension_filtered_walk_binds_only_that_extension(self):
        root = make_root({"src/lib.rs": self.RS_WALK, "src/AGENTS.md": "x"})
        self.assertEqual(scope.binding_paths(root)["src"]["exts"], ["rs"])
        self.assertTrue(scope.decide(["src/AGENTS.md"], root)[0])
        self.assertFalse(scope.decide(["src/lib.rs"], root)[0])

    def test_unfiltered_walk_binds_markdown(self):
        unfiltered = self.RS_WALK.replace('path.extension().is_some_and(|e| e == "rs")', "true")
        root = make_root({"src/lib.rs": unfiltered, "src/AGENTS.md": "x"})
        self.assert_required(["src/AGENTS.md"], root, "src/ is bound by manifest-read")

    def test_walk_that_also_names_markdown_binds_markdown(self):
        reads_md = self.RS_WALK.replace("for f in files { read_to_string(f); }",
                                        'for f in files { read_to_string(f.join("SKILL.md")); }')
        root = make_root({"src/lib.rs": reads_md, "src/AGENTS.md": "x"})
        self.assert_required(["src/AGENTS.md"], root, "src/ is bound by manifest-read")

    def test_ends_with_filter_narrows_and_a_md_filter_binds(self):
        rs = ('#[test]\nfn g() { let r = Path::new(env!("CARGO_MANIFEST_DIR")).join("guides");'
              ' for p in walk(r) { if p.ends_with(".rs") { read(p); } } }\n')
        root = make_root({"src/lib.rs": rs, "guides/a.md": "x"})
        self.assertTrue(scope.decide(["guides/a.md"], root)[0])
        root = make_root({"src/lib.rs": rs.replace('".rs"', '".md"'), "guides/a.md": "x"})
        self.assert_required(["guides/a.md"], root, "guides/ is bound by manifest-read")

    def test_extension_filter_in_a_transitive_helper_is_seen(self):
        chained = self.RS_WALK.replace("    rs_files(&root, &mut files);\n", "    collect(&root, &mut files);\n")
        chained += "fn collect(dir: &Path, out: &mut Vec<PathBuf>) { rs_files(dir, out); }\n"
        root = make_root({"src/lib.rs": chained, "src/AGENTS.md": "x"})
        self.assertTrue(scope.decide(["src/AGENTS.md"], root)[0])

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

    def test_include_dir_binds_even_when_its_directory_is_empty(self):
        root = make_root({"Cargo.toml": "[package]\n",
                          "src/lib.rs": 'static D: Dir = include_dir!("$CARGO_MANIFEST_DIR/apps");\n'})
        self.assert_required(["apps/README.md"], root, "apps/ is bound by include_dir")

    def test_raw_strings_do_not_desync_the_scanner(self):
        root = make_root({
            "src/lib.rs": (
                'const SCHEMA: &str = r#"{"tools": "x", "odd": "\""}"#;\n'
                'const B: &[u8] = br#"a"b"#;\n'
                'const DOC: &str = include_str!(r"../guides/raw.md");\n'
                "#[test]\nfn walks() {\n"
                '    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));\n'
                '    read_dir(root.join("guides"));\n'
                "}\n"
            ),
            "guides/raw.md": "x", "guides/other.md": "x", "tools/T.md": "x",
        })
        bindings = scope.binding_paths(root)
        self.assertEqual(bindings["guides/raw.md"]["how"], "include_str")
        self.assertEqual(bindings["guides"]["how"], "manifest-read")
        self.assertNotIn("tools", bindings)

    def test_scanner_desync_fails_closed(self):
        root = make_root({"src/lib.rs": 'fn f() { read(Path::new(env!("CARGO_MANIFEST_DIR")).join("guides")); }\n',
                          "guides/a.md": "x"})
        # A scanner that loses string/code parity must not leave the manifest rule silently blind.
        with patch.object(scope, "_scan", side_effect=lambda text: (text, {}, [])):
            self.assert_required(["notes/guide.md"], root, "scanner lost literal sync")

    def test_unterminated_raw_string_fails_closed(self):
        root = make_root({"src/lib.rs": 'const S: &str = r#"never closed;\n'})
        self.assert_required(["notes/guide.md"], root, "binding set unavailable")

    def test_std_env_var_manifest_dir_roots_reads(self):
        root = make_root({
            "build.rs": ('fn main() { let d = std::env::var_os("CARGO_MANIFEST_DIR").unwrap();'
                         ' read_dir(Path::new(&d).join("guides")); }\n'),
            "guides/a.md": "x",
        })
        self.assert_required(["guides/a.md"], root, "manifest-read")

    def test_templated_markdown_in_a_manifest_item_binds_or_fails_closed(self):
        files = {"guides/a.md": "x"}
        bound = make_root({**files, "src/lib.rs": (
            'fn f(n: &str) { let r = env!("CARGO_MANIFEST_DIR");'
            ' read(Path::new(r).join(format!("guides/{n}.md"))); }\n')})
        self.assert_required(["guides/a.md"], bound, "guides/ is bound by manifest-read")
        opaque = make_root({**files, "src/lib.rs": (
            'fn f(d: &str) { let r = env!("CARGO_MANIFEST_DIR"); read(format!("{d}/{n}.md")); }\n')})
        self.assert_required(["notes/x.md"], opaque, "cannot resolve templated markdown path")

    def test_unresolvable_include_argument_fails_closed(self):
        root = make_root({"src/lib.rs": "const A: &str = include_str!(PATH);\n"})
        self.assert_required(["notes/guide.md"], root, "cannot resolve include_str! argument")

    def test_concat_manifest_dir_include_resolves(self):
        root = make_root({"Cargo.toml": "[package]\n", "guides/a.md": "x", "src/lib.rs": (
            'const A: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/guides/a.md"));\n')})
        self.assert_required(["guides/a.md"], root, "guides/a.md is bound by include_str")
        self.assertTrue(scope.decide(["notes/guide.md"], root)[0])

    def test_includes_resolve_through_tracked_symlinks(self):
        root = make_root({"real/a.md": "x", "other/b.md": "x", "Cargo.toml": "[package]\n", "src/lib.rs": (
            'const A: &str = include_str!("../linked/a.md");\n'
            'static D: Dir = include_dir!("$CARGO_MANIFEST_DIR/bundle");\n')})
        os.symlink("real", root / "linked")
        (root / "bundle").mkdir()
        os.symlink("../other", root / "bundle/other")
        self.assert_required(["real/a.md"], root, "include_str")
        self.assert_required(["other/b.md"], root, "include_dir")

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
    def test_proof_reads_the_named_commits_not_the_work_tree(self):
        root = make_root({"src/lib.rs": "pub fn f() {}\n", "docs/A.md": "x"})
        clean = commit(root)
        write(root, {"src/lib.rs": 'const A: &str = include_str!("../docs/A.md");\n'})
        embedded = commit(root)
        write(root, {"src/lib.rs": "pub fn g() {}\n"})  # uncommitted noise must not matter
        first = scope.proof(["docs/A.md"], root, clean, clean)
        self.assertTrue(first["skip"], first)
        self.assertEqual(first, scope.proof(["docs/A.md"], root, "HEAD~1", "HEAD~1"))
        self.assertEqual(first["head"], clean)
        second = scope.proof(["docs/A.md"], root, embedded, embedded)
        self.assertFalse(second["skip"], second)
        self.assertNotEqual(first["digests"], second["digests"])

    def test_proof_requires_skip_against_the_base_the_squash_lands_on(self):
        # The base gained an include_str! after the branch forked; the head tree lacks it.
        root = make_root({"src/lib.rs": "pub fn f() {}\n", "docs/A.md": "old"})
        fork = commit(root)
        write(root, {"src/lib.rs": 'const A: &str = include_str!("../docs/A.md");\n'})
        base = commit(root)
        git(root, "checkout", "-q", "-b", "branch", fork)
        write(root, {"docs/A.md": "new"})
        head = commit(root)
        result = scope.proof(["docs/A.md"], root, head, base)
        self.assertFalse(result["skip"], result)
        self.assertTrue(any(reason.startswith("at base:") for reason in result["reasons"]), result)

    def test_proof_of_unknown_commit_raises(self):
        root = make_root({"src/lib.rs": "pub fn f() {}\n"})
        commit(root)
        with self.assertRaises(scope.ScopeError):
            scope.proof(["docs/A.md"], root, "f" * 40, "HEAD")


class ScopedTestsShellTests(unittest.TestCase):
    """scoped-tests.sh selects no filters for markdown only on a positive skip."""

    def select(self, classifier):
        root = Path(tempfile.mkdtemp(prefix="scoped-tests-"))
        self.addCleanup(shutil.rmtree, root)
        (root / "scripts").mkdir()
        for name in ("scoped-tests.sh", "scoped-tests.table"):
            shutil.copy(SCRIPT.with_name(name), root / "scripts" / name)
        (root / "scripts/docs_gate_scope.py").write_text(classifier)
        return subprocess.run(("bash", str(root / "scripts/scoped-tests.sh"), "notes/guide.md"),
                              capture_output=True, text=True, cwd=root).stdout.strip()

    def test_markdown_selects_nothing_only_when_the_classifier_says_skip(self):
        self.assertEqual(self.select("raise SystemExit(0)\n"), "")
        for classifier in ("raise SystemExit(1)\n", "raise SystemExit(2)\n",
                           "raise ValueError('crash')\n", "this is not python\n"):
            with self.subTest(classifier=classifier):
                self.assertEqual(self.select(classifier), "::")


class RealRepoSanityTests(unittest.TestCase):
    """The real tree must classify its known-compiled or test-read markdown as required."""

    def test_real_repo_scanner_sees_every_manifest_dir_literal(self):
        root = Path(__file__).resolve().parent.parent
        _, sources, _ = scope.tree(root)
        for path, text in sources.items():
            if '"CARGO_MANIFEST_DIR"' not in text:
                continue
            blanked, literals, comments = scope._scan(text)
            quoted = text.count('"CARGO_MANIFEST_DIR"')
            seen = sum(value == "CARGO_MANIFEST_DIR" for value in literals.values())
            commented = sum(text[start:end].count('"CARGO_MANIFEST_DIR"') for start, end in comments)
            with self.subTest(path=path):
                self.assertEqual(seen + commented, quoted)

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
        # Narrowed groups: file-level includes and .rs-only walks leave their siblings skippable.
        for path in ("AGENTS.md", "NORTH_STAR.md", "docs/AGENTS.md", "src/testing/TESTING.md",
                     "src/cli/AGENTS.md", "website/src/content/docs/NEW.md"):
            with self.subTest(path=path):
                self.assertTrue(scope.decide_with([path], bindings)[0], path)
        listing = subprocess.run(("git", "-C", str(root), "ls-files", "-z", "*.md"),
                                 check=True, capture_output=True, text=True).stdout
        markdown = [path for path in listing.split("\0") if path]
        skippable = [path for path in markdown if scope.decide_with([path], bindings)[0]]
        print(f"\nreal repo: {len(skippable)} of {len(markdown)} tracked markdown files skippable",
              file=sys.stderr)


if __name__ == "__main__":
    unittest.main()
