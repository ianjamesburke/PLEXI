"""Real Cargo regression fixtures for optional channel input tracking.

Runs the repository build.rs and invocation helper, with local Git repositories,
no network, isolated lease files, and both local and external target directories.
"""

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

REPO = Path(__file__).resolve().parent.parent
HELPER = REPO / "scripts/cargo-channel-inputs.py"
LEASE = REPO / "scripts/cargo-with-lease.sh"
ROOT_ENV = "PLEXI_BUILD_CHANNEL_ROOT"
INPUT_ENV = "PLEXI_BUILD_CHANNEL_INPUT"


class ChannelInputs(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="plexi-channel-input-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.env = dict(os.environ)
        for key in (ROOT_ENV, INPUT_ENV, "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR", "PLEXI_CARGO_LOCK_HELD", "PLEXI_BUILD_TAG", "RUSTFLAGS"):
            self.env.pop(key, None)
        self.env["PLEXI_CARGO_LOCK"] = str(self.base / "lease.lock")
        self.env["PLEXI_RUN_LOG_DIR"] = str(self.base / "run-logs")  # keep logs out of the fixture's git tree
        self.env["CARGO_BUILD_JOBS"] = "2"

    def command(self, args, cwd, env=None):
        result = subprocess.run(args, cwd=cwd, env=env or self.env, capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def package(self, name):
        root = self.base / name
        (root / "src").mkdir(parents=True)
        (root / "Cargo.toml").write_text('[package]\nname="channel_probe"\nversion="0.1.0"\nedition="2021"\n[build-dependencies]\nsha2="0.10"\n')
        shutil.copyfile(REPO / "build.rs", root / "build.rs")
        (root / "src/main.rs").write_text('fn main() { println!("{}", env!("PLEXI_APP_TITLE")); }\n')
        (root / ".gitignore").write_text('.channel\ntarget/\n')
        for args in (["git", "init", "-q"], ["git", "add", "."], ["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture"]):
            self.command(args, root)
        return root

    def build(self, root, target, *, managed=True, cwd=None, extra=(), overrides=None, nested=False):
        env = dict(self.env, CARGO_TARGET_DIR=str(target))
        env.update(overrides or {})
        cargo = [sys.executable, str(HELPER)] if managed else ["cargo"]
        args = [*cargo, "build", "--offline", "--message-format=json", *extra]
        if managed:
            args = ["bash", str(LEASE), *args]
            if nested:
                args = ["bash", str(LEASE), *args]
        started = time.monotonic()
        result = self.command(args, cwd or root, env)
        elapsed = time.monotonic() - started
        artifacts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
        executable = next(item for item in artifacts if item.get("reason") == "compiler-artifact" and item.get("executable"))
        title = self.command([executable["executable"]], root, env).stdout.strip()
        return executable["fresh"], title, elapsed

    def test_channel_transitions_and_freshness_in_both_target_layouts(self):
        for layout in ("inside", "outside"):
            with self.subTest(layout=layout):
                root = self.package(layout)
                target = root / "target" if layout == "inside" else self.base / "outside-target"
                transitions = [(None, False, "Plexi"), (None, True, "Plexi"), ("alpha", False, "Plexi Alpha"), (None, True, "Plexi Alpha"), ("beta", False, "Plexi Beta"), ("DELETE", False, "Plexi"), (None, True, "Plexi"), ("alpha", False, "Plexi Alpha"), ("", False, "Plexi"), (None, True, "Plexi")]
                for content, fresh, title in transitions:
                    if content == "DELETE":
                        (root / ".channel").unlink()
                    elif content is not None:
                        (root / ".channel").write_text(content)
                    actual = self.build(root, target)
                    self.assertEqual(actual[:2], (fresh, title))
                    print(f"channel-fixture layout={layout} change={content!r} fresh={actual[0]} title={actual[1]!r} elapsed={actual[2]:.3f}s", flush=True)

    def test_raw_and_managed_switching_preserves_creation_detection(self):
        root = self.package("switch")
        target = root / "target"
        self.assertEqual(self.build(root, target)[:2], (False, "Plexi"))
        self.assertEqual(self.build(root, target)[:2], (True, "Plexi"))
        self.assertEqual(self.build(root, target, managed=False)[:2], (False, "Plexi"))
        self.assertEqual(self.build(root, target, managed=False)[:2], (False, "Plexi"))
        (root / ".channel").write_text("alpha")
        self.assertEqual(self.build(root, target, managed=False)[:2], (False, "Plexi Alpha"))
        self.assertEqual(self.build(root, target)[:2], (False, "Plexi Alpha"))
        self.assertEqual(self.build(root, target)[:2], (True, "Plexi Alpha"))

    def test_manifest_workspace_and_inherited_snapshot_under_nested_lease(self):
        source = self.package("driver")
        selected = self.package("selected workspace")
        (source / ".channel").write_text("alpha")
        (selected / ".channel").write_text("beta")
        target = selected / "target"
        inherited = {ROOT_ENV: str(source), INPUT_ENV: "stale"}
        for extra in (("--manifest-path", str(selected / "Cargo.toml")), ("--manifest-path=" + str(selected / "Cargo.toml"),)):
            value = self.build(selected, target, cwd=source, extra=extra, overrides=inherited, nested=True)
            self.assertEqual(value[1], "Plexi Beta")
            self.assertTrue(self.build(selected, target, cwd=source, extra=extra, overrides=inherited, nested=True)[0])
        (selected / ".channel").unlink()
        self.assertEqual(self.build(selected, target, cwd=source, extra=extra, overrides=inherited, nested=True)[:2], (False, "Plexi"))
        self.assertTrue(self.build(selected, target, cwd=source, extra=extra, overrides=inherited, nested=True)[0])
        self.assertEqual((source / ".channel").read_text(), "alpha")

    def test_workspace_member_invocation_and_mismatched_snapshot_fallback(self):
        root = self.package("workspace")
        with (root / "Cargo.toml").open("a") as file:
            file.write('[workspace]\nmembers=["member"]\n')
        member = root / "member"
        (member / "src").mkdir(parents=True)
        (member / "Cargo.toml").write_text('[package]\nname="member"\nversion="0.1.0"\nedition="2021"\n')
        (member / "src/lib.rs").write_text('pub fn member() {}\n')
        (root / ".channel").write_text("beta")
        target = root / "target"
        self.assertEqual(self.build(root, target, cwd=member, extra=("-p", "channel_probe"))[:2], (False, "Plexi Beta"))
        self.assertEqual(self.build(root, target, cwd=member, extra=("-p", "channel_probe"))[:2], (True, "Plexi Beta"))
        wrong = {ROOT_ENV: str(member), INPUT_ENV: "missing"}
        (root / ".channel").unlink()
        self.assertEqual(self.build(root, target, managed=False, overrides=wrong)[:2], (False, "Plexi"))
        (root / ".channel").write_text("alpha")
        self.assertEqual(self.build(root, target, managed=False, overrides=wrong)[:2], (False, "Plexi Alpha"))

    def test_canonical_clippy_recipe_keeps_managed_input_on_repeat(self):
        root = self.package("lint-recipe")
        with (root / "Cargo.toml").open("a") as manifest:
            manifest.write('[[bin]]\nname="plexi"\npath="src/main.rs"\n')
        (root / "scripts").mkdir()
        shutil.copyfile(HELPER, root / "scripts/cargo-channel-inputs.py")
        shutil.copyfile(LEASE, root / "scripts/cargo-with-lease.sh")
        shutil.copyfile(REPO / "scripts/run_logged.py", root / "scripts/run_logged.py")
        env = dict(self.env, CARGO_TARGET_DIR=str(root / "target"), CARGO_NET_OFFLINE="true")
        command = ["just", "--justfile", str(REPO / "justfile"), "--working-directory", str(root), "clippy"]
        first = self.command(command, root, env)
        self.assertIn("cargo-channel-input:", first.stderr)
        second = self.command(command, root, env)
        self.assertIn("cargo-channel-input:", second.stderr)
        self.assertNotIn("Checking channel_probe", second.stderr)
        self.assertNotIn("Compiling channel_probe", second.stderr)
        outputs = list((root / "target/debug/build").glob("channel_probe-*/output"))
        self.assertTrue(outputs)
        for output in outputs:
            self.assertIn("cargo:rerun-if-env-changed=PLEXI_BUILD_CHANNEL_INPUT", output.read_text())
            self.assertNotIn("cargo:rerun-if-changed=.channel", output.read_text())

    def test_lookup_and_read_failures_do_not_reuse_cached_input(self):
        root = self.package("failure")
        env = dict(self.env, CARGO_TARGET_DIR=str(root / "target"))
        missing = root / "missing.toml"
        for prefix in ([sys.executable, str(HELPER)], ["cargo"]):
            result = subprocess.run([*prefix, "build", "--offline", "--manifest-path", str(missing)], cwd=root, env=env, capture_output=True, text=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("missing.toml", result.stderr)
        (root / ".channel").mkdir()
        result = subprocess.run([sys.executable, str(HELPER), "build", "--offline"], cwd=root, env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 1)
        self.assertIn("cargo-channel-input:", result.stderr)
        self.assertIn(".channel", result.stderr)

    def test_unsupported_invocation_clears_inherited_snapshot(self):
        spec = importlib.util.spec_from_file_location("channel_inputs", HELPER)
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        for args in (["metadata"], ["+nightly", "build", "-C", "/tmp"], ["build", "--manifest-path"]):
            env = helper.cargo_environment(args, {ROOT_ENV: "/stale", INPUT_ENV: "stale", "KEEP": "yes"})
            self.assertEqual(env, {"KEEP": "yes"})


if __name__ == "__main__":
    unittest.main()
