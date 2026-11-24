"""Exercise skill-check's install-path check without starting a host."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/skill-check.sh"


class SkillCheckPaths(unittest.TestCase):
    def run_install_check(self, channel, installed_name, agents=("claude", "codex")):
        with tempfile.TemporaryDirectory(prefix="plexi-1477-") as temp:
            root = Path(temp)
            script = root / "scripts/skill-check.sh"
            script.parent.mkdir()
            # The real guard judges compiled hook names and installed receipts;
            # this fixture has no real build, so the fake accepts its fake PR-channel binary.
            guard = script.parent / "require_test_hook_binary.py"
            guard.write_text("import sys\nprint(sys.argv[-1])\n")
            # Execute the real initialization and step 1, stopping before the
            # unrelated command-surface and live-host checks.
            script.write_text(SCRIPT.read_text().split("# ── 2.", 1)[0])
            binary = root / f"plexi-{channel}"
            binary.write_text(
                '#!/bin/bash\nset -eu\n'
                'case "$*" in\n'
                '  --version) echo "plexi 1.0.0" ;;\n'
                '  "host stop") ;;\n'
                '  "skill install --agent all")\n'
                '    for agent in $FIXTURE_AGENTS; do\n'
                '      dir="$HOME/.$agent/skills/$FIXTURE_SKILL_NAME"\n'
                '      mkdir -p "$dir"\n'
                '      printf "fixture\\n" > "$dir/SKILL.md"\n'
                '    done ;;\n'
                '  *) echo "unexpected command: $*" >&2; exit 90 ;;\n'
                'esac\n'
            )
            binary.chmod(0o755)
            commands = root / "bin"
            commands.mkdir()
            # Keep the fixture independent of Linux session services and Git.
            for name, output in (("uname", "Darwin"), ("git", "fixture-head")):
                command = commands / name
                command.write_text(f'#!/bin/sh\nprintf "%s\\n" "{output}"\n')
                command.chmod(0o755)
            env = {
                **os.environ,
                "HOME": str(root / "home"),
                "CHANNEL": channel,
                "BIN": str(binary),
                "PROFILE": str(root / "profile"),
                "EVID": str(root / "evidence"),
                "XDG_RUNTIME_DIR": str(root / "runtime"),
                "SKILL_CHECK_INNER": "1",
                "FIXTURE_SKILL_NAME": installed_name,
                "FIXTURE_AGENTS": " ".join(agents),
                "PATH": f"{commands}:/usr/bin:/bin:/usr/sbin:/sbin",
            }
            result = subprocess.run(["bash", str(script)], env=env, capture_output=True, text=True)
            return result

    def test_channel_install_paths(self):
        for channel, name in (
            ("alpha", "plexi-cli-alpha"),
            ("beta", "plexi-cli-beta"),
            ("pr-1477", "plexi-cli-pr-1477"),
            ("main", "plexi-cli"),
            ("rc-1", "plexi-cli"),
        ):
            with self.subTest(channel=channel):
                result = self.run_install_check(channel, name)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("PASS\tstep1", result.stdout)

    def test_wrong_channel_is_rejected(self):
        for name in ("plexi-cli", "plexi-cli-alpha", "plexi-cli-pr-1478"):
            with self.subTest(name=name):
                result = self.run_install_check("pr-1477", name)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("FAIL\tstep1", result.stdout)

    def test_both_agent_installs_are_required(self):
        for agents in (("claude",), ("codex",)):
            with self.subTest(agents=agents):
                result = self.run_install_check("pr-1477", "plexi-cli-pr-1477", agents)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("FAIL\tstep1", result.stdout)


if __name__ == "__main__":
    unittest.main()
