"""Exercise the CI snapshot guard against local Git history."""
from pathlib import Path
import subprocess
import tempfile
import unittest


GUARD = Path(__file__).with_name("check-wit-snapshots.sh")


class WitSnapshotsTest(unittest.TestCase):
    def test_existing_snapshots_are_immutable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def git(*args):
                return subprocess.run(
                    ["git", "-C", directory, *args], check=True,
                    capture_output=True, text=True,
                ).stdout.strip()

            git("init")
            snapshots = root / "tests/wit-versions"
            snapshots.mkdir(parents=True)
            old = snapshots / "0.1.0.sha256"
            old.write_text("original\n")

            def commit():
                git("add", ".")
                git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                    "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false",
                    "commit", "-m", "fixture")
                return git("rev-parse", "HEAD")

            base = commit()

            def guard(head, compare_base=base):
                return subprocess.run(
                    ["bash", str(GUARD), compare_base, head], cwd=root,
                    capture_output=True, text=True,
                )

            self.assertEqual(guard(base).returncode, 0)
            (snapshots / "0.2.0.sha256").write_text("new\n")
            added = commit()
            self.assertEqual(guard(added).returncode, 0)
            old.write_text("tampered\n")
            modified = commit()
            self.assertNotEqual(guard(modified).returncode, 0)
            self.assertIn("0.1.0.sha256", guard(modified).stdout)
            old.unlink()
            deleted = commit()
            self.assertNotEqual(guard(deleted).returncode, 0)
            git("reset", "--hard", added)
            old.rename(snapshots / "renamed.sha256")
            renamed = commit()
            self.assertNotEqual(guard(renamed).returncode, 0)
            self.assertNotEqual(guard("missing-ref").returncode, 0)
            git("reset", "--hard", added)
            (snapshots / "target-only.sha256").write_text("target branch addition\n")
            advanced_base = commit()
            git("reset", "--hard", added)
            (snapshots / "pr-only.sha256").write_text("PR addition\n")
            pr_head = commit()
            self.assertEqual(guard(pr_head, advanced_base).returncode, 0)

    def test_wit_checkout_uses_lf(self):
        root = GUARD.parent.parent
        result = subprocess.run(
            ["git", "check-attr", "eol", "--", "wit/plexi.wit", "wit/deps/test/nested.wit",
             "tests/wit-versions/0.3.0.sha256"],
            cwd=root, check=True, capture_output=True, text=True,
        )
        self.assertTrue(all(line.endswith(": lf") for line in result.stdout.splitlines()), result.stdout)


if __name__ == "__main__":
    unittest.main()
