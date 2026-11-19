"""A shared build slot never serves an artifact compiled from other sources (stint 1630).

Cargo decides freshness of path packages by comparing source mtimes against the
slot's outputs, keyed by paths relative to the package root. Two worktrees share
a slot, so a tree whose differing file is older on disk than the slot's last
build reuses the other tree's artifact. These tests drive the real slot runner,
lease wrapper and Cargo against two worktrees of a fixture repository.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

from fixture_env import utf8_env

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('build_cache', ROOT / 'scripts' / 'build-cache.py')
CACHE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(CACHE)

OLD = 1_000_000_000  # 2001: older than any build output.


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def age(tree, seconds):
    """Set every tracked file's mtime to `seconds`, as an old checkout would."""
    for name in subprocess.check_output(['git', 'ls-files', '-z'], cwd=tree, text=True).split('\0'):
        if name:
            os.utime(tree / name, (seconds, seconds))


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.repo = self.root / 'repo'
        self.env = {**utf8_env(), 'PLEXI_BUILD_CACHE_ROOT': str(self.root / 'cache'),
                    'PLEXI_BUILD_MIN_FREE_BYTES': '1', 'PLEXI_CARGO_LOCK': str(self.root / 'lease/lock'),
                    'CARGO_TERM_COLOR': 'never'}
        for name in ('CARGO_TARGET_DIR', 'PLEXI_BUILD_CACHE_SLOT', 'PLEXI_BUILD_CACHE_RUN',
                     'PLEXI_BUILD_CACHE_OWNER', 'PLEXI_BUILD_TEST_CHANNEL', 'PLEXI_BUILD_TELEMETRY'):
            self.env.pop(name, None)
        write(self.repo / 'Cargo.toml', '[workspace]\nmembers = ["app", "crates/lib"]\nresolver = "2"\n')
        write(self.repo / 'app/Cargo.toml', '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n\n'
              '[dependencies]\nlib = { path = "../crates/lib" }\n')
        write(self.repo / 'app/src/main.rs', 'fn main() { println!("{}", lib::SOURCE); }\n')
        write(self.repo / 'crates/lib/Cargo.toml', '[package]\nname = "lib"\nversion = "0.1.0"\nedition = "2021"\n')
        write(self.repo / 'crates/lib/src/lib.rs', 'pub const SOURCE: &str = "one";\n')
        for script in ('build-cache.py', 'cargo-with-lease.sh'):
            write(self.repo / 'scripts' / script, (ROOT / 'scripts' / script).read_text())
        self.git('init', '-q', '-b', 'main')
        self.call('cargo', 'generate-lockfile', cwd=self.repo)
        self.git('add', '-A')
        self.git('commit', '-qm', 'one')
        self.one = self.git('rev-parse', 'HEAD')
        write(self.repo / 'crates/lib/src/lib.rs', 'pub const SOURCE: &str = "two";\n')
        self.git('commit', '-qam', 'two')
        self.two = self.git('rev-parse', 'HEAD')

    def call(self, *args, cwd, env=None):
        return subprocess.run(args, cwd=cwd, env=env or self.env, capture_output=True, text=True, check=True)

    def git(self, *args, cwd=None):
        return self.call('git', '-c', 'user.email=t@t', '-c', 'user.name=t', *args, cwd=cwd or self.repo).stdout.strip()

    def worktree(self, name, commit):
        path = self.root / name
        self.git('worktree', 'add', '-q', '--detach', str(path), commit)
        return path

    def build(self, tree):
        """Build `tree` in the task slot exactly as `just` recipes do; return what the binary prints."""
        result = self.call('python3', 'scripts/build-cache.py', 'task-1630', 'bash', 'scripts/cargo-with-lease.sh',
                           'cargo', 'build', '--bin', 'app', cwd=tree)
        target = Path(next(line.split('slot=')[1].split()[0] for line in result.stdout.splitlines()
                           if line.startswith('build-cache: slot='))) / 'target'
        printed = subprocess.run([str(target / 'debug/app')], capture_output=True, text=True, check=True).stdout
        return printed.strip(), result.stdout + result.stderr


class SharedSlotTest(Fixture):
    def test_older_checkout_of_other_sources_is_rebuilt_not_reused(self):
        # PR #3245: the slot last compiled a newer tree; the next tree's copy of
        # the changed file is older on disk than that build.
        newer = self.worktree('newer', self.two)
        older = self.worktree('older', self.one)
        age(older, OLD)
        age(newer, OLD + 60)
        self.assertEqual(self.build(newer)[0], 'two')
        printed, log = self.build(older)
        self.assertEqual(printed, 'one', 'slot served the other tree\'s artifact:\n' + log)
        self.assertIn('build-cache: sources tree=', log)
        # Alternating back is rebuilt too, and an unchanged tree is a cache hit.
        self.assertEqual(self.build(newer)[0], 'two')
        printed, log = self.build(newer)
        self.assertEqual(printed, 'two')
        self.assertNotIn('Compiling', log)

    def test_identical_sources_in_another_worktree_reuse_the_slot(self):
        first = self.worktree('first', self.two)
        second = self.worktree('second', self.two)
        age(first, OLD)
        age(second, OLD)
        self.build(first)
        printed, log = self.build(second)
        self.assertEqual(printed, 'two')
        self.assertNotIn('Compiling lib', log)
        self.assertIn('changed=0', log)


