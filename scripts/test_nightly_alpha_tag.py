"""Stint 1480: the nightly tags the commit its host tests ran on, never a
moving alpha head, and refuses a tested commit that is not on alpha."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'scripts' / 'nightly-alpha-tag.sh'


class NightlyAlphaTag(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.env = {**os.environ, 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1'}
        self.git('init', '-q', '-b', 'alpha')
        (self.repo / 'Cargo.toml').write_text('[package]\nversion = "0.5.0"\n')
        self.commit('base')
        self.git('tag', 'v0.5.0-alpha.1')

    def git(self, *args):
        return subprocess.run(['git', '-c', 'user.email=t@t', '-c', 'user.name=t', *args],
                              cwd=self.repo, env=self.env, capture_output=True, text=True,
                              check=True).stdout.strip()

    def commit(self, message):
        (self.repo / 'change.txt').write_text(message)
        self.git('add', '-A')
        self.git('commit', '-qm', message)
        return self.git('rev-parse', 'HEAD')

    def run_script(self, tested):
        return subprocess.run(['bash', str(SCRIPT), tested, 'alpha'], cwd=self.repo,
                              env=self.env, capture_output=True, text=True)

    def test_tag_targets_the_tested_commit_when_alpha_moved_on(self):
        tested = self.commit('tested by the nightly')
        self.commit('pushed to alpha during the run')
        result = self.run_script(tested)
        self.assertEqual(result.returncode, 0, result.stderr)
        tag = result.stdout.strip()
        self.assertEqual(tag, 'v0.5.0-alpha.2')
        # The workflow's tag step: tag the tested sha, never HEAD.
        self.git('tag', tag, tested)
        self.assertEqual(self.git('rev-list', '-n1', tag), tested)

    def test_nothing_new_when_the_tested_commit_is_already_tagged(self):
        tested = self.git('rev-parse', 'HEAD')
        self.commit('pushed to alpha during the run')
        result = self.run_script(tested)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), '', 'commits after the tested sha wait for the next night')

    def test_refuses_a_tested_commit_not_on_alpha(self):
        self.git('checkout', '-q', '-b', 'side')
        tested = self.commit('only on a side branch')
        self.git('checkout', '-q', 'alpha')
        result = self.run_script(tested)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), '')
        self.assertIn('not an ancestor of alpha', result.stderr)


if __name__ == '__main__':
    unittest.main()
