"""Native installer cache and isolated authoritative cleanup checks."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]

class Cleanup(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.native = Path(subprocess.check_output(['bash', str(ROOT / 'scripts/distribution-tool.sh'), 'prepare'], text=True).strip())

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.share = self.root / 'share'
        self.share.mkdir()
        self.env = {**os.environ, 'HOME': str(self.root), 'PLEXI_DISTRIBUTION_HOME': str(self.share),
                    'LC_ALL': 'en_US.UTF-8', 'LANG': 'en_US.UTF-8'}
        self.scripts = self.root / 'scripts'
        self.scripts.mkdir()
        for name in ['uninstall.sh', 'channel-clean.sh']:
            shutil.copy(ROOT / 'scripts' / name, self.scripts)
        # Real native locate/remove, no Cargo and no actual user HOME/keychain.
        (self.scripts / 'distribution-tool.sh').write_text(f'exec "{self.native}" "$@"\n')
        bin_dir = self.root / 'test-bin'
        bin_dir.mkdir()
        (bin_dir / 'security').write_text('#!/bin/sh\nexit 1\n')
        (bin_dir / 'security').chmod(0o755)
        self.env['PATH'] = f'{bin_dir}:' + self.env['PATH']
        self.normal = self.root / '.plexi-alpha/keep'
        self.normal.parent.mkdir()
        self.normal.write_text('preserve')

    def call(self, *args, env=None):
        return subprocess.run(args, env=env or self.env, capture_output=True, text=True)

    def clean(self):
        return self.call('bash', str(self.scripts / 'channel-clean.sh'), 'pr-901')

    def fixture(self, channel='pr-901'):
        root = self.share / channel
        generation = root / 'generations/fixture'
        generation.mkdir(parents=True)
        executable = generation / 'plexi'
        executable.write_text(f'#!/bin/sh\nexec "{self.native}" remove --receipt "{root}"\n')
        executable.chmod(0o755)
        integration = self.root / 'owned-launcher'
        integration.write_text('owned')
        receipt = {'schema': 1, 'channel': channel, 'root': str(root), 'bin_dir': str(self.root / 'bin'),
                   'applications_dir': str(self.root / 'apps'), 'active': {'path': str(generation),
                   'build_id': 'fixture', 'tag': 'fixture', 'executable': 'plexi'}, 'previous': None,
                   'integrations': [{'path': str(integration), 'sha256': hashlib.sha256(b'owned').hexdigest()}]}
        (root / 'installation.json').write_text(json.dumps(receipt))
        registry = self.share / 'installations'
        registry.mkdir(exist_ok=True)
        (registry / 'pr-901.json').write_text(json.dumps(str(root)))
        return root, integration

    def test_absent_is_idempotent_and_preserves_unowned_bundle(self):
        stray = self.root / 'Applications/Plexi PR901.app'
        stray.mkdir(parents=True)
        for _ in range(2):
            result = self.clean()
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(stray.is_dir())
        self.assertEqual(self.normal.read_text(), 'preserve')

    def test_managed_removal_retry_and_changed_integration(self):
        root, integration = self.fixture()
        integration.write_text('changed')
        result = self.clean()
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue((root / 'installation.json').exists())
        integration.write_text('owned')
        result = self.clean()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(integration.exists())
        self.assertEqual(self.clean().returncode, 0)
        self.assertTrue(self.normal.exists())

    def test_corrupt_and_cross_channel_receipts_preserve_state(self):
        root, integration = self.fixture('alpha')
        result = self.clean()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('another channel', result.stderr)
        self.assertTrue(integration.exists())
        (root / 'installation.json').write_text('{broken')
        self.assertNotEqual(self.clean().returncode, 0)
        self.assertTrue(integration.exists())
        self.assertTrue(self.normal.exists())

    def test_tampered_native_cache_requires_leased_fallback(self):
        cache = self.root / 'cache'
        private = cache / self.native.parents[1].name / self.native.parent.name
        private.mkdir(parents=True)
        shutil.copy2(self.native, private / 'plexi-installer')
        shutil.copy2(self.native.parent / 'evidence.json', private / 'evidence.json')
        lock_path = self.root / 'cargo.lock'
        env = {**os.environ, 'PLEXI_DISTRIBUTION_CACHE_ROOT': str(cache),
               'PLEXI_CARGO_LOCK': str(lock_path), 'PLEXI_CARGO_LEASE_TIMEOUT_SECS': '0.01'}
        command = ['bash', str(ROOT / 'scripts/distribution-tool.sh'), 'prepare']
        with lock_path.open('w') as lock, Path(f'{lock_path}.1').open('w') as reserved:
            fcntl.flock(lock, fcntl.LOCK_EX)  # a busy lease holds both permits
            fcntl.flock(reserved, fcntl.LOCK_EX)
            self.assertEqual(self.call(*command, env=env).returncode, 0)
            with (private / 'plexi-installer').open('ab') as stream:
                stream.write(b'tampered')
            result = self.call(*command, env=env)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('cache miss', result.stderr)
            self.assertIn('timed out', result.stderr)
            shutil.copy2(self.native, private / 'plexi-installer')
            (private / 'evidence.json').write_text('{broken')
            self.assertNotEqual(self.call(*command, env=env).returncode, 0)

    def test_warm_authoritative_lookup_ignores_busy_cargo_lease(self):
        lock_path = self.root / 'cargo.lock'
        env = {**os.environ, 'PLEXI_DISTRIBUTION_HOME': str(self.share),
               'PLEXI_CARGO_LOCK': str(lock_path), 'PLEXI_CARGO_LEASE_TIMEOUT_SECS': '0.1'}
        with lock_path.open('w') as lock, Path(f'{lock_path}.1').open('w') as reserved:
            fcntl.flock(lock, fcntl.LOCK_EX)  # a busy lease holds both permits
            fcntl.flock(reserved, fcntl.LOCK_EX)
            started = time.monotonic()
            result = self.call('bash', str(ROOT / 'scripts/distribution-tool.sh'), 'locate', '--channel', 'pr-901', env=env)
            elapsed = time.monotonic() - started
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertNotIn('cargo-lease:', result.stderr)
        print(json.dumps({'boundary': 'real cached native locate, absent isolated channel', 'busy_lease': True, 'seconds': elapsed}))

if __name__ == '__main__':
    unittest.main()
