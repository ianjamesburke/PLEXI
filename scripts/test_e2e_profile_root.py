"""Standalone e2e scripts root Plexi in a disposable dir, never HOME (stint 1695).

Runs each script's real initialization, up to and including its
`plexi_profile_root`, against a stub channel binary that records the
`--profile-root` every call carries. A standalone run must root every call in
a fresh temp dir and move HOME there; under the V1 harness it roots in the
harness's temp home. Nothing may land in the caller's HOME.
"""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = (
    "scripts/no-self-approval-e2e.sh",
    "scripts/permission-gate-e2e.sh",
    "scripts/skill-check.sh",
    "scripts/e2e/ledger/run.sh",
)
MARK = 'plexi_profile_root "$SCRIPT_ROOT"\n'
STUB = (
    "#!/bin/bash\n"
    "set -eu\n"
    '[ "$1" = --profile-root ] || { echo "unrooted: $*" >&2; exit 91; }\n'
    'echo "$2" >> "$FIXTURE_ROOTS"\n'
    'root="$2"; shift 2\n'
    'case "$*" in\n'
    '  "host status --json") echo "{\\"socket\\": \\"$root/.plexi-alpha/notify.sock\\"}" ;;\n'
    '  *) echo "unexpected command: $*" >&2; exit 90 ;;\n'
    "esac\n"
)


class StandaloneRoots(unittest.TestCase):
    def run_init(self, relative, harness_root=None):
        with tempfile.TemporaryDirectory(prefix="plexi-1695-") as temp:
            sandbox = Path(temp)
            source = (ROOT / relative).read_text()
            self.assertIn(MARK, source, relative)
            head = source.split(MARK, 1)[0] + MARK
            head += 'printf "ROOT=%s\\nHOME=%s\\nPROFILE=%s\\n" "$PROFILE_ROOT" "$HOME" "$PROFILE"\nexit 0\n'
            script = sandbox / relative
            script.parent.mkdir(parents=True, exist_ok=True)
            script.write_text(head)
            for helper in ("scripts/e2e/profile-root.sh", "scripts/e2e/human.sh"):
                copy = sandbox / helper
                copy.parent.mkdir(parents=True, exist_ok=True)
                copy.write_text((ROOT / helper).read_text())
            binary = sandbox / "bin/plexi-alpha"
            binary.parent.mkdir()
            binary.write_text(STUB)
            binary.chmod(0o755)
            home = sandbox / "home"
            (home / ".plexi/wasm-bundles").mkdir(parents=True)
            tmp = sandbox / "tmp"
            tmp.mkdir()
            env = {
                key: value
                for key, value in os.environ.items()
                if not key.startswith("PLEXI_")
            }
            env.update(
                HOME=str(home),
                TMPDIR=str(tmp),
                PLEXI_BIN=str(binary),
                EVID=str(sandbox / "evidence"),
                FIXTURE_ROOTS=str(sandbox / "roots.txt"),
                # Keep Linux from re-executing under a session bus.
                NOSELF_E2E_INNER="1",
                GATE_E2E_INNER="1",
                SKILL_CHECK_INNER="1",
            )
            if harness_root is not None:
                env["PLEXI_E2E_PROFILE_ROOT"] = str(sandbox / harness_root)
                (sandbox / harness_root).mkdir()
            result = subprocess.run(
                ["bash", str(script)], env=env, capture_output=True, text=True, timeout=60
            )
            self.assertEqual(result.returncode, 0, relative + result.stdout + result.stderr)
            out = dict(
                line.split("=", 1) for line in result.stdout.splitlines() if "=" in line
            )
            roots = set((sandbox / "roots.txt").read_text().split())
            self.assertEqual(roots, {out["ROOT"]}, relative)
            self.assertEqual(list(home.iterdir()), [home / ".plexi"], relative)
            return sandbox, home, tmp, out

    def test_a_standalone_run_never_roots_in_home(self):
        for relative in SCRIPTS:
            with self.subTest(script=relative):
                sandbox, home, tmp, out = self.run_init(relative)
                self.assertTrue(out["ROOT"].startswith(str(tmp)), out)
                self.assertNotEqual(out["ROOT"], str(home))
                self.assertEqual(out["HOME"], out["ROOT"], "HOME moves to the disposable root")
                self.assertTrue(out["PROFILE"].startswith(out["ROOT"] + "/.plexi"), out)

    def test_the_harness_root_is_shared(self):
        for relative in SCRIPTS:
            with self.subTest(script=relative):
                sandbox, home, tmp, out = self.run_init(relative, harness_root="harness")
                self.assertEqual(out["ROOT"], str(sandbox / "harness"))


if __name__ == "__main__":
    unittest.main()