class StampTest(Fixture):
    """The per-file stamps directly: what is raised, what is kept, what is forgotten."""

    def setUp(self):
        super().setUp()
        self.slot = self.root / 'slot'
        self.slot.mkdir()
        (self.slot / 'owner').write_text(CACHE.MARKER)

    def manifest(self):
        return json.loads((self.slot / CACHE.SOURCES).read_text())

    def test_first_registration_raises_every_file_and_a_match_keeps_its_stamp(self):
        tree = self.worktree('tree', self.two)
        age(tree, OLD)
        before = time.time_ns()
        changed = CACHE.sync_sources(self.slot, tree, 'run-1')
        self.assertEqual(changed, len(self.manifest()['files']))
        lib = tree / 'crates/lib/src/lib.rs'
        stamp = lib.stat().st_mtime_ns
        self.assertGreaterEqual(stamp, before)
        # Same content on the next registration: nothing is raised past its stamp.
        age(tree, OLD)
        self.assertEqual(CACHE.sync_sources(self.slot, tree, 'run-1'), 0)
        self.assertEqual(lib.stat().st_mtime_ns, stamp)

    def test_differing_content_is_raised_and_mtimes_never_move_back(self):
        newer = self.worktree('newer', self.two)
        older = self.worktree('older', self.one)
        CACHE.sync_sources(self.slot, newer, 'run-1')
        CACHE.settle_sources(self.slot, 'run-1')
        age(older, OLD)
        main = older / 'app/src/main.rs'
        future = time.time_ns() + 10**12
        os.utime(main, ns=(future, future))
        self.assertEqual(CACHE.sync_sources(self.slot, older, 'run-2'), 1)
        self.assertGreater((older / 'crates/lib/src/lib.rs').stat().st_mtime_ns, OLD * 10**9)
        self.assertEqual(main.stat().st_mtime_ns, future)

    def test_an_edit_during_the_run_forgets_that_file(self):
        tree = self.worktree('tree', self.two)
        CACHE.sync_sources(self.slot, tree, 'run-1')
        write(tree / 'crates/lib/src/lib.rs', 'pub const SOURCE: &str = "edited";\n')
        CACHE.settle_sources(self.slot, 'run-1')
        self.assertNotIn('crates/lib/src/lib.rs', self.manifest()['files'])
        self.assertIn('app/src/main.rs', self.manifest()['files'])

    def test_a_run_that_never_registered_resets_the_slot_sources(self):
        tree = self.worktree('tree', self.two)
        CACHE.sync_sources(self.slot, tree, 'run-1')
        CACHE.settle_sources(self.slot, 'run-1')
        # run-2 compiled through a lease wrapper that does not register sources.
        CACHE.settle_sources(self.slot, 'run-2')
        self.assertFalse((self.slot / CACHE.SOURCES).exists())

    def test_an_unsettled_earlier_run_resets_before_registering(self):
        tree = self.worktree('tree', self.two)
        CACHE.sync_sources(self.slot, tree, 'run-1')  # killed: never settled
        age(tree, OLD)
        self.assertEqual(CACHE.sync_sources(self.slot, tree, 'run-2'), len(self.manifest()['files']))

    def test_a_file_missing_from_the_tree_is_forgotten(self):
        tree = self.worktree('tree', self.two)
        CACHE.sync_sources(self.slot, tree, 'run-1')
        self.git('rm', '-q', 'crates/lib/src/lib.rs', cwd=tree)
        CACHE.sync_sources(self.slot, tree, 'run-1')
        self.assertNotIn('crates/lib/src/lib.rs', self.manifest()['files'])

    def test_sync_refuses_outside_a_slot_run(self):
        tree = self.worktree('tree', self.two)
        env = {k: v for k, v in self.env.items() if k != 'PLEXI_BUILD_CACHE_SLOT'}
        result = subprocess.run(['python3', 'scripts/build-cache.py', 'sync-sources'], cwd=tree, env=env,
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('PLEXI_BUILD_CACHE_SLOT', result.stderr)


if __name__ == '__main__':
    if shutil.which('cargo') is None:
        raise SystemExit('test_build_cache_sources: cargo is required (real Cargo fingerprints are under test)')
    unittest.main()
