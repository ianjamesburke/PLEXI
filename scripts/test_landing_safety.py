#!/usr/bin/env python3
"""Exercise lane startup, alpha sync, and cleanup against disposable Git repos."""
import json
import os
import shutil
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
        # Generated outputs are declared through check-generated, as in the real justfile.
        (self.root / 'justfile').write_text(
            'check-generated artifact generator regen ok:\n    true\n\n'
            'check-schema: (check-generated "gen/schema.json" "cat src" "gen-schema" "ok")\n')
        (self.root / '.gitignore').write_text('ignored-data/\ntarget/\nassets/python/\n.venv/\n.pytest_cache/\n__pycache__/\n')
        for name in ('assets/keep', 'sdk/python/tests/keep', 'plugins/todo/keep', 'scripts/keep'):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('tracked')
        self.run_cmd('git', 'add', 'a', 'b', 'Cargo.toml', 'justfile', '.gitignore', 'assets', 'sdk', 'plugins', 'scripts')
        self.run_cmd('git', 'commit', '-m', 'base')
        self.run_cmd('git', 'remote', 'add', 'origin', str(self.remote))
        self.run_cmd('git', 'push', '-u', 'origin', 'alpha')
        self.bin = self.base / 'bin'
        self.bin.mkdir()
        self.env = utf8_env()
        real_just = shutil.which('just')
        if real_just is None:
            raise RuntimeError('just is required: merge-pr reads generated artifacts through just --dump')
        self.env['REAL_JUST'] = real_just
        self.env.pop('PLEXI_PANE_ID', None)
        self.env['PATH'] = str(self.bin) + os.pathsep + self.env['PATH']
        self.stub('stint', 'exit 0')
        self.stub('just', 'if [ "$1" = --justfile ]; then exec "$REAL_JUST" "$@"; fi\necho "$*" >> "$TEST_LOG"')
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

    def test_writer_entry_points_verify_lock_before_dispatch(self):
        # Stub the GitHub call to verify the inherited lock at the actual
        # dispatch boundary. No remote command or version mutation runs.
        self.env["ALPHA_LOCK_SCRIPT"] = str(SCRIPTS / "alpha_lock.py")
        self.stub("gh", 'python3 -I "$ALPHA_LOCK_SCRIPT" --verify; echo locked >&2; exit 32')
        for args in (("squash", "1", "0" * 40), ("1", "--reviewed-head", "0" * 40)):
            with self.subTest(args=args):
                result = self.script("merge-pr.sh", *args)
                self.assertEqual(result.returncode, 32, result.stdout + result.stderr)
                self.assertIn("locked", result.stderr)
        for script in ("merge-pr.sh", "release-version.sh"):
            result = self.script(script, "--alpha-lock-inner", "patch")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("alpha writer lock", result.stderr)

    def remote_commit(self, filename):
        other = self.base / 'other'
        self.run_cmd('git', 'clone', '-b', 'alpha', str(self.remote), str(other), cwd=self.base)
        self.run_cmd('git', 'config', 'user.email', 'fixture@example.invalid', cwd=other)
        self.run_cmd('git', 'config', 'user.name', 'Fixture', cwd=other)
        (other / filename).write_text('remote\n')
        self.run_cmd('git', 'add', filename, cwd=other)
        self.run_cmd('git', 'commit', '-m', 'remote', cwd=other)
        self.run_cmd('git', 'push', cwd=other)

    def full_flow_lane(self, path):
        """Pushed lane editing `path`, a stint task, and a gh stub whose squash
        is a three-way merge onto current alpha (what GitHub does to a behind PR)."""
        branch = 'feature/stint-1234-fix'
        lane = self.root / 'worktrees' / branch
        self.run_cmd('git', 'worktree', 'add', '-b', branch, str(lane))
        (lane / path).write_text('feature\n')
        self.run_cmd('git', 'commit', '-am', 'feature', cwd=lane)
        self.run_cmd('git', 'push', '-u', 'origin', branch, cwd=lane)
        sha = self.run_cmd('git', 'rev-parse', branch).stdout.strip()
        (self.root / '.stint/tasks').mkdir(parents=True)
        (self.root / '.stint/tasks/1234-fix.md').write_text('status: doing\n')
        self.env.update(TEST_BRANCH=branch, TEST_SHA=sha, TEST_REMOTE=str(self.remote),
                        TEST_GH_STATE=str(self.base / 'merged'))
        self.stub('gh', r"""
if [ "$1 $2" = 'pr view' ]; then
  if [ -f "$TEST_GH_STATE" ]; then state=MERGED; else state=OPEN; fi
  if [[ "$*" == *closingIssuesReferences* ]]; then
    printf '\n'
  elif [[ "$*" == *'--jq'* ]]; then
    printf '{"branch":"%s","state":"%s","base":"alpha","body":"Stint: 1234"}\n' "$TEST_BRANCH" "$state"
  else
    printf '{"headRefName":"%s","headRefOid":"%s","state":"%s"}\n' "$TEST_BRANCH" "$TEST_SHA" "$state"
  fi
elif [ "$1 $2" = 'pr merge' ]; then
  echo "$*" >> "$TEST_GH_MERGE_LOG"
  test "$4 $5 ${6:-}" = "--squash --match-head-commit $TEST_SHA"
  tmp=$(mktemp -d)
  git clone -q -b alpha "$TEST_REMOTE" "$tmp"
  git -C "$tmp" config user.email fixture@example.invalid
  git -C "$tmp" config user.name Fixture
  if [ -n "${TEST_MOVE_HEAD:-}" ]; then
    # Someone pushes to the PR branch after merge-pr fetched it.
    git -C "$tmp" fetch -q origin "$TEST_BRANCH"
    git -C "$tmp" checkout -q -b moved FETCH_HEAD
    echo moved > "$tmp/a"
    git -C "$tmp" commit -qam moved
    git -C "$tmp" push -q origin "moved:$TEST_BRANCH"
    git -C "$tmp" checkout -q alpha
  fi
  # GitHub refuses --match-head-commit when the PR head differs.
  head=$(git ls-remote "$TEST_REMOTE" "refs/heads/$TEST_BRANCH" | cut -f1)
  if [ "$head" != "$6" ]; then
    echo "Head branch was modified. Review and try the merge again." >&2
    exit 1
  fi
  git -C "$tmp" fetch -q origin "$TEST_BRANCH"
  git -C "$tmp" merge -q --squash FETCH_HEAD
  git -C "$tmp" commit -qm squash
  git -C "$tmp" push -q origin alpha
  touch "$TEST_GH_STATE"
fi
""")
        self.env['TEST_GH_MERGE_LOG'] = str(self.base / 'gh-merge-calls')
        return branch, lane, sha

    def test_full_merge_allows_behind_pr_with_disjoint_paths(self):
        branch, lane, sha = self.full_flow_lane('a')
        self.remote_commit('b')
        result = self.script('merge-pr.sh', '1', '--reviewed-head', sha)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('PR is behind alpha', result.stdout)
        self.assertRegex(result.stdout, r'PR paths:\n +a\n')
        self.assertRegex(result.stdout, r'alpha paths since merge base:\n +b\n')
        self.assertIn('Disjoint', result.stdout)
        self.assertEqual((self.root / 'a').read_text(), 'feature\n')
        self.assertEqual((self.root / 'b').read_text(), 'remote\n')
        self.assertFalse(lane.exists())
        self.assertEqual((self.base / 'gh-merge-calls').read_text(),
                         f'pr merge 1 --squash --match-head-commit {sha}\n')

    def test_full_merge_refuses_when_pr_head_moves_after_fetch(self):
        branch, lane, sha = self.full_flow_lane('a')
        self.env['TEST_MOVE_HEAD'] = '1'
        alpha_before = self.run_cmd('git', 'ls-remote', 'origin', 'refs/heads/alpha').stdout
        result = self.script('merge-pr.sh', '1', '--reviewed-head', sha)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Head branch was modified', result.stderr)
        self.assertFalse((self.base / 'merged').exists())
        self.assertEqual(self.run_cmd('git', 'ls-remote', 'origin', 'refs/heads/alpha').stdout, alpha_before)
        self.assertTrue(lane.exists())

    def assert_global_path_refuses(self, path):
        branch, lane, sha = self.full_flow_lane('a')
        other = self.base / 'other'
        self.run_cmd('git', 'clone', '-b', 'alpha', str(self.remote), str(other), cwd=self.base)
        (other / path).parent.mkdir(parents=True, exist_ok=True)
        (other / path).write_text('alpha change\n')
        self.run_cmd('git', 'add', path, cwd=other)
        self.run_cmd('git', '-c', 'user.email=f@example.invalid', '-c', 'user.name=F', 'commit', '-m', 'global', cwd=other)
        self.run_cmd('git', 'push', cwd=other)
        result = self.script('merge-pr.sh', '1', '--reviewed-head', sha)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('global paths', result.stderr)
        self.assertIn('\n  ' + path + '\n', result.stderr)
        self.assertFalse((self.base / 'merged').exists())

    def test_full_merge_refuses_behind_pr_when_alpha_changed_cargo_toml(self):
        self.assert_global_path_refuses('Cargo.toml')

    def test_full_merge_refuses_behind_pr_when_alpha_changed_justfile_generated_output(self):
        self.assert_global_path_refuses('gen/schema.json')

    def test_full_merge_refuses_behind_pr_when_alpha_changed_sdk_lock_inputs(self):
        self.assert_global_path_refuses('sdk/python/uv.lock')

    def test_full_merge_refuses_behind_pr_when_alpha_changed_sdk_pyproject(self):
        self.assert_global_path_refuses('sdk/python/pyproject.toml')

    def assert_bad_justfile_refuses(self, justfile, message):
        """Alpha replaces the justfile; the behind, disjoint PR must fail closed."""
        branch, lane, sha = self.full_flow_lane('a')
        other = self.base / 'other'
        self.run_cmd('git', 'clone', '-b', 'alpha', str(self.remote), str(other), cwd=self.base)
        (other / 'justfile').write_text(justfile)
        self.run_cmd('git', '-c', 'user.email=f@example.invalid', '-c', 'user.name=F', 'commit', '-am', 'justfile', cwd=other)
        self.run_cmd('git', 'push', cwd=other)
        result = self.script('merge-pr.sh', '1', '--reviewed-head', sha)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected 'just --dump' shape", result.stderr)
        self.assertIn(message, result.stderr)
        self.assertFalse((self.base / 'merged').exists())

    def test_full_merge_refuses_when_generated_artifact_is_an_expression(self):
        self.assert_bad_justfile_refuses(
            'out := "gen/schema.json"\n'
            'check-generated artifact generator regen ok:\n    true\n\n'
            'check-schema: (check-generated out "cat src" "gen-schema" "ok")\n',
            'not a string literal: ["variable","out"]')

    def test_full_merge_refuses_when_no_recipe_declares_generated_artifacts(self):
        self.assert_bad_justfile_refuses(
            'check-generated artifact generator regen ok:\n    true\n',
            'no recipe depends on check-generated')

    def test_full_merge_refuses_criss_cross_history(self):
        branch, lane, p1 = self.full_flow_lane('a')
        other = self.base / 'other'
        self.run_cmd('git', 'clone', '-b', 'alpha', str(self.remote), str(other), cwd=self.base)
        ident = ('-c', 'user.email=f@example.invalid', '-c', 'user.name=F')
        (other / 'b').write_text('alpha side\n')
        self.run_cmd('git', *ident, 'commit', '-am', 'A1', cwd=other)
        self.run_cmd('git', 'push', cwd=other)
        self.run_cmd('git', 'fetch', 'origin', branch, cwd=other)
        self.run_cmd('git', *ident, 'merge', '--no-ff', '-m', 'A2', 'FETCH_HEAD', cwd=other)
        self.run_cmd('git', 'fetch', 'origin', 'alpha', cwd=lane)
        self.run_cmd('git', 'merge', '--no-ff', '-m', 'P2', 'origin/alpha', cwd=lane)
        self.run_cmd('git', 'push', 'origin', branch, cwd=lane)
        self.run_cmd('git', 'push', cwd=other)
        p2 = self.run_cmd('git', 'rev-parse', 'HEAD', cwd=lane).stdout.strip()
        self.env['TEST_SHA'] = p2
        result = self.script('merge-pr.sh', '1', '--reviewed-head', p2)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('more than one merge base', result.stderr)
        self.assertFalse((self.base / 'merged').exists())

    def test_full_merge_refuses_behind_pr_with_overlapping_path(self):
        branch, lane, sha = self.full_flow_lane('a')
        self.remote_commit('a')
        result = self.script('merge-pr.sh', '1', '--reviewed-head', sha)
        self.assertNotEqual(result.returncode, 0)
        self.assertRegex(result.stderr, r'both changed these paths.*\n +a\n')
        self.assertFalse((self.base / 'merged').exists())
        self.assertTrue(lane.exists())

    def test_full_merge_refuses_behind_pr_when_alpha_renamed_its_path(self):
        branch, lane, sha = self.full_flow_lane('a')
        other = self.base / 'other'
        self.run_cmd('git', 'clone', '-b', 'alpha', str(self.remote), str(other), cwd=self.base)
        self.run_cmd('git', '-c', 'user.email=f@example.invalid', '-c', 'user.name=F', 'mv', 'a', 'moved', cwd=other)
        self.run_cmd('git', '-c', 'user.email=f@example.invalid', '-c', 'user.name=F', 'commit', '-m', 'rename', cwd=other)
        self.run_cmd('git', 'push', cwd=other)
        result = self.script('merge-pr.sh', '1', '--reviewed-head', sha)
        self.assertNotEqual(result.returncode, 0)
        self.assertRegex(result.stderr, r'both changed these paths.*\n +a\n')
        self.assertFalse((self.base / 'merged').exists())

    def test_cleanup_accepts_server_side_rebased_lane_only(self):
        branch = 'feature/stint-1234-fix'
        lane = self.root / 'worktrees' / branch
        self.run_cmd('git', 'worktree', 'add', '-b', branch, str(lane))
        (lane / 'a').write_text('feature\n')
        self.run_cmd('git', 'commit', '-am', 'feature', cwd=lane)
        self.run_cmd('git', 'push', '-u', 'origin', branch, cwd=lane)
        reviewed = self.run_cmd('git', 'rev-parse', branch).stdout.strip()
        # Server-side rebase: GitHub rewrites the remote branch; the lane keeps `reviewed`.
        self.remote_commit('b')
        other = self.base / 'other'
        self.run_cmd('git', 'fetch', 'origin', branch, cwd=other)
        self.run_cmd('git', 'checkout', '-b', branch, 'FETCH_HEAD', cwd=other)
        self.run_cmd('git', 'rebase', 'alpha', cwd=other)
        self.run_cmd('git', 'push', '--force', 'origin', branch, cwd=other)
        rebased = self.run_cmd('git', 'rev-parse', 'HEAD', cwd=other).stdout.strip()
        self.assertNotEqual(reviewed, rebased)
        # Shape recorded from the real GitHub GraphQL API (PR timeline, HeadRefForcePushedEvent).
        events = {'data': {'repository': {'pullRequest': {'timelineItems': {'nodes': [
            {'beforeCommit': {'oid': reviewed}, 'afterCommit': {'oid': rebased}}]}}}}}
        self.env['TEST_PR'] = json.dumps({'state': 'MERGED', 'headRefName': branch, 'headRefOid': rebased})
        self.env['TEST_EVENTS'] = json.dumps(events)
        self.stub('gh', 'if [ "$1 $2" = "api graphql" ]; then printf "%s\\n" "$TEST_EVENTS"; else printf "%s\\n" "$TEST_PR"; fi')
        # A lane at a commit that was never a PR head is still refused.
        (lane / 'a').write_text('unpushed\n')
        self.run_cmd('git', 'commit', '-am', 'unpushed', cwd=lane)
        result = self.script('merge-pr.sh', 'cleanup', '1', branch, rebased)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('lane branch has moved', result.stderr)
        self.assertTrue(lane.exists())
        self.run_cmd('git', 'reset', '--hard', reviewed, cwd=lane)
        result = self.script('merge-pr.sh', 'cleanup', '1', branch, rebased)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('pre-rebase head', result.stdout)
        self.assertEqual(self.run_cmd('git', 'rev-parse', f'refs/plexi-preserved/pr-1/{reviewed}').stdout.strip(), reviewed)
        self.assertFalse(lane.exists())
        self.assertEqual(self.run_cmd('git', 'ls-remote', '--heads', 'origin', branch).stdout, '')

    def merged_lane_with_force_push(self, drop_commit):
        """Lane pushed at `old`; the remote branch is then force-pushed to `new`
        (a rebase, or a replacement that drops the lane's last commit)."""
        branch = 'feature/stint-1234-fix'
        lane = self.root / 'worktrees' / branch
        self.run_cmd('git', 'worktree', 'add', '-b', branch, str(lane))
        (lane / 'a').write_text('feature\n')
        self.run_cmd('git', 'commit', '-am', 'feature', cwd=lane)
        (lane / 'b').write_text('valuable\n')
        self.run_cmd('git', 'commit', '-am', 'valuable', cwd=lane)
        self.run_cmd('git', 'push', '-u', 'origin', branch, cwd=lane)
        old = self.run_cmd('git', 'rev-parse', branch).stdout.strip()
        other = self.base / 'other'
        self.run_cmd('git', 'clone', str(self.remote), str(other), cwd=self.base)
        self.run_cmd('git', 'checkout', '-b', 'replacement', f'origin/{branch}' + ('~1' if drop_commit else ''), cwd=other)
        (other / 'c').write_text('replacement\n')
        self.run_cmd('git', 'add', 'c', cwd=other)
        self.run_cmd('git', '-c', 'user.email=f@example.invalid', '-c', 'user.name=F', 'commit', '-m', 'replacement', cwd=other)
        self.run_cmd('git', 'push', '--force', 'origin', f'replacement:{branch}', cwd=other)
        new = self.run_cmd('git', 'rev-parse', 'HEAD', cwd=other).stdout.strip()
        # Shape recorded from the real GitHub GraphQL API (PR timeline, HeadRefForcePushedEvent).
        events = {'data': {'repository': {'pullRequest': {'timelineItems': {'nodes': [
            {'beforeCommit': {'oid': old}, 'afterCommit': {'oid': new}}]}}}}}
        self.env['TEST_PR'] = json.dumps({'state': 'MERGED', 'headRefName': branch, 'headRefOid': new})
        self.env['TEST_EVENTS'] = json.dumps(events)
        self.stub('gh', 'if [ "$1 $2" = "api graphql" ]; then printf "%s\\n" "$TEST_EVENTS"; else printf "%s\\n" "$TEST_PR"; fi')
        return branch, lane, old, new

    def test_cleanup_preserves_lane_head_dropped_by_force_push(self):
        branch, lane, old, new = self.merged_lane_with_force_push(drop_commit=True)
        result = self.script('merge-pr.sh', 'cleanup', '1', branch, new)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f'refs/plexi-preserved/pr-1/{old}', result.stdout)
        self.assertFalse(lane.exists())
        self.assertEqual(self.run_cmd('git', 'show', f'refs/plexi-preserved/pr-1/{old}:b').stdout, 'valuable\n')

    def test_cleanup_keeps_branch_committed_to_during_cleanup(self):
        branch, lane, old, new = self.merged_lane_with_force_push(drop_commit=False)
        self.env['TEST_RACE_LANE'] = str(lane)
        self.stub('just', 'if [ "$1" = --justfile ]; then exec "$REAL_JUST" "$@"; fi\n'
                          'if [ "$1" = channel-clean ]; then git -C "$TEST_RACE_LANE" -c user.email=f@example.invalid '
                          '-c user.name=F commit -q --allow-empty -m race; fi')
        result = self.script('merge-pr.sh', 'cleanup', '1', branch, new)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('moved during cleanup', result.stderr)
        raced = self.run_cmd('git', 'rev-parse', branch).stdout.strip()
        self.assertEqual(self.run_cmd('git', 'rev-parse', f'{raced}~1').stdout.strip(), old)
        self.assertNotEqual(self.run_cmd('git', 'ls-remote', '--heads', 'origin', branch).stdout, '')

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
                     'plugins/todo/__pycache__'):
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
