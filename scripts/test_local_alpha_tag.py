"""local-alpha-tag.sh: the build tag a source install of the alpha channel gets."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parent / 'local-alpha-tag.sh'


class LocalAlphaTag(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.repo = Path(temp.name) / 'repo'
        self.bin = Path(temp.name) / 'bin'
        self.repo.mkdir()
        self.bin.mkdir()
        path = os.pathsep.join(p for p in os.environ['PATH'].split(os.pathsep)
                               if not (Path(p) / 'plexi-alpha').exists())
        self.env = {**os.environ, 'PATH': f'{self.bin}{os.pathsep}{path}',
                    'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1'}
        self.git('init', '-q', '-b', 'alpha')
        (self.repo / 'Cargo.toml').write_text('[package]\nname = "plexi"\nversion = "0.6.1"\n')
        self.git('add', '.')
        self.git('commit', '-q', '-m', 'base')

    def git(self, *args):
        subprocess.run(['git', '-c', 'user.email=t@t', '-c', 'user.name=t', *args],
                       cwd=self.repo, env=self.env, capture_output=True, text=True, check=True)

    def installed(self, body):
        script = self.bin / 'plexi-alpha'
        script.write_text(f'#!/bin/sh\n{body}\n')
        script.chmod(0o755)

    def tag(self):
        result = subprocess.run(['bash', str(SCRIPT)], cwd=self.repo, env=self.env,
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_first_local_build_is_n1(self):
        self.assertEqual(self.tag(), 'v0.6.1-alpha.1')

    def test_counts_above_the_newest_published_alpha_tag(self):
        self.git('tag', 'v0.6.1-alpha.2')
        self.assertEqual(self.tag(), 'v0.6.1-alpha.3')

    def test_counts_above_the_installed_build_of_the_same_version(self):
        self.git('tag', 'v0.6.1-alpha.2')
        self.installed('echo "plexi-alpha 0.6.1-alpha.7 (abc1234)"')
        self.assertEqual(self.tag(), 'v0.6.1-alpha.8')

    def test_restarts_when_the_version_changes(self):
        self.installed('echo "plexi-alpha 0.6.0-alpha.9 (abc1234)"')
        self.assertEqual(self.tag(), 'v0.6.1-alpha.1')

    def test_counts_on_the_next_patch_once_the_version_is_released(self):
        self.git('tag', 'v0.6.1')
        self.installed('echo "plexi-alpha 0.6.1 (abc1234)"')
        self.assertEqual(self.tag(), 'v0.6.2-alpha.1')

    def test_a_broken_install_still_gets_a_tag(self):
        self.git('tag', 'v0.6.1-alpha.2')
        self.installed('echo boom >&2; exit 3')
        self.assertEqual(self.tag(), 'v0.6.1-alpha.3')


if __name__ == '__main__':
    unittest.main()
