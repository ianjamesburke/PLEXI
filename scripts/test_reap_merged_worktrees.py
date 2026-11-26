"""reap-merged-worktrees.sh: never removes live work. Local fixtures, stubbed gh and wtp."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parent / 'reap-merged-worktrees.sh'


class Reap(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        self.repo = root / 'repo'
        self.bin = root / 'bin'
        self.bin.mkdir()
        (self.repo / 'scripts').mkdir(parents=True)
        (self.repo / 'scripts' / SCRIPT.name).write_text(SCRIPT.read_text())
        self.removed = root / 'removed.log'
        self.env = {**os.environ, 'PATH': f'{self.bin}{os.pathsep}{os.environ["PATH"]}',
                    'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1',
                    'PRS_FILE': str(root / 'prs.tsv')}
        (root / 'prs.tsv').write_text('')
        self.stub('gh', 'cat "$PRS_FILE"')
        self.stub('wtp', f'echo "$@" >> {self.removed}')
        self.git(self.repo, 'init', '-q', '-b', 'alpha')
        self.git(self.repo, 'remote', 'add', 'origin', str(root / 'nowhere.git'))
        self.commit(self.repo, 'base')
        self.git(self.repo, 'update-ref', 'refs/remotes/origin/alpha', 'HEAD')

    def stub(self, name, body):
        f = self.bin / name
        f.write_text(f'#!/bin/sh\n{body}\n')
        f.chmod(0o755)

    def git(self, cwd, *args):
        return subprocess.run(['git', '-c', 'user.email=t@t', '-c', 'user.name=t', *args], cwd=cwd,
                              env=self.env, capture_output=True, text=True, check=True).stdout.strip()

    def commit(self, cwd, msg):
        (Path(cwd) / f'{msg}.txt').write_text(msg)
        self.git(cwd, 'add', '.')
        self.git(cwd, 'commit', '-q', '-m', msg)

    def worktree(self, name, own_commit):
        path = self.repo / 'worktrees' / name
        self.git(self.repo, 'worktree', 'add', '-q', '-b', name, str(path), 'alpha')
        if own_commit:
            self.commit(path, 'work')
        self.git(path, 'update-ref', f'refs/remotes/origin/{name}', 'HEAD')
        self.git(path, 'branch', '-q', f'--set-upstream-to=origin/{name}')
        return path

    def pr(self, name, state, oid):
        with open(self.env['PRS_FILE'], 'a') as f:
            f.write(f'{name}\t{state}\t{oid}\n')

    def reap(self):
        r = subprocess.run(['bash', str(self.repo / 'scripts' / SCRIPT.name)], env=self.env,
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout

    def was_removed(self):
        return self.removed.exists() and self.removed.read_text()

    def test_dirty_tree_is_kept(self):
        path = self.worktree('feature/1-dirty', True)
        self.pr('feature/1-dirty', 'MERGED', self.git(path, 'rev-parse', 'HEAD'))
        (path / 'wip.txt').write_text('uncommitted')
        out = self.reap()
        self.assertIn('KEEP  feature/1-dirty', out)
        self.assertFalse(self.was_removed())

    def test_no_commits_and_no_pr_is_kept(self):
        self.worktree('feature/2-empty', False)
        out = self.reap()
        self.assertIn('KEEP  feature/2-empty', out)
        self.assertIn('no commits of its own', out)
        self.assertFalse(self.was_removed())

    def test_no_commits_with_old_merged_pr_of_same_name_is_kept(self):
        self.worktree('feature/3-reused', False)
        self.pr('feature/3-reused', 'MERGED', 'f' * 40)
        self.reap()
        self.assertFalse(self.was_removed())

    def test_clean_merged_pr_is_removed(self):
        path = self.worktree('feature/4-done', True)
        self.pr('feature/4-done', 'MERGED', self.git(path, 'rev-parse', 'HEAD'))
        out = self.reap()
        self.assertIn('reap  feature/4-done', out)
        self.assertIn('feature/4-done', self.was_removed())


if __name__ == '__main__':
    unittest.main()
