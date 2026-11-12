#!/usr/bin/env python3
"""Exercise lane startup, alpha sync, and cleanup against disposable Git repos."""
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from fixture_env import utf8_env

SCRIPTS = Path(__file__).resolve().parent


class LandingSafety(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name)
        self.remote = self.base / 'origin.git'
        self.root = self.base / 'alpha'
        self.run_cmd('git', 'init', '--bare', str(self.remote), cwd=self.base)
        self.run_cmd('git', 'init', '-b', 'alpha', str(self.root), cwd=self.base)
        self.run_cmd('git', 'config', 'user.email', 'fixture@example.invalid')
        self.run_cmd('git', 'config', 'user.name', 'Fixture')
        (self.root / 'a').write_text('base\n')
        (self.root / 'b').write_text('base\n')
        (self.root / 'Cargo.toml').write_text('version = \"1.0.0\"\n')
        (self.root / '.gitignore').write_text('ignored-data/\ntarget/\nassets/python/\n.venv/\n.pytest_cache/\n__pycache__/\n')
        for name in ('assets/keep', 'sdk/python/tests/keep', 'apps/todo/keep', 'scripts/keep'):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('tracked')
        self.run_cmd('git', 'add', 'a', 'b', 'Cargo.toml', '.gitignore', 'assets', 'sdk', 'apps', 'scripts')
        self.run_cmd('git', 'commit', '-m', 'base')
        self.run_cmd('git', 'remote', 'add', 'origin', str(self.remote))
        self.run_cmd('git', 'push', '-u', 'origin', 'alpha')
        self.bin = self.base / 'bin'
        self.bin.mkdir()
        self.env = utf8_env()
        self.env.pop('PLEXI_PANE_ID', None)
        self.env['PATH'] = str(self.bin) + os.pathsep + self.env['PATH']
        self.stub('stint', 'exit 0')
        self.stub('just', 'echo "$*" >> "$TEST_LOG"')
        self.env['TEST_LOG'] = str(self.base / 'calls')

    def tearDown(self):
        self.tmp.cleanup()

    def run_cmd(self, *args, cwd=None, check=True, env=None):
        return subprocess.run(args, cwd=cwd or self.root, text=True, capture_output=True,
                              check=check, env=env)

    def stub(self, name, body):
        path = self.bin / name
        path.write_text('#!/bin/bash\nset -e\n' + body + '\n')
        path.chmod(0o755)

    def script(self, name, *args):
        return self.run_cmd('bash', str(SCRIPTS / name), *args, env=self.env, check=False)

    def remote_commit(self, filename):
        other = self.base / 'other'
        self.run_cmd('git', 'clone', '-b', 'alpha', str(self.remote), str(other), cwd=self.base)
        self.run_cmd('git', 'config', 'user.email', 'fixture@example.invalid', cwd=other)
        self.run_cmd('git', 'config', 'user.name', 'Fixture', cwd=other)
        (other / filename).write_text('remote\n')
        self.run_cmd('git', 'add', filename, cwd=other)
        self.run_cmd('git', 'commit', '-m', 'remote', cwd=other)
        self.run_cmd('git', 'push', cwd=other)

    def test_sync_preserves_disjoint_and_refuses_overlapping_or_divergent(self):
        (self.root / 'b').write_text('local\n')
        self.remote_commit('a')
        result = self.script('merge-pr.sh', 'sync')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / 'a').read_text(), 'remote\n')
        self.assertEqual((self.root / 'b').read_text(), 'local\n')
        (self.root / 'a').write_text('local overlap\n')
        other = self.base / 'other'
        (other / 'a').write_text('another remote\n')
        self.run_cmd('git', 'commit', '-am', 'next', cwd=other)
        self.run_cmd('git', 'push', cwd=other)
        old = self.run_cmd('git', 'rev-parse', 'HEAD').stdout
        result = self.script('merge-pr.sh', 'sync')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.root / 'a').read_text(), 'local overlap\n')
        self.assertEqual(self.run_cmd('git', 'rev-parse', 'HEAD').stdout, old)
        self.run_cmd('git', 'add', 'a', 'b')
        self.run_cmd('git', 'commit', '-m', 'local')
        old = self.run_cmd('git', 'rev-parse', 'HEAD').stdout
        self.assertNotEqual(self.script('merge-pr.sh', 'sync').returncode, 0)
        self.assertEqual(self.run_cmd('git', 'rev-parse', 'HEAD').stdout, old)

    def test_sync_refuses_ahead_only_before_remote_merge(self):
        (self.root / 'a').write_text('local committed\n')
        self.run_cmd('git', 'commit', '-am', 'local')
        old = self.run_cmd('git', 'rev-parse', 'HEAD').stdout
        result = self.script('merge-pr.sh', 'sync')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('commits absent from origin', result.stderr)
        self.assertEqual(self.run_cmd('git', 'rev-parse', 'HEAD').stdout, old)

    def test_lane_start_warns_and_excludes_dirty_alpha(self):
        (self.root / '.stint/tasks').mkdir(parents=True)
        (self.root / '.stint/tasks/1234-fix.md').write_text('status: todo\n')
        (self.root / 'a').write_text('uncommitted\n')
        self.stub('wtp', 'git worktree add -b "$3" "worktrees/$3" "$4"')
        result = self.script('bs-start.sh', '1234')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('excludes these edits', result.stderr)
        lane = self.root / 'worktrees/feature/stint-1234-fix'
        self.assertEqual((lane / 'a').read_text(), 'base\n')
        self.assertEqual((self.root / 'a').read_text(), 'uncommitted\n')

    def test_cleanup_refuses_dirty_and_preserves_unrelated_lane(self):
        branch = 'feature/stint-1234-fix'
        lane = self.root / 'worktrees' / branch
        sibling = self.root / 'worktrees/feature/other'
        self.run_cmd('git', 'worktree', 'add', '-b', branch, str(lane))
        self.run_cmd('git', 'worktree', 'add', '-b', 'feature/other', str(sibling))
        sha = self.run_cmd('git', 'rev-parse', branch).stdout.strip()
        payload = json.dumps({'state': 'MERGED', 'headRefName': branch, 'headRefOid': sha})
        self.stub('gh', "printf '%s\\n' '" + payload + "'")
        self.assertNotEqual(self.script('merge-pr.sh', 'cleanup', '1', branch, '0' * 40).returncode, 0)
        self.assertTrue(lane.exists())
        (lane / 'a').write_text('valuable\n')
        self.assertNotEqual(self.script('merge-pr.sh', 'cleanup', '1', branch, sha).returncode, 0)
        self.assertEqual((lane / 'a').read_text(), 'valuable\n')
        self.run_cmd('git', 'restore', 'a', cwd=lane)
        (lane / 'ignored-data').mkdir()
        (lane / 'ignored-data/important').write_text('preserve me\n')
        result = self.script('merge-pr.sh', 'cleanup', '1', branch, sha)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('retaining lane', result.stderr)
        self.assertEqual((lane / 'ignored-data/important').read_text(), 'preserve me\n')
        (lane / 'ignored-data/important').unlink()
        (lane / 'ignored-data').rmdir()
        result = self.script('merge-pr.sh', 'cleanup', '1', branch, sha)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(lane.exists())
        self.assertTrue(sibling.exists())
        self.assertEqual((self.base / 'calls').read_text().splitlines(), ['channel-clean pr-1', 'channel-clean pr-1'])

    def test_cleanup_retires_generated_only_and_preserves_symlink_target(self):
        branch = 'feature/stint-1234-generated'
        lane = self.root / 'worktrees' / branch
        self.run_cmd('git', 'worktree', 'add', '-b', branch, str(lane))
        sha = self.run_cmd('git', 'rev-parse', branch).stdout.strip()
        payload = json.dumps({'state': 'MERGED', 'headRefName': branch, 'headRefOid': sha})
        self.stub('gh', "printf '%s\\n' '" + payload + "'")
        outside = self.base / 'valuable'
        outside.mkdir()
        (outside / 'data').write_text('preserve')
        (lane / 'target').symlink_to(outside, target_is_directory=True)
        result = self.script('merge-pr.sh', 'cleanup', '1', branch, sha)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(lane.exists())
        self.assertEqual((outside / 'data').read_text(), 'preserve')
        (lane / 'target').unlink()
        for name in ('target', 'assets/python', 'scripts/__pycache__', 'sdk/python/.venv',
                     'sdk/python/.pytest_cache', 'sdk/python/tests/__pycache__',
                     'apps/todo/__pycache__'):
            path = lane / name
            path.mkdir(parents=True, exist_ok=True)
            (path / 'generated').write_text('cache')
        result = self.script('merge-pr.sh', 'cleanup', '1', branch, sha)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(lane.exists())
        self.assertEqual((outside / 'data').read_text(), 'preserve')

    def test_full_merge_requires_reviewed_head_and_preserves_unrelated_edits(self):
        branch = 'feature/stint-1234-fix'
        lane = self.root / 'worktrees' / branch
        self.run_cmd('git', 'worktree', 'add', '-b', branch, str(lane))
        (lane / 'a').write_text('feature\n')
        self.run_cmd('git', 'commit', '-am', 'feature', cwd=lane)
        self.run_cmd('git', 'push', '-u', 'origin', branch, cwd=lane)
        sha = self.run_cmd('git', 'rev-parse', branch).stdout.strip()
        (self.root / '.stint/tasks').mkdir(parents=True)
        (self.root / '.stint/tasks/1234-fix.md').write_text('status: doing\n')
        (self.root / 'b').write_text('local work\n')
        self.env['TEST_BRANCH'] = branch
        self.env['TEST_SHA'] = sha
        self.env['TEST_REMOTE'] = str(self.remote)
        self.env['TEST_GH_STATE'] = str(self.base / 'merged')
        self.env['TEST_SCRIPTS'] = str(SCRIPTS)
        self.env['TEST_STINT_LOG'] = str(self.base / 'stint-calls')
        self.stub('stint', 'echo \"$*\" >> \"$TEST_STINT_LOG\"')
        (lane / 'target').mkdir()
        (lane / 'target/build-output').write_text('compiled output\n')
        (lane / 'ignored-data').mkdir()
        (lane / 'ignored-data/important').write_text('preserve me\n')
        self.stub('just', 'if [ \"$1\" = merge-pr ]; then shift; exec bash \"$TEST_SCRIPTS/merge-pr.sh\" \"$@\"; fi\necho \"$*\" >> \"$TEST_LOG\"')
        self.stub('gh', r"""
if [ "$1 $2" = 'pr view' ]; then
  if [ -f "$TEST_GH_STATE" ]; then state=MERGED; else state=OPEN; fi
  if [[ "$*" == *closingIssuesReferences* ]]; then
    printf '\n'
  elif [[ "$*" == *"--json headRefName --jq"* ]]; then
    printf '%s\n' "$TEST_BRANCH"
  elif [[ "$*" == *'--jq'* ]]; then
    printf '{"branch":"%s","state":"%s","base":"alpha","body":"Stint: 1234"}\n' "$TEST_BRANCH" "$state"
  else
    printf '{"headRefName":"%s","headRefOid":"%s","state":"%s"}\n' "$TEST_BRANCH" "$TEST_SHA" "$state"
  fi
elif [ "$1 $2" = 'pr merge' ]; then
  test "${6:-}" = "$TEST_SHA"
  tmp=$(mktemp -d)
  git clone -q -b alpha "$TEST_REMOTE" "$tmp"
  git -C "$tmp" config user.email fixture@example.invalid
  git -C "$tmp" config user.name Fixture
  git -C "$tmp" fetch -q origin "$TEST_BRANCH"
  git -C "$tmp" merge -q --squash FETCH_HEAD
  git -C "$tmp" commit -qm squash
  git -C "$tmp" push -q origin alpha
  touch "$TEST_GH_STATE"
fi
""")
        result = self.script('bs-merge.sh', '1')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.base / 'merged').exists())
        result = self.script('bs-merge.sh', '1', '--reviewed-head', '0' * 40)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.base / 'merged').exists())
        (self.root / 'a').write_text('overlap\n')
        result = self.script('bs-merge.sh', '1', '--reviewed-head', sha)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.root / 'a').read_text(), 'overlap\n')
        self.assertFalse((self.base / 'merged').exists())
        self.run_cmd('git', 'restore', 'a')
        result = self.script('bs-merge.sh', '1', '--reviewed-head', sha)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.root / 'a').read_text(), 'feature\n')
        self.assertEqual((self.root / 'b').read_text(), 'local work\n')
        self.assertTrue(lane.exists())
        self.assertEqual((lane / 'target/build-output').read_text(), 'compiled output\n')
        self.assertEqual((lane / 'ignored-data/important').read_text(), 'preserve me\n')
        self.assertIn('lane retained', result.stdout)
        self.assertIn('done 1234', (self.base / 'stint-calls').read_text())
        (lane / 'target/build-output').unlink()
        (lane / 'target').rmdir()
        (lane / 'ignored-data/important').unlink()
        (lane / 'ignored-data').rmdir()
        result = self.script('merge-pr.sh', 'cleanup', '1', branch, sha)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(lane.exists())


if __name__ == '__main__':
    unittest.main()
