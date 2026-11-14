"""Isolated evidence invalidation, ownership and real-Cargo cache controls."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

from fixture_env import utf8_env

ROOT = Path(__file__).resolve().parents[1]

def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / f'{name}.py')
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result

EVIDENCE = module('build-evidence')
CACHE = module('build-cache')

class Lifecycle(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.env = {**utf8_env(), 'PLEXI_BUILD_CACHE_ROOT': str(self.root / 'cache')}
        self.call('git', 'init', '-q')
        self.call('git', '-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'initial', '--allow-empty')

    def call(self, *args, **kwargs):
        return subprocess.run(args, cwd=kwargs.pop('cwd', self.repo), env=kwargs.pop('env', self.env),
                              capture_output=True, text=True, check=True, **kwargs)

    def test_artifact_rejects_mismatch_dirty_and_tamper(self):
        head = self.call('git', 'rev-parse', 'HEAD').stdout.strip()
        binary = self.root / 'plexi'
        binary.write_text('#!/bin/sh\necho \'{"source_commit":"' + head + '","test_channel":"pr-1"}\'\n')
        binary.chmod(0o755)
        command = ['python3', str(ROOT / 'scripts/build-evidence.py')]
        env = {**self.env, 'PLEXI_BUILD_TEST_CHANNEL': 'pr-1'}
        args = [str(self.repo), head, 'pr-1', str(binary)]
        self.call(*command, 'stamp', *args, env=env)
        self.call(*command, 'check', *args, env=env)
        manifest = binary.parent / '.plexi-candidate.json'
        original = manifest.read_text()
        for field in ('schema', 'toolchain', 'configs', 'head'):
            value = json.loads(original)
            value['identity'][field] = 'mismatch'
            manifest.write_text(json.dumps(value))
            with self.assertRaises(subprocess.CalledProcessError):
                self.call(*command, 'check', *args, env=env)
        manifest.write_text(original)
        for name, value in [('RUSTFLAGS', '-D warnings -C debuginfo=0'), ('PLEXI_BUILD_TEST_CHANNEL', 'pr-2')]:
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
        env = {**self.env, 'CARGO_TARGET_DIR': str(self.root / 'target'), 'FIXTURE_RUNS': str(runs),
               'PLEXI_PR_REQUIRED_PTY_FILTERS': 'none', 'PLEXI_PR_REQUIRED_SCENES': 'none'}
        command = ['python3', str(scripts / 'build-evidence.py'), 'pretests', str(self.repo), 'base', 'scoped', 'branch', 'alpha']
        self.call(*command, env=env)
        self.call(*command, env=env)
        self.assertEqual(len(runs.read_text().splitlines()), 1)
        with self.assertRaises(subprocess.CalledProcessError):
            self.call(*command, env={**env, 'PLEXI_PR_REQUIRED_SCENES': ''})
        with self.assertRaises(subprocess.CalledProcessError):
            self.call(*command, env={**env, 'PLEXI_PR_REQUIRED_SCENES': '  '})
        self.assertEqual(len(runs.read_text().splitlines()), 1)
        self.call(*command[:-1], 'other-base', env=env)
        self.assertEqual(len(runs.read_text().splitlines()), 2)
        self.call(*command, env={**env, 'RUSTFLAGS': '-D warnings'})
        self.assertEqual(len(runs.read_text().splitlines()), 3)
        self.call(*command, env={**env, 'PLEXI_PR_REQUIRED_SCENES': 'tests/scenes/example.toml'})
        self.assertEqual(len(runs.read_text().splitlines()), 4)
        receipt = self.root / 'target/.plexi-pretests.json'
        value = json.loads(receipt.read_text())
        value['result'] = 1
        receipt.write_text(json.dumps(value))
        self.call(*command, env=env)
        self.assertEqual(len(runs.read_text().splitlines()), 5)
        (self.root / 'target/.plexi-pretests.log').write_text('tampered')
        self.call(*command, env=env)
        self.assertEqual(len(runs.read_text().splitlines()), 6)
        receipt.unlink()
        with self.assertRaises(subprocess.CalledProcessError):
            self.call(*command, env={**env, 'FIXTURE_EXIT': '1'})
        self.assertFalse(receipt.exists())

    def test_sdk_receipt_canonicalizes_interpreter_alias_but_not_inputs(self):
        scripts = self.repo / 'scripts'
        scripts.mkdir()
        shutil.copy(ROOT / 'scripts/build-evidence.py', scripts)
        (scripts / 'pr-install-pretests.sh').write_text('echo run >> "$FIXTURE_RUNS"\n')
        project = self.repo / 'sdk/python'
        project.mkdir(parents=True)
        (project / 'pyproject.toml').write_text('[project]\nname="fixture"\nversion="1"\n')
        self.call('git', 'add', '.')
        self.call('git', '-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'fixture')

        executables = self.root / 'interpreters'
        executables.mkdir()
        (executables / 'actual').write_text('interpreter one')
        (executables / 'other').write_text('interpreter two')
        (executables / 'python').symlink_to('actual')
        (executables / 'python3').symlink_to('actual')
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        uv = bin_dir / 'uv'
        uv.write_text('#!/usr/bin/env python3\n'
                      'import os, sys, types, importlib.metadata\n'
                      'if sys.argv[1:] == ["--version"]: print("uv fixture"); sys.exit()\n'
                      'sys.executable = os.environ["FIXTURE_PYTHON"]\n'
                      'sys.version = os.environ.get("FIXTURE_VERSION", "version one")\n'
                      'importlib.metadata.distributions = lambda: [types.SimpleNamespace(metadata={"Name":"fixture"}, version=os.environ.get("FIXTURE_PACKAGE", "1"))]\n'
                      'exec(sys.argv[-1])\n')
        uv.chmod(0o755)
        runs = self.root / 'runs'
        env = {**self.env, 'PATH': f'{bin_dir}:{self.env["PATH"]}',
               'CARGO_TARGET_DIR': str(self.root / 'target'), 'FIXTURE_RUNS': str(runs),
               'FIXTURE_PYTHON': str(executables / 'python'),
               'PLEXI_PR_REQUIRED_PTY_FILTERS': 'none', 'PLEXI_PR_REQUIRED_SCENES': 'none'}
        command = ['python3', str(scripts / 'build-evidence.py'), 'pretests', str(self.repo), 'base', 'scoped', 'branch', 'alpha']
        self.call(*command, env=env)
        self.call(*command, env={**env, 'FIXTURE_PYTHON': str(executables / 'python3')})
        self.assertEqual(len(runs.read_text().splitlines()), 1, 'equivalent aliases should reuse receipt')
        for changed in ({'FIXTURE_PYTHON': str(executables / 'other')},
                        {'FIXTURE_VERSION': 'version two'}, {'FIXTURE_PACKAGE': '2'}):
            self.call(*command, env={**env, **changed})
            count = len(runs.read_text().splitlines())
            self.call(*command, env=env)
            self.assertEqual(len(runs.read_text().splitlines()), count + 1)
        (executables / 'python').unlink()
        (executables / 'python').symlink_to('other')
        count = len(runs.read_text().splitlines())
        self.call(*command, env=env)
        self.assertEqual(len(runs.read_text().splitlines()), count + 1, 'retargeted interpreter invalidates receipt')
        self.call(*command, env={**env, 'FIXTURE_PYTHON': ''})
        self.assertFalse((self.root / 'target/.plexi-pretests.json').exists())

    def test_install_leaf_refuses_unverified_reuse_flag(self):
        scripts = self.repo / 'scripts'
        scripts.mkdir()
        for name in ('install.sh', 'build-evidence.py'):
            shutil.copy(ROOT / 'scripts' / name, scripts)
        self.call('git', 'add', '.')
        self.call('git', '-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'leaf')
        target = self.root / 'target'
        (target / 'release').mkdir(parents=True)
        binary = target / 'release/plexi'
        binary.write_text("#!/bin/sh\necho '{}'\n")
        binary.chmod(0o755)
        commands = self.root / 'commands'
        commands.mkdir()
        fake_cargo = commands / 'cargo'
        fake_cargo.write_text('#!/bin/sh\nif [ "$1" = metadata ]; then\n'
                              + "echo '" + json.dumps({'target_directory': str(target)}) + "'\n"
                              + 'else echo cargo-fixture; fi\n')
        fake_uv = commands / 'uv'
        fake_uv.write_text('#!/bin/sh\nif [ "$1" = --version ]; then echo uv-fixture; exit; fi\n'
                           'while [ "$1" != python ]; do shift; done\nshift\nexec python3 "$@"\n')
        fake_just = commands / 'just'
        fake_just.write_text('#!/bin/sh\necho should-not-run > "$FIXTURE_JUST"\nexit 99\n')
        for executable in (fake_cargo, fake_uv, fake_just):
            executable.chmod(0o755)
        env = {**self.env, 'PATH': str(commands) + ':' + self.env['PATH'],
               'PLEXI_REUSE_RELEASE_BUILD': '1', 'FIXTURE_JUST': str(self.root / 'just-ran'),
               'PLEXI_BUILD_EVIDENCE': str(ROOT / 'scripts/build-evidence.py')}
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            self.call('bash', str(scripts / 'install.sh'), '--from-source', 'pr-1', env=env)
        self.assertIn('compiled PR channel mismatch', raised.exception.stderr)
        self.assertFalse((self.root / 'just-ran').exists())
        # Positive route with a real driver receipt: the PR copy lacks the
        # driver's sibling harness files, so using that copy would reject it.
        native = self.root / 'installer'
        native.write_text('#!/bin/sh\necho installed > "$FIXTURE_INSTALLED"\n')
        native.chmod(0o755)
        (scripts / 'distribution-tool.sh').write_text('printf "%s\\n" "' + str(native) + '"\n')
        fake_uv.write_text(fake_uv.read_text().replace('exec python3 "$@"',
                            'if [ "$1" = scripts/package.py ]; then exit 0; fi\nexec python3 "$@"'))
        security = commands / 'security'
        security.write_text('#!/bin/sh\nexit 1\n')
        security.chmod(0o755)
        self.call('git', 'add', '.')
        self.call('git', '-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'packaging fixture')
        head = self.call('git', 'rev-parse', 'HEAD').stdout.strip()
        binary.write_text("#!/bin/sh\necho '" + json.dumps({'source_commit': head, 'test_channel': 'pr-1'}) + "'\n")
        env.update({'PLEXI_BUILD_TEST_CHANNEL': 'pr-1', 'FIXTURE_INSTALLED': str(self.root / 'installed')})
        self.call('python3', str(ROOT / 'scripts/build-evidence.py'), 'stamp', str(self.repo), head,
                  'pr-1', str(binary), env=env)
        missing = dict(env)
        missing.pop('PLEXI_BUILD_EVIDENCE')
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            self.call('bash', str(scripts / 'install.sh'), '--from-source', 'pr-1', env=missing)
        self.assertIn('requires the driver evidence checker', raised.exception.stderr)
        self.call('bash', str(scripts / 'install.sh'), '--from-source', 'pr-1', env=env)
        self.assertTrue((self.root / 'installed').exists())
        self.assertFalse((self.root / 'just-ran').exists())

    def test_custom_cargo_home_and_driver_are_fingerprinted(self):
        from unittest.mock import patch
        custom = self.root / 'custom-cargo'
        custom.mkdir()
        config = custom / 'config.toml'
        config.write_text('[build]\njobs=1\n')
        with patch.dict(os.environ, {'CARGO_HOME': str(custom)}):
            first = EVIDENCE.identity(self.repo, [])
            self.assertEqual(first['configs'][str(config)], EVIDENCE.digest(config))
            self.assertIn('pr-install-pretests.sh', first['driver'])
            config.write_text('[build]\njobs=2\n')
            self.assertNotEqual(first, EVIDENCE.identity(self.repo, []))

    def test_actual_install_dispatch_preserves_reuse_and_leaf_leases(self):
        recipe = (ROOT / 'justfile').read_text()
        dispatch = recipe[recipe.index('    if rg -q '):].split('\n    bash "$_driver_root/scripts/pr-install-seed-profile.sh"', 1)[0]
        dispatch = dispatch.replace('{{number}}', '1')
        scripts = self.repo / 'scripts'
        scripts.mkdir()
        wrapper = scripts / 'cargo-with-lease.sh'
        wrapper.write_text('#!/bin/bash\necho outer >> "$FIXTURE_OUTER"\nexec "$@"\n')
        wrapper.chmod(0o755)
        installed = self.root / 'installed'
        outer = self.root / 'outer'
        env = {**self.env, 'FIXTURE_INSTALLED': str(installed), 'FIXTURE_OUTER': str(outer)}
        # Execute the production dispatch for each installer generation. The
        # current installer delegates leases through distribution-tool.sh;
        # older leaf wrappers name cargo-with-lease.sh directly.
        for marker, expects_outer in [('distribution-tool.sh', False), ('cargo-with-lease.sh', False), ('legacy', True)]:
            with self.subTest(marker=marker):
                installed.unlink(missing_ok=True)
                outer.unlink(missing_ok=True)
                (scripts / 'install.sh').write_text('#!/bin/bash\n# ' + marker + '\n'
                    'test "${PLEXI_REUSE_RELEASE_BUILD:-}" = 1 || exit 91\n'
                    'printf "%s\\n" "$*" > "$FIXTURE_INSTALLED"\n')
                self.call('bash', '-c', '_wt="$1"; _driver_root="$1"; _reuse_env=1;\n' + dispatch,
                          'fixture', str(self.repo), env=env)
                self.assertEqual(installed.read_text().strip(), '--from-source pr-1')
                self.assertEqual(outer.exists(), expects_outer)

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

    def test_task_owner_carries_slot_without_claiming_pr_identity(self):
        script = str(ROOT / 'scripts/build-cache.py')
        probe = ['python3', '-c', 'import os; print(os.environ["CARGO_TARGET_DIR"], os.environ.get("PLEXI_BUILD_TEST_CHANNEL", "none"))']
        task = self.call('python3', script, 'task-1164', *probe,
                         env={**self.env, 'PLEXI_BUILD_TEST_CHANNEL': 'pr-999'}).stdout.splitlines()[-1].split()
        candidate = self.call('python3', script, 'pr-42', *probe,
                              env={**self.env, 'PLEXI_BUILD_CACHE_OWNER': 'task-1164'}).stdout.splitlines()[-1].split()
        self.assertEqual(task[0], candidate[0])
        self.assertEqual(task[1], 'none')
        self.assertEqual(candidate[1], 'pr-42')
        with self.assertRaises(subprocess.CalledProcessError):
            self.call('python3', script, 'pr-42', 'true', env={**self.env, 'PLEXI_BUILD_CACHE_OWNER': '../bad'})
        with self.assertRaises(subprocess.CalledProcessError):
            self.call('python3', script, 'pr-²', 'true')

    def test_busy_slots_timeout_and_oversized_retention(self):
        import fcntl
        command = ['python3', str(ROOT / 'scripts/build-cache.py'), 'pr-1', 'true']
        self.call(*command)
        namespace = next((self.root / 'cache').iterdir())
        locks = [(slot / 'lock').open('a+') for slot in namespace.iterdir() if slot.is_dir()]
        try:
            for lock in locks:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(subprocess.CalledProcessError):
                self.call(*command, env={**self.env, 'PLEXI_BUILD_CACHE_TIMEOUT': '0'})
        finally:
            for lock in locks:
                lock.close()
        self.call(*command, env={**self.env, 'PLEXI_BUILD_CACHE_MAX_BYTES': '0'})

    def test_free_space_floor_refuses_before_the_build_command(self):
        # A floor above any real disk forces the refusal without filling one.
        marker = self.root / 'built'
        command = ['python3', str(ROOT / 'scripts/build-cache.py'), 'pr-1',
                   'python3', '-c', f'import pathlib; pathlib.Path({str(marker)!r}).touch()']
        result = subprocess.run(command, cwd=self.repo, capture_output=True, text=True,
                                env={**self.env, 'PLEXI_BUILD_MIN_FREE_BYTES': str(10**18)})
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(marker.exists(), 'the build command must not start')
        self.assertIn('free', result.stderr)
        self.assertIn('floor', result.stderr)
        self.assertIn('just clean-stale-targets', result.stderr)

    def test_free_space_floor_names_volume_free_and_floor(self):
        with self.assertRaises(CACHE.DiskFloorError) as raised:
            CACHE.require_free_space(self.root, floor=12 * 1024**3, free=3 * 1024**3)
        message = str(raised.exception)
        self.assertIn(str(self.root), message)
        self.assertIn('3.0 GiB free', message)
        self.assertIn('floor 12.0 GiB', message)
        CACHE.require_free_space(self.root, floor=12 * 1024**3, free=13 * 1024**3)

    def _slot(self, namespace, index, payload, age, artifact=True):
        slot = self.root / 'cache' / namespace / f'slot-{index}'
        (slot / 'target' / 'debug').mkdir(parents=True)
        (slot / 'owner').write_text(CACHE.MARKER)
        (slot / 'target' / 'debug' / 'deps.bin').write_bytes(b'x' * payload)
        if artifact:
            (slot / 'target' / 'release').mkdir()
            (slot / 'target' / 'release' / 'plexi').write_bytes(b'b' * 10)
            (slot / 'target' / 'release' / '.plexi-candidate.json').write_text('{}')
        record = slot / 'last-channel'
        record.write_text(f'pr-{index}')
        os.utime(record, (age, age))
        return slot

    def test_total_cap_prunes_oldest_unleased_slots_and_keeps_artifacts(self):
        import fcntl
        oldest = self._slot('ns-a', 0, 1000, 100)
        leased = self._slot('ns-a', 1, 1000, 50)
        newer = self._slot('ns-b', 0, 1000, 200)
        current = self._slot('ns-b', 1, 1000, 300)
        lock = (leased / 'lock').open('a+')
        self.addCleanup(lock.close)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        CACHE.prune_to_cap(self.root / 'cache', cap=3100, keep=current)
        self.assertFalse((oldest / 'target' / 'debug').exists(), 'oldest unleased slot pruned first')
        self.assertTrue((leased / 'target' / 'debug' / 'deps.bin').exists(), 'a live lease is never pruned')
        self.assertTrue((newer / 'target' / 'debug' / 'deps.bin').exists(), 'stops once under the cap')
        self.assertTrue((current / 'target' / 'debug' / 'deps.bin').exists())
        for slot in (oldest, leased, newer, current):
            self.assertTrue((slot / 'target' / 'release' / 'plexi').exists(), 'stamped artifacts survive')
            self.assertTrue((slot / 'target' / 'release' / '.plexi-candidate.json').exists(),
                            'the stamp build-evidence reads survives with its binary')

    def test_default_cap_fits_two_warm_lanes_and_floor_stays_12_gib(self):
        self.assertEqual(CACHE.DEFAULT_TOTAL_MAX_BYTES, 64 * 1024**3)
        self.assertEqual(CACHE.DEFAULT_MIN_FREE_BYTES, 12 * 1024**3)

    def test_oversized_slot_keeps_its_release_artifact(self):
        slot = self._slot('ns-a', 0, 1000, 100)
        CACHE.prune_slot(slot)
        self.assertFalse((slot / 'target' / 'debug').exists())
        self.assertEqual((slot / 'target' / 'release' / 'plexi').read_bytes(), b'b' * 10)
        self.assertEqual((slot / 'target' / 'release' / '.plexi-candidate.json').read_text(), '{}')

    def test_prune_skips_a_slot_reached_through_a_symlinked_namespace(self):
        outside = self.root / 'outside'
        foreign = outside / 'slot-0'
        (foreign / 'target' / 'debug').mkdir(parents=True)
        (foreign / 'owner').write_text(CACHE.MARKER)
        (foreign / 'target' / 'debug' / 'deps.bin').write_bytes(b'x' * 1000)
        (self.root / 'cache').mkdir(exist_ok=True)
        (self.root / 'cache' / 'ns-link').symlink_to(outside)
        current = self._slot('ns-a', 0, 10, 300)
        CACHE.prune_to_cap(self.root / 'cache', cap=1, keep=current)
        self.assertTrue((foreign / 'target' / 'debug' / 'deps.bin').exists(),
                        'nothing outside the cache root is pruned')

    def test_invalid_overrides_are_rejected_by_name_before_the_command(self):
        marker = self.root / 'built'
        command = ['python3', str(ROOT / 'scripts/build-cache.py'), 'pr-1',
                   'python3', '-c', f'import pathlib; pathlib.Path({str(marker)!r}).touch()']
        for name in ('PLEXI_BUILD_MIN_FREE_BYTES', 'PLEXI_BUILD_CACHE_TOTAL_MAX_BYTES'):
            for value in ('abc', '0', '-5'):
                result = subprocess.run(command, cwd=self.repo, capture_output=True, text=True,
                                        env={**self.env, name: value})
                self.assertNotEqual(result.returncode, 0, f'{name}={value}')
                self.assertIn(name, result.stderr, f'{name}={value}')
                self.assertFalse(marker.exists(), f'{name}={value} must not start the build')

    def test_sizing_tolerates_a_file_another_build_removes(self):
        from unittest import mock
        slot = self._slot('ns-a', 0, 1000, 100)
        (slot / 'target' / 'debug' / 'gone.bin').write_bytes(b'y' * 500)
        real = os.lstat
        def vanishing(path, *args, **kwargs):
            if str(path).endswith('gone.bin'):
                raise FileNotFoundError(path)
            return real(path, *args, **kwargs)
        with mock.patch.object(CACHE.os, 'lstat', side_effect=vanishing):
            self.assertEqual(CACHE.tree_bytes(slot / 'target'), 1000 + 10 + 2)

    def test_floor_check_failure_is_named(self):
        from unittest import mock
        with mock.patch.object(CACHE.shutil, 'disk_usage', side_effect=OSError(5, 'Input/output error')):
            with self.assertRaises(CACHE.DiskFloorError) as raised:
                CACHE.require_free_space(self.root, floor=1)
        self.assertIn('floor check failed', str(raised.exception))

    def test_lease_records_queue_execution_and_failure(self):
        env = {**self.env, 'PLEXI_CARGO_LOCK': str(self.root / 'lease'),
               'PLEXI_BUILD_TELEMETRY': str(self.root / 'telemetry')}
        env.pop('PLEXI_CARGO_LOCK_HELD', None)
        self.call('bash', str(ROOT / 'scripts/cargo-with-lease.sh'), 'true', env=env)
        events = [json.loads(line) for line in (self.root / 'telemetry').read_text().splitlines()]
        self.assertEqual([event['event'] for event in events], ['request', 'acquire', 'finish'])
        self.assertEqual(events[-1]['result'], 0)
        self.call('bash', str(ROOT / 'scripts/cargo-with-lease.sh'), 'true',
                  env={**env, 'PLEXI_BUILD_TELEMETRY': str(self.root / 'missing/telemetry')})
        self.assertGreaterEqual(events[-1]['execution_seconds'], 0)
        with self.assertRaises(subprocess.CalledProcessError):
            self.call('bash', str(ROOT / 'scripts/cargo-with-lease.sh'), 'false', env=env)
        self.assertEqual(json.loads((self.root / 'telemetry').read_text().splitlines()[-1])['result'], 1)

    # --- Permit queueing (stint 1250) -------------------------------------
    # Permits 0 and 1 are general; permit 2 (`<lock>.short`) is short-only.
    # Full jobs may use both general permits (PLEXI_CARGO_FULL_PERMITS=2, the default).

    LEASE = str(ROOT / 'scripts/cargo-with-lease.sh')
    # A short job must start within this many seconds while a full job holds
    # the lease. Generous for loaded CI, still far below the 900 s default queue.
    SHORT_START_BOUND_SECS = 5.0

    def lease_env(self, **extra):
        env = {**self.env, 'PLEXI_CARGO_LOCK': str(self.root / 'permits'),
               'PLEXI_BUILD_TELEMETRY': str(self.root / 'permit-telemetry'),
               'PLEXI_CARGO_LEASE_REPORT_SECS': '1', **extra}
        env.pop('PLEXI_CARGO_LOCK_HELD', None)
        return env

    def start_holder(self, cls, seconds, env):
        """Start a wrapped `sleep` and wait until it owns a permit."""
        args = ['bash', self.LEASE] + (['--class', cls] if cls else []) + ['sleep', str(seconds)]
        proc = subprocess.Popen(args, env=env, cwd=self.repo, stderr=subprocess.DEVNULL,
                                start_new_session=True)
        self.addCleanup(self.reap, proc)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if self.holder_record():
                return proc
            time.sleep(0.05)
        self.fail('holder never acquired a permit')

    def holder_record(self):
        for path in self.root.glob('permits*.holder'):
            try:
                return json.loads(path.read_text())
            except (ValueError, OSError):
                continue
        return None

    def reap(self, proc):
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass  # group already gone (PermissionError: macOS, reaped leader)
        proc.wait()

    def run_lease(self, *args, env, timeout=30):
        return subprocess.run(['bash', self.LEASE, *args], env=env, cwd=self.repo, capture_output=True,
                              text=True, timeout=timeout)

    def test_lease_caps_build_jobs_unless_caller_sets_them(self):
        env = self.lease_env()
        env.pop('CARGO_BUILD_JOBS', None)
        expected = str(max(1, (os.cpu_count() or 1) // 3))
        result = self.run_lease('sh', '-c', 'printf %s "$CARGO_BUILD_JOBS"', env=env)
        self.assertEqual(result.stdout, expected, result.stderr)
        result = self.run_lease('sh', '-c', 'printf %s "$CARGO_BUILD_JOBS"', env={**env, 'CARGO_BUILD_JOBS': '7'})
        self.assertEqual(result.stdout, '7', result.stderr)

    def test_short_job_starts_while_full_job_holds_the_lease(self):
        env = self.lease_env()
        self.start_holder(None, 30, env)
        started = time.monotonic()
        result = self.run_lease('--class', 'short', 'true', env=env)
        elapsed = time.monotonic() - started
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(elapsed, self.SHORT_START_BOUND_SECS)
        events = [json.loads(l) for l in (self.root / 'permit-telemetry').read_text().splitlines()]
        short_acquire = [e for e in events if e['event'] == 'acquire' and e['class'] == 'short'][0]
        self.assertEqual(short_acquire['permit'], 2)

    def test_full_jobs_serialize_on_permit_zero_when_confined_and_waiter_names_the_holder(self):
        env = self.lease_env(PLEXI_CARGO_FULL_PERMITS='1')
        self.start_holder('full', 3, env)
        started = time.monotonic()
        result = self.run_lease('true', env=env)  # unmarked call defaults to full
        waited = time.monotonic() - started
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertGreater(waited, 1.5, 'second full job must queue behind the first')
        self.assertIn('waiting', result.stderr)
        self.assertIn('permit 0 held by pid', result.stderr)
        self.assertNotIn('permit 1', result.stderr)
        self.assertIn('class=full', result.stderr)
        self.assertIn('sleep 3', result.stderr)
        self.assertRegex(result.stderr, r'for \d+s')
        events = [json.loads(l) for l in (self.root / 'permit-telemetry').read_text().splitlines()]
        second = [e for e in events if e['event'] == 'acquire' and e['command'] == ['true']][0]
        self.assertEqual((second['class'], second['permit']), ('full', 0))
        self.assertGreater(second['queue_seconds'], 1.5)

    def test_two_full_jobs_run_together_and_a_third_queues(self):
        env = self.lease_env()
        self.start_holder('full', 30, env)
        second = subprocess.Popen(['bash', self.LEASE, 'sleep', '30'], env=env, cwd=self.repo,
                                  stderr=subprocess.DEVNULL, start_new_session=True)
        self.addCleanup(self.reap, second)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and len(list(self.root.glob('permits*.holder'))) < 2:
            time.sleep(0.05)
        self.assertEqual(len(list(self.root.glob('permits*.holder'))), 2, 'both permits held by full jobs')
        env_short_timeout = {**env, 'PLEXI_CARGO_LEASE_TIMEOUT_SECS': '1'}
        third = self.run_lease('true', env=env_short_timeout)
        self.assertEqual(third.returncode, 1)
        self.assertIn('permit 0 held by pid', third.stderr)
        self.assertIn('permit 1 held by pid', third.stderr)

    def test_short_job_starts_while_two_full_jobs_hold_both_general_permits(self):
        env = self.lease_env()
        self.start_holder('full', 30, env)
        second = subprocess.Popen(['bash', self.LEASE, 'sleep', '30'], env=env, cwd=self.repo,
                                  stderr=subprocess.DEVNULL, start_new_session=True)
        self.addCleanup(self.reap, second)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and len(list(self.root.glob('permits*.holder'))) < 2:
            time.sleep(0.05)
        self.assertEqual(len(list(self.root.glob('permits*.holder'))), 2, 'both general permits held by full jobs')
        started = time.monotonic()
        result = self.run_lease('--class', 'short', 'true', env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(time.monotonic() - started, self.SHORT_START_BOUND_SECS)
        events = [json.loads(l) for l in (self.root / 'permit-telemetry').read_text().splitlines()]
        acquire = [e for e in events if e['event'] == 'acquire' and e['class'] == 'short'][0]
        self.assertEqual(acquire['permit'], 2, 'short-only permit')

    def test_short_job_falls_back_to_a_free_general_permit(self):
        env = self.lease_env()
        self.start_holder('short', 30, env)  # takes the short-only permit
        started = time.monotonic()
        result = self.run_lease('--class', 'short', 'true', env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(time.monotonic() - started, self.SHORT_START_BOUND_SECS)
        events = [json.loads(l) for l in (self.root / 'permit-telemetry').read_text().splitlines()]
        acquires = [e for e in events if e['event'] == 'acquire' and e['command'] == ['true']]
        self.assertIn(acquires[0]['permit'], (0, 1))

    def test_full_job_never_takes_the_short_only_permit(self):
        env = self.lease_env(PLEXI_CARGO_FULL_PERMITS='1', PLEXI_CARGO_LEASE_TIMEOUT_SECS='1')
        self.start_holder('full', 30, env)
        result = self.run_lease('true', env=env)
        self.assertEqual(result.returncode, 1, 'short-only permit is free but a full job must not use it')

    def test_killed_holder_frees_its_permit(self):
        env = self.lease_env(PLEXI_CARGO_FULL_PERMITS='1')  # only permit 0 usable: the freed permit is what lets it in
        holder = self.start_holder('full', 60, env)
        record = self.holder_record()
        os.kill(record['pid'], signal.SIGKILL)  # no trap runs; only the kernel releases the flock
        started = time.monotonic()
        result = self.run_lease('true', env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(time.monotonic() - started, self.SHORT_START_BOUND_SECS)
        self.assertNotIn('held by', result.stderr, 'stale record must not be reported as a live holder')

    def test_timeout_names_the_holder(self):
        env = self.lease_env(PLEXI_CARGO_LEASE_TIMEOUT_SECS='1', PLEXI_CARGO_FULL_PERMITS='1')
        holder = self.start_holder('full', 30, env)
        record = self.holder_record()
        result = self.run_lease('true', env=env)
        self.assertEqual(result.returncode, 1)
        self.assertIn('timed out', result.stderr)
        self.assertIn(f"pid {record['pid']}", result.stderr)
        self.assertIn('sleep 30', result.stderr)
        events = [json.loads(l) for l in (self.root / 'permit-telemetry').read_text().splitlines()]
        self.assertEqual(events[-1]['event'], 'timeout')

    def test_nesting_guard_strips_class_and_runs_directly(self):
        env = self.lease_env()
        result = self.run_lease('bash', self.LEASE, '--class', 'short', 'echo', 'nested-ok', env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), 'nested-ok')
        events = [json.loads(l) for l in (self.root / 'permit-telemetry').read_text().splitlines()]
        self.assertEqual([e['event'] for e in events], ['request', 'acquire', 'finish'])

    def test_unknown_class_is_rejected(self):
        result = self.run_lease('--class', 'medium', 'true', env=self.lease_env())
        self.assertEqual(result.returncode, 2)
        self.assertIn('--class', result.stderr)

    def test_actual_build_info_reports_compiled_channel_and_cargo_invalidates(self):
        source = (ROOT / 'src/distribution.rs').read_text()
        function = source[source.index('pub fn build_info()'):].split('\n}', 1)[0] + '\n}\n'
        (self.repo / 'Cargo.toml').write_text('[package]\nname="identity_probe"\nversion="0.1.0"\nedition="2021"\n[dependencies]\nserde_json="1"\n')
        (self.repo / 'src').mkdir()
        (self.repo / 'src/main.rs').write_text(function +
            'fn version_text() -> &' + "'static str" + ' { "fixture" }\n'
            'fn build_tag() -> String { "fixture".into() }\n'
            'fn main() { println!("{}", build_info()); }\n')
        env = {**self.env, 'CARGO_TARGET_DIR': str(self.root / 'target'),
               'RUSTC_WRAPPER': '', 'PLEXI_CARGO_LOCK': str(self.root / 'fixture-lease'),
               'CARGO_BUILD_JOBS': '1', 'PLEXI_SOURCE_COMMIT': 'fixture', 'PLEXI_BUILD_ID': 'fixture'}
        for channel in ('pr-1', 'pr-2', None):
            if channel:
                env['PLEXI_BUILD_TEST_CHANNEL'] = channel
            else:
                env.pop('PLEXI_BUILD_TEST_CHANNEL', None)
            self.call('bash', str(ROOT / 'scripts/cargo-with-lease.sh'), 'cargo', 'build', '--offline', env=env)
            info = json.loads(self.call(str(self.root / 'target/debug/identity_probe'), env=env).stdout)
            self.assertEqual(info['test_channel'], channel)

    def test_real_cargo_warm_path_move_and_seed(self):
        (self.repo / 'Cargo.toml').write_text('[package]\nname="cache_probe"\nversion="0.1.0"\nedition="2021"\n')
        (self.repo / 'src').mkdir()
        (self.repo / 'src/main.rs').write_text('fn main() { println!("probe"); }\n')
        target = self.root / 'target'
        env = {**self.env, 'CARGO_TARGET_DIR': str(target), 'RUSTC_WRAPPER': '', 'CARGO_BUILD_JOBS': '1', 'PLEXI_CARGO_LOCK': str(self.root / 'fixture-lease')}
        timings = []
        def build(repo, env):
            started = time.monotonic()
            result = self.call('bash', str(ROOT / 'scripts/cargo-with-lease.sh'), 'cargo', 'build', '--offline', '--message-format=json', cwd=repo, env=env)
            timings.append(time.monotonic() - started)
            return [json.loads(line)['fresh'] for line in result.stdout.splitlines()
                    if json.loads(line).get('reason') == 'compiler-artifact']
        self.assertEqual(build(self.repo, env), [False])
        self.assertEqual(build(self.repo, env), [True])
        other = self.root / 'moved-source'
        shutil.copytree(self.repo, other)
        moved = build(other, env)
        seeded = self.root / 'seeded'
        seed_started = time.monotonic()
        shutil.copytree(target, seeded)
        seed_seconds = time.monotonic() - seed_started
        seeded_result = build(other, {**env, 'CARGO_TARGET_DIR': str(seeded)})
        print(json.dumps({'fixture': 'small real Cargo', 'warm': True, 'moved_fresh': moved, 'seeded_fresh': seeded_result, 'build_seconds_cold_warm_moved_seeded': timings, 'seed_copy_seconds': seed_seconds}))

if __name__ == '__main__':
    unittest.main()
