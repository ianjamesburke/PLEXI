"""release-next-tag.sh: the tag a push to alpha, beta or main publishes."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parent / 'release-next-tag.sh'


class ReleaseNextTag(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.repo = Path(temp.name)
        self.env = {**os.environ, 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1'}
        self.git('init', '-q', '-b', 'alpha')
        self.commit('base', version='0.5.3')

    def git(self, *args):
        return subprocess.run(['git', '-c', 'user.email=t@t', '-c', 'user.name=t', *args],
                              cwd=self.repo, env=self.env, capture_output=True, text=True,
                              check=True).stdout.strip()

    def commit(self, message, version=None):
        if version:
            (self.repo / 'Cargo.toml').write_text(f'[package]\nname = "plexi"\nversion = "{version}"\n')
        (self.repo / 'change.txt').write_text(message)
        self.git('add', '.')
        self.git('commit', '-q', '-m', message)
        return self.git('rev-parse', 'HEAD')

    def run_script(self, branch, sha=None):
        return subprocess.run(['bash', str(SCRIPT), branch, sha or self.git('rev-parse', 'HEAD')],
                              cwd=self.repo, env=self.env, capture_output=True, text=True)

    def next_tag(self, branch):
        result = self.run_script(branch)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_first_prerelease_of_an_unreleased_version_is_n1(self):
        self.assertEqual(self.next_tag('alpha'), 'v0.5.3-alpha.1')
        self.assertEqual(self.next_tag('beta'), 'v0.5.3-beta.1')

    def test_prerelease_counts_up_only_when_commits_landed(self):
        self.git('tag', 'v0.5.3-alpha.9')
        self.assertEqual(self.next_tag('alpha'), '')
        self.commit('fix')
        self.assertEqual(self.next_tag('alpha'), 'v0.5.3-alpha.10')

    def test_prereleases_move_to_next_patch_once_stable_is_released(self):
        # semver ranks 0.5.3-alpha.N below 0.5.3; the updater only offers newer.
        self.git('tag', 'v0.5.3')
        self.commit('fix')
        self.assertEqual(self.next_tag('alpha'), 'v0.5.4-alpha.1')
        self.assertEqual(self.next_tag('beta'), 'v0.5.4-beta.1')

    def test_main_releases_an_untagged_version(self):
        self.assertEqual(self.next_tag('main'), 'v0.5.3')

    def test_main_is_silent_on_a_repush_of_the_released_commit(self):
        self.git('tag', 'v0.5.3')
        self.assertEqual(self.next_tag('main'), '')

    def test_main_refuses_new_commits_without_a_version_bump(self):
        self.git('tag', 'v0.5.3')
        self.commit('fix')
        result = self.run_script('main')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('just bump', result.stderr)

    def test_rejects_unknown_branch_and_prerelease_cargo_version(self):
        self.assertNotEqual(self.run_script('feature').returncode, 0)
        self.commit('pre', version='0.6.0-alpha.1')
        self.assertNotEqual(self.run_script('alpha').returncode, 0)


if __name__ == '__main__':
    unittest.main()
