"""Isolated evidence invalidation, ownership and real-Cargo cache controls."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / f'{name}.py')
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result

EVIDENCE = module('build-evidence')

class Lifecycle(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.env = {**os.environ, 'LC_ALL': 'en_US.UTF-8', 'LANG': 'en_US.UTF-8',
                    'PLEXI_BUILD_CACHE_ROOT': str(self.root / 'cache')}
        self.call('git', 'init', '-q')
        self.call('git', '-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'initial', '--allow-empty')

    def call(self, *args, **kwargs):
        return subprocess.run(args, cwd=kwargs.pop('cwd', self.repo), env=kwargs.pop('env', self.env),
                              capture_output=True, text=True, check=True, **kwargs)

    def test_artifact_rejects_mismatch_dirty_and_tamper(self):
        head = self.call('git', 'rev-parse', 'HEAD').stdout.strip()
        binary = self.root / 'plexi'
        binary.write_text('#!/bin/sh\necho \'{"source_commit":"' + head + '"}\'\n')
        binary.chmod(0o755)
        command = ['python3', str(ROOT / 'scripts/build-evidence.py')]
        env = {**self.env, 'PLEXI_BUILD_TEST_CHANNEL': 'pr-1'}
        args = [str(self.repo), head, 'pr-1', str(binary)]
        self.call(*command, 'stamp', *args, env=env)
        self.call(*command, 'check', *args, env=env)
        for name, value in [('RUSTFLAGS', '-D warnings'), ('PLEXI_BUILD_TEST_CHANNEL', 'pr-2')]:
            with self.assertRaises(subprocess.CalledProcessError):
                self.call(*command, 'check', *args, env={**env, name: value})
        (self.repo / 'dirty').write_text('dirty')
        with self.assertRaises(subprocess.CalledProcessError):
            self.call(*command, 'check', *args, env=env)
        (self.repo / 'dirty').unlink()
        binary.write_text(binary.read_text() + '# changed\n')
        with self.assertRaises(subprocess.CalledProcessError):
            self.call(*command, 'check', *args, env=env)

    def test_receipt_reuses_only_matching_success_and_log(self):
        scripts = self.repo / 'scripts'
        scripts.mkdir()
        helper = scripts / 'pr-install-pretests.sh'
        helper.write_text('echo executed >> "$FIXTURE_RUNS"\necho tests-passed\nexit "${FIXTURE_EXIT:-0}"\n')
        # A copy of the production wrapper executes its sibling, keeping fixture
        # commands isolated while exercising the actual receipt implementation.
        shutil.copy(ROOT / 'scripts/build-evidence.py', scripts)
        self.call('git', 'add', '.')
        self.call('git', '-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'fixture')
        runs = self.root / 'runs'
        env = {**self.env, 'CARGO_TARGET_DIR': str(self.root / 'target'), 'FIXTURE_RUNS': str(runs)}
        command = ['python3', str(scripts / 'build-evidence.py'), 'pretests', str(self.repo), 'base', 'scoped', 'branch', 'alpha']
        self.call(*command, env=env)
        self.call(*command, env=env)
        self.assertEqual(len(runs.read_text().splitlines()), 1)
        self.call(*command[:-1], 'other-base', env=env)
        self.assertEqual(len(runs.read_text().splitlines()), 2)
        self.call(*command, env={**env, 'RUSTFLAGS': '-D warnings'})
        self.assertEqual(len(runs.read_text().splitlines()), 3)
        receipt = self.root / 'target/.plexi-pretests.json'
        value = json.loads(receipt.read_text())
        value['result'] = 1
        receipt.write_text(json.dumps(value))
        self.call(*command, env=env)
        self.assertEqual(len(runs.read_text().splitlines()), 4)
        (self.root / 'target/.plexi-pretests.log').write_text('tampered')
        self.call(*command, env=env)
        self.assertEqual(len(runs.read_text().splitlines()), 5)
        receipt.unlink()
        with self.assertRaises(subprocess.CalledProcessError):
            self.call(*command, env={**env, 'FIXTURE_EXIT': '1'})
        self.assertFalse(receipt.exists())

    def test_slots_retain_and_reject_unowned_target(self):
        command = ['python3', str(ROOT / 'scripts/build-cache.py'), 'pr-1',
                   'python3', '-c', 'import os,pathlib; p=pathlib.Path(os.environ["CARGO_TARGET_DIR"]); print(p); (p/"keep").touch()']
        result = self.call(*command)
        target = Path(result.stdout.splitlines()[-1])
        self.assertTrue((target / 'keep').is_file())
        self.assertEqual(self.call(*command).stdout.splitlines()[-1], str(target))
        (target.parent / 'owner').write_text('unknown')
        with self.assertRaises(subprocess.CalledProcessError):
            self.call(*command)
        self.assertTrue((target / 'keep').is_file())

    def test_busy_slots_timeout_and_oversized_retention(self):
        import fcntl
        command = ['python3', str(ROOT / 'scripts/build-cache.py'), 'pr-1', 'true']
        self.call(*command)
        namespace = next((self.root / 'cache').iterdir())
        locks = [(slot / 'lock').open('a+') for slot in namespace.iterdir()]
        try:
            for lock in locks:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(subprocess.CalledProcessError):
                self.call(*command, env={**self.env, 'PLEXI_BUILD_CACHE_TIMEOUT': '0'})
        finally:
            for lock in locks:
                lock.close()
        self.call(*command, env={**self.env, 'PLEXI_BUILD_CACHE_MAX_BYTES': '0'})

    def test_lease_records_queue_execution_and_failure(self):
        env = {**self.env, 'PLEXI_CARGO_LOCK': str(self.root / 'lease'),
               'PLEXI_BUILD_TELEMETRY': str(self.root / 'telemetry')}
        env.pop('PLEXI_CARGO_LOCK_HELD', None)
        self.call('bash', str(ROOT / 'scripts/cargo-with-lease.sh'), 'true', env=env)
        events = [json.loads(line) for line in (self.root / 'telemetry').read_text().splitlines()]
        self.assertEqual([event['event'] for event in events], ['request', 'acquire', 'finish'])
        self.assertEqual(events[-1]['result'], 0)
        self.assertGreaterEqual(events[-1]['execution_seconds'], 0)
        with self.assertRaises(subprocess.CalledProcessError):
            self.call('bash', str(ROOT / 'scripts/cargo-with-lease.sh'), 'false', env=env)
        self.assertEqual(json.loads((self.root / 'telemetry').read_text().splitlines()[-1])['result'], 1)

    def test_real_cargo_warm_path_move_and_seed(self):
        (self.repo / 'Cargo.toml').write_text('[package]\nname="cache_probe"\nversion="0.1.0"\nedition="2021"\n')
        (self.repo / 'src').mkdir()
        (self.repo / 'src/main.rs').write_text('fn main() { println!("probe"); }\n')
        target = self.root / 'target'
        env = {**self.env, 'CARGO_TARGET_DIR': str(target), 'RUSTC_WRAPPER': ''}
        def build(repo, env):
            result = self.call('cargo', 'build', '--offline', '--message-format=json', cwd=repo, env=env)
            return [json.loads(line)['fresh'] for line in result.stdout.splitlines()
                    if json.loads(line).get('reason') == 'compiler-artifact']
        self.assertEqual(build(self.repo, env), [False])
        self.assertEqual(build(self.repo, env), [True])
        other = self.root / 'moved-source'
        shutil.copytree(self.repo, other)
        moved = build(other, env)
        seeded = self.root / 'seeded'
        shutil.copytree(target, seeded)
        seeded_result = build(other, {**env, 'CARGO_TARGET_DIR': str(seeded)})
        print(json.dumps({'fixture': 'small real Cargo', 'warm': True, 'moved_fresh': moved, 'seeded_fresh': seeded_result}))

if __name__ == '__main__':
    unittest.main()
